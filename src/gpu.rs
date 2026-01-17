use cyan_skillfish_governor_direct::Bc250Smu;
use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use std::{collections::BTreeMap, fs::File, io::Error as IoError, os::fd::AsRawFd};

// cyan_skillfish.gfx1013.mmGRBM_STATUS
const GRBM_STATUS_REG: u32 = 0x2004;
// cyan_skillfish.gfx1013.mmGRBM_STATUS.GUI_ACTIVE
const GPU_ACTIVE_BIT: u8 = 31;

pub struct GPU {
    dev_handle: DeviceHandle,
    samples: u64,
    pub min_freq: u32,
    pub max_freq: u32,

    smu: Bc250Smu,
    safe_points: BTreeMap<u32, u32>,
}

impl GPU {
    pub fn new(safe_points: BTreeMap<u32, u32>) -> Result<GPU, Box<dyn std::error::Error>> {
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

        let info = dev_handle
            .device_info()
            .map_err(IoError::from_raw_os_error)?;

        // given in kHz, we need MHz
        let min_engine_clock = info.min_engine_clock / 1000;
        let max_engine_clock = info.max_engine_clock / 1000;
        let mut min_freq = *safe_points.first_key_value().unwrap().0;
        if u64::from(min_freq) < min_engine_clock {
            eprintln!("GPU minimum frequency lower than lowest safe frequency, clamping");
            min_freq = u32::try_from(min_engine_clock)?;
        }
        let mut max_freq = *safe_points.last_key_value().unwrap().0;
        if u64::from(max_freq) > max_engine_clock {
            eprintln!("GPU maximum frequency higher than highest safe frequency, clamping");
            max_freq = u32::try_from(max_engine_clock)?;
        }

        let smu = Bc250Smu::new("0000:00:00.0", true, false, 500)?;
        smu.check_test_message()?;
        println!("SMU communication verified!");
        Ok(GPU {
            dev_handle: dev_handle,
            samples: 0,
            min_freq: min_freq,
            max_freq: max_freq,

            smu: smu,
            safe_points: safe_points,
        })
    }

    pub fn poll_and_get_load(&mut self) -> Result<(f32, u32), IoError> {
        let res = self
            .dev_handle
            .read_mm_registers(GRBM_STATUS_REG)
            .map_err(IoError::from_raw_os_error)?;
        let gui_busy = (res & (1 << GPU_ACTIVE_BIT)) > 0;
        self.samples <<= 1;
        if gui_busy {
            self.samples |= 1;
        }
        let average_load = (self.samples.count_ones() as f32) / 64.0;
        let burst_length = (!self.samples).trailing_zeros();
        Ok((average_load, burst_length))
    }

    pub fn read_temperature(&mut self) -> Result<u32, IoError> {
        let temp = self
            .dev_handle
            .sensor_info(libdrm_amdgpu_sys::AMDGPU::SENSOR_INFO::SENSOR_TYPE::GPU_TEMP)
            .map_err(IoError::from_raw_os_error)?;
        Ok((temp / 1000) as u32)
    }

    pub fn change_freq(&mut self, freq: u32) -> Result<(), IoError> {
        let vol = *self
            .safe_points
            .range(freq..)
            .next()
            .ok_or(IoError::other(
                "tried to set a frequency beyond max safe point",
            ))?
            .1;
        println!("Set GPU frequency to {} MHz", freq);
        println!("Set GPU voltage to {} mV", vol);
        //self.smu.force_gfx_vid(vol)?;
        //self.smu.force_gfx_freq(freq)?;
        // Read back current settings
        let freq = self.smu.get_gfx_frequency()?;
        let vid = self.smu.get_gfx_vid()?;
        println!("Current GPU: {} MHz @ {} mV", freq, vid);

        Ok(())
    }
}
