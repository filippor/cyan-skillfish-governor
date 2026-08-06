use super::UsageStrategy;
use crate::app_error::Result;
use std::{
    io::Error as IoError,
    path::{Path, PathBuf},
};

pub(super) struct KernelUsageStrategy {
    gpu_busy_percent_path: PathBuf,
}

impl KernelUsageStrategy {
    pub(super) fn new(render_path: PathBuf) -> Result<Self> {
        let render_name = render_path
            .file_name()
            .ok_or(IoError::other("render node path has no file name"))?;
        let gpu_busy_percent_path = Path::new("/sys/class/drm")
            .join(render_name)
            .join("device")
            .join("gpu_busy_percent");
        Ok(Self {
            gpu_busy_percent_path,
        })
    }
}

impl UsageStrategy for KernelUsageStrategy {
    fn poll_and_get_load(&mut self) -> Result<(f32, u32)> {
        let raw = std::fs::read_to_string(&self.gpu_busy_percent_path)?;
        let percent: f32 = raw
            .trim()
            .parse::<u32>()
            .map_err(|e| IoError::other(format!("gpu_busy_percent parse failed: {e}")))?
            as f32;
        Ok((percent / 100.0, 0))
    }
}
