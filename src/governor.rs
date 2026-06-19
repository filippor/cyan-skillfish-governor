use crate::app_error::Result;
use crate::config::GovernorParams;
use crate::gpu::GPU;
use crate::gpu_usage_fix::GpuUsageFix;
use log::{debug, error, info};
use std::ops::RangeInclusive;
use std::time::{Duration, Instant};

const UP_EVENTS: i16 = 2;

pub struct Governor {
    params: GovernorParams,
    gpu: GPU,
    gpu_usage_fix: Option<GpuUsageFix>,
    curr_freq: u32,
    target_freq: u32,
    status: i16,
    usage_fix_cycle: u32,
    max_freq: u32,
    requested_range: RangeInclusive<u32>,
    performance_mode: bool,
    test_mode: bool,
}

impl Governor {
    pub fn new(
        params: GovernorParams,
        gpu: GPU,
        gpu_usage_fix: Option<GpuUsageFix>,
    ) -> Result<Self> {
        let curr_freq = gpu.get_freq()?;
        let target_freq = *params.allowed_frequency_range.start();
        let max_freq = *params.allowed_frequency_range.end();
        let requested_range = params.initial_frequency_range.clone();

        Ok(Self {
            params,
            gpu,
            gpu_usage_fix,
            curr_freq,
            target_freq,
            status: 0,
            usage_fix_cycle: 0,
            max_freq,
            requested_range,
            performance_mode: false,
            test_mode: false,
        })
    }

    pub fn run_iteration(&mut self) -> Result<()> {
        let loop_start = Instant::now();

        let (average_load, burst_length) = if !self.performance_mode || self.gpu_usage_fix.is_some()
        {
            self.gpu.poll_and_get_load()?
        } else {
            (1.0, 0)
        };

        if let Some(fix) = self.gpu_usage_fix.as_mut() {
            self.usage_fix_cycle += 1;
            if self.performance_mode || self.usage_fix_cycle >= self.params.flush_every {
                if let Err(err) = fix.set_usage_percent(average_load * 100.0) {
                    error!("GPU usage metrics requested_rangefix write failed: {err}");
                }
            }
        }

        let temp = self.update_max_freq_for_temperature()?;
        if !self.test_mode {
            let (next_target, next_status, should_apply_change) = compute_frequency_decision(
                self.curr_freq,
                self.target_freq,
                self.status,
                self.max_freq,
                *self.requested_range.start(),
                self.performance_mode,
                average_load,
                burst_length,
                &self.params,
            );
            self.target_freq = next_target;
            self.status = next_status;

            if should_apply_change {
                debug!(
                    "freq curr {} target {} temp {} load {:.2} status {}  burst_length {}, performance_mode {}",
                    self.curr_freq,
                    self.target_freq,
                    temp,
                    average_load,
                    self.status,
                    burst_length,
                    self.performance_mode
                );
                self.gpu.change_freq(self.target_freq)?;
                self.status = 0;
                self.curr_freq = self.target_freq;
            }
        }
        let target_cycle_interval = if self.performance_mode {
            self.params
                .adjustment_interval
                .checked_mul(self.params.flush_every.max(1))
                .unwrap_or(Duration::MAX)
        } else {
            self.params.adjustment_interval
        };

        let elapsed = loop_start.elapsed();
        if elapsed < target_cycle_interval {
            std::thread::sleep(target_cycle_interval - elapsed);
        }

        Ok(())
    }

    pub fn apply_enable_command(&mut self) {
        info!("Performance mode enabled");
        self.performance_mode = true;
        self.test_mode = false;
        self.requested_range = self.params.allowed_frequency_range.clone();
        info!(
            "Updated performance mode: {} range : {}..={}",
            self.performance_mode,
            self.requested_range.start(),
            self.requested_range.end()
        );
    }

