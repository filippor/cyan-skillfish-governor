mod app_error;
mod config;
mod dbus;
mod gpu;
mod gpu_usage_fix;
use app_error::{AppError, Result};
use clap::Parser;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::{Config, TimingConfig};
use dbus::PerformanceModeCommand;
use gpu::GPU;
use gpu_usage_fix::GpuUsageFix;
use log::{debug, error, info};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::ops::RangeInclusive;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::time::{Duration, Instant};

const UP_EVENTS: i16 = 2;
const BUILD_VERSION: &str = env!("GIT_VERSION");

#[derive(Debug, Parser)]
#[command(name = "cyan-skillfish-governor-smu", version = BUILD_VERSION)]
#[command(about = "GPU frequency governor for AMD Cyan Skillfish APU")]
#[command(
    long_about = "Adaptive GPU frequency governor for AMD Cyan Skillfish APU\n\nFor detailed documentation and configuration options, see:\nhttps://github.com/filippor/cyan-skillfish-governor/blob/smu/README.md"
)]
struct Args {
    #[command(flatten)]
    verbose: Verbosity<InfoLevel>,
    #[arg(value_name = "CONFIG")]
    config_path: Option<String>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    init_logger(args.verbose);

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    install_signal_handler(shutdown_tx)?;
    let config = load_config(args.config_path.as_deref())?;

    // Start D-Bus service only if enabled in config
    let dbus_rx = if config.dbus.enabled {
        info!("D-Bus service listening enabled");
        Some(dbus::DbusService::start()?)
    } else {
        info!("D-Bus service listening disabled in configuration");
        None
    };

    let mut gpu = GPU::new(
        config.safe_points,
        config.gpu.set_method,
        config.gpu_usage.method,
        config.timing.sampling_interval,
    )?;

    let mut gpu_usage_fix = if config.gpu_usage.fix_metrics {
        info!("GPU usage metrics fix enabled");
        Some(GpuUsageFix::start(gpu.get_sysfs_path())?)
    } else {
        None
    };

    let default_allowed_range = gpu.min_freq..=gpu.max_freq;

    let mut curr_freq = gpu.get_freq()?;
    let mut target_freq = gpu.min_freq;
    let mut status = 0;
    let mut usage_fix_cycle = 0;
    let mut max_freq = gpu.max_freq;
    let mut allowed_range = gpu.min_freq..=gpu.max_freq;
    let mut performance_mode = false;

    let adjustment_millis = config.timing.adjustment_interval.as_millis() as f32;
    let burst_freq_step = (config.timing.ramp_rate_burst * adjustment_millis) as u32;
    let freq_step = (config.timing.ramp_rate * adjustment_millis) as u32;

    info!("freq min {} max {}", gpu.min_freq, max_freq);

