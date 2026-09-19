use crate::app_error::{AppError, Result};
use crate::config::{GpuSetMethod, GpuTempRead, GpuUsageMethod};
use cyan_skillfish_governor_smu::Bc250Smu;
use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use log::debug;
use log::info;
use log::warn;

use std::{
    collections::BTreeMap,
    fs::File,
    io::Error as IoError,
    path::{Path, PathBuf},
    time::Duration,
};

#[path = "gpu/kernel_freq_strategy.rs"]
mod kernel_freq_strategy;
use kernel_freq_strategy::KernelFreqStrategy;
#[path = "gpu/kernel_usage_strategy.rs"]
mod kernel_usage_strategy;
use kernel_usage_strategy::KernelUsageStrategy;
#[path = "gpu/process_usage_strategy.rs"]
mod process_usage_strategy;
use process_usage_strategy::ProcessUsageStrategy;

trait FreqStrategy: Send {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<()>;
    fn get_freq(&self) -> Result<u32>;
    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
    fn clamp_safe_points(&self, safe_points: BTreeMap<u32, u32>) -> Result<BTreeMap<u32, u32>> {
        Ok(safe_points)
    }
}

trait UsageStrategy: Send {
    fn poll_and_get_load(&mut self) -> Result<(f32, u32)>;
}

/// Where `read_temperature` gets its value from, chosen by
/// `gpu-usage.temp-read`. `Drm` keeps a device handle open for the life of the
/// process; `Sysfs` holds only a path and opens the file per read.
///
/// `Sysfs` also carries the last good reading and a consecutive-failure count,
/// so a transient read error can be ridden out instead of permanently giving
/// up on the source. See `read_temperature`.
enum TempSource {
    Drm(DeviceHandle),
    Sysfs {
        path: PathBuf,
        last_good: i64,
        failures: u32,
    },
}

/// How many reads in a row have to fail before the sysfs source is abandoned.
///
/// One failure means nothing: the value only drives thermal throttling, and a
/// reading a few hundred milliseconds stale serves that just as well. Demoting
/// is the expensive outcome, not the error -- it reopens the DRM render node
/// for the rest of the run, which is the thing `temp-read = "sysfs"` exists to
/// avoid.
const TEMP_SYSFS_MAX_CONSECUTIVE_FAILURES: u32 = 5;

pub struct GPU {
    temp_source: TempSource,
    pub min_freq: u32,
    pub max_freq: u32,
    freq_strategy: Box<dyn FreqStrategy + Send>,
    usage_strategy: Box<dyn UsageStrategy + Send>,
    safe_points: BTreeMap<u32, u32>,
    location: BUS_INFO,
}

impl GPU {
    pub fn new(
        safe_points: BTreeMap<u32, u32>,
        gpu_set_method: GpuSetMethod,
        gpu_usage_method: GpuUsageMethod,
        gpu_temp_read: GpuTempRead,
        sampling_interval: Duration,
    ) -> Result<GPU> {
        let location = BUS_INFO {
            domain: 0,
            bus: 1,
            dev: 0,
            func: 0,
        };
        validate_device_identity(&location)?;
        info!(
            "GPU usage method: {} set method: {} temperature read: {}",
            gpu_usage_method.as_config_value(),
            gpu_set_method.as_config_value(),
            gpu_temp_read.as_config_value()
        );

        let freq_strategy: Box<dyn FreqStrategy + Send> = match gpu_set_method {
            GpuSetMethod::Smu => Box::new(SmuFreqStrategy::new()?),
            GpuSetMethod::Kernel => {
                Box::new(KernelFreqStrategy::new(location.get_drm_render_path()?)?)
            }
        };

        let safe_points = freq_strategy.clamp_safe_points(safe_points)?;

        let usage_strategy: Box<dyn UsageStrategy + Send> = match gpu_usage_method {
            GpuUsageMethod::BusyFlag => Box::new(BusyFlagUsageStrategy::new(
                location.get_drm_render_path()?,
                sampling_interval,
            )?),
            GpuUsageMethod::Process => Box::new(ProcessUsageStrategy {
                render_node_path: location.get_drm_render_path()?,
                prev_gfx_time: None,
                prev_time: None,
            }),
            GpuUsageMethod::Kernel => {
                Box::new(KernelUsageStrategy::new(location.get_drm_render_path()?)?)
            }
        };

        Ok(GPU {
            temp_source: init_temp_source(gpu_temp_read, &location)?,
            min_freq: *safe_points
                .first_key_value()
                .ok_or_else(|| IoError::other("safe_points cannot be empty"))?
                .0,
            max_freq: *safe_points
                .last_key_value()
                .ok_or_else(|| IoError::other("safe_points cannot be empty"))?
                .0,
            freq_strategy,
            usage_strategy,
            safe_points,
            location,
        })
    }

