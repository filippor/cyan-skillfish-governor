use crate::app_error::Result;
use std::io::{Error as IoError, ErrorKind};

pub struct MemoryFabricProfile {
    lower_utilization: f64,
    upper_utilization: f64,
    current_profile: Option<u32>,
}

impl MemoryFabricProfile {
    pub fn new(lower_utilization: f64, upper_utilization: f64) -> Self {
        Self {
            lower_utilization,
            upper_utilization,
            current_profile: None,
        }
    }

    pub fn sample(&mut self, fallback_activity: f64) -> Result<Option<u32>> {
        let meminfo = std::fs::read_to_string("/proc/meminfo")?;
        let memory_utilization = parse_memory_utilization(&meminfo)?;
        let utilization = effective_utilization(memory_utilization, fallback_activity);
        Ok(self.select_profile(utilization))
    }

    pub fn reset(&mut self) -> Option<u32> {
        if self.current_profile.is_some_and(|profile| profile != 1) {
            self.current_profile = Some(1);
            Some(1)
        } else {
            None
        }
    }

    fn select_profile(&mut self, utilization: f64) -> Option<u32> {
        let selected = if utilization >= self.upper_utilization {
            3
        } else if utilization <= self.lower_utilization {
            1
        } else {
            2
        };

        if self.current_profile == Some(selected) {
            None
        } else {
            self.current_profile = Some(selected);
            Some(selected)
        }
    }
}

fn effective_utilization(memory_utilization: f64, fallback_activity: f64) -> f64 {
    memory_utilization.max(fallback_activity.clamp(0.0, 1.0))
}

fn parse_memory_utilization(meminfo: &str) -> Result<f64> {
    let mut total_kib = None;
    let mut available_kib = None;

    for line in meminfo.lines() {
        let mut fields = line.split_whitespace();
        match fields.next() {
            Some("MemTotal:") => total_kib = fields.next().and_then(|value| value.parse().ok()),
            Some("MemAvailable:") => {
                available_kib = fields.next().and_then(|value| value.parse().ok())
            }
            _ => {}
        }
    }

    let total_kib: u64 = total_kib.ok_or_else(|| {
        IoError::new(
            ErrorKind::InvalidData,
            "MemTotal missing from /proc/meminfo",
        )
    })?;
    let available_kib: u64 = available_kib.ok_or_else(|| {
        IoError::new(
            ErrorKind::InvalidData,
            "MemAvailable missing from /proc/meminfo",
        )
    })?;
    if total_kib == 0 || available_kib > total_kib {
        return Err(IoError::new(ErrorKind::InvalidData, "invalid /proc/meminfo values").into());
    }

    Ok((total_kib - available_kib) as f64 / total_kib as f64)
}

#[cfg(test)]
mod tests {
    use super::{MemoryFabricProfile, effective_utilization, parse_memory_utilization};

    #[test]
    fn parses_unified_memory_utilization() {
        let utilization = parse_memory_utilization(
            "MemTotal:       1000 kB\nMemFree:         100 kB\nMemAvailable:    250 kB\n",
        )
        .unwrap();

        assert!((utilization - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn selects_all_three_profiles_and_avoids_duplicate_writes() {
        let mut profile = MemoryFabricProfile::new(0.60, 0.80);

        assert_eq!(profile.select_profile(0.50), Some(1));
        assert_eq!(profile.select_profile(0.70), Some(2));
        assert_eq!(profile.select_profile(0.75), None);
        assert_eq!(profile.select_profile(0.85), Some(3));
        assert_eq!(profile.select_profile(0.70), Some(2));
        assert_eq!(profile.select_profile(0.60), Some(1));
        assert_eq!(profile.select_profile(0.50), None);
        assert_eq!(profile.reset(), None);

        assert_eq!(profile.select_profile(0.70), Some(2));
        assert_eq!(profile.reset(), Some(1));
        assert_eq!(profile.select_profile(0.90), Some(3));
        assert_eq!(profile.reset(), Some(1));
    }

    #[test]
    fn fallback_activity_raises_profile_when_memory_occupancy_is_low() {
        let mut profile = MemoryFabricProfile::new(0.60, 0.80);

        assert_eq!(
            profile.select_profile(effective_utilization(0.20, 1.0)),
            Some(3)
        );
    }
}
