use crate::app_error::{AppError, Result};
use crate::config::{GpuSetMethod, GpuUsageMethod};
use cyan_skillfish_governor_smu::Bc250Smu;
use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use log::debug;
use log::info;

use std::{collections::BTreeMap, fs::File, io::Error as IoError, path::PathBuf, time::Duration};

#[path = "gpu/kernel_freq_strategy.rs"]
mod kernel_freq_strategy;
use kernel_freq_strategy::KernelFreqStrategy;
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

pub struct GPU {
    dev_handle: DeviceHandle,
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
            "GPU usage method: {} set method: {}",
            gpu_usage_method.as_config_value(),
            gpu_set_method.as_config_value()
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
        };

        Ok(GPU {
            dev_handle: init_device_handle(location.get_drm_render_path()?)?,
            min_freq: *safe_points.first_key_value().unwrap().0,
            max_freq: *safe_points.last_key_value().unwrap().0,
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
        let temp = self
            .dev_handle
            .sensor_info(libdrm_amdgpu_sys::AMDGPU::SENSOR_INFO::SENSOR_TYPE::GPU_TEMP)
            .map_err(IoError::from_raw_os_error)?;
        Ok((temp / 1000) as u32)
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
                    let vol_delta = next_vol  - prev_vol ;
                    prev_vol  + (vol_delta * freq_offset ) / freq_span 
                }
                None => {
                    return Err(IoError::other("tried to set a frequency below min safe point").into())
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
    use super::voltage_for_freq;
    use std::collections::BTreeMap;

    #[test]
    fn voltage_for_freq_interpolates_between_safe_points() {
        let safe_points = BTreeMap::from([(800, 700),(950, 850), (1000, 900)]);

        assert_eq!(voltage_for_freq(&safe_points, 900).unwrap(), 800);
    }
     #[test]
    fn voltage_for_freq_use_safe_points() {
        let safe_points = BTreeMap::from([(800, 700),(950, 850), (1000, 900)]);

        assert_eq!(voltage_for_freq(&safe_points, 950).unwrap(), 850);
    }

      #[test]
    fn voltage_for_freq_rejects_frequency_below_min_safe_point() {
        let safe_points = BTreeMap::from([(800, 700),(950, 850), (1000, 900)]);

        assert!(voltage_for_freq(&safe_points, 750).is_err());
    }


    #[test]
    fn voltage_for_freq_rejects_frequency_above_max_safe_point() {
        let safe_points = BTreeMap::from([(800, 700), (1000, 900)]);

        assert!(voltage_for_freq(&safe_points, 1100).is_err());
    }
}
