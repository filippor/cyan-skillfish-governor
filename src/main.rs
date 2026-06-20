mod app_error;
mod config;
mod dbus;
mod governor;
mod gpu;
mod gpu_usage_fix;
use app_error::Result;
use clap::Parser;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::{Config, GovernorParams};
use dbus::PerformanceModeCommand;
use governor::Governor;
use gpu::GPU;
use gpu_usage_fix::GpuUsageFix;
use log::{info, warn};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::sync::mpsc::{self, Sender, TryRecvError};
use std::sync::{Arc, Mutex};

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

    let gpu = GPU::new(
        config.safe_points.clone(),
        config.gpu.set_method,
        config.gpu_usage.method,
        config.timing.sampling_interval,
    )?;
    let params: GovernorParams = config.to_governor_params(&gpu);

    let gpu_usage_fix = if config.gpu_usage.fix_metrics {
        info!("GPU usage metrics fix enabled");
        Some(GpuUsageFix::start(gpu.get_sysfs_path())?)
    } else {
        None
    };

    let governor = Arc::new(Mutex::new(Governor::new(params, gpu, gpu_usage_fix)?));

    let mut dbus_rx = if config.dbus.enabled {
        info!("D-Bus service listening enabled");
        let handle = dbus::DbusService::start(Arc::clone(&governor))?;
        Some(handle.command_rx)
    } else {
        info!("D-Bus service listening disabled in configuration");
        None
    };

    loop {
        match shutdown_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        let mut dbus_channel_closed = false;

        if let Some(rx) = dbus_rx.as_ref() {
            match rx.try_recv() {
                Ok(command) => match command {
                    PerformanceModeCommand::Enable => governor
                        .lock()
                        .expect("governor lock poisoned")
                        .apply_enable_performance_mode_command(true),
                    PerformanceModeCommand::Disable => governor
                        .lock()
                        .expect("governor lock poisoned")
                        .apply_enable_performance_mode_command(false),
                    PerformanceModeCommand::SetFixedFrequency(frequency) => governor
                        .lock()
                        .expect("governor lock poisoned")
                        .apply_fixed_frequency_command(frequency),
                    PerformanceModeCommand::SetParameters {
                        min_freq,
                        max_freq,
                        load_min,
                        load_max,
                        throttling_temp,
                        recovery_temp,
                    } => {
                        let mut governor = governor.lock().expect("governor lock poisoned");
                        governor.apply_range_command(min_freq, max_freq);
                        governor.apply_load_target_command(load_min, load_max)?;
                        governor.apply_temperature_thresholds_command(
                            throttling_temp.unwrap_or(0),
                            recovery_temp.unwrap_or(0),
                        )?;
                    }
                    PerformanceModeCommand::SetRange(min, max) => governor
                        .lock()
                        .expect("governor lock poisoned")
                        .apply_range_command(min, max),
                    PerformanceModeCommand::SetLoadTarget(min, max) => governor
                        .lock()
                        .expect("governor lock poisoned")
                        .apply_load_target_command(min, max)?,
                    PerformanceModeCommand::SetTemperatureThresholds(throttling, recovery) => {
                        governor
                            .lock()
                            .expect("governor lock poisoned")
                            .apply_temperature_thresholds_command(throttling, recovery)?
                    }
                    PerformanceModeCommand::SetTestMode(frequency, voltage) => governor
                        .lock()
                        .expect("governor lock poisoned")
                        .apply_test_mode_command(frequency, voltage)?,
                },
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => dbus_channel_closed = true,
            }
        }

        if dbus_channel_closed {
            warn!("D-Bus command channel closed; disabling D-Bus command handling");
            dbus_rx = None;
        }

        governor
            .lock()
            .expect("governor lock poisoned")
            .run_iteration()?;
    }

    info!("Shutting down gracefully...");
    governor
        .lock()
        .expect("governor lock poisoned")
        .shutdown()?;
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
