mod app_error;
mod config;
mod gpu;
mod gpu_usage_fix;
use app_error::Result;
use clap::Parser;
use config::{Config, TimingConfig};
use gpu::GPU;
use gpu_usage_fix::GpuUsageFix;
use log::{debug, error, info};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::time::{Duration, Instant};

const UP_EVENTS: i16 = 2;

#[derive(Debug, Parser)]
#[command(name = "cyan-skillfish-governor-smu")]
struct Args {
    #[arg(short, long)]
    verbose: bool,
    #[arg(value_name = "CONFIG")]
    config_path: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    init_logger(args.verbose);

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    install_signal_handler(shutdown_tx)?;
    let config = load_config(args.config_path.as_deref())?;
    let mut gpu_usage_fix = start_gpu_usage_fix(config.gpu_usage.fix_metrics)?;

    info!(
        "GPU usage method configured: {}",
        config.gpu_usage.method.as_config_value()
    );
    info!(
        "GPU set method configured: {}",
        config.gpu.set_method.as_config_value()
    );

    let mut gpu = GPU::new(
        config.safe_points,
        config.gpu.set_method,
        config.gpu_usage.method,
        config.timing.sampling_interval,
    )?;

    let mut curr_freq = gpu.get_freq()?;
    let mut target_freq = gpu.min_freq;
    let mut status = 0;
    let mut usage_fix_cycle = 0;
    let mut max_freq = gpu.max_freq;

    gpu.change_freq(target_freq)?;

    let adjustment_millis = config.timing.adjustment_interval.as_millis() as f32;
    let burst_freq_step = (config.timing.ramp_rate_burst * adjustment_millis) as u32;
    let freq_step = (config.timing.ramp_rate * adjustment_millis) as u32;

    info!("freq min {} max {}", gpu.min_freq, max_freq);

    loop {
        match shutdown_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        let loop_start = Instant::now();
        let (average_load, burst_length) = gpu.poll_and_get_load()?;

        flush_gpu_usage_fix(
            gpu_usage_fix.as_mut(),
            &mut usage_fix_cycle,
            config.gpu_usage.flush_every,
            average_load,
        );

        let temp = update_max_freq_for_temperature(
            &mut gpu,
            &config.temperature,
            &config.frequency_thresholds,
            &mut max_freq,
            freq_step,
        )?;

        let burst = is_burst(&config.timing, average_load, burst_length);
        if burst {
            target_freq += burst_freq_step;
        } else {
            if average_load > config.load_target.up_thresh && status <= UP_EVENTS {
                status += UP_EVENTS;
            } else if average_load < config.load_target.down_thresh && curr_freq > gpu.min_freq {
                status -= 1;
            } else if status < 0 {
                status += 1;
            } else if status > 0 {
                status -= 1;
            }

            if status <= -config.timing.down_events {
                target_freq -= freq_step;
            } else if status >= UP_EVENTS {
                target_freq += freq_step;
            }
        }

        target_freq = target_freq.clamp(gpu.min_freq, max_freq);

        let hit_bounds = target_freq == gpu.min_freq || target_freq == max_freq;
        let big_change =
            curr_freq.abs_diff(target_freq) >= config.frequency_thresholds.significant_change;
        let should_apply_change = curr_freq != target_freq && (burst || hit_bounds || big_change);

        if should_apply_change {
            debug!(
                "freq curr {} target {} temp {} status {} de {} load {:.2} bl {}",
                curr_freq,
                target_freq,
                temp,
                status,
                config.timing.down_events,
                average_load,
                burst_length
            );

            gpu.change_freq(target_freq)?;
            status = 0;
            curr_freq = target_freq;
        }

        sleep_remaining(loop_start, config.timing.adjustment_interval);
    }

    info!("Shutting down gracefully...");
    shutdown_gpu_usage_fix(gpu_usage_fix.as_mut());
    shutdown_gpu(&mut gpu);
    Ok(())
}

