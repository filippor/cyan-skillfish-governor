mod app_error;
mod config;
mod dbus;
mod gpu;
mod gpu_usage_fix;
use app_error::Result;
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

struct GovernorState {
    curr_freq: u32,
    target_freq: u32,
    status: i16,
    usage_fix_cycle: u32,
    max_freq: u32,
    requested_range: RangeInclusive<u32>,
    performance_mode: bool,
}

fn main() -> Result<()> {
    let args = Args::parse();
    init_logger(args.verbose);

    let (shutdown_tx, shutdown_rx) = mpsc::channel::<()>();
    install_signal_handler(shutdown_tx)?;
    let config = load_config(args.config_path.as_deref())?;

    let adjustment_millis = config.timing.adjustment_interval.as_millis() as f32;
    let burst_freq_step = (config.timing.ramp_rate_burst * adjustment_millis) as u32;
    let freq_step = (config.timing.ramp_rate * adjustment_millis) as u32;

    let mut gpu = GPU::new(
        config.safe_points,
        config.gpu.set_method,
        config.gpu_usage.method,
        config.timing.sampling_interval,
    )?;
    let allowed_frequency_range = gpu.min_freq..=gpu.max_freq;
    let initial_frequency_range = {
        let min = config.frequency_range.min.unwrap_or(gpu.min_freq).clamp(
            *allowed_frequency_range.start(),
            *allowed_frequency_range.end(),
        );
        let max = config.frequency_range.max.unwrap_or(gpu.max_freq).clamp(
            *allowed_frequency_range.start(),
            *allowed_frequency_range.end(),
        );
        min..=max
    };

    let dbus_rx = if config.dbus.enabled {
        info!("D-Bus service listening enabled");
        Some(dbus::DbusService::start(&allowed_frequency_range)?)
    } else {
        info!("D-Bus service listening disabled in configuration");
        None
    };

    let mut gpu_usage_fix = if config.gpu_usage.fix_metrics {
        info!("GPU usage metrics fix enabled");
        Some(GpuUsageFix::start(gpu.get_sysfs_path())?)
    } else {
        None
    };

    let mut state = GovernorState {
        curr_freq: gpu.get_freq()?,
        target_freq: gpu.min_freq,
        status: 0,
        usage_fix_cycle: 0,
        max_freq: gpu.max_freq,
        requested_range: initial_frequency_range.clone(),
        performance_mode: false,
    };

    info!(
        "allowed frequency range {}..={}",
        gpu.min_freq, gpu.max_freq
    );

    info!(
        "initial frequency range: {}..={}",
        initial_frequency_range.start(),
        initial_frequency_range.end()
    );

    loop {
        let loop_start = Instant::now();
        let should_apply_change;

        match shutdown_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        if let Some(rx) = dbus_rx.as_ref() {
            apply_dbus_command(
                &mut state,
                rx,
                &allowed_frequency_range,
                &initial_frequency_range,
            );
        }

        let (average_load, burst_length) = if !state.performance_mode || gpu_usage_fix.is_some() {
            gpu.poll_and_get_load()?
        } else {
            (1.0, 0)
        };

        if let Some(fix) = gpu_usage_fix.as_mut() {
            state.usage_fix_cycle = state.usage_fix_cycle + 1;
            if state.performance_mode || state.usage_fix_cycle >= config.gpu_usage.flush_every {
                if let Err(err) = fix.set_usage_percent(average_load * 100.0) {
                    error!("GPU usage metrics fix write failed: {err}");
                }
            }
        }

        let temp = update_max_freq_for_temperature(
            &mut state,
            &mut gpu,
            &config.temperature,
            config.frequency_thresholds.significant_change,
            &allowed_frequency_range.start(),
        )?;

        if state.performance_mode {
            state.target_freq = state.max_freq;
            should_apply_change = state.curr_freq != state.target_freq;
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
                state.target_freq += burst_freq_step;
            } else {
                if average_load > config.load_target.up_thresh && state.status <= UP_EVENTS {
                    state.status += UP_EVENTS;
                } else if average_load < config.load_target.down_thresh
                    && state.curr_freq > *state.requested_range.start()
                {
                    state.status -= 1;
                } else if state.status < 0 {
                    state.status += 1;
                } else if state.status > 0 {
                    state.status -= 1;
                }

                if state.status <= -config.timing.down_events {
                    state.target_freq -= freq_step;
                } else if state.status >= UP_EVENTS {
                    state.target_freq += freq_step;
                }
            }

            state.target_freq = state
                .target_freq
                .clamp(*state.requested_range.start(), state.max_freq);

            let hit_bounds = state.target_freq == *state.requested_range.start()
                || state.target_freq == state.max_freq;
            let big_change = state.curr_freq.abs_diff(state.target_freq)
                >= config.frequency_thresholds.significant_change;

            should_apply_change =
                state.curr_freq != state.target_freq && (burst || hit_bounds || big_change);
        }

        if should_apply_change {
            debug!(
                "freq curr {} target {} temp {} load {:.2} status {}  burst_length {}, performance_mode {}",
                state.curr_freq,
                state.target_freq,
                temp,
                average_load,
                state.status,
                burst_length,
                state.performance_mode
            );
            gpu.change_freq(state.target_freq)?;
            state.status = 0;
            state.curr_freq = state.target_freq;
        }

        let target_cycle_interval = if state.performance_mode {
            config
                .timing
                .adjustment_interval
                .checked_mul(config.gpu_usage.flush_every.max(1))
                .unwrap_or(Duration::MAX)
        } else {
            config.timing.adjustment_interval
        };
        let elapsed = loop_start.elapsed();
        if elapsed < target_cycle_interval {
            std::thread::sleep(target_cycle_interval - elapsed);
        }
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

fn apply_dbus_command(
    state: &mut GovernorState,
    rx: &mpsc::Receiver<PerformanceModeCommand>,
    allowed_frequency_range: &RangeInclusive<u32>,
    initial_frequency_range: &RangeInclusive<u32>,
) {
    match rx.try_recv() {
        Ok(PerformanceModeCommand::Enable) => {
            info!("Performance mode enabled");
            state.performance_mode = true;
            state.requested_range = allowed_frequency_range.clone();
            info!(
                "Updated performance mode: {} range : {}..={}",
                state.performance_mode,
                state.requested_range.start(),
                state.requested_range.end()
            );
        }
        Ok(PerformanceModeCommand::Disable) => {
            info!("Performance mode disabled: reverting to adaptive frequency control");
            state.performance_mode = false;
            state.requested_range = initial_frequency_range.clone();
            info!(
                "Updated performance mode: {} range : {}..={}",
                state.performance_mode,
                state.requested_range.start(),
                state.requested_range.end()
            );
        }
        Ok(PerformanceModeCommand::SetFixedFrequency(frequency)) => {
            info!(
                "Performance mode enabled with fixed frequency request: {}",
                frequency
            );
            state.performance_mode = true;
            state.requested_range = *initial_frequency_range.start()..=frequency;
        }
        Ok(PerformanceModeCommand::SetRange(min, max)) => match (min, max) {
            (0, 0) => {
                info!("Frequency range cleared: both limits removed");
                state.performance_mode = false;
                state.requested_range = initial_frequency_range.clone();
                info!(
                    "Updated range : {}..={}",
                    state.requested_range.start(),
                    state.requested_range.end()
                );
            }
            (0, ma) if allowed_frequency_range.contains(&ma) => {
                info!("Upper limit set to {} MHz, lower limit removed", ma);
                state.performance_mode = false;
                state.requested_range = *initial_frequency_range.start()..=ma;
                info!(
                    "Updated range : {}..={}",
                    state.requested_range.start(),
                    state.requested_range.end()
                );
            }
            (mi, 0) if allowed_frequency_range.contains(&mi) => {
                info!("Lower limit set to {} MHz, upper limit removed", mi);
                state.performance_mode = false;
                state.requested_range = mi..=*initial_frequency_range.end();
                info!(
                    "Updated range : {}..={}",
                    state.requested_range.start(),
                    state.requested_range.end()
                );
            }
            (mi, ma)
                if allowed_frequency_range.contains(&mi)
                    && allowed_frequency_range.contains(&ma) =>
            {
                info!("Frequency range set: min={} MHz, max={} MHz", mi, ma);
                state.performance_mode = false;
                state.requested_range = mi..=ma;
                info!(
                    "Updated range : {}..={}",
                    state.requested_range.start(),
                    state.requested_range.end()
                );
            }
            (mi, ma) => {
                error!(
                    "D-Bus command handling failed, keeping previous state: Invalid frequency range request: min={} MHz, max={} MHz (allowed {}..={} MHz)",
                    mi,
                    ma,
                    *allowed_frequency_range.start(),
                    *allowed_frequency_range.end()
                );
            }
        },
        Err(TryRecvError::Empty) => {}
        Err(err) => {
            error!("D-Bus command handling failed, keeping previous state: {err}");
            info!(
                "performance mode: {} range : {}..={}",
                state.performance_mode,
                state.requested_range.start(),
                state.requested_range.end()
            );
        }
    }
}

fn update_max_freq_for_temperature(
    state: &mut GovernorState,
    gpu: &mut GPU,
    temperature: &config::TemperatureConfig,
    step_down: u32,
    min_freq: &u32,
) -> Result<u32> {
    let temp = gpu.read_temperature()?;

    if let Some(max_temp) = temperature.throttling_temp {
        if temp > max_temp && state.max_freq - step_down > *min_freq {
            state.max_freq -= step_down;
            debug!("throttling temp {temp} freq {}", state.max_freq);
        } else if let Some(recovery_temp) = temperature.throttling_recovery_temp
            && temp < recovery_temp
            && state.max_freq != *state.requested_range.end()
        {
            state.max_freq = *state.requested_range.end();
            debug!("recover throttling temp {temp} freq {}", state.max_freq);
        }
    }

    Ok(temp)
}
