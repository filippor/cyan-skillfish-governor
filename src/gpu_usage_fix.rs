use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicU16, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

const REAL_METRICS: &str = "/sys/class/drm/card1/device/gpu_metrics";
const PATCHED_METRICS: &str = "/var/amd_gpu_usage_fix/patched_metrics";
const USAGE_OFFSET: usize = 0x1C; // Byte 28

pub struct GpuUsageFix {
    stop: Arc<AtomicBool>,
    usage_percent: Arc<AtomicU16>,
    worker: Option<JoinHandle<()>>,
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

        let stop = Arc::new(AtomicBool::new(false));
        let usage_percent = Arc::new(AtomicU16::new(0));

        let stop_worker = Arc::clone(&stop);
        let usage_worker = Arc::clone(&usage_percent);

        let worker = thread::spawn(move || {
            let mut real_file = real_file;
            let mut patched_file = patched_file;
            let mut raw = [0u8; 128];

            while !stop_worker.load(Ordering::Relaxed) {
                thread::sleep(Duration::from_millis(250));

                if real_file.seek(SeekFrom::Start(0)).is_err() {
                    continue;
                }
                let n = match real_file.read(&mut raw) {
                    Ok(n) => n,
                    Err(_) => continue,
                };
                if n < USAGE_OFFSET + 2 {
                    continue;
                }

                let usage = usage_worker.load(Ordering::Relaxed).min(100);
                raw[USAGE_OFFSET] = (usage & 0x00FF) as u8;
                raw[USAGE_OFFSET + 1] = (usage >> 8) as u8;

                if patched_file.seek(SeekFrom::Start(0)).is_err() {
                    continue;
                }
                let _ = patched_file.write_all(&raw);
                let _ = patched_file.flush();
            }
        });

        Ok(Self {
            stop,
            usage_percent,
            worker: Some(worker),
        })
    }

    pub fn set_usage_percent(&self, usage: f32) {
        let clamped = usage.clamp(0.0, 100.0).round() as u16;
        self.usage_percent.store(clamped, Ordering::Relaxed);
    }
}

impl Drop for GpuUsageFix {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
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