fn init_logger(verbose: bool) {
    let default_filter = if verbose { "debug" } else { "info" };
    let env = env_logger::Env::default().default_filter_or(default_filter);
    let _ = env_logger::Builder::from_env(env)
        .format_timestamp_millis()
        .try_init();
}

fn install_signal_handler(shutdown_tx: Sender<()>) -> Result<()> {
    let mut signals = Signals::new(&[SIGINT, SIGTERM])?;
    std::thread::spawn(move || {
        for _sig in signals.forever() {
            let _ = shutdown_tx.send(());
        }
    });

    Ok(())
}

fn load_config(config_path: Option<&str>) -> Result<Config> {
    let config_text = config_path
        .map(std::fs::read_to_string)
        .unwrap_or_else(|| Ok(String::new()));
    Config::new(config_text)
}

fn start_gpu_usage_fix(enabled: bool) -> Result<Option<GpuUsageFix>> {
    if enabled {
        Ok(Some(GpuUsageFix::start()?))
    } else {
        info!("GPU usage metrics fix disabled by config");
        Ok(None)
    }
}

fn flush_gpu_usage_fix(
    gpu_usage_fix: Option<&mut GpuUsageFix>,
    usage_fix_cycle: &mut u32,
    flush_every: u32,
    average_load: f32,
) {
    let Some(fix) = gpu_usage_fix else {
        return;
    };

    *usage_fix_cycle = usage_fix_cycle.saturating_add(1);
    if *usage_fix_cycle < flush_every {
        return;
    }

    *usage_fix_cycle = 0;
    if let Err(err) = fix.set_usage_percent(average_load * 100.0) {
        error!("GPU usage metrics fix write failed: {err}");
    }
}

fn is_burst(timing: &TimingConfig, average_load: f32, burst_length: u32) -> bool {
    average_load >= 0.99
        || timing
            .burst_samples
            .is_some_and(|burst_samples| burst_length >= burst_samples)
}

fn update_max_freq_for_temperature(
    gpu: &mut GPU,
    temperature: &config::TemperatureConfig,
    frequency_thresholds: &config::FrequencyThresholdConfig,
    max_freq: &mut u32,
    freq_step: u32,
) -> Result<u32> {
    let temp = gpu.read_temperature()?;

    if let Some(max_temp) = temperature.throttling_temp {
        if temp > max_temp && *max_freq >= gpu.min_freq + freq_step {
            *max_freq -= frequency_thresholds.significant_change;
            debug!("throttling temp {temp} freq {}", *max_freq);
        } else if let Some(recovery_temp) = temperature.throttling_recovery_temp
            && temp < recovery_temp
            && *max_freq != gpu.max_freq
        {
            *max_freq = gpu.max_freq;
            debug!("recover throttling temp {temp} freq {}", *max_freq);
        }
    }

    Ok(temp)
}

fn sleep_remaining(loop_start: Instant, adjustment_interval: Duration) {
    let elapsed = loop_start.elapsed();
    if elapsed < adjustment_interval {
        std::thread::sleep(adjustment_interval - elapsed);
    }
}

fn shutdown_gpu_usage_fix(gpu_usage_fix: Option<&mut GpuUsageFix>) {
    if let Some(fix) = gpu_usage_fix
        && let Err(err) = fix.shutdown()
    {
        error!("GPU usage metrics fix cleanup failed: {err}");
    }
}

