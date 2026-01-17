
use std::{
    collections::BTreeMap,
    io::{Error as IoError, ErrorKind},
    time::{Duration},
};
use toml::Table;

pub struct Config {
    pub sampling_interval: Duration,
    pub adjustment_interval: Duration,
    pub ramp_rate: f32,
    pub ramp_rate_burst: f32,
    pub burst_samples: Option<u32>,
    pub significant_change: u32,
    pub up_thresh: f32,
    pub down_thresh: f32,
    pub down_events: i16,
    pub throttling_temp: Option<u32>,
    pub throttling_recovery_temp: Option<u32>,
    pub safe_points: BTreeMap<u32, u32>,
}

impl Config 
{
    pub fn new(
    path: Result<String, std::io::Error>,
) -> Result<Config, Box<dyn std::error::Error>> {
    let config = path?.parse::<Table>()?;

    let timing = config.get("timing").and_then(|t| t.as_table());
    let intervals = timing
        .and_then(|t| t.get("intervals"))
        .and_then(|t| t.as_table());
    // us
    let sampling_interval: u32 = intervals
        .and_then(|t| t.get("sample"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u32::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u32::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!("timing.intervals.sample {s}, replaced with the default value of 2 ms");
            2000
        });
    // us
    let adjustment_interval = intervals
        .and_then(|t| t.get("adjust"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            (v >= i64::from(sampling_interval))
                .then_some(v)
                .ok_or("must be at least as high as timing.intervals.sample")
        })
        .and_then(|v| {
            u64::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u64::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.intervals.adjust {s}, replaced with the default of \
                10 * timing.intervals.sample"
            );
            10 * u64::from(sampling_interval)
        });
    
