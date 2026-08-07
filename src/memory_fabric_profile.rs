use crate::app_error::Result;
use log::trace;
use std::fs::File;
use std::io::{Error as IoError, Read};
use std::os::fd::FromRawFd;
use std::time::{Duration, Instant};

const CACHE_LINE_BYTES: f64 = 64.0;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const PROFILE_HYSTERESIS: f64 = 0.05;
const BANDWIDTH_EMA_HALF_LIFE: Duration = Duration::from_millis(500);
const PERF_TYPE_RAW: u32 = 4;
const PERF_FLAG_FD_CLOEXEC: usize = 8;
const DEMAND_DRAM_REFILLS: u64 = 0x843;
const PREFETCH_DRAM_REFILLS: u64 = 0x85a;

pub struct MemoryFabricProfile {
    lower_utilization: f64,
    upper_utilization: f64,
    bandwidth_scale: f64,
    current_profile: Option<u32>,
    memory_bandwidth: Option<MemoryBandwidth>,
}

impl MemoryFabricProfile {
    pub fn new(lower_utilization: f64, upper_utilization: f64, bandwidth_scale_gib: f64) -> Self {
        Self {
            lower_utilization,
            upper_utilization,
            bandwidth_scale: bandwidth_scale_gib * GIB,
            current_profile: None,
            memory_bandwidth: None,
        }
    }

    pub fn sample(&mut self, gpu_load: f32) -> Result<Option<u32>> {
        if self.memory_bandwidth.is_none() {
            self.memory_bandwidth = Some(MemoryBandwidth::new(self.bandwidth_scale)?);
        }
        let Some((bandwidth, mean_bandwidth, cpu_bandwidth_utilization)) =
            self.memory_bandwidth.as_mut().unwrap().sample()?
        else {
            trace!("Memory fabric bandwidth counter warm-up sample");
            return Ok(None);
        };
        let utilization = effective_utilization(cpu_bandwidth_utilization, gpu_load);
        let profile_change = self.select_profile(utilization);
        trace!(
            "Memory fabric utilization: bandwidth={:.2} GiB/s, ema={:.2} GiB/s, cpu_bandwidth={cpu_bandwidth_utilization:.3}, gpu_load={gpu_load:.3}, effective={utilization:.3}, profile={}",
            bandwidth / GIB,
            mean_bandwidth / GIB,
            self.current_profile.unwrap_or(3),
        );
        Ok(profile_change)
    }

    pub fn reset(&mut self) -> Option<u32> {
        if self.current_profile != Some(3) {
            self.current_profile = Some(3);
            Some(3)
        } else {
            None
        }
    }

    fn select_profile(&mut self, utilization: f64) -> Option<u32> {
        let lower_down = (self.lower_utilization - PROFILE_HYSTERESIS).max(0.0);
        let upper_up = (self.upper_utilization + PROFILE_HYSTERESIS).min(1.0);

        let selected = match self.current_profile {
            Some(1) if utilization >= upper_up => 3,
            Some(1) => 1,
            Some(3) if utilization <= lower_down => 1,
            Some(3) => 3,
            None if utilization >= self.upper_utilization => 3,
            None => 1,
            Some(_) => unreachable!("memory fabric profile must be 1 or 3"),
        };

        if self.current_profile == Some(selected) {
            None
        } else {
            self.current_profile = Some(selected);
            Some(selected)
        }
    }
}

struct MemoryBandwidth {
    counters: Vec<PerfCounter>,
    bandwidth_scale: f64,
    last_sample: Option<(Instant, u64)>,
    bandwidth_ema: ExponentialMovingAverage,
}

