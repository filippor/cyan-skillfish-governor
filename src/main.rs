mod app_error;
mod bind_overlay;
mod config;
mod dbus;
mod governor;
mod gpu;
mod gpu_frequency_fix;
mod gpu_usage_fix;
mod memory_fabric_profile;
use app_error::{AppError, Result};
use clap::Parser;
use clap_verbosity_flag::{InfoLevel, Verbosity};
use config::{Config, GovernorParams};
use governor::Governor;
use gpu::GPU;
use gpu_frequency_fix::GpuFrequencyFix;
use gpu_usage_fix::GpuUsageFix;
use log::info;
use log::warn;
use memory_fabric_profile::MemoryFabricProfile;
use signal_hook::consts::signal::*;
use signal_hook::iterator::Signals;
use std::sync::mpsc::{self, Sender};
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

    if config.memory_fabric_profile.is_some() {
        warn!(
            "Experimental memory fabric profiles are enabled; SMU queue 3 message 0x1E is not fully understood and may cause a hardware reset"
        );
    }

    let gpu = GPU::new(
        config.safe_points.clone(),
        config.gpu.set_method,
        config.gpu_usage.method,
        config.timing.sampling_interval,
        config.memory_fabric_profile.is_some(),
    )?;
    let params: GovernorParams = config.to_governor_params(&gpu);
    let mut memory_fabric_profile = config.memory_fabric_profile.map(|profile| {
        MemoryFabricProfile::new(
            profile.lower_utilization,
            profile.upper_utilization,
            profile.capacities.map(|capacity| {
                (
                    capacity.bandwidth_scale_gib,
                    capacity.core_bandwidth_scale_gib,
                )
            }),
        )
    });

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

    loop {
        let loop_start = Instant::now();

        if shutdown_rx.try_recv().is_ok() {
            break;
        }

        let mut governor = governor
            .lock()
            .map_err(|_| AppError::from("governor lock poisoned"))?;
        let gpu_load = governor.run_iteration()?;

        if let Some(memory_fabric_profile) = memory_fabric_profile.as_mut()
            && let Some(perf_profile) = memory_fabric_profile.sample(gpu_load)?
        {
            governor.set_memory_fabric_profile(perf_profile)?;
            info!("Memory fabric performance profile changed to {perf_profile}");
        }

        let target_cycle_interval = governor.target_cycle_interval();
        drop(governor);
        let elapsed = loop_start.elapsed();
        if elapsed < target_cycle_interval {
            std::thread::sleep(target_cycle_interval - elapsed);
        }
    }

    info!("Shutting down gracefully...");
    let mut governor = governor.lock().expect("governor lock poisoned");
    if let Some(perf_profile) = memory_fabric_profile
        .as_mut()
        .and_then(MemoryFabricProfile::reset)
    {
        governor.set_memory_fabric_profile(perf_profile)?;
    }
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
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
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
