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
use log::{error, info};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::sync::mpsc::{self, Sender, TryRecvError};

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

    let dbus_rx = if config.dbus.enabled {
        info!("D-Bus service listening enabled");
        Some(dbus::DbusService::start(&params)?)
    } else {
        info!("D-Bus service listening disabled in configuration");
        None
    };

    let gpu_usage_fix = if config.gpu_usage.fix_metrics {
        info!("GPU usage metrics fix enabled");
        Some(GpuUsageFix::start(gpu.get_sysfs_path())?)
    } else {
        None
    };

    let mut governor = Governor::new(params, gpu, gpu_usage_fix)?;

    loop {
        match shutdown_rx.try_recv() {
            Ok(()) | Err(TryRecvError::Disconnected) => break,
            Err(TryRecvError::Empty) => {}
        }

        if let Some(rx) = dbus_rx.as_ref() {
            match rx.try_recv() {
                Ok(command) => match command {
                    PerformanceModeCommand::Enable => governor.apply_enable_command(),
                    PerformanceModeCommand::Disable => governor.apply_disable_command(),
                    PerformanceModeCommand::SetFixedFrequency(frequency) => {
                        governor.apply_fixed_frequency_command(frequency)
                    }
                    PerformanceModeCommand::SetRange(min, max) => {
                        governor.apply_range_command(min, max)
                    }
                    PerformanceModeCommand::SetLoadTarget(min, max) => {
                        governor.apply_load_target_command(min, max)?
                    }
                    PerformanceModeCommand::SetTemperatureThresholds(throttling, recovery) => {
                        governor.apply_temperature_thresholds_command(throttling, recovery)?
                    }
                    PerformanceModeCommand::SetTestMode(frequency, voltage) => {
                        governor.apply_test_mode_command(frequency, voltage)?
                    }
                },
                Err(TryRecvError::Empty) => {}
                Err(err) => error!("D-Bus command receive failed: {err}"),
            }
        }

        governor.run_iteration()?;
    }

    info!("Shutting down gracefully...");
    governor.shutdown()?;
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