    pub fn apply_disable_command(&mut self) {
        info!("Performance mode disabled: reverting to adaptive frequency control");
        self.performance_mode = false;
        self.test_mode = false;
        self.requested_range = self.params.initial_frequency_range.clone();
        info!(
            "Updated performance mode: {} range : {}..={}",
            self.performance_mode,
            self.requested_range.start(),
            self.requested_range.end()
        );
    }

    pub fn apply_fixed_frequency_command(&mut self, frequency: u32) {
        info!(
            "Performance mode enabled with fixed frequency request: {}",
            frequency
        );
        self.performance_mode = true;
        self.test_mode = false;
        self.requested_range = *self.params.initial_frequency_range.start()..=frequency;
        if self.max_freq > frequency {
            self.max_freq = frequency;
        }
    }

    pub fn apply_range_command(&mut self, min: u32, max: u32) {
        self.test_mode = false;
        match (min, max) {
            (0, 0) => {
                info!("Frequency range cleared: both limits removed");
                self.performance_mode = false;
                self.requested_range = self.params.initial_frequency_range.clone();
                info!(
                    "Updated range : {}..={}",
                    self.requested_range.start(),
                    self.requested_range.end()
                );
            }
            (0, ma) if self.params.allowed_frequency_range.contains(&ma) => {
                info!("Upper limit set to {} MHz, lower limit removed", ma);
                self.performance_mode = false;
                self.requested_range = *self.params.initial_frequency_range.start()..=ma;
                info!(
                    "Updated range : {}..={}",
                    self.requested_range.start(),
                    self.requested_range.end()
                );
            }
            (mi, 0) if self.params.allowed_frequency_range.contains(&mi) => {
                info!("Lower limit set to {} MHz, upper limit removed", mi);
                self.performance_mode = false;
                self.requested_range = mi..=*self.params.initial_frequency_range.end();
                info!(
                    "Updated range : {}..={}",
                    self.requested_range.start(),
                    self.requested_range.end()
                );
            }
            (mi, ma)
                if self.params.allowed_frequency_range.contains(&mi)
                    && self.params.allowed_frequency_range.contains(&ma) =>
            {
                info!("Frequency range set: min={} MHz, max={} MHz", mi, ma);
                self.performance_mode = false;
                self.requested_range = mi..=ma;
                info!(
                    "Updated range : {}..={}",
                    self.requested_range.start(),
                    self.requested_range.end()
                );
            }
            (mi, ma) => {
                error!(
                    "D-Bus command handling failed, keeping previous state: Invalid frequency range request: min={} MHz, max={} MHz (allowed {}..={} MHz)",
                    mi,
                    ma,
                    *self.params.allowed_frequency_range.start(),
                    *self.params.allowed_frequency_range.end()
                );
            }
        }
        if self.max_freq > *self.requested_range.end() {
            self.max_freq = *self.requested_range.end();
        }
    }

    pub fn apply_test_mode_command(&mut self, frequency: u32, voltage: u32) -> Result<()> {
        self.gpu.change_freq_vol(frequency, voltage)?;
        self.test_mode = true;
        self.curr_freq = 0; // Force update when test disabled
        info!(
            "Test mode enabled with fixed frequency {} MHz and voltage {} mV",
            frequency, voltage
        );
        Ok(())
    }

    pub fn shutdown(&mut self) -> Result<()> {
        if let Some(fix) = self.gpu_usage_fix.as_mut()
            && let Err(err) = fix.shutdown()
        {
            error!("GPU usage metrics fix cleanup failed: {err}");
        }
        if let Err(err) = self.gpu.shutdown() {
            error!("System exit restore failed: {err}");
        }
        Ok(())
    }

    fn update_max_freq_for_temperature(&mut self) -> Result<u32> {
        let temp = self.gpu.read_temperature()?;
        if let Some(max_temp) = self.params.temperature.throttling_temp {
            let min_freq = *self.params.allowed_frequency_range.start();
            if temp > max_temp && self.max_freq - self.params.significant_change > min_freq {
                self.max_freq -= self.params.significant_change;
                debug!("throttling temp {temp} freq {}", self.max_freq);
            } else if let Some(recovery_temp) = self.params.temperature.throttling_recovery_temp
                && temp < recovery_temp
                && self.max_freq != *self.requested_range.end()
            {
                self.max_freq = *self.requested_range.end();
                debug!("recover throttling temp {temp} freq {}", self.max_freq);
            }
        }

        Ok(temp)
    }
}

