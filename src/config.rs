use crate::app_error::Result;
use log::warn;
use std::{
    collections::BTreeMap,
    io::{Error as IoError, ErrorKind},
    time::Duration,
};
use toml::Table;

type ValueResult<T> = std::result::Result<T, &'static str>;

#[derive(Clone, Copy, Debug)]
pub enum GpuUsageMethod {
    BusyFlag,
    Process,
}

impl GpuUsageMethod {
    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::BusyFlag => "busy-flag",
            Self::Process => "process",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum GpuSetMethod {
    Smu,
    Kernel,
}

impl GpuSetMethod {
    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::Smu => "smu",
            Self::Kernel => "kernel",
        }
    }
}

pub struct Config {
    pub timing: TimingConfig,
    pub load_target: LoadTargetConfig,
    pub frequency_thresholds: FrequencyThresholdConfig,
    pub temperature: TemperatureConfig,
    pub safe_points: BTreeMap<u32, u32>,
    pub gpu_usage: GpuUsageConfig,
    pub gpu: GpuConfig,
}

pub struct TimingConfig {
    pub sampling_interval: Duration,
    pub adjustment_interval: Duration,
    pub ramp_rate: f32,
    pub ramp_rate_burst: f32,
    pub burst_samples: Option<u32>,
    pub down_events: i16,
}

pub struct LoadTargetConfig {
    pub up_thresh: f32,
    pub down_thresh: f32,
}

pub struct FrequencyThresholdConfig {
    pub significant_change: u32,
}

pub struct TemperatureConfig {
    pub throttling_temp: Option<u32>,
    pub throttling_recovery_temp: Option<u32>,
}

pub struct GpuUsageConfig {
    pub fix_metrics: bool,
    pub flush_every: u32,
    pub method: GpuUsageMethod,
}

pub struct GpuConfig {
    pub set_method: GpuSetMethod,
}

fn nested_table<'a>(table: Option<&'a Table>, key: &str) -> Option<&'a Table> {
    table
        .and_then(|t| t.get(key))
        .and_then(|value| value.as_table())
}

fn integer_value(table: Option<&Table>, key: &str) -> ValueResult<i64> {
    table
        .and_then(|t| t.get(key))
        .ok_or("is missing")
        .and_then(|value| value.as_integer().ok_or("must be an integer"))
}

fn number_value(table: Option<&Table>, key: &str) -> ValueResult<f64> {
    integer_value(table, key)
        .map(|value| value as f64)
        .or_else(|_| {
            table
                .and_then(|t| t.get(key))
                .ok_or("is missing")
                .and_then(|value| {
                    value
                        .as_float()
                        .or_else(|| value.as_integer().map(|integer| integer as f64))
                        .ok_or("must be a number")
                })
        })
}

impl Config {
    pub fn new(config_text: std::io::Result<String>) -> Result<Config> {
        let config = config_text?.parse::<Table>()?;

        let timing = parse_timing_config(&config);
        let load_target = parse_load_target_config(&config);
        let significant_change = parse_significant_change(&config);
        let safe_points = parse_safe_points(&config)?;
        let temperature = parse_temperature_config(&config);
        let gpu_usage = parse_gpu_usage_config(&config);
        let gpu_set_method = parse_gpu_set_method(&config);

        Ok(Config {
            timing,
            load_target,
            frequency_thresholds: FrequencyThresholdConfig { significant_change },
            temperature,
            safe_points,
            gpu_usage,
            gpu: GpuConfig {
                set_method: gpu_set_method,
            },
        })
    }
}

