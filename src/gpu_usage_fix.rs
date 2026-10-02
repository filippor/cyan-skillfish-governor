use crate::bind_overlay::BindOverlay;
use log::{debug, trace};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read, Seek, SeekFrom},
    path::PathBuf,
};

const METRICS_FNAME: &str = "gpu_metrics";
const PATCHED_METRICS_PATH: &str = "/dev/shm/patched_gpu_metrics";
const USAGE_OFFSET: usize = 0x1C; // Byte 28

pub struct GpuUsageFix {
    real_file: File,
    real_metrics_path: PathBuf,
    overlay: BindOverlay,
}

impl GpuUsageFix {
    pub fn start(path: PathBuf) -> io::Result<Self> {
        debug!("Searching {} file at {path:?}", METRICS_FNAME);
        let real_metrics_path_buf = path.join(METRICS_FNAME);
        let real_metrics_path = real_metrics_path_buf
            .as_path()
            .to_str()
            .ok_or_else(|| io::Error::other("metrics path contains invalid UTF-8"))?;

        trace!("Opening real metrics: {real_metrics_path}");
        let mut real_file = OpenOptions::new()
            .read(true)
            .open(&real_metrics_path_buf)
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "failed to open GPU metrics sysfs file '{}': {error}",
                        real_metrics_path_buf.display()
                    ),
                )
            })?;

        // Reading real metrics to buffer
        let mut raw = [0u8; 128];
        real_file.seek(SeekFrom::Start(0)).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to seek GPU metrics sysfs file '{}': {error}",
                    real_metrics_path_buf.display()
                ),
            )
        })?;
        real_file.read(&mut raw).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to read GPU metrics sysfs file '{}': {error}",
                    real_metrics_path_buf.display()
                ),
            )
        })?;

        let overlay = BindOverlay::create(PATCHED_METRICS_PATH, real_metrics_path, &raw)?;

        Ok(Self {
            real_file,
            real_metrics_path: real_metrics_path_buf,
            overlay,
        })
    }

    pub fn set_usage_percent(&mut self, usage: f32) -> io::Result<()> {
        trace!("Set usage percent: {usage:.2}");

        let clamped = (usage * 100.0).clamp(0.0, 10000.0).round() as u16;
        let mut raw = [0u8; 128];

        self.real_file.seek(SeekFrom::Start(0)).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to seek GPU metrics sysfs file '{}': {error}",
                    self.real_metrics_path.display()
                ),
            )
        })?;
        let n = self.real_file.read(&mut raw).map_err(|error| {
            io::Error::new(
                error.kind(),
                format!(
                    "failed to read GPU metrics sysfs file '{}': {error}",
                    self.real_metrics_path.display()
                ),
            )
        })?;

        if n < USAGE_OFFSET + 2 {
            return Ok(());
        }

        raw[USAGE_OFFSET] = (clamped & 0x00FF) as u8;
        raw[USAGE_OFFSET + 1] = (clamped >> 8) as u8;

        self.overlay.replace(&raw)?;

        Ok(())
    }

    pub fn shutdown(&self) -> io::Result<()> {
        self.overlay.shutdown()
    }
}