    pub fn get_sysfs_path(&self) -> PathBuf {
        self.location.get_sysfs_path()
    }

    pub fn poll_and_get_load(&mut self) -> Result<(f32, u32)> {
        self.usage_strategy.poll_and_get_load()
    }

    pub fn read_temperature(&mut self) -> Result<u32> {
        // A sysfs read that starts failing mid-run degrades rather than
        // killing the control loop: the temperature only drives thermal
        // throttling, and losing it would stop the governor entirely.
        //
        // A single failure is ridden out with the last good reading. Only
        // TEMP_SYSFS_MAX_CONSECUTIVE_FAILURES in a row demote the source,
        // because demoting is itself disruptive -- it opens the DRM render
        // node for the rest of the run.
        let mut demote = false;

        if let TempSource::Sysfs {
            path,
            last_good,
            failures,
        } = &mut self.temp_source
        {
            match read_hwmon_millidegrees(path) {
                Ok(millidegrees) => {
                    *last_good = millidegrees;
                    *failures = 0;
                    return Ok(millidegrees_to_celsius(millidegrees));
                }
                Err(e) => {
                    *failures += 1;
                    if *failures < TEMP_SYSFS_MAX_CONSECUTIVE_FAILURES {
                        warn!(
                            "gpu-usage.temp-read = \"sysfs\": {e}; reusing the last good reading ({} failure(s) in a row, demoting at {TEMP_SYSFS_MAX_CONSECUTIVE_FAILURES})",
                            *failures
                        );
                        return Ok(millidegrees_to_celsius(*last_good));
                    }
                    warn!(
                        "gpu-usage.temp-read = \"sysfs\": {e}; {TEMP_SYSFS_MAX_CONSECUTIVE_FAILURES} reads in a row failed, falling back to the DRM ioctl for the rest of this run"
                    );
                    demote = true;
                }
            }
        }

        if demote {
            self.temp_source =
                TempSource::Drm(init_device_handle(self.location.get_drm_render_path()?)?);
        }

        match &self.temp_source {
            TempSource::Drm(dev_handle) => {
                let temp = dev_handle
                    .sensor_info(libdrm_amdgpu_sys::AMDGPU::SENSOR_INFO::SENSOR_TYPE::GPU_TEMP)
                    .map_err(IoError::from_raw_os_error)?;
                Ok(millidegrees_to_celsius(i64::from(temp)))
            }
            // Demoted to Drm just above, so this cannot be reached.
            TempSource::Sysfs { path, .. } => Err(IoError::other(format!(
                "temperature source {} unexpectedly still sysfs",
                path.display()
            ))
            .into()),
        }
    }

    pub fn change_freq(&mut self, freq: u32) -> Result<()> {
        let vol = voltage_for_freq(&self.safe_points, freq)?;
        self.freq_strategy.change_freq(freq, vol)
    }

    pub fn change_freq_vol(&mut self, freq: u32, vol: u32) -> Result<()> {
        self.freq_strategy.change_freq(freq, vol)
    }

    pub fn get_freq(&self) -> Result<u32> {
        self.freq_strategy.get_freq()
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.freq_strategy.shutdown()
    }
}

fn voltage_for_freq(safe_points: &BTreeMap<u32, u32>, freq: u32) -> Result<u32> {
    let mut prev_point: Option<(u32, u32)> = None;

    for (&next_freq, &next_vol) in safe_points {
        if next_freq == freq {
            return Ok(next_vol);
        }

        if next_freq > freq {
            let vol = match prev_point {
                Some((prev_freq, prev_vol)) => {
                    let freq_span = next_freq - prev_freq;
                    let freq_offset = freq - prev_freq;
                    let vol_delta = next_vol - prev_vol;
                    prev_vol + (vol_delta * freq_offset) / freq_span
                }
                None => {
                    return Err(
                        IoError::other("tried to set a frequency below min safe point").into(),
                    );
                }
            };

            return Ok(vol);
        }

        prev_point = Some((next_freq, next_vol));
    }

    Err(IoError::other("tried to set a frequency beyond max safe point").into())
}