    loop {
        let loop_start = Instant::now();
        let should_apply_change;
        match shutdown_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        if let Some(rx) = dbus_rx.as_ref() {
            match handle_dbus_performance_mode_command(rx, default_allowed_range.clone()) {
                Ok((new_performance_mode, new_allowed_range)) => {
                    performance_mode = new_performance_mode;
                    allowed_range = new_allowed_range;
                }
                Err(AppError::TryRecv(TryRecvError::Empty)) => {}
                Err(err) => {
                    error!("D-Bus command handling failed, keeping previous state: {err}");
                }
            }
        }

        let (average_load, burst_length) = if !performance_mode || gpu_usage_fix.is_some() {
            gpu.poll_and_get_load()?
        } else {
            (1.0, 0)
        };

        flush_gpu_usage_fix(
            gpu_usage_fix.as_mut(),
            &mut usage_fix_cycle,
            config.gpu_usage.flush_every,
            performance_mode,
            average_load,
        );

        let temp = update_max_freq_for_temperature(
            &mut gpu,
            &config.temperature,
            config.frequency_thresholds.significant_change,
            *allowed_range.end(),
            &mut max_freq,
        )?;

        if performance_mode {
            target_freq = max_freq;
            should_apply_change = true;
        } else {
            // Normal adaptive frequency control
            let burst = {
                let timing: &TimingConfig = &config.timing;
                average_load >= 0.99
                    || timing
                        .burst_samples
                        .is_some_and(|burst_samples| burst_length >= burst_samples)
            };
            if burst {
                target_freq += burst_freq_step;
            } else {
                if average_load > config.load_target.up_thresh && status <= UP_EVENTS {
                    status += UP_EVENTS;
                } else if average_load < config.load_target.down_thresh
                    && curr_freq > *allowed_range.start()
                {
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

            target_freq = target_freq.clamp(*allowed_range.start(), max_freq);

            let hit_bounds = target_freq == *allowed_range.start() || target_freq == max_freq;
            let big_change =
                curr_freq.abs_diff(target_freq) >= config.frequency_thresholds.significant_change;

            should_apply_change = curr_freq != target_freq && (burst || hit_bounds || big_change);
        }

        if should_apply_change {
            debug!(
                "freq curr {} target {} temp {} load {:.2} status {}  burst_length {}, performance_mode {}",
                curr_freq, target_freq, temp, average_load, status, burst_length, performance_mode
            );
            gpu.change_freq(target_freq)?;
            status = 0;
            curr_freq = target_freq;
        }

        sleep_for_next_cycle(
            config.timing.adjustment_interval,
            config.gpu_usage.flush_every,
            performance_mode,
            loop_start,
        );
    }

    info!("Shutting down gracefully...");
    if let Some(fix) = gpu_usage_fix
        && let Err(err) = fix.shutdown()
    {
        error!("GPU usage metrics fix cleanup failed: {err}");
    }
    if let Err(err) = gpu.shutdown() {
        error!("System exit restore failed: {err}");
    }
    Ok(())
}

fn init_logger(verbose: Verbosity<InfoLevel>) {
    let _ = env_logger::Builder::new()
        .filter_level(verbose.log_level_filter())
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

fn sleep_for_next_cycle(
    adjustment_interval: Duration,
    flush_every: u32,
    performance_mode: bool,
    loop_start: Instant,
) {
    let target_cycle_interval = if performance_mode {
        adjustment_interval
            .checked_mul(flush_every.max(1))
            .unwrap_or(Duration::MAX)
    } else {
        adjustment_interval
    };

    let elapsed = loop_start.elapsed();
    if elapsed < target_cycle_interval {
        std::thread::sleep(target_cycle_interval - elapsed);
    }
}

fn handle_dbus_performance_mode_command(
    dbus_rx: &mpsc::Receiver<PerformanceModeCommand>,
    allowed_freq_range: RangeInclusive<u32>,
) -> Result<(bool, RangeInclusive<u32>)> {
    let command = dbus_rx.try_recv()?;

    match command {
        PerformanceModeCommand::Enable => {
            info!("Performance mode enabled");
            Ok((true, allowed_freq_range))
        }
        PerformanceModeCommand::Disable => {
            info!("Performance mode disabled: reverting to adaptive frequency control");
            Ok((false, allowed_freq_range))
        }
        PerformanceModeCommand::SetFixedFrequency(frequency) => {
            info!(
                "Performance mode enabled with fixed frequency request: {}",
                frequency
            );
            Ok((true, *allowed_freq_range.start()..=frequency))
        }
        PerformanceModeCommand::SetRange(min, max) => match (min, max) {
            (0, 0) => {
                info!("Frequency range cleared: both limits removed");
                Ok((false, allowed_freq_range.clone()))
            }
            (0, ma) if allowed_freq_range.contains(&ma) => {
                info!("Upper limit set to {} MHz, lower limit removed", ma);
                Ok((false, *allowed_freq_range.start()..=ma))
            }
            (mi, 0) if allowed_freq_range.contains(&mi) => {
                info!("Lower limit set to {} MHz, upper limit removed", mi);
                Ok((false, mi..=*allowed_freq_range.end()))
            }
            (mi, ma) if allowed_freq_range.contains(&mi) && allowed_freq_range.contains(&ma) => {
                info!("Frequency range set: min={} MHz, max={} MHz", mi, ma);
                Ok((false, mi..=ma))
            }
            (mi, ma) => {
                Err(format!(
                    "Invalid frequency range request: min={} MHz, max={} MHz (allowed {}..={} MHz)",
                    mi,
                    ma,
                    allowed_freq_range.start(),
                    *allowed_freq_range.end()
                )
                .into())
            }
        },
    }
}

fn flush_gpu_usage_fix(
    gpu_usage_fix: Option<&mut GpuUsageFix>,
    usage_fix_cycle: &mut u32,
    flush_every: u32,
    performance_mode: bool,
    average_load: f32,
) {
    let Some(fix) = gpu_usage_fix else {
        return;
    };

    if !performance_mode {
        *usage_fix_cycle = usage_fix_cycle.saturating_add(1);
        if *usage_fix_cycle < flush_every {
            return;
        }
    }

    *usage_fix_cycle = 0;
    if let Err(err) = fix.set_usage_percent(average_load * 100.0) {
        error!("GPU usage metrics fix write failed: {err}");
    }
}

fn update_max_freq_for_temperature(
    gpu: &mut GPU,
    temperature: &config::TemperatureConfig,
    step_down: u32,
    recover_freq: u32,
    max_freq: &mut u32,
) -> Result<u32> {
    let temp = gpu.read_temperature()?;

    if let Some(max_temp) = temperature.throttling_temp {
        if temp > max_temp {
            *max_freq -= step_down;
            debug!("throttling temp {temp} freq {}", *max_freq);
        } else if let Some(recovery_temp) = temperature.throttling_recovery_temp
            && temp < recovery_temp
            && *max_freq != recover_freq
        {
            *max_freq = recover_freq;
            debug!("recover throttling temp {temp} freq {}", *max_freq);
        }
    }

    Ok(temp)
}
