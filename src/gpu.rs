use crate::config::{GpuSetMethod, GpuUsageMethod};
use cyan_skillfish_governor_smu::Bc250Smu;
use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::{Error as IoError, Write},
    os::fd::AsRawFd,
    path::PathBuf,
    time::{Duration, Instant},
};

// cyan_skillfish.gfx1013.mmGRBM_STATUS
const GRBM_STATUS_REG: u32 = 0x2004;
// cyan_skillfish.gfx1013.mmGRBM_STATUS.GUI_ACTIVE
const GPU_ACTIVE_BIT: u8 = 31;

trait FreqStrategy {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<(), IoError>;
    fn get_freq(&self) -> Result<u32, IoError>;
}

trait UsageStrategy {
    fn poll_and_get_load(
        &mut self,
        dev_handle: &DeviceHandle,
        process_fdinfo_pdev: &str,
        sampling_interval: Duration,
    ) -> Result<(f32, u32), IoError>;
}

struct SmuFreqStrategy {
    smu: Bc250Smu,
}

struct BusyFlagUsageStrategy {
    samples: u64,
}

struct ProcessUsageStrategy {
    prev_gfx_time: Option<u64>,
    prev_time: Option<Instant>,
}

pub struct GPU {
    dev_handle: DeviceHandle,
    process_fdinfo_pdev: String,
    pub min_freq: u32,
    pub max_freq: u32,
    freq_strategy: Box<dyn FreqStrategy>,
    usage_strategy: Box<dyn UsageStrategy>,
    safe_points: BTreeMap<u32, u32>,
}

impl GPU {
    pub fn new(
        safe_points: BTreeMap<u32, u32>,
        gpu_set_method: GpuSetMethod,
        gpu_usage_method: GpuUsageMethod,
    ) -> Result<GPU, Box<dyn std::error::Error>> {
        let location = BUS_INFO {
            domain: 0,
            bus: 1,
            dev: 0,
            func: 0,
        };
        let sysfs_path = location.get_sysfs_path();
        let vendor = std::fs::read_to_string(sysfs_path.join("vendor"))?;
        let device = std::fs::read_to_string(sysfs_path.join("device"))?;
        if !((vendor == "0x1002\n") && (device == "0x13fe\n")) {
            Err(IoError::other(
                "Cyan Skillfish GPU not found at expected PCI bus location",
            ))?;
        }
        let card = File::open(location.get_drm_render_path()?)?;
        let (dev_handle, _, _) =
            DeviceHandle::init(card.as_raw_fd()).map_err(IoError::from_raw_os_error)?;

        let gpu_sysfs_path = dev_handle
            .get_sysfs_path()
            .map_err(IoError::from_raw_os_error)?;

        let process_fdinfo_pdev = format!(
            "{:04x}:{:02x}:{:02x}.{}",
            location.domain, location.bus, location.dev, location.func
        );

        let min_freq = *safe_points.first_key_value().unwrap().0;
        let max_freq = *safe_points.last_key_value().unwrap().0;

        let freq_strategy: Box<dyn FreqStrategy> = match gpu_set_method {
            GpuSetMethod::Smu => Box::new(SmuFreqStrategy::new()?),
            GpuSetMethod::Kernel => Box::new(KernelFreqStrategy::new(gpu_sysfs_path)?),
        };

        let usage_strategy: Box<dyn UsageStrategy> = match gpu_usage_method {
            GpuUsageMethod::BusyFlag => Box::new(BusyFlagUsageStrategy { samples: 0 }),
            GpuUsageMethod::Process => Box::new(ProcessUsageStrategy {
                prev_gfx_time: None,
                prev_time: None,
            }),
        };

        Ok(GPU {
            dev_handle,
            process_fdinfo_pdev,
            min_freq,
            max_freq,
            freq_strategy,
            usage_strategy,
            safe_points,
        })
    }

    pub fn poll_and_get_load(
        &mut self,
        sampling_interval: Duration,
    ) -> Result<(f32, u32), IoError> {
        self.usage_strategy.poll_and_get_load(
            &self.dev_handle,
            &self.process_fdinfo_pdev,
            sampling_interval,
        )
    }

    pub fn read_temperature(&mut self) -> Result<u32, IoError> {
        let temp = self
            .dev_handle
            .sensor_info(libdrm_amdgpu_sys::AMDGPU::SENSOR_INFO::SENSOR_TYPE::GPU_TEMP)
            .map_err(IoError::from_raw_os_error)?;
        Ok((temp / 1000) as u32)
    }

    pub fn change_freq(&mut self, freq: u32) -> Result<(), IoError> {
        let vol = self
            .safe_points
            .range(freq..)
            .next()
            .map(|(_, voltage)| *voltage)
            .ok_or(IoError::other(
                "tried to set a frequency beyond max safe point",
            ))?;

        self.freq_strategy.change_freq(freq, vol)
    }

    pub fn get_freq(&self) -> Result<u32, IoError> {
        self.freq_strategy.get_freq()
    }
}

