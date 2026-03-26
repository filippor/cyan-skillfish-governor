use crate::app_error::{AppError, Result};
use crate::config::{GpuSetMethod, GpuUsageMethod};
use cyan_skillfish_governor_smu::Bc250Smu;
use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use log::info;
use std::{
    collections::BTreeMap,
    fs::File,
    io::Error as IoError,
    os::fd::AsRawFd,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[path = "gpu/kernel_freq_strategy.rs"]
mod kernel_freq_strategy;
use kernel_freq_strategy::KernelFreqStrategy;
#[path = "gpu/process_usage_strategy.rs"]
mod process_usage_strategy;
use process_usage_strategy::ProcessUsageStrategy;

// cyan_skillfish.gfx1013.mmGRBM_STATUS
const GRBM_STATUS_REG: u32 = 0x2004;
// cyan_skillfish.gfx1013.mmGRBM_STATUS.GUI_ACTIVE
const GPU_ACTIVE_BIT: u8 = 31;
const EXPECTED_VENDOR_ID: &str = "0x1002";
const EXPECTED_DEVICE_ID: &str = "0x13fe";

trait FreqStrategy {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<()>;
    fn get_freq(&self) -> Result<u32>;
    fn shutdown(&mut self) -> Result<()> {
        Ok(())
    }
    fn clamp_safe_points(&self, safe_points: BTreeMap<u32, u32>) -> Result<BTreeMap<u32, u32>> {
        Ok(safe_points)
    }
}

trait UsageStrategy {
    fn poll_and_get_load(&mut self) -> Result<(f32, u32)>;
}

struct SmuFreqStrategy {
    smu: Bc250Smu,
}
struct BusyFlagUsageStrategy {
    dev_handle: Arc<DeviceHandle>,
    samples: u64,
    sampling_interval: Duration,
}

pub struct GPU {
    dev_handle: Arc<DeviceHandle>,
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
        sampling_interval: Duration,
    ) -> Result<GPU> {
        let location = BUS_INFO {
            domain: 0,
            bus: 1,
            dev: 0,
            func: 0,
        };
        Self::validate_device_identity(&location)?;

        let render_path = location.get_drm_render_path()?;
        let dev_handle = Self::init_device_handle(&render_path)?;

        let freq_strategy: Box<dyn FreqStrategy> = match gpu_set_method {
            GpuSetMethod::Smu => Box::new(SmuFreqStrategy::new()?),
            GpuSetMethod::Kernel => {
                let gpu_sysfs_path = Self::resolve_gpu_sysfs_path(&render_path)?;
                Box::new(KernelFreqStrategy::new(gpu_sysfs_path)?)
            }
        };

        let safe_points = freq_strategy.clamp_safe_points(safe_points)?;
        let min_freq = *safe_points.first_key_value().unwrap().0;
        let max_freq = *safe_points.last_key_value().unwrap().0;

        let usage_strategy: Box<dyn UsageStrategy> = match gpu_usage_method {
            GpuUsageMethod::BusyFlag => Box::new(BusyFlagUsageStrategy {
                dev_handle: Arc::clone(&dev_handle),
                samples: 0,
                sampling_interval,
            }),
            GpuUsageMethod::Process => Box::new(ProcessUsageStrategy {
                render_node_path: render_path,
                prev_gfx_time: None,
                prev_time: None,
            }),
        };

        Ok(GPU {
            dev_handle,
            min_freq,
            max_freq,
            freq_strategy,
            usage_strategy,
            safe_points,
        })
    }

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

    fn init_device_handle(render_path: &Path) -> Result<Arc<DeviceHandle>> {
        let card = File::open(render_path)?;
        let (dev_handle, _, _) = DeviceHandle::init(card.as_raw_fd())
            .map_err(|e| IoError::other(format!("DeviceHandle::init failed: {e}")))?;
        Ok(Arc::new(dev_handle))
    }

    fn resolve_gpu_sysfs_path(render_path: &Path) -> Result<PathBuf> {
        let render_name = render_path
            .file_name()
            .ok_or(IoError::other("render node path has no file name"))?;
        let sysfs_device_path = Path::new("/sys/class/drm")
            .join(render_name)
            .join("device")
            .canonicalize()
            .map_err(|e| {
                IoError::other(format!(
                    "failed to resolve sysfs device path for render node {}: {e}",
                    render_path.display()
                ))
            })?;
        Ok(sysfs_device_path)
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

    pub fn get_freq(&self) -> Result<u32> {
        self.freq_strategy.get_freq()
    }

    pub fn shutdown(&mut self) -> Result<()> {
        self.freq_strategy.shutdown()
    }
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
