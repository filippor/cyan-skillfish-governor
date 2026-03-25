mod config;
mod gpu;
mod gpu_usage_fix;
use config::Config;
use gpu::GPU;
use gpu_usage_fix::GpuUsageFix;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let running = Arc::new(AtomicBool::new(true));
    let running_signal = Arc::clone(&running);
    ctrlc::set_handler(move || {
        running_signal.store(false, Ordering::SeqCst);
    })?;

    let mut verbose = false;
    let mut config_path: Option<String> = None;

    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-v" | "--verbose" => verbose = true,
            s if s.starts_with('-') => return Err(format!("unknown option: {s}").into()),
            _ => {
                if config_path.is_some() {
                    return Err("too many positional arguments".into());
                }
                config_path = Some(arg);
            }
        }
    }

    let config = Config::new(
        config_path
            .map(std::fs::read_to_string)
            .unwrap_or(Ok("".to_string())),
    )?;

    let mut gpu_usage_fix = if config.gpu_metric_fix {
        Some(GpuUsageFix::start()?)
    } else {
        println!("GPU usage metrics fix disabled by config");
        None
    };

    let mut usage_fix_cycle: u32 = 0;

    println!(
        "GPU usage method configured: {}",
        config.gpu_usage_method.as_config_value()
    );
    println!(
        "GPU set method configured: {}",
        config.gpu_set_method.as_config_value()
    );

    let mut gpu = GPU::new(
        config.safe_points,
        config.gpu_set_method,
        config.gpu_usage_method,
        config.sampling_interval,
    )?;

    let mut curr_freq: u32 = gpu.get_freq()?;
    let mut target_freq = gpu.min_freq;
    let mut status: i16 = 0;
    const UP_EVENTS: i16 = 2;
    gpu.change_freq(target_freq)?;
    let mut max_freq = gpu.max_freq;

    let burst_freq_step =
        (config.ramp_rate_burst * config.adjustment_interval.as_millis() as f32) as u32;
    let freq_step = (config.ramp_rate * config.adjustment_interval.as_millis() as f32) as u32;

    println!("freq min {} max {} ", gpu.min_freq, max_freq);

    while running.load(Ordering::Relaxed) {
        let loop_start = std::time::Instant::now();

        let average_load: f32;
        let burst_length: u32;

        (average_load, burst_length) = gpu.poll_and_get_load()?;

        if let Some(fix) = gpu_usage_fix.as_mut() {
            usage_fix_cycle = usage_fix_cycle.saturating_add(1);
            if usage_fix_cycle >= config.gpu_metric_fix_flush_every {
                usage_fix_cycle = 0;
                if let Err(e) = fix.set_usage_percent(average_load * 100.0) {
                    eprintln!("GPU usage metrics fix write failed: {e}");
                }
            }
        }

        let burst = average_load >= 0.99
            || config
                .burst_samples
                .map_or(false, |burst_samples| burst_length >= burst_samples);

        //Temperature Management
        let temp = gpu.read_temperature()?;
        if let Some(max_temp) = config.throttling_temp {
            if (temp > max_temp) && (max_freq >= gpu.min_freq + freq_step) {
                max_freq -= config.significant_change;
                if verbose {
                    println!("throttling temp {temp} freq {max_freq}");
                }
            } else if let Some(recovery_temp) = config.throttling_recovery_temp
                && temp < recovery_temp
                && max_freq != gpu.max_freq
            {
                max_freq = gpu.max_freq;
                if verbose {
                    println!("recover throttling temp {temp} freq {max_freq}");
                }
            }
        }

        if burst {
            target_freq += burst_freq_step;
        } else {
            if average_load > config.up_thresh && status <= UP_EVENTS {
                status += UP_EVENTS;
            } else if average_load < config.down_thresh && curr_freq > gpu.min_freq {
                status -= 1;
            } else if status < 0 {
                status += 1;
            } else if status > 0 {
                status -= 1;
            }

            if status <= -config.down_events {
                target_freq -= freq_step;
            } else if status >= UP_EVENTS {
                target_freq += freq_step;
            }
        }

        target_freq = target_freq.clamp(gpu.min_freq, max_freq);
        let hit_bounds = target_freq == gpu.min_freq || target_freq == max_freq;
        let big_change = curr_freq.abs_diff(target_freq) >= config.significant_change;

        if curr_freq != target_freq && (burst || hit_bounds || big_change) {
            if verbose {
                let de = config.down_events;
                println!(
                    "freq curr {} target {} temp {} status {} de {} load {:.2} bl {}",
                    curr_freq, target_freq, temp, status, de, average_load, burst_length
                );
            }

            gpu.change_freq(target_freq)?;
            status = 0;
            curr_freq = target_freq;
        }

        let elapsed = loop_start.elapsed();
        if elapsed < config.adjustment_interval {
            std::thread::sleep(config.adjustment_interval - elapsed);
        }
    }
    println!("Shutting down gracefully...");
    if let Some(fix) = gpu_usage_fix.as_mut()
        && let Err(e) = fix.shutdown()
    {
        eprintln!("GPU usage metrics fix cleanup failed: {e}");
    }
    if let Err(e) = gpu.shutdown() {
        eprintln!("System exit restore failed: {e}");
    }
    Ok(())
}
