mod app_error;
mod bind_overlay;
mod config;
mod dbus;
mod governor;
mod gpu;
mod gpu_frequency_fix;
mod gpu_usage_fix;
use app_error::{AppError, Result};
use clap::Parser;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::{Config, GovernorParams};
use governor::Governor;
use gpu::GPU;
use gpu_frequency_fix::GpuFrequencyFix;
use gpu_usage_fix::GpuUsageFix;
use log::{error, info};
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

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
        config.gpu_usage.temp_read,
        config.timing.sampling_interval,
    )?;
    let params: GovernorParams = config.to_governor_params(&gpu);

    let gpu_usage_fix = if config.gpu_usage.fix_metrics {
        info!("GPU usage metrics fix enabled");
        Some(GpuUsageFix::start(gpu.get_sysfs_path())?)
    } else {
        None
    };
    let gpu_frequency_fix = if config.gpu_usage.fix_freq {
        info!("GPU frequency fix enabled");
        Some(GpuFrequencyFix::start(gpu.get_sysfs_path())?)
    } else {
        None
    };

    let governor = Arc::new(Mutex::new(Governor::new(
        params,
        gpu,
        gpu_usage_fix,
        gpu_frequency_fix,
    )?));

    if config.dbus.enabled {
        info!("D-Bus service listening enabled");
        dbus::DbusService::start(Arc::clone(&governor))?;
    } else {
        info!("D-Bus service listening disabled in configuration");
    }

    let result = run_control_loop(&governor, &shutdown_rx);

    // Also on the error path: with set-method = "smu", shutdown() is what sends
    // UnforceGfxFreq/UnforceGfxVid, and a governor that exits on an error would
    // otherwise leave the forced clock and voltage held until the service
    // restart re-runs SmuFreqStrategy::new. The loop's error stays the exit
    // status; a shutdown error on top of it is logged rather than substituted.
    info!("Shutting down gracefully...");
    let shutdown = match governor.lock() {
        Ok(mut governor) => governor.shutdown(),
        Err(_) => Err(AppError::from("governor lock poisoned")),
    };
    match (result, shutdown) {
        (Ok(()), shutdown) => shutdown,
        (Err(loop_err), Err(shutdown_err)) => {
            error!("shutdown after a failed control loop also failed: {shutdown_err}");
            Err(loop_err)
        }
        (result, Ok(())) => result,
    }
}

/// The control loop, until a signal or the first error. Separate from main so
/// the shutdown runs whichever way it ends.
fn run_control_loop(governor: &Arc<Mutex<Governor>>, shutdown_rx: &Receiver<()>) -> Result<()> {
    loop {
        let loop_start = Instant::now();

        if shutdown_rx.try_recv().is_ok() {
            return Ok(());
        }

        governor
            .lock()
            .map_err(|_| AppError::from("governor lock poisoned"))?
            .run_iteration()?;

        let target_cycle_interval = governor
            .lock()
            .map_err(|_| AppError::from("governor lock poisoned"))?
            .target_cycle_interval();
        let elapsed = loop_start.elapsed();
        if elapsed < target_cycle_interval {
            std::thread::sleep(target_cycle_interval - elapsed);
        }
    }
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