fn compute_frequency_decision(
    curr_freq: u32,
    mut target_freq: u32,
    mut status: i16,
    max_freq: u32,
    requested_start: u32,
    performance_mode: bool,
    average_load: f32,
    burst_length: u32,
    params: &GovernorParams,
) -> (u32, i16, bool) {
    if performance_mode {
        target_freq = max_freq;
        return (target_freq, status, curr_freq != target_freq);
    }

    let burst = average_load >= 0.99
        || params
            .burst_samples
            .is_some_and(|burst_samples| burst_length >= burst_samples);
    if burst {
        target_freq += params.burst_freq_step;
    } else {
        if average_load > params.up_thresh && status <= UP_EVENTS {
            status += UP_EVENTS;
        } else if average_load < params.down_thresh && curr_freq > requested_start {
            status -= 1;
        } else if status < 0 {
            status += 1;
        } else if status > 0 {
            status -= 1;
        }

        if status <= -params.down_events {
            target_freq -= params.freq_step;
        } else if status >= UP_EVENTS {
            target_freq += params.freq_step;
        }
    }

    target_freq = target_freq.clamp(requested_start.min(max_freq), max_freq);

    let hit_bounds = target_freq == requested_start || target_freq == max_freq;
    let big_change = curr_freq.abs_diff(target_freq) >= params.significant_change;

    let should_apply_change = curr_freq != target_freq && (burst || hit_bounds || big_change);
    (target_freq, status, should_apply_change)
}

#[cfg(test)]
mod tests {
    use super::compute_frequency_decision;
    use crate::config::{GovernorParams, TemperatureConfig};

    fn sample_params() -> GovernorParams {
        GovernorParams {
            burst_freq_step: 50,
            freq_step: 10,
            allowed_frequency_range: 300..=1200,
            initial_frequency_range: 400..=1000,
            flush_every: 10,
            temperature: TemperatureConfig {
                throttling_temp: Some(85),
                throttling_recovery_temp: Some(80),
            },
            significant_change: 10,
            burst_samples: Some(3),
            up_thresh: 0.95,
            down_thresh: 0.80,
            down_events: 10,
            adjustment_interval: std::time::Duration::from_micros(20_000),
        }
    }

    #[test]
    fn run_iteration_decision_perf_mode_targets_max() {
        let p = sample_params();
        let (target, status, should_apply) =
            compute_frequency_decision(600, 600, 0, 1000, 400, true, 0.1, 0, &p);

        assert_eq!(target, 1000);
        assert_eq!(status, 0);
        assert!(should_apply);
    }

    #[test]
    fn run_iteration_decision_burst_increases_target() {
        let p = sample_params();
        let (target, _status, should_apply) =
            compute_frequency_decision(600, 600, 0, 1000, 400, false, 1.0, 0, &p);

        assert_eq!(target, 650);
        assert!(should_apply);
    }

    #[test]
    fn run_iteration_decision_down_events_reduce_target() {
        let p = sample_params();
        let (target, status, should_apply) =
            compute_frequency_decision(700, 700, -9, 1000, 400, false, 0.2, 0, &p);

        assert_eq!(status, -10);
        assert_eq!(target, 690);
        assert!(should_apply);
    }

    #[test]
    fn run_iteration_decision_no_big_change_no_burst_no_apply() {
        let p = sample_params();
        let (target, status, should_apply) =
            compute_frequency_decision(700, 700, 0, 1000, 400, false, 0.85, 0, &p);

        assert_eq!(target, 700);
        assert_eq!(status, 0);
        assert!(!should_apply);
    }
}
