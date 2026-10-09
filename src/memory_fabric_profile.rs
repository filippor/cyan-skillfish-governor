use crate::app_error::Result;
use cyan_skillfish_governor_smu::Bc250Smu;
use log::trace;
use std::fs::File;
use std::io::{Error as IoError, Read};
use std::os::fd::FromRawFd;
use std::time::{Duration, Instant};
use log::debug;
use log::info;

const CACHE_LINE_BYTES: f64 = 64.0;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const PROFILE_HYSTERESIS: f64 = 0.05;
const UTILIZATION_EMA_HALF_LIFE: Duration = Duration::from_millis(500);
const PERF_TYPE_RAW: u32 = 4;
const PERF_FLAG_FD_CLOEXEC: usize = 8;
const DEMAND_DRAM_REFILLS: u64 = 0x843;
const PREFETCH_DRAM_REFILLS: u64 = 0x85a;

trait MemoryFabricStrategy: Send {
    fn set_memory_fabric_profile(&self, perf_profile: u32) -> Result<()>;
}


pub struct MemoryFabricProfile {
    lower_utilization: f64,
    upper_utilization: f64,
    capacities: [ProfileCapacity; 3],
    current_profile: Option<u32>,
    effective_utilization_ema: ExponentialMovingAverage,
    memory_bandwidth: Option<MemoryBandwidth>,
    memory_fabric_strategy: Box<dyn MemoryFabricStrategy + Send>,}

#[derive(Clone, Copy)]
struct ProfileCapacity {
    bandwidth: f64,
    core_bandwidth: f64,
}

impl MemoryFabricProfile {
    pub fn new(
        lower_utilization: f64,
        upper_utilization: f64,
        capacities_gib: [(f64, f64); 3],
    ) -> Result<Self> {
        Ok(Self {
            lower_utilization,
            upper_utilization,
            capacities: capacities_gib.map(|(bandwidth, core_bandwidth)| ProfileCapacity {
                bandwidth: bandwidth * GIB,
                core_bandwidth: core_bandwidth * GIB,
            }),
            current_profile: None,
            effective_utilization_ema: ExponentialMovingAverage::new(UTILIZATION_EMA_HALF_LIFE),
            memory_bandwidth: None,
            memory_fabric_strategy: Box::new(MemoryFabricSmuStrategy::new()?),
        })
    }

   
    pub fn sample(&mut self, gpu_load: f32) -> Result<Option<u32>> {
        if self.memory_bandwidth.is_none() {
            self.memory_bandwidth = Some(MemoryBandwidth::new()?);
        }
        let Some((bandwidth, max_core_bandwidth)) =
            self.memory_bandwidth.as_mut().unwrap().sample()?
        else {
            trace!("Memory bandwidth counter warm-up sample");
            return Ok(None);
        };
        let active_profile = self.current_profile.unwrap_or(3);
        let (aggregate_utilization, core_utilization) =
            self.memory_utilization(active_profile, bandwidth, max_core_bandwidth);
        let gpu_load = f64::from(gpu_load).clamp(0.0, 1.0);
        let effective = aggregate_utilization.max(core_utilization).max(gpu_load);
        let utilization = self
            .effective_utilization_ema
            .update(Instant::now(), effective);
        let profile_change = self.select_profile(utilization);
        trace!(
            "Memory fabric utilization: bandwidth={:.2} GiB/s, max_core_bandwidth={:.2} GiB/s, bandwidth_utilization={aggregate_utilization:.3}, core_utilization={core_utilization:.3}, gpu_load={gpu_load:.3}, effective={effective:.3}, effective_ema={utilization:.3}, sampled_profile={active_profile}, profile={}",
            bandwidth / GIB,
            max_core_bandwidth / GIB,
            self.current_profile.unwrap_or(3),
        );
        Ok(profile_change)
    }

     pub fn update_profile(&mut self, gpu_load: f32) -> Result<()> {
        if let Some(perf_profile) = self.sample(gpu_load)? {
            self.memory_fabric_strategy.set_memory_fabric_profile(perf_profile)?;
            info!("Memory fabric performance profile changed to {perf_profile}");
        }
        Ok(())
    }