fn shutdown_gpu(gpu: &mut GPU) {
    if let Err(err) = gpu.shutdown() {
        error!("System exit restore failed: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::is_burst;
    use crate::config::{
        Config, FrequencyThresholdConfig, GpuConfig, GpuSetMethod, GpuUsageConfig, GpuUsageMethod,
        LoadTargetConfig, TemperatureConfig, TimingConfig,
    };
    use std::{collections::BTreeMap, time::Duration};

    fn test_config() -> Config {
        Config {
            timing: TimingConfig {
                sampling_interval: Duration::from_millis(2),
                adjustment_interval: Duration::from_millis(20),
                ramp_rate: 1.0,
                ramp_rate_burst: 4.0,
                burst_samples: Some(48),
                down_events: 3,
            },
            load_target: LoadTargetConfig {
                up_thresh: 0.95,
                down_thresh: 0.80,
            },
            frequency_thresholds: FrequencyThresholdConfig {
                significant_change: 10,
            },
            temperature: TemperatureConfig {
                throttling_temp: None,
                throttling_recovery_temp: None,
            },
            safe_points: BTreeMap::from([(350, 700), (2000, 1000)]),
            gpu_usage: GpuUsageConfig {
                fix_metrics: false,
                flush_every: 10,
                method: GpuUsageMethod::BusyFlag,
            },
            gpu: GpuConfig {
                set_method: GpuSetMethod::Smu,
            },
        }
    }

    #[test]
    fn burst_triggers_on_high_average_load() {
        let config = test_config();

        assert!(is_burst(&config.timing, 0.99, 0));
    }

    #[test]
    fn burst_triggers_on_burst_length_threshold() {
        let config = test_config();

        assert!(is_burst(&config.timing, 0.50, 48));
        assert!(!is_burst(&config.timing, 0.50, 47));
    }

    #[test]
    fn burst_is_disabled_when_threshold_is_missing() {
        let mut config = test_config();
        config.timing.burst_samples = None;

        assert!(!is_burst(&config.timing, 0.50, 64));
    }

    #[test]
    fn frequency_change_is_applied_for_burst_even_without_big_change() {
        let curr_freq: u32 = 1000;
        let target_freq: u32 = 1001;
        let burst = true;
        let min_freq: u32 = 350;
        let max_freq: u32 = 2000;
        let significant_change: u32 = 10;
        let hit_bounds = target_freq == min_freq || target_freq == max_freq;
        let big_change = curr_freq.abs_diff(target_freq) >= significant_change;

        assert!(curr_freq != target_freq && (burst || hit_bounds || big_change));
    }

    #[test]
    fn frequency_change_is_applied_when_target_hits_bound() {
        let curr_freq: u32 = 1000;
        let significant_change: u32 = 10;

        {
            let target_freq: u32 = 350;
            let burst = false;
            let hit_bounds = true;
            let big_change = curr_freq.abs_diff(target_freq) >= significant_change;
            assert!(curr_freq != target_freq && (burst || hit_bounds || big_change));
        }

        {
            let target_freq: u32 = 2000;
            let burst = false;
            let hit_bounds = true;
            let big_change = curr_freq.abs_diff(target_freq) >= significant_change;
            assert!(curr_freq != target_freq && (burst || hit_bounds || big_change));
        }
    }

    #[test]
    fn frequency_change_requires_reason_when_not_bursting() {
        let curr_freq: u32 = 1000;
        let min_freq: u32 = 350;
        let max_freq: u32 = 2000;
        let burst = false;
        let significant_change: u32 = 10;

        {
            let target_freq: u32 = 1005;
            let hit_bounds = target_freq == min_freq || target_freq == max_freq;
            let big_change = curr_freq.abs_diff(target_freq) >= significant_change;
            assert!(!(curr_freq != target_freq && (burst || hit_bounds || big_change)));
        }

        {
            let target_freq: u32 = 1010;
            let hit_bounds = target_freq == min_freq || target_freq == max_freq;
            let big_change = curr_freq.abs_diff(target_freq) >= significant_change;
            assert!(curr_freq != target_freq && (burst || hit_bounds || big_change));
        }

        {
            let target_freq: u32 = 1000;
            let hit_bounds = target_freq == min_freq || target_freq == max_freq;
            let big_change = curr_freq.abs_diff(target_freq) >= significant_change;
            assert!(!(curr_freq != target_freq && (burst || hit_bounds || big_change)));
        }
    }
}