fn get_total_gfx_time_from_fdinfo(target_pdev: &str) -> u64 {
    let mut gfx_times: HashMap<u32, u64> = HashMap::new();

    let proc_entries = match fs::read_dir("/proc") {
        Ok(v) => v,
        Err(_) => return 0,
    };

    for proc_entry in proc_entries.flatten() {
        let name = proc_entry.file_name();
        let name = name.to_string_lossy();
        if !name.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let fdinfo_dir = proc_entry.path().join("fdinfo");
        let fdinfos = match fs::read_dir(fdinfo_dir) {
            Ok(v) => v,
            Err(_) => continue,
        };

        for fdinfo in fdinfos.flatten() {
            let content = match fs::read_to_string(fdinfo.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            let mut cid: Option<u32> = None;
            let mut pdev: Option<&str> = None;
            let mut engine_total: u64 = 0;

            for line in content.lines() {
                if let Some(v) = line.strip_prefix("drm-client-id:") {
                    cid = v.trim().parse::<u32>().ok();
                    continue;
                }
                if let Some(v) = line.strip_prefix("drm-pdev:") {
                    pdev = Some(v.trim());
                    continue;
                }
                if let Some((_, rest)) = line
                    .strip_prefix("drm-engine-")
                    .and_then(|v| v.split_once(':'))
                {
                    let mut fields = rest.split_whitespace();
                    let Some(value_tok) = fields.next() else {
                        continue;
                    };
                    // Skip non-time drm-engine fields by requiring ns unit.
                    if fields.next() != Some("ns") {
                        continue;
                    }
                    let value = value_tok.parse::<u64>().unwrap_or(0);
                    engine_total = engine_total.saturating_add(value);
                }
            }

            if let Some(pdev) = pdev
                && pdev != target_pdev
            {
                continue;
            }

            if let Some(cid) = cid {
                let e = gfx_times.entry(cid).or_insert(0);
                if engine_total > *e {
                    *e = engine_total;
                }
            }
        }
    }

    gfx_times.values().copied().sum()
}

impl SmuFreqStrategy {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let smu = Bc250Smu::new("0000:00:00.0", true, true, 500)?;
        smu.check_test_message()?;
        println!("SMU communication verified!");
        smu.set_gpu_max_temperature(80)?;
        smu.unforce_gfx_freq()?;
        smu.unforce_gfx_vid()?;
        Ok(Self { smu })
    }
}

impl FreqStrategy for SmuFreqStrategy {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<(), IoError> {
        self.smu.force_gfx_vid(vol)?;
        self.smu.force_gfx_freq(freq)?;
        Ok(())
    }

    fn get_freq(&self) -> Result<u32, IoError> {
        Ok(self.smu.get_gfx_frequency()?)
    }
}

struct KernelFreqStrategy {
    pp_file: File,
    dpm_sclk: PathBuf,
}

impl KernelFreqStrategy {
    fn new(gpu_sysfs_path: PathBuf) -> Result<Self, IoError> {
        let pp_file = std::fs::OpenOptions::new()
            .write(true)
            .open(gpu_sysfs_path.join("pp_od_clk_voltage"))?;
        let dpm_sclk = gpu_sysfs_path.join("pp_dpm_sclk");
        Ok(Self { pp_file, dpm_sclk })
    }
}

impl FreqStrategy for KernelFreqStrategy {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<(), IoError> {
        self.pp_file
            .write_all(format!("vc 0 {freq} {vol}").as_bytes())?;
        self.pp_file.write_all("c".as_bytes())?;
        Ok(())
    }

    fn get_freq(&self) -> Result<u32, IoError> {
        let content = std::fs::read_to_string(&self.dpm_sclk)?;

        let line = content
            .lines()
            .find(|line| line.contains('*'))
            .ok_or(IoError::other("failed to find active pp_dpm_sclk level"))?;

        let freq_mhz = line
            .split_whitespace()
            .find_map(|token| token.strip_suffix("Mhz"))
            .ok_or(IoError::other(
                "failed to parse pp_dpm_sclk frequency token",
            ))?
            .parse::<u32>()
            .map_err(IoError::other)?;

        Ok(freq_mhz)
    }
}

impl UsageStrategy for BusyFlagUsageStrategy {
    fn poll_and_get_load(
        &mut self,
        dev_handle: &DeviceHandle,
        _process_fdinfo_pdev: &str,
        sampling_interval: Duration,
    ) -> Result<(f32, u32), IoError> {
        for _ in 0..65 {
            let res = dev_handle
                .read_mm_registers(GRBM_STATUS_REG)
                .map_err(IoError::from_raw_os_error)?;
            let gpu_busy = (res & (1 << GPU_ACTIVE_BIT)) > 0;

            self.samples <<= 1;
            if gpu_busy {
                self.samples |= 1;
            }
            std::thread::sleep(sampling_interval);
        }

        let average_load = (self.samples.count_ones() as f32) / 64.0;
        let burst_length = (!self.samples).trailing_zeros();
        Ok((average_load, burst_length))
    }
}

impl UsageStrategy for ProcessUsageStrategy {
    fn poll_and_get_load(
        &mut self,
        _dev_handle: &DeviceHandle,
        process_fdinfo_pdev: &str,
        _sampling_interval: Duration,
    ) -> Result<(f32, u32), IoError> {
        let current_gfx_time = get_total_gfx_time_from_fdinfo(process_fdinfo_pdev);
        let current_time = Instant::now();

        let Some(prev_gfx_time) = self.prev_gfx_time.replace(current_gfx_time) else {
            self.prev_time = Some(current_time);
            return Ok((0.0, 0));
        };

        let prev_time = self.prev_time.replace(current_time).unwrap_or(current_time);
        let delta_gfx_time = current_gfx_time.saturating_sub(prev_gfx_time);
        let delta_time_ns = current_time.duration_since(prev_time).as_nanos() as u64;
        let usage = ((delta_gfx_time as f64) / (delta_time_ns as f64)).clamp(0.0, 1.0) as f32;

        Ok((usage, 0))
    }
}
