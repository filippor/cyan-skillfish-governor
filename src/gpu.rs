use cyan_skillfish_governor_smu::Bc250Smu;
use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    io::Error as IoError,
    os::fd::AsRawFd,
};

// cyan_skillfish.gfx1013.mmGRBM_STATUS
const GRBM_STATUS_REG: u32 = 0x2004;
// cyan_skillfish.gfx1013.mmGRBM_STATUS.GUI_ACTIVE
const GPU_ACTIVE_BIT: u8 = 31;

pub struct GPU {
    dev_handle: DeviceHandle,
    samples: u64,
    process_prev_gfx_time: Option<u64>,
    process_prew_time: Option<std::time::Instant>,
    process_fdinfo_pdev: String,
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

        let process_fdinfo_pdev = format!(
            "{:04x}:{:02x}:{:02x}.{}",
            location.domain, location.bus, location.dev, location.func
        );

        let  min_freq = *safe_points.first_key_value().unwrap().0;
        let max_freq = *safe_points.last_key_value().unwrap().0;
       
        let smu = Bc250Smu::new("0000:00:00.0", true, true, 500)?;
        smu.check_test_message()?;
        println!("SMU communication verified!");
        smu.set_gpu_max_temperature(80)?;
        smu.unforce_gfx_freq()?;
        smu.unforce_gfx_vid()?;
        Ok(GPU {
            dev_handle,
            samples: 0,
            process_prev_gfx_time: None,
            process_prew_time: None,
            process_fdinfo_pdev,
            min_freq,
            max_freq,
            smu,
            safe_points,
        })
    }

    pub fn poll_and_get_load(&mut self, sampling_interval: std::time::Duration) -> Result<(f32, u32), IoError> {
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
            std::thread::sleep(sampling_interval);
        }

        let average_load = (self.samples.count_ones() as f32) / 64.0;
        let burst_length = (!self.samples).trailing_zeros();
        Ok((average_load, burst_length))
    }

    pub fn poll_and_get_load_from_process(&mut self) -> Result<(f32, u32), IoError> {
        let current_gfx_time = get_total_gfx_time_from_fdinfo(&self.process_fdinfo_pdev);
        let current_time = std::time::Instant::now();
        // First call: seed state and return 0 — no previous sample to diff against.
        let Some(prev_gfx_time) = self.process_prev_gfx_time.replace(current_gfx_time) else {
            self.process_prew_time = Some(current_time);
            return Ok((0.0, 0));
        };
        let prev_time = self.process_prew_time.replace(current_time).unwrap_or(current_time);
        let delta_gfx_time = current_gfx_time.saturating_sub(prev_gfx_time);
        let delta_time_ns = current_time.duration_since(prev_time).as_nanos() as u64;
        let usage = ((delta_gfx_time as f64) / (delta_time_ns as f64)).clamp(0.0, 1.0) as f32;
    
        Ok((usage, 0))
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

        self.smu.force_gfx_vid(vol)?;
        self.smu.force_gfx_freq(freq)?;

        Ok(())
    }
    pub fn get_freq(& self) -> Result<u32,IoError>{
        Ok(self.smu.get_gfx_frequency()?)
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
