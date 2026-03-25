use super::UsageStrategy;
use libdrm_amdgpu_sys::AMDGPU::DeviceHandle;
use std::{
    collections::HashMap,
    fs,
    io::Error as IoError,
    time::{Duration, Instant},
};

pub(super) struct ProcessUsageStrategy {
    pub(super) prev_gfx_time: Option<u64>,
    pub(super) prev_time: Option<Instant>,
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

impl UsageStrategy for ProcessUsageStrategy {
    fn poll_and_get_load(
        &mut self,
        _dev_handle: &DeviceHandle,
        process_fdinfo_pdev: &str,
        _sampling_interval: Duration,
    ) -> Result<(f32, u32), IoError> {
        let current_gfx_time = get_total_gfx_time_from_fdinfo(process_fdinfo_pdev);
        let current_time = Instant::now();

        let Some(prev_gfx_time) = self.prev_gfx_time.replace(current_gfx_time) else {
            self.prev_time = Some(current_time);
            return Ok((0.0, 0));
        };

        let prev_time = self.prev_time.replace(current_time).unwrap_or(current_time);
        let delta_gfx_time = current_gfx_time.saturating_sub(prev_gfx_time);
        let delta_time_ns = current_time.duration_since(prev_time).as_nanos() as u64;
        let usage = ((delta_gfx_time as f64) / (delta_time_ns as f64)).clamp(0.0, 1.0) as f32;

        Ok((usage, 0))
    }
}
