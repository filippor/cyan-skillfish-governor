use super::UsageStrategy;
use std::{
    collections::HashSet,
    fs,
    io::Error as IoError,
    path::{Path, PathBuf},
    time::Instant,
};

pub(super) struct ProcessUsageStrategy {
    pub(super) render_node_path: PathBuf,
    pub(super) prev_gfx_time: Option<u64>,
    pub(super) prev_time: Option<Instant>,
}

fn get_total_gfx_time_from_fdinfo(render_node_path: &Path) -> u64 {
    let mut seen_cids: HashSet<u32> = HashSet::new();
    let mut total_engine_time: u64 = 0;

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

        // Collect fd numbers whose symlink points exactly to our render node.
        let fd_dir = proc_entry.path().join("fd");
        let gpu_fds: HashSet<u64> = fs::read_dir(&fd_dir)
            .map(|rd| {
                rd.flatten()
                    .filter_map(|e| {
                        let target = fs::read_link(e.path()).ok()?;
                        if target == render_node_path {
                            e.file_name().to_string_lossy().parse::<u64>().ok()
                        } else {
                            None
                        }
                    })
                    .collect()
            })
            .unwrap_or_default();

        if gpu_fds.is_empty() {
            continue;
        }

        let fdinfo_dir = proc_entry.path().join("fdinfo");
        let fdinfos = match fs::read_dir(fdinfo_dir) {
            Ok(v) => v,
            Err(_) => continue,
        };

        for fdinfo in fdinfos.flatten() {
            let fd_num = fdinfo
                .file_name()
                .to_string_lossy()
                .parse::<u64>()
                .unwrap_or(u64::MAX);
            if !gpu_fds.contains(&fd_num) {
                continue;
            }

            let content = match fs::read_to_string(fdinfo.path()) {
                Ok(c) => c,
                Err(_) => continue,
            };

            // Skip header lines before drm-client-id.
            let mut lines = content.lines().skip_while(|l| !l.starts_with("drm-client-id"));
            let Some(cid_line) = lines.next() else {
                continue;
            };
            let Some(cid) = cid_line
                .strip_prefix("drm-client-id:")
                .and_then(|v| v.trim().parse::<u32>().ok())
            else {
                continue;
            };
            // Skip contexts already counted (shared across processes).
            if !seen_cids.insert(cid) {
                continue;
            }

            let mut engine_total: u64 = 0;
            for line in lines {
                let Some((_, rest)) = line
                    .strip_prefix("drm-engine-")
                    .and_then(|v| v.split_once(':'))
                else {
                    continue;
                };
                let mut fields = rest.split_whitespace();
                let Some(value_tok) = fields.next() else {
                    continue;
                };
                if fields.next() != Some("ns") {
                    continue;
                }
                engine_total = engine_total.saturating_add(value_tok.parse::<u64>().unwrap_or(0));
            }
            total_engine_time = total_engine_time.saturating_add(engine_total);
        }
    }

    total_engine_time
}

impl UsageStrategy for ProcessUsageStrategy {
    fn poll_and_get_load(&mut self) -> Result<(f32, u32), IoError> {
        let current_gfx_time = get_total_gfx_time_from_fdinfo(&self.render_node_path);
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