fn parse_timing_config(config: &Table) -> TimingConfig {
    let timing = config.get("timing").and_then(|t| t.as_table());
    let intervals = nested_table(timing, "intervals");

    let sampling_interval: u32 = integer_value(intervals, "sample")
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| u32::try_from(v).map_err(|_| "cannot be greater than u32::MAX"))
        .unwrap_or_else(|s| {
            warn!("timing.intervals.sample {s}, replaced with the default value of 2 ms");
            2000
        });

    let adjustment_interval = integer_value(intervals, "adjust")
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            (v >= i64::from(sampling_interval))
                .then_some(v)
                .ok_or("must be at least as high as timing.intervals.sample")
        })
        .and_then(|v| u64::try_from(v).map_err(|_| "cannot be greater than u64::MAX"))
        .unwrap_or_else(|s| {
            warn!(
                "timing.intervals.adjust {s}, replaced with the default of \
                10 * timing.intervals.sample"
            );
            10 * u64::from(sampling_interval)
        });

    let burst_samples = match integer_value(timing, "burst-samples") {
        Err(s) => {
            warn!(
                "timing.burst-samples {s}, replaced with the default of \
        48"
            );
            Some(48)
        }
        Ok(0) => None,
        Ok(v @ 1..=64) => Some(v as u32),
        Ok(65..) => {
            warn!("timing.burst-samples can be at most 64, clamping");
            Some(64)
        }
        Ok(i64::MIN..0) => {
            warn!("timing.burst-samples is negative, disabling burst");
            None
        }
    };

    const I16_MAX: i64 = i16::MAX as i64;
    let down_events = match integer_value(timing, "down-events") {
        Err(s) => {
            warn!(
                "timing.down-events {s}, replaced with the default of \
        10"
            );
            10
        }
        Ok(v @ 0..=I16_MAX) => v as i16,
        Ok(v) if v < 0 => {
            warn!("timing.down-events is negative, using default 10");
            10
        }
        Ok(_) => {
            warn!("timing.down-events exceeds i16::MAX, using default 10");
            10
        }
    };

    let ramp_rates = nested_table(timing, "ramp-rates");
    let ramp_rate = number_value(ramp_rates, "normal")
        .and_then(|v| {
            v.is_sign_positive()
                .then_some(v)
                .ok_or("must have positive sign")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            warn!(
                "timing.ramp-rates.normal {s}, replaced with the default value of \
            1 MHz/ms"
            );
            1.0
        });

    let ramp_rate_burst = number_value(ramp_rates, "burst")
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
            warn!(
                "timing.ramp-rates.burst {s}, replaced with the default value of \
            200 * timing.ramp-rates.normal"
            );
            200.0 * ramp_rate
        });

    TimingConfig {
        sampling_interval: Duration::from_micros(u64::from(sampling_interval)),
        adjustment_interval: Duration::from_micros(adjustment_interval),
        ramp_rate,
        ramp_rate_burst,
        burst_samples,
        down_events,
    }
}

fn parse_significant_change(config: &Table) -> u32 {
    let freq_threshs = config
        .get("frequency-thresholds")
        .and_then(|t| t.as_table());

    integer_value(freq_threshs, "adjust")
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| u32::try_from(v).map_err(|_| "cannot be greater than u32::MAX"))
        .unwrap_or_else(|s| {
            warn!(
                "frequency-thresholds.adjust {s}, replaced with the default of \
            10"
            );
            10
        })
}

fn parse_load_target_config(config: &Table) -> LoadTargetConfig {
    let load_threshs = config.get("load-target").and_then(|t| t.as_table());

    let up_thresh = number_value(load_threshs, "upper")
        .and_then(|v| {
            (0.0..1.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be fractional")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            warn!(
                "load-target.upper {s}, replaced with the default value of \
            0.95"
            );
            0.95
        });

    let down_thresh = number_value(load_threshs, "lower")
        .and_then(|v| {
            (0.0..1.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be fractional")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            warn!(
                "load-target.lower {s}, replaced with the default value of \
            load-target.upper - 0.15"
            );
            (up_thresh - 0.15).max(0.0)
        });

    let down_thresh = if down_thresh > up_thresh {
        warn!("load-target.lower can't be greater than load-target.upper, clamping");
        up_thresh
    } else {
        down_thresh
    };

    LoadTargetConfig {
        up_thresh,
        down_thresh,
    }
}

fn parse_safe_points(config: &Table) -> Result<BTreeMap<u32, u32>> {
    if let Some(array) = config.get("safe-points") {
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
        for (index, entry) in array.iter().enumerate() {
            let entry = entry.as_table().ok_or_else(|| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!("safe-points[{index}] must be a table"),
                )
            })?;

            let frequency = parse_safe_point_value(entry, index, "frequency")?;
            let voltage = parse_safe_point_value(entry, index, "voltage")?;

            if safe_points.insert(frequency, voltage).is_some() {
                Err(IoError::new(
                    ErrorKind::InvalidInput,
                    format!("multiple supposedly safe voltages for {frequency} MHz"),
                ))?;
            }
        }

        validate_safe_points(&safe_points)?;
        Ok(safe_points)
    } else {
        warn!(
            "safe-points undefined, using conservative defaults:\n\
            * 350 MHz @ 700 mV\n\
            * 2000 MHz @ 1000 mV"
        );
        Ok(BTreeMap::from([(350, 700), (2000, 1000)]))
    }
}

fn parse_safe_point_value(entry: &Table, index: usize, key: &str) -> Result<u32> {
    let value = entry
        .get(key)
        .ok_or_else(|| {
            IoError::new(
                ErrorKind::InvalidInput,
                format!("safe-points[{index}].{key} must exist"),
            )
        })?
        .as_integer()
        .ok_or_else(|| {
            IoError::new(
                ErrorKind::InvalidInput,
                format!("safe-points[{index}].{key} must be an integer"),
            )
        })?;

    u32::try_from(value).map_err(|_| {
        IoError::new(
            ErrorKind::InvalidInput,
            format!(
                "safe-points[{index}].{key} must be between 0 and {} inclusive",
                u32::MAX
            ),
        )
        .into()
    })
}