impl MemoryBandwidth {
    fn new(bandwidth_scale: f64) -> Result<Self> {
        let online = std::fs::read_to_string("/sys/devices/system/cpu/online")?;
        let mut counters = Vec::new();
        for cpu in parse_cpu_list(&online)? {
            counters.push(PerfCounter::open(cpu, DEMAND_DRAM_REFILLS)?);
            counters.push(PerfCounter::open(cpu, PREFETCH_DRAM_REFILLS)?);
        }
        Ok(Self {
            counters,
            bandwidth_scale,
            last_sample: None,
            bandwidth_ema: ExponentialMovingAverage::new(BANDWIDTH_EMA_HALF_LIFE),
        })
    }

    fn sample(&mut self) -> Result<Option<(f64, f64, f64)>> {
        let total = self
            .counters
            .iter()
            .try_fold(0u64, |sum, counter| Ok::<_, IoError>(sum + counter.read()?))?;
        let now = Instant::now();
        let previous = self.last_sample.replace((now, total));
        let Some((previous_time, previous_total)) = previous else {
            return Ok(None);
        };
        let bandwidth = dram_bandwidth(previous_total, total, now.duration_since(previous_time));
        let mean_bandwidth = self.bandwidth_ema.update(now, bandwidth);
        Ok(Some((
            bandwidth,
            mean_bandwidth,
            bandwidth_utilization(mean_bandwidth, self.bandwidth_scale),
        )))
    }
}

struct ExponentialMovingAverage {
    half_life: Duration,
    last_update: Option<Instant>,
    value: Option<f64>,
}

impl ExponentialMovingAverage {
    fn new(half_life: Duration) -> Self {
        Self {
            half_life,
            last_update: None,
            value: None,
        }
    }

    fn update(&mut self, now: Instant, sample: f64) -> f64 {
        let value = match (self.last_update, self.value) {
            (Some(last_update), Some(previous)) => {
                let elapsed = now.duration_since(last_update).as_secs_f64();
                let decay = 0.5_f64.powf(elapsed / self.half_life.as_secs_f64());
                previous * decay + sample * (1.0 - decay)
            }
            _ => sample,
        };
        self.last_update = Some(now);
        self.value = Some(value);
        value
    }
}

struct PerfCounter(File);

impl PerfCounter {
    fn open(cpu: u32, config: u64) -> Result<Self> {
        let attr = PerfEventAttr {
            type_: PERF_TYPE_RAW,
            size: size_of::<PerfEventAttr>() as u32,
            config,
            ..Default::default()
        };
        let fd = unsafe {
            nix::libc::syscall(
                nix::libc::SYS_perf_event_open,
                &attr,
                -1,
                cpu as i32,
                -1,
                PERF_FLAG_FD_CLOEXEC,
            )
        };
        if fd < 0 {
            let error = IoError::last_os_error();
            return Err(IoError::new(
                error.kind(),
                format!("cannot open CPU {cpu} memory bandwidth counter: {error}"),
            )
            .into());
        }
        Ok(Self(unsafe { File::from_raw_fd(fd as i32) }))
    }

    fn read(&self) -> std::io::Result<u64> {
        let mut value = [0; size_of::<u64>()];
        (&self.0).read_exact(&mut value)?;
        Ok(u64::from_ne_bytes(value))
    }
}

#[repr(C)]
#[derive(Default)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
    config2: u64,
}

fn parse_cpu_list(list: &str) -> Result<Vec<u32>> {
    let mut cpus = Vec::new();
    for range in list.trim().split(',') {
        let (start, end) = range
            .split_once('-')
            .map_or((range, range), |(start, end)| (start, end));
        let start: u32 = start.parse().map_err(|_| "invalid online CPU list")?;
        let end: u32 = end.parse().map_err(|_| "invalid online CPU list")?;
        if start > end {
            return Err("invalid online CPU range".into());
        }
        cpus.extend(start..=end);
    }
    Ok(cpus)
}

fn dram_bandwidth(previous_total: u64, total: u64, elapsed: Duration) -> f64 {
    let seconds = elapsed.as_secs_f64();
    if seconds == 0.0 {
        0.0
    } else {
        total.saturating_sub(previous_total) as f64 * CACHE_LINE_BYTES / seconds
    }
}

