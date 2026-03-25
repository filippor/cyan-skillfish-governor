use crate::config::{GpuSetMethod, GpuUsageMethod};
use cyan_skillfish_governor_smu::Bc250Smu;
use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use std::{
    collections::BTreeMap,
    fs::File,
    io::Error as IoError,
    os::fd::AsRawFd,
    path::PathBuf,
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

trait FreqStrategy {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<(), IoError>;
    fn get_freq(&self) -> Result<u32, IoError>;
    fn shutdown(&mut self) -> Result<(), IoError> {
        Ok(())
    }
    fn clamp_safe_points(
        &self,
        safe_points: BTreeMap<u32, u32>,
    ) -> Result<BTreeMap<u32, u32>, IoError> {
        Ok(safe_points)
    }
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
        let render_path: PathBuf = location.get_drm_render_path()?;
        let card = File::open(&render_path)?;
        let (dev_handle, _, _) = DeviceHandle::init(card.as_raw_fd())
            .map_err(|e| IoError::other(format!("DeviceHandle::init failed: {e}")))?;

        let gpu_sysfs_path = dev_handle
            .get_sysfs_path()
            .map_err(|e| IoError::other(format!("get_sysfs_path failed: {e}")))?;

        let process_fdinfo_pdev = format!(
            "{:04x}:{:02x}:{:02x}.{}",
            location.domain, location.bus, location.dev, location.func
        );

        let freq_strategy: Box<dyn FreqStrategy> = match gpu_set_method {
            GpuSetMethod::Smu => Box::new(SmuFreqStrategy::new()?),
            GpuSetMethod::Kernel => Box::new(KernelFreqStrategy::new(gpu_sysfs_path)?),
        };

        let safe_points = freq_strategy.clamp_safe_points(safe_points)?;
        let min_freq = *safe_points.first_key_value().unwrap().0;
        let max_freq = *safe_points.last_key_value().unwrap().0;

        let usage_strategy: Box<dyn UsageStrategy> = match gpu_usage_method {
            GpuUsageMethod::BusyFlag => Box::new(BusyFlagUsageStrategy { samples: 0 }),
            GpuUsageMethod::Process => Box::new(ProcessUsageStrategy {
                render_node_path: render_path,
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

    pub fn shutdown(&mut self) -> Result<(), IoError> {
        self.freq_strategy.shutdown()
    }
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

    fn shutdown(&mut self) -> Result<(), IoError> {
        let _ = self.smu.unforce_gfx_freq();
        let _ = self.smu.unforce_gfx_vid();
        Ok(())
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