fn validate_safe_points(safe_points: &BTreeMap<u32, u32>) -> Result<()> {
    let mut highest_pair = (0, 0);
    for (frequency, voltage) in safe_points {
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
        }
        highest_pair = pair;
    }

    Ok(())
}

fn parse_temperature_config(config: &Table) -> TemperatureConfig {
    let temperature = config.get("temperature").and_then(|t| t.as_table());

    let throttling_temp = match integer_value(temperature, "throttling") {
        Err(s) => {
            warn!("temperature.throttling {s}, disabled");
            None
        }
        Ok(v @ 0..=110) => Some(v as u32),
        Ok(111..) => {
            warn!("temperature.throttling can be at most 110, clamping");
            Some(110)
        }
        Ok(i64::MIN..0) => {
            warn!("temperature.throttling is negative, disabling throttling");
            None
        }
    };

    let throttling_recovery_temp = if let Some(max_recovery) = throttling_temp {
        match integer_value(temperature, "throttling_recovery") {
            Err(s) => {
                warn!("temperature.throttling_recovery {s}, disabled");
                None
            }
            Ok(0) => None,
            Ok(v @ 1..=i64::MAX) => {
                if v >= max_recovery as i64 {
                    let tmp = max_recovery - 1;
                    warn!(
                        "temperature.throttling_recovery can be at most temperature.throttling -1 ({tmp}), clamping"
                    );
                    Some(max_recovery - 1)
                } else {
                    Some(v as u32)
                }
            }
            Ok(i64::MIN..0) => {
                warn!("temperature.throttling_recovery is negative, disabling recovery");
                None
            }
        }
    } else {
        None
    };

    TemperatureConfig {
        throttling_temp,
        throttling_recovery_temp,
    }
}

fn parse_gpu_usage_config(config: &Table) -> GpuUsageConfig {
    let gpu_usage = config
        .get("gpu-usage")
        .or_else(|| config.get("gpu_usage"))
        .and_then(|t| t.as_table());

    let gpu_metric_fix = gpu_usage
        .and_then(|t| {
            t.get("fix-metrics")
                .or_else(|| t.get("fix-metric"))
                .or_else(|| t.get("fix_metric"))
        })
        .ok_or("is missing")
        .and_then(|v| v.as_bool().ok_or("must be a boolean"))
        .unwrap_or_else(|s| {
            warn!("gpu-usage.fix-metrics {s}, replaced with the default value of true");
            true
        });

    let gpu_metric_fix_flush_every = match gpu_usage
        .and_then(|t| {
            t.get("flush-every")
                .or_else(|| t.get("flush_every"))
                .or_else(|| t.get("flush-every-cycles"))
        })
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
    {
        Ok(v) if (1..=i64::from(u32::MAX)).contains(&v) => v as u32,
        Ok(_) => {
            warn!(
                "gpu-usage.flush-every cannot be greater than {} or lower than 1, replaced with the default value of 10",
                u32::MAX
            );
            10
        }
        Err(s) => {
            warn!("gpu-usage.flush-every {s}, replaced with the default value of 10");
            10
        }
    };

    let gpu_usage_method = match gpu_usage
        .and_then(|t| t.get("method"))
        .and_then(|v| v.as_str())
    {
        Some("busy-flag") => GpuUsageMethod::BusyFlag,
        Some("process") => GpuUsageMethod::Process,
        Some(other) => {
            warn!(
                "gpu-usage.method '{}' is invalid, using default busy-flag",
                other
            );
            GpuUsageMethod::BusyFlag
        }
        _ => GpuUsageMethod::BusyFlag,
    };

    GpuUsageConfig {
        fix_metrics: gpu_metric_fix,
        flush_every: gpu_metric_fix_flush_every,
        method: gpu_usage_method,
    }
}

fn parse_gpu_set_method(config: &Table) -> GpuSetMethod {
    let gpu_section = config.get("gpu").and_then(|t| t.as_table());

    match gpu_section
        .and_then(|t| t.get("set-method"))
        .and_then(|v| v.as_str())
    {
        Some("smu") => GpuSetMethod::Smu,
        Some("kernel") => GpuSetMethod::Kernel,
        Some(other) => {
            warn!("gpu.set-method '{}' is invalid, using default smu", other);
            GpuSetMethod::Smu
        }
        _ => GpuSetMethod::Smu,
    }
}

#[cfg(test)]
mod tests {
    use super::{Config, GpuSetMethod, GpuUsageMethod};

