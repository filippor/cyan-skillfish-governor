mod config;
mod gpu;
use config::Config;
use gpu::GPU;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config = Config::new(
        std::env::args()
            .nth(1)
            .map(std::fs::read_to_string)
            .unwrap_or(Ok("".to_string())),
    )?;

    let mut gpu = GPU::new(config.safe_points)?;

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
    loop {
        let mut average_load: f32 = 0.0;
        let mut burst_length: u32 = 0;

        //fill the sample buffer
        for _ in 0..65 {
            (average_load, burst_length) = gpu.poll_and_get_load()?;
            std::thread::sleep(config.sampling_interval);
        }

        let burst = config
            .burst_samples
            .map_or(false, |burst_samples| burst_length >= burst_samples);

        //Temperature Management
        let temp = gpu.read_temperature()?;
        if let Some(max_temp) = config.throttling_temp {
            if (temp > max_temp) && (max_freq >= gpu.min_freq + freq_step) {
                max_freq -= config.significant_change;
                println!("throttling temp {temp} freq {max_freq}");
            } else if let Some(recovery_temp) = config.throttling_recovery_temp
                && temp < recovery_temp
                && max_freq != gpu.max_freq
            {
                max_freq = gpu.max_freq;
                println!("recover throttling temp {temp} freq {max_freq}");
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
            println!(
                "freq curr {curr_freq} target {target_freq} temp {temp} status {status} load {average_load} bl {burst_length}"
            );
            gpu.change_freq(target_freq)?;
            status = 0;
            curr_freq = target_freq;
        }

        std::thread::sleep(config.adjustment_interval - 64 * config.sampling_interval);
    }
}