const EXPECTED_VENDOR_ID: &str = "0x1002";
const EXPECTED_DEVICE_ID: &str = "0x13fe";
fn validate_device_identity(location: &BUS_INFO) -> Result<()> {
    let sysfs_path = location.get_sysfs_path();
    let vendor = std::fs::read_to_string(sysfs_path.join("vendor"))?;
    let device = std::fs::read_to_string(sysfs_path.join("device"))?;

    if vendor.trim() == EXPECTED_VENDOR_ID && device.trim() == EXPECTED_DEVICE_ID {
        return Ok(());
    }

    Err(AppError::from(
        "Cyan Skillfish GPU not found at expected PCI bus location",
    ))
}

/// Plausible range for an edge temperature, in millidegrees C. Anything
/// outside it is treated as an unusable reading rather than passed on -- a
/// bogus value here would feed straight into thermal throttling.
const TEMP_MIN_MILLIDEGREES: i64 = -40_000;
const TEMP_MAX_MILLIDEGREES: i64 = 150_000;

fn millidegrees_to_celsius(millidegrees: i64) -> u32 {
    (millidegrees / 1000).max(0) as u32
}

/// Read one temperature from an amdgpu hwmon `temp1_input`, rejecting anything
/// that is missing, unreadable, not an integer, or out of plausible range.
fn read_hwmon_millidegrees(path: &Path) -> Result<i64> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| IoError::other(format!("{}: {e}", path.display())))?;
    let millidegrees: i64 = raw.trim().parse().map_err(|_| {
        IoError::other(format!(
            "{} did not contain an integer: {:?}",
            path.display(),
            raw.trim()
        ))
    })?;

    if !(TEMP_MIN_MILLIDEGREES..=TEMP_MAX_MILLIDEGREES).contains(&millidegrees) {
        return Err(IoError::other(format!(
            "{} reported an implausible temperature: {} millidegrees C",
            path.display(),
            millidegrees
        ))
        .into());
    }

    Ok(millidegrees)
}

/// Pick the temperature source, degrading to the DRM ioctl if sysfs is not
/// usable.
///
/// The hwmon attribute is plain upstream amdgpu -- `temp1_input` is registered
/// for every ASIC except multi-AID parts, and Cyan Skillfish already implements
/// `AMDGPU_PP_SENSOR_EDGE_TEMP` in a stock kernel -- so this normally succeeds.
/// It is probed once anyway rather than trusted, so a kernel that does not
/// expose it costs a warning instead of a governor that will not start.
fn init_temp_source(gpu_temp_read: GpuTempRead, location: &BUS_INFO) -> Result<TempSource> {
    if let GpuTempRead::Sysfs = gpu_temp_read {
        // The probe read is kept, not discarded: it seeds `last_good`, so a
        // failure on the very first control cycle already has something
        // sensible to fall back on.
        match find_hwmon_temp_input(&location.get_sysfs_path())
            .and_then(|path| read_hwmon_millidegrees(&path).map(|millidegrees| (path, millidegrees)))
        {
            Ok((path, millidegrees)) => {
                info!("reading GPU temperature from {}", path.display());
                return Ok(TempSource::Sysfs {
                    path,
                    last_good: millidegrees,
                    failures: 0,
                });
            }
            Err(e) => warn!(
                "gpu-usage.temp-read = \"sysfs\" is not usable: {e}; using the DRM ioctl instead"
            ),
        }
    }

    Ok(TempSource::Drm(init_device_handle(
        location.get_drm_render_path()?,
    )?))
}

/// Locate the amdgpu hwmon temperature input for this device.
///
/// Resolved once rather than per read: the hwmon index is stable for the life
/// of the bound device, and a readdir on every control loop would be wasteful.
fn find_hwmon_temp_input(sysfs_path: &Path) -> Result<PathBuf> {
    let hwmon_root = sysfs_path.join("hwmon");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&hwmon_root)
        .map_err(|e| IoError::other(format!("{}: {e}", hwmon_root.display())))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("temp1_input"))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();

    candidates.into_iter().next().ok_or_else(|| {
        AppError::from(format!(
            "no hwmon temp1_input under {}",
            hwmon_root.display()
        ))
    })
}

fn init_device_handle(render_path: PathBuf) -> Result<DeviceHandle> {
    let card = File::open(render_path)?;
    let (dev_handle, _, _) = DeviceHandle::init_with_fd(&card)
        .map_err(|e| IoError::other(format!("DeviceHandle::init_with_fd failed: {e}")))?;
    Ok(dev_handle)
}
struct SmuFreqStrategy {
    smu: Bc250Smu,
}

impl SmuFreqStrategy {
    fn new() -> Result<Self> {
        let smu = Bc250Smu::new("0000:00:00.0", true, true, 500)?;
        smu.check_test_message()?;
        info!("SMU communication verified");
        smu.set_gpu_max_temperature(80)?;
        smu.unforce_gfx_freq()?;
        smu.unforce_gfx_vid()?;
        Ok(Self { smu })
    }
}