    fn parse_config(text: &str) -> Config {
        Config::new(Ok(text.to_string())).expect("config should parse")
    }

    #[test]
    fn defaults_are_applied_for_empty_config() {
        let cfg = parse_config("");

        assert_eq!(cfg.timing.sampling_interval.as_micros(), 2000);
        assert_eq!(cfg.timing.adjustment_interval.as_micros(), 20000);
        assert_eq!(cfg.timing.burst_samples, Some(48));
        assert_eq!(cfg.timing.down_events, 10);
        assert!((cfg.load_target.up_thresh - 0.95).abs() < f32::EPSILON);
        assert!((cfg.load_target.down_thresh - 0.80).abs() < f32::EPSILON);
        assert_eq!(cfg.frequency_thresholds.significant_change, 10);
        assert_eq!(cfg.temperature.throttling_temp, None);
        assert_eq!(cfg.gpu_usage.fix_metrics, true);
        assert_eq!(cfg.gpu_usage.flush_every, 10);
        assert!(matches!(cfg.gpu_usage.method, GpuUsageMethod::BusyFlag));
        assert!(matches!(cfg.gpu.set_method, GpuSetMethod::Smu));
        assert_eq!(cfg.safe_points.get(&350), Some(&700));
        assert_eq!(cfg.safe_points.get(&2000), Some(&1000));
    }

    #[test]
    fn parses_explicit_nested_config_values() {
        let cfg = parse_config(
            r#"
            [timing.intervals]
            sample = 3000
            adjust = 15000

            [timing.ramp-rates]
            normal = 2.5
            burst = 5.0

            [timing]
            burst-samples = 32
            down-events = 7

            [frequency-thresholds]
            adjust = 25

            [load-target]
            upper = 0.9
            lower = 0.7

            [temperature]
            throttling = 85
            throttling_recovery = 80

            [gpu-usage]
            fix-metrics = false
            flush-every = 15
            method = "process"

            [gpu]
            set-method = "kernel"

            [[safe-points]]
            frequency = 500
            voltage = 750

            [[safe-points]]
            frequency = 1800
            voltage = 950
            "#,
        );

        assert_eq!(cfg.timing.sampling_interval.as_micros(), 3000);
        assert_eq!(cfg.timing.adjustment_interval.as_micros(), 15000);
        assert!((cfg.timing.ramp_rate - 2.5).abs() < f32::EPSILON);
        assert!((cfg.timing.ramp_rate_burst - 5.0).abs() < f32::EPSILON);
        assert_eq!(cfg.timing.burst_samples, Some(32));
        assert_eq!(cfg.timing.down_events, 7);
        assert_eq!(cfg.frequency_thresholds.significant_change, 25);
        assert!((cfg.load_target.up_thresh - 0.9).abs() < f32::EPSILON);
        assert!((cfg.load_target.down_thresh - 0.7).abs() < f32::EPSILON);
        assert_eq!(cfg.temperature.throttling_temp, Some(85));
        assert_eq!(cfg.temperature.throttling_recovery_temp, Some(80));
        assert_eq!(cfg.gpu_usage.fix_metrics, false);
        assert_eq!(cfg.gpu_usage.flush_every, 15);
        assert!(matches!(cfg.gpu_usage.method, GpuUsageMethod::Process));
        assert!(matches!(cfg.gpu.set_method, GpuSetMethod::Kernel));
        assert_eq!(cfg.safe_points.get(&500), Some(&750));
        assert_eq!(cfg.safe_points.get(&1800), Some(&950));
    }

    #[test]
    fn safe_points_must_not_be_empty() {
        let err = match Config::new(Ok("safe-points = []".to_string())) {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };
        assert!(err.to_string().contains("safe-points must not be empty"));
    }

    #[test]
    fn duplicate_safe_point_frequency_is_rejected() {
        let err = match Config::new(Ok(r#"
            [[safe-points]]
            frequency = 1000
            voltage = 900

            [[safe-points]]
            frequency = 1000
            voltage = 950
            "#
        .to_string()))
        {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };

        assert!(
            err.to_string()
                .contains("multiple supposedly safe voltages for 1000 MHz")
        );
    }

    #[test]
    fn decreasing_voltage_with_higher_frequency_is_rejected() {
        let err = match Config::new(Ok(r#"
            [[safe-points]]
            frequency = 1000
            voltage = 950

            [[safe-points]]
            frequency = 1500
            voltage = 900
            "#
        .to_string()))
        {
            Ok(_) => panic!("must fail"),
            Err(err) => err,
        };

        assert!(err.to_string().contains(
            "supposedly safe voltage 950 mV for 1000 MHz is higher than 900 mV for 1500 MHz"
        ));
    }
}
