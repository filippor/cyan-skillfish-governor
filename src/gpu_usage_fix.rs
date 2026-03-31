use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    path::PathBuf,
    process::{Command, Stdio},
};
use log::{debug, trace};

const METRICS_FNAME: &str = "gpu_metrics";
const PATCHED_METRICS_PATH: &str = "/dev/shm/patched_gpu_metrics";
const USAGE_OFFSET: usize = 0x1C; // Byte 28

pub struct GpuUsageFix {
    real_file: File,
    patched_file: File,
    path: String,
}

impl GpuUsageFix {
    pub fn start(path: PathBuf) -> io::Result<Self> {
        debug!("Searching {} file at {path:?}", METRICS_FNAME);
        let real_metrics_path_buf = path.join(METRICS_FNAME);
        let real_metrics_path = real_metrics_path_buf.as_path().to_str().unwrap();

        trace!("Unmounting stale bind: {real_metrics_path}");
        let _ = umount_bind(&real_metrics_path);

        trace!("Opening real metrics: {real_metrics_path}");
        let mut real_file = OpenOptions::new().read(true).open(&real_metrics_path)?;

        // Reading real metrics to buffer
        let mut raw = [0u8; 128];
        real_file.seek(SeekFrom::Start(0))?;
        real_file.read(&mut raw)?;

        trace!("Creating patched metrics: {}", PATCHED_METRICS_PATH);
        let mut patched_file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(PATCHED_METRICS_PATH)?;
        patched_file.write_all(&raw)?;
        patched_file.flush()?;

        trace!("Binding patched metrics {} to real metrics: {real_metrics_path}", PATCHED_METRICS_PATH);
        mount_bind(PATCHED_METRICS_PATH, &real_metrics_path)?;

        trace!("Removing file from filesystem: {}", PATCHED_METRICS_PATH);
        fs::remove_file(PATCHED_METRICS_PATH)?;

        Ok(Self {
            real_file,
            patched_file,
            path: String::from(real_metrics_path),
        })
    }

    pub fn set_usage_percent(&mut self, usage: f32) -> io::Result<()> {
        trace!("Set usage percent: {usage:.2}");

        let clamped = (usage * 100.0).clamp(0.0, 10000.0).round() as u16;
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

    pub fn shutdown(&self) -> io::Result<()> {
        umount_bind(self.path.as_str())
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
    let status = Command::new("umount")
        .arg(dst)
        .stderr(Stdio::null())
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "umount {} failed: {}",
            dst, status
        )))
    }
}
