use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    process::Command,
};

const REAL_METRICS: &str = "/sys/class/drm/card1/device/gpu_metrics";
const PATCHED_METRICS: &str = "/var/amd_gpu_usage_fix/patched_metrics";
const USAGE_OFFSET: usize = 0x1C; // Byte 28

pub struct GpuUsageFix {
    real_file: File,
    patched_file: File,
}

impl GpuUsageFix {
    pub fn start() -> io::Result<Self> {
        let _ = umount_bind(REAL_METRICS);

        // Open real metrics before bind-mount.
        let real_file = OpenOptions::new().read(true).open(REAL_METRICS)?;

        // Create zeroed patched file.
        if let Some(parent) = Path::new(PATCHED_METRICS).parent() {
            fs::create_dir_all(parent)?;
        }
        {
            let mut f = OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .open(PATCHED_METRICS)?;
            f.write_all(&[0u8; 128])?;
            f.flush()?;
        }

        mount_bind(PATCHED_METRICS, REAL_METRICS)?;
        let patched_file = OpenOptions::new().write(true).open(PATCHED_METRICS)?;

        Ok(Self {
            real_file,
            patched_file,
        })
    }

    pub fn set_usage_percent(&mut self, usage: f32) -> io::Result<()> {
        let clamped = usage.clamp(0.0, 100.0).round() as u16;
        let mut raw = [0u8; 128];

        self.real_file.seek(SeekFrom::Start(0))?;
        let n = self.real_file.read(&mut raw)?;

        if n < USAGE_OFFSET + 2 {
            return Ok(());
        }

        raw[USAGE_OFFSET] = (clamped & 0x00FF) as u8;
        raw[USAGE_OFFSET + 1] = (clamped >> 8) as u8;

        self.patched_file.seek(SeekFrom::Start(0))?;
        self.patched_file.write_all(&raw)?;
        self.patched_file.flush()?;

        Ok(())
    }
}

impl Drop for GpuUsageFix {
    fn drop(&mut self) {
        let _ = umount_bind(REAL_METRICS);
    }
}

fn mount_bind(src: &str, dst: &str) -> io::Result<()> {
    let status = Command::new("mount").args(["--bind", src, dst]).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "mount --bind {} {} failed: {}",
            src, dst, status
        )))
    }
}

fn umount_bind(dst: &str) -> io::Result<()> {
    let status = Command::new("umount").arg(dst).status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!("umount {} failed: {}", dst, status)))
    }
}