    fn memory_utilization(
        &self,
        active_profile: u32,
        bandwidth: f64,
        max_core_bandwidth: f64,
    ) -> (f64, f64) {
        let capacity = self.capacities[(active_profile - 1) as usize];
        (
            bandwidth_utilization(bandwidth, capacity.bandwidth),
            bandwidth_utilization(max_core_bandwidth, capacity.core_bandwidth),
        )
    }

    pub fn reset(&mut self) -> Result<()> {
        self.memory_fabric_strategy.set_memory_fabric_profile(3)

    }

    fn select_profile(&mut self, utilization: f64) -> Option<u32> {
        let lower_down = (self.lower_utilization - PROFILE_HYSTERESIS).max(0.0);
        let lower_up = (self.lower_utilization + PROFILE_HYSTERESIS).min(1.0);
        let upper_down = (self.upper_utilization - PROFILE_HYSTERESIS).max(0.0);
        let upper_up = (self.upper_utilization + PROFILE_HYSTERESIS).min(1.0);

        let selected = match self.current_profile {
            Some(1) if utilization >= upper_up => 3,
            Some(1) if utilization >= lower_up => 2,
            Some(1) => 1,
            Some(2) if utilization >= upper_up => 3,
            Some(2) if utilization <= lower_down => 1,
            Some(2) => 2,
            Some(3) if utilization <= lower_down => 1,
            Some(3) if utilization <= upper_down => 2,
            Some(3) => 3,
            None if utilization >= self.upper_utilization => 3,
            None if utilization <= self.lower_utilization => 1,
            None => 2,
            Some(_) => unreachable!("memory fabric profile must be 1, 2, or 3"),
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
    last_sample: Option<(Instant, u64)>,
    last_core_totals: Option<Vec<u64>>,
}

impl MemoryBandwidth {
    fn new() -> Result<Self> {
        let online = std::fs::read_to_string("/sys/devices/system/cpu/online")?;
        let mut counters = Vec::new();
        for cpu in parse_cpu_list(&online)? {
            counters.push(PerfCounter::open(cpu, DEMAND_DRAM_REFILLS)?);
            counters.push(PerfCounter::open(cpu, PREFETCH_DRAM_REFILLS)?);
        }
        Ok(Self {
            counters,
            last_sample: None,
            last_core_totals: None,
        })
    }

    fn sample(&mut self) -> Result<Option<(f64, f64)>> {
        let core_totals = self
            .counters
            .chunks_exact(2)
            .map(|counters| Ok::<_, IoError>(counters[0].read()? + counters[1].read()?))
            .collect::<std::io::Result<Vec<_>>>()?;
        let total = core_totals.iter().sum();
        let now = Instant::now();
        let previous = self.last_sample.replace((now, total));
        let previous_core_totals = self.last_core_totals.replace(core_totals);
        let Some((previous_time, previous_total)) = previous else {
            return Ok(None);
        };
        let elapsed = now.duration_since(previous_time);
        let bandwidth = dram_bandwidth(previous_total, total, elapsed);
        let max_core_bandwidth = max_core_bandwidth(
            previous_core_totals.as_ref().unwrap(),
            self.last_core_totals.as_ref().unwrap(),
            elapsed,
        )
        .unwrap_or(0.0);
        Ok(Some((bandwidth, max_core_bandwidth)))
    }
}

struct MemoryFabricSmuStrategy {
    smu: Bc250Smu,
}



impl MemoryFabricSmuStrategy {
    fn new() -> Result<Self> {
        let smu = Bc250Smu::new("0000:00:00.0", true, true, 500)?;
        smu.check_test_message()?;
        info!("SMU communication verified");
        smu.set_gpu_max_temperature(80)?;
        smu.unforce_gfx_freq()?;
        smu.unforce_gfx_vid()?;
        Ok(Self { smu })
    }
}

impl MemoryFabricStrategy  for MemoryFabricSmuStrategy {
 fn set_memory_fabric_profile(&self, perf_profile: u32) -> Result<()> {
        self.smu.q3_set_perf_profile_index(perf_profile)?;
        debug!("SMU set memory fabric performance profile to {perf_profile}");
        Ok(())
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

fn max_core_bandwidth(previous: &[u64], current: &[u64], elapsed: Duration) -> Option<f64> {
    previous
        .iter()
        .zip(current)
        .map(|(&previous, &current)| dram_bandwidth(previous, current, elapsed))
        .reduce(f64::max)
}

fn bandwidth_utilization(bandwidth: f64, bandwidth_scale: f64) -> f64 {
    (bandwidth / bandwidth_scale).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::{
        ExponentialMovingAverage, GIB, MemoryFabricProfile, bandwidth_utilization, dram_bandwidth,
        max_core_bandwidth, parse_cpu_list,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn parses_online_cpu_ranges() {
        assert_eq!(
            parse_cpu_list("0-3,8,10-11\n").unwrap(),
            vec![0, 1, 2, 3, 8, 10, 11]
        );
    }

    #[test]
    fn calculates_and_normalizes_dram_bandwidth() {
        let bandwidth = dram_bandwidth(1_000, 2_000, Duration::from_millis(100));

        assert!((bandwidth - 640_000.0).abs() < f64::EPSILON);
        assert_eq!(bandwidth_utilization(2.5 * GIB, 5.0 * GIB), 0.5);
    }

    #[test]
    fn calculates_maximum_per_core_bandwidth() {
        let bandwidth =
            max_core_bandwidth(&[1_000, 2_000], &[1_500, 3_000], Duration::from_millis(100));

        assert_eq!(bandwidth, Some(640_000.0));
    }

    #[test]
    fn normalizes_bandwidth_against_the_active_profile_capacity() {
        if let Ok(profile) = MemoryFabricProfile::new(0.40, 0.80, [(4.0, 2.3), (12.4, 6.1), (18.1, 4.4)]){
        
        

        assert_eq!(
            profile.memory_utilization(1, 4.0 * GIB, 2.3 * GIB),
            (1.0, 1.0)
        );
        assert_eq!(
            profile.memory_utilization(2, 12.4 * GIB, 6.1 * GIB),
            (1.0, 1.0)
        );
        assert_eq!(
            profile.memory_utilization(3, 18.1 * GIB, 4.4 * GIB),
            (1.0, 1.0)
        );
        assert!(profile.memory_utilization(1, 4.0 * GIB, 0.0).0 > 0.99);
        assert!(profile.memory_utilization(2, 4.0 * GIB, 0.0).0 < 0.33);
    }
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
    fn selects_all_three_profiles_without_duplicate_writes() {
        if let  Ok(mut profile) =
            MemoryFabricProfile::new(0.60, 0.80, [(4.0, 2.3), (12.4, 6.1), (18.1, 4.4)])
{
        assert_eq!(profile.select_profile(0.50), Some(1));
        assert_eq!(profile.select_profile(0.70), Some(2));
        assert_eq!(profile.select_profile(0.75), None);
        assert_eq!(profile.select_profile(0.86), Some(3));
        assert_eq!(profile.select_profile(0.70), Some(2));
        assert_eq!(profile.select_profile(0.60), None);
        assert_eq!(profile.select_profile(0.54), Some(1));
        assert_eq!(profile.select_profile(0.50), None);
       

        assert_eq!(profile.select_profile(0.70), Some(2));
        assert_eq!(profile.select_profile(0.90), Some(3));
        
    }
    }
    #[test]
    fn hysteresis_prevents_flapping_at_both_profile_two_boundaries() {
        if let Ok(mut profile) =
            MemoryFabricProfile::new(0.40, 0.80, [(4.0, 2.3), (12.4, 6.1), (18.1, 4.4)]) {

        assert_eq!(profile.select_profile(0.39), Some(1));
        assert_eq!(profile.select_profile(0.41), None);
        assert_eq!(profile.select_profile(0.46), Some(2));
        assert_eq!(profile.select_profile(0.39), None);
        assert_eq!(profile.select_profile(0.34), Some(1));
        assert_eq!(profile.select_profile(0.46), Some(2));
        assert_eq!(profile.select_profile(0.81), None);
        assert_eq!(profile.select_profile(0.86), Some(3));
        assert_eq!(profile.select_profile(0.81), None);
        assert_eq!(profile.select_profile(0.74), Some(2));
    }
    }

}