    // samples
    let burst_samples = match timing
        .and_then(|t| t.get("burst-samples"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
    {
        Err(s) => {
            println!(
                "timing.burst-samples {s}, replaced with the default of \
            48"
            );
            Some(48)
        }
        Ok(0) => None,
        Ok(v @ 1..=64) => Some(v as u32),
        Ok(65..) => {
            println!("timing.burst-samples can be at most 64, clamping");
            Some(64)
        }
        Ok(i64::MIN..0) => {
            println!("timing.burst-samples is negative Disabling burst");
            None
        }
    };

    const I16_MAX : i64= i16::MAX as i64;
    let down_events = match timing
        .and_then(|t| t.get("down-events"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
    {
        Err(s) => {
            println!(
                "timing.down-events{s}, replaced with the default of \
            10"
            );
            10
        }
        Ok(v @ 0..=I16_MAX) => v as i16,
        Ok(_) => {
            println!("timing.down-events is negative use default 10");
            10
        }
    };


    let ramp_rates = timing
        .and_then(|t| t.get("ramp-rates"))
        .and_then(|t| t.as_table());
    // MHz/ms
    let ramp_rate = ramp_rates
        .and_then(|t| t.get("normal"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            v.is_sign_positive()
                .then_some(v)
                .ok_or("must have positive sign")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "timing.ramp-rates.normal {s}, replaced with the default value of \
                1 MHz/ms"
            );
            1.0
        });
    // MHz/ms
    let ramp_rate_burst = ramp_rates
        .and_then(|t| t.get("burst"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            v.is_sign_positive()
                .then_some(v)
                .ok_or("must have positive sign")
        })
        .map(|v| v as f32)
        .and_then(|v| {
            (v > ramp_rate || burst_samples.is_none())
                .then_some(v)
                .ok_or(
                    "must, if bursting is active, be greater than timing.ramp-rates.normal \
                (if you want to turn bursting off, set timing.burst-samples = 0)",
                )
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.ramp-rates.burst {s}, replaced with the default value of \
                200 * timing.ramp-rates.normal"
            );
            200.0 * ramp_rate
        });

    let freq_threshs = config
        .get("frequency-thresholds")
        .and_then(|t| t.as_table());
    
    // MHz
    let significant_change = freq_threshs
        .and_then(|t| t.get("adjust"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u32::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u32::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "frequency-thresholds.adjust {s}, replaced with the default of \
                10"
            );
            10
        });

    let load_threshs = config.get("load-target").and_then(|t| t.as_table());
    // fraction
    let up_thresh = load_threshs
        .and_then(|t| t.get("upper"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            (0.0..1.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be fractional")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "load-target.upper {s}, replaced with the default value of \
                0.95"
            );
            0.95
        });
    // fraction
    let down_thresh = load_threshs
        .and_then(|t| t.get("lower"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            (0.0..1.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be fractional")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "load-target.lower {s}, replaced with the default value of \
                load-target.upper - 0.15"
            );
            (up_thresh - 0.15).max(0.0)
        });
    let down_thresh = if down_thresh > up_thresh {
        println!("load-target.lower can't be greater than load-target.upper, clamping");
        up_thresh
    } else {
        down_thresh
    };

    // MHz, mV
    let safe_points: BTreeMap<u32, u32> = if let Some(array) = config.get("safe-points") {
        let array = array.as_array().ok_or(IoError::new(
            ErrorKind::InvalidInput,
            "safe-points must be an array",
        ))?;
        if array.is_empty() {
            Err(IoError::new(
                ErrorKind::InvalidInput,
                "safe-points must not be empty",
            ))?;
        }
        let mut safe_points = BTreeMap::new();
        for (i, t) in array.iter().enumerate() {
            let t = t.as_table().ok_or_else(|| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!("safe-points[{i}] must be a table"),
                )
            })?;

            // MHz
            let frequency = t
                .get("frequency")
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].frequency must exist"),
                    )
                })?
                .as_integer()
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].frequency must be an integer"),
                    )
                })?;
            let frequency = u32::try_from(frequency).map_err(|_| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "safe-points[{i}].frequency must be between 0 and {} inclusive",
                        u32::MAX
                    ),
                )
            })?;

            // mV
            let voltage = t
                .get("voltage")
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].voltage must exist"),
                    )
                })?
                .as_integer()
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].voltage must be an integer"),
                    )
                })?;
            let voltage = u32::try_from(voltage).map_err(|_| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "safe-points[{i}].voltage must be between 0 and {} inclusive",
                        u32::MAX
                    ),
                )
            })?;

            if safe_points.insert(frequency, voltage).is_some() {
                Err(IoError::new(
                    ErrorKind::InvalidInput,
                    format!("multiple supposedly safe voltages for {frequency} MHz"),
                ))?;
            }
        }
        let mut highest_pair = (0, 0);
        for (frequency, voltage) in &safe_points {
            let pair = (*voltage, *frequency);
            if pair < highest_pair {
                Err(IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "supposedly safe voltage {} mV for {} MHz is higher than \
                        {voltage} mV for {frequency} MHz",
                        highest_pair.0, highest_pair.1,
                    ),
                ))?;
            } else {
                highest_pair = pair;
            }
        }
        safe_points
    } else {
        println!(
            "safe-points undefined, using conservative defaults:\n\
            * 350 MHz @ 700 mV\n\
            * 2000 MHz @ 1000 mV"
        );
        BTreeMap::from([(350, 700), (2000, 1000)])
    };

    let temperature = config.get("temperature").and_then(|t| t.as_table());
    let throttling_temp = match temperature
        .and_then(|t| t.get("throttling"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
    {
        Err(s) => {
            println!("temperature.throttling {s}, disabled");
            None
        }
        Ok(v @ 0..=110) => Some(v as u32),
        Ok(111..) => {
            println!("temperature.throttling can be at most 110, clamping");
            Some(110)
        }
        Ok(i64::MIN..0) => {
            println!("temperature.throttling is negative disable throttling");
            None
        }
    };
    let throttling_recovery_temp = if let Some(max_recovery) = throttling_temp {
        match temperature
            .and_then(|t| t.get("throttling_recovery"))
            .ok_or("is missing")
            .and_then(|v| v.as_integer().ok_or("must be an integer"))
        {
            Err(s) => {
                println!("temperature.throttling_recovery {s}, disabled");
                None
            }
            Ok(0) => None,
            Ok(v @ 1..=i64::MAX) => {
                if v >= max_recovery as i64 {
                    let tmp = max_recovery - 1;
                    println!(
                        "temperature.throttling_recovery can be at most temperature.throttling -1 ({tmp}), clamping"
                    );
                    Some(max_recovery - 1)
                } else {
                    Some(v as u32)
                }
            }
            Ok(i64::MIN..0) => {
                println!("temperature.throttling_recovery is negative disable recovery");
                None
            }
        }
    } else {
        None
    };

    Ok(
        Config {
            sampling_interval: Duration::from_micros(u64::from(sampling_interval)),
            ramp_rate: ramp_rate,
            burst_samples: burst_samples,
            down_events:down_events,
            ramp_rate_burst: ramp_rate_burst,
            up_thresh: up_thresh,
            down_thresh: down_thresh,
            adjustment_interval: Duration::from_micros(adjustment_interval),
            significant_change: significant_change,
            throttling_temp: throttling_temp,
            throttling_recovery_temp: throttling_recovery_temp,
            safe_points: safe_points,
        }        
    )
}
}