fn bandwidth_utilization(bandwidth: f64, bandwidth_scale: f64) -> f64 {
    (bandwidth / bandwidth_scale).clamp(0.0, 1.0)
}

fn effective_utilization(cpu_bandwidth_utilization: f64, gpu_load: f32) -> f64 {
    cpu_bandwidth_utilization.max(f64::from(gpu_load).clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::{
        ExponentialMovingAverage, GIB, MemoryFabricProfile, bandwidth_utilization, dram_bandwidth,
        effective_utilization, parse_cpu_list,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn parses_online_cpu_ranges() {
        let cpus = parse_cpu_list("0-3,8,10-11\n").unwrap();

        assert_eq!(cpus, vec![0, 1, 2, 3, 8, 10, 11]);
    }

    #[test]
    fn calculates_dram_bandwidth_from_cache_line_refills() {
        let bandwidth = dram_bandwidth(1_000, 2_000, Duration::from_millis(100));

        assert!((bandwidth - 640_000.0).abs() < f64::EPSILON);
        assert!(5.0 * GIB > bandwidth);
    }

    #[test]
    fn normalizes_bandwidth_using_configured_scale() {
        assert_eq!(bandwidth_utilization(2.5 * GIB, 5.0 * GIB), 0.5);
        assert_eq!(bandwidth_utilization(6.0 * GIB, 5.0 * GIB), 1.0);
    }

    #[test]
    fn gpu_load_contributes_to_effective_memory_demand() {
        assert_eq!(effective_utilization(0.2, 0.9), f64::from(0.9_f32));
        assert_eq!(effective_utilization(0.8, 0.3), 0.8);
    }

    #[test]
    fn exponential_mean_halves_old_value_weight_each_half_life() {
        let start = Instant::now();
        let mut mean = ExponentialMovingAverage::new(Duration::from_millis(500));

        assert_eq!(mean.update(start, 1.0), 1.0);
        assert_eq!(mean.update(start + Duration::from_millis(500), 3.0), 2.0);
        assert_eq!(mean.update(start + Duration::from_secs(1), 6.0), 4.0);
    }

    #[test]
    fn selects_profiles_one_and_three_without_duplicate_writes() {
        let mut profile = MemoryFabricProfile::new(0.60, 0.80, 5.0);

        assert_eq!(profile.select_profile(0.50), Some(1));
        assert_eq!(profile.select_profile(0.70), None);
        assert_eq!(profile.select_profile(0.75), None);
        assert_eq!(profile.select_profile(0.86), Some(3));
        assert_eq!(profile.select_profile(0.70), None);
        assert_eq!(profile.select_profile(0.60), None);
        assert_eq!(profile.select_profile(0.54), Some(1));
        assert_eq!(profile.select_profile(0.50), None);
        assert_eq!(profile.reset(), Some(3));
        assert_eq!(profile.reset(), None);

        assert_eq!(profile.select_profile(0.70), None);
        assert_eq!(profile.select_profile(0.90), None);
        assert_eq!(profile.reset(), None);
    }

    #[test]
    fn wide_hysteresis_band_prevents_profile_flapping() {
        let mut profile = MemoryFabricProfile::new(0.40, 0.80, 5.0);

        assert_eq!(profile.select_profile(0.39), Some(1));
        assert_eq!(profile.select_profile(0.41), None);
        assert_eq!(profile.select_profile(0.84), None);
        assert_eq!(profile.select_profile(0.86), Some(3));
        assert_eq!(profile.select_profile(0.39), None);
        assert_eq!(profile.select_profile(0.34), Some(1));
    }

    #[test]
    fn reset_selects_profile_three_before_first_sample() {
        let mut profile = MemoryFabricProfile::new(0.60, 0.80, 5.0);

        assert_eq!(profile.reset(), Some(3));
    }
}
