use log::{info, warn};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const CPUFREQ_DIR: &str = "/sys/devices/system/cpu/cpufreq";
const GOVERNOR: &str = "performance";

/// Sets each cpufreq policy to the performance governor while performance
/// mode is on, and restores the previous ones afterwards.
#[derive(Default)]
pub struct CpuGovernorSwitch {
    saved: Vec<(PathBuf, String)>,
}

impl CpuGovernorSwitch {
    pub fn apply(&mut self) {
        let entries = match fs::read_dir(CPUFREQ_DIR) {
            Ok(entries) => entries,
            Err(err) => {
                warn!("CPU governor not switched: cannot read {CPUFREQ_DIR}: {err}");
                return;
            }
        };

        let mut found = false;
        for entry in entries.flatten() {
            if !entry.file_name().to_string_lossy().starts_with("policy") {
                continue;
            }
            found = true;
            let policy = entry.path();
            match switch_policy(&policy, GOVERNOR) {
                Ok(Some(previous)) => self.saved.push((policy, previous)),
                Ok(None) => {}
                Err(err) => warn!(
                    "CPU governor not switched on {}: {err}",
                    policy.display()
                ),
            }
        }

        if !found {
            warn!("CPU governor not switched: no cpufreq policy found");
        } else if !self.saved.is_empty() {
            info!(
                "CPU governor set to {GOVERNOR} on {} policies",
                self.saved.len()
            );
        }
    }

    pub fn restore(&mut self) {
        for (policy, previous) in self.saved.drain(..) {
            if let Err(err) = fs::write(policy.join("scaling_governor"), &previous) {
                warn!(
                    "CPU governor {previous} not restored on {}: {err}",
                    policy.display()
                );
            }
        }
    }
}

/// Returns the governor it replaced, or None when the policy is already on
/// the requested one or does not offer it.
fn switch_policy(policy: &Path, governor: &str) -> io::Result<Option<String>> {
    let available = fs::read_to_string(policy.join("scaling_available_governors"))?;
    if !available.split_whitespace().any(|name| name == governor) {
        warn!(
            "{} does not offer the {governor} CPU governor",
            policy.display()
        );
        return Ok(None);
    }

    let current = fs::read_to_string(policy.join("scaling_governor"))?;
    let current = current.trim();
    if current == governor {
        return Ok(None);
    }

    fs::write(policy.join("scaling_governor"), governor)?;
    Ok(Some(current.to_string()))
}