impl FreqStrategy for SmuFreqStrategy {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<()> {
        self.smu.force_gfx_vid(vol)?;
        self.smu.force_gfx_freq(freq)?;
        debug!("SMU set frequency to {} MHz with voltage {} mV", freq, vol);
        Ok(())
    }

    fn get_freq(&self) -> Result<u32> {
        Ok(self.smu.get_gfx_frequency()?)
    }

    fn shutdown(&mut self) -> Result<()> {
        let _ = self.smu.unforce_gfx_freq();
        let _ = self.smu.unforce_gfx_vid();
        Ok(())
    }
}

struct BusyFlagUsageStrategy {
    dev_handle: DeviceHandle,
    samples: u64,
    sampling_interval: Duration,
}

impl BusyFlagUsageStrategy {
    fn new(render_path: PathBuf, sampling_interval: Duration) -> Result<Self> {
        Ok(Self {
            dev_handle: init_device_handle(render_path)?,
            samples: 0,
            sampling_interval,
        })
    }
}
// cyan_skillfish.gfx1013.mmGRBM_STATUS
const GRBM_STATUS_REG: u32 = 0x2004;
// cyan_skillfish.gfx1013.mmGRBM_STATUS.GUI_ACTIVE
const GPU_ACTIVE_BIT: u8 = 31;
impl UsageStrategy for BusyFlagUsageStrategy {
    fn poll_and_get_load(&mut self) -> Result<(f32, u32)> {
        for _ in 0..65 {
            let res = self
                .dev_handle
                .read_mm_registers(GRBM_STATUS_REG)
                .map_err(IoError::from_raw_os_error)?;
            let gpu_busy = (res & (1 << GPU_ACTIVE_BIT)) > 0;

            self.samples <<= 1;
            if gpu_busy {
                self.samples |= 1;
            }
            std::thread::sleep(self.sampling_interval);
        }

        let average_load = (self.samples.count_ones() as f32) / 64.0;
        let burst_length = (!self.samples).trailing_zeros();
        Ok((average_load, burst_length))
    }
}

#[cfg(test)]
mod tests {
    use super::{TEMP_MAX_MILLIDEGREES, read_hwmon_millidegrees, voltage_for_freq};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// Write `contents` to a uniquely named file and hand back its path.
    ///
    /// The crate has no dev-dependencies and this is the only test that needs a
    /// file on disk, so a counter plus the pid is cheaper than pulling in a
    /// temporary-file crate.
    fn temp_file_with(contents: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "cs-governor-hwmon-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).expect("failed to write test file");
        path
    }

    #[test]
    fn read_hwmon_accepts_a_plausible_reading() {
        let path = temp_file_with("45000\n");

        assert_eq!(read_hwmon_millidegrees(&path).unwrap(), 45000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_hwmon_rejects_an_implausible_reading() {
        let path = temp_file_with(&format!("{}\n", TEMP_MAX_MILLIDEGREES + 1));

        assert!(read_hwmon_millidegrees(&path).is_err());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_hwmon_rejects_a_non_integer_reading() {
        let path = temp_file_with("not a number\n");

        assert!(read_hwmon_millidegrees(&path).is_err());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_hwmon_rejects_a_missing_file() {
        let path = std::env::temp_dir().join("cs-governor-hwmon-test-does-not-exist");
        let _ = std::fs::remove_file(&path);

        assert!(read_hwmon_millidegrees(&path).is_err());
    }

    #[test]
    fn voltage_for_freq_interpolates_between_safe_points() {
        let safe_points = BTreeMap::from([(800, 700), (950, 850), (1000, 900)]);

        assert_eq!(voltage_for_freq(&safe_points, 900).unwrap(), 800);
    }
    #[test]
    fn voltage_for_freq_use_safe_points() {
        let safe_points = BTreeMap::from([(800, 700), (950, 850), (1000, 900)]);

        assert_eq!(voltage_for_freq(&safe_points, 950).unwrap(), 850);
    }

    #[test]
    fn voltage_for_freq_rejects_frequency_below_min_safe_point() {
        let safe_points = BTreeMap::from([(800, 700), (950, 850), (1000, 900)]);

        assert!(voltage_for_freq(&safe_points, 750).is_err());
    }

    #[test]
    fn voltage_for_freq_rejects_frequency_above_max_safe_point() {
        let safe_points = BTreeMap::from([(800, 700), (1000, 900)]);

        assert!(voltage_for_freq(&safe_points, 1100).is_err());
    }
}
