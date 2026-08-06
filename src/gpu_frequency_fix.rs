use crate::bind_overlay::BindOverlay;
use cyan_skillfish_governor_smu::Bc250Smu;
use log::trace;
use std::{fs, io, path::PathBuf};

const PATCHED_FREQ_PATH: &str = "/dev/shm/patched_freq_metrics";
const SMU_BDF: &str = "0000:00:00.0";

pub struct GpuFrequencyFix {
    smu: Bc250Smu,
    overlay: BindOverlay,
}

impl GpuFrequencyFix {
    pub fn start(device_path: PathBuf) -> io::Result<Self> {
        let smu = Bc250Smu::new(SMU_BDF, true, true, 500)
            .map_err(|e| io::Error::other(format!("SMU init for gfxclk fix failed: {e}")))?;
        let freq_path = find_hwmon_freq_path(&device_path)?;
        let freq_path = freq_path
            .to_str()
            .ok_or_else(|| io::Error::other("hwmon frequency path contains invalid UTF-8"))?;

        let initial_frequency = fs::read_to_string(freq_path)?;
        let overlay =
            BindOverlay::create(PATCHED_FREQ_PATH, freq_path, initial_frequency.as_bytes())?;

        Ok(Self { smu, overlay })
    }

    pub fn update(&mut self) -> io::Result<()> {
        let mhz = self
            .smu
            .get_gfx_frequency()
            .map_err(|e| io::Error::other(format!("gfxclk SMU read failed: {e}")))?;
        trace!("patching hwmon gfxclk: {mhz} MHz");
        self.overlay
            .replace(format!("{}\n", u64::from(mhz) * 1_000_000).as_bytes())
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.overlay.shutdown()
    }
}

fn find_hwmon_freq_path(device_path: &PathBuf) -> io::Result<PathBuf> {
    for entry in fs::read_dir(device_path.join("hwmon"))? {
        let hwmon_path = entry?.path();
        let name = fs::read_to_string(hwmon_path.join("name"))?;
        let freq_path = hwmon_path.join("freq1_input");
        if name.trim() == "amdgpu" && freq_path.exists() {
            return Ok(freq_path);
        }
    }

    Err(io::Error::new(
        io::ErrorKind::NotFound,
        "amdgpu hwmon freq1_input not found",
    ))
}
