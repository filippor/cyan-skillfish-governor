use crate::app_error::Result;
use crate::gpu::GPU;
use log::{info, warn};
use std::ops::RangeInclusive;
use std::{
    collections::BTreeMap,
    io::{Error as IoError, ErrorKind},
    time::Duration,
};
use toml::Table;

#[derive(Clone, Copy, Debug)]
pub enum GpuUsageMethod {
    BusyFlag,
    Process,
    Kernel,
}

impl GpuUsageMethod {
    pub fn as_config_value(self) -> &'static str {
        match self {
            Self::BusyFlag => "busy-flag",
            Self::Process => "process",
            Self::Kernel => "kernel",
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
    pub dbus: DbusConfig,
    pub frequency_range: FrequencyRangeConfig,
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

#[derive(Clone, Copy)]
pub struct TemperatureConfig {
    pub throttling_temp: Option<u32>,
    pub throttling_recovery_temp: Option<u32>,
}

pub struct GpuUsageConfig {
    pub fix_metrics: bool,
    pub fix_freq: bool,
    pub flush_every: u32,
    pub method: GpuUsageMethod,
}

pub struct GpuConfig {
    pub set_method: GpuSetMethod,
}

pub struct DbusConfig {
    pub enabled: bool,
}

pub struct FrequencyRangeConfig {
    pub min: Option<u32>,
    pub max: Option<u32>,
}

pub struct GovernorParams {
    pub burst_freq_step: u32,
    pub freq_step: u32,
    pub allowed_frequency_range: RangeInclusive<u32>,
    pub initial_frequency_range: RangeInclusive<u32>,
    pub flush_every: u32,
    pub temperature: TemperatureConfig,
    pub significant_change: u32,
    pub burst_samples: Option<u32>,
    pub up_thresh: f32,
    pub down_thresh: f32,
    pub down_events: i16,
    pub adjustment_interval: Duration,
}

impl Config {
    pub fn new(config_text: std::io::Result<String>) -> Result<Config> {
        let config = config_text?.parse::<Table>()?;

        Ok(Config {
            timing: parse_timing_config(&config),
            load_target: parse_load_target_config(&config),
            frequency_thresholds: FrequencyThresholdConfig {
                significant_change: parse_significant_change(&config),
            },
            temperature: parse_temperature_config(&config),
            safe_points: parse_safe_points(&config)?,
            gpu_usage: parse_gpu_usage_config(&config),
            gpu: GpuConfig {
                set_method: parse_gpu_set_method(&config),
            },
            dbus: parse_dbus_config(&config),
            frequency_range: parse_frequency_range_config(&config),
        })
    }

    pub fn to_governor_params(&self, gpu: &GPU) -> GovernorParams {
        let adjustment_millis = self.timing.adjustment_interval.as_millis() as f32;
        let allowed_frequency_range = gpu.min_freq..=gpu.max_freq;
        let initial_frequency_range = {
            let min = self.frequency_range.min.unwrap_or(gpu.min_freq).clamp(
                *allowed_frequency_range.start(),
                *allowed_frequency_range.end(),
            );
            let max = self.frequency_range.max.unwrap_or(gpu.max_freq).clamp(
                *allowed_frequency_range.start(),
                *allowed_frequency_range.end(),
            );
            min..=max
        };
        let result = GovernorParams {
            burst_freq_step: (self.timing.ramp_rate_burst * adjustment_millis) as u32,
            freq_step: (self.timing.ramp_rate * adjustment_millis) as u32,
            allowed_frequency_range,
            initial_frequency_range,
            flush_every: self.gpu_usage.flush_every,
            temperature: self.temperature,
            significant_change: self.frequency_thresholds.significant_change,
            burst_samples: self.timing.burst_samples,
            up_thresh: self.load_target.up_thresh,
            down_thresh: self.load_target.down_thresh,
            down_events: self.timing.down_events,
            adjustment_interval: self.timing.adjustment_interval,
        };
        info!(
            "allowed frequency range {}..={}",
            result.allowed_frequency_range.start(),
            result.allowed_frequency_range.end()
        );
        info!(
            "initial frequency range: {}..={}",
            result.initial_frequency_range.start(),
            result.initial_frequency_range.end()
        );
        result
    }
}

fn parse_timing_config(config: &Table) -> TimingConfig {
    let timing = config.get("timing").and_then(|t| t.as_table());
    let intervals = nested_table(timing, "intervals");

    let sampling_interval = parse_integer_in_range_optional(
        intervals,
        "sample",
        "timing.intervals.sample",
        1..=i64::from(u32::MAX),
        Some(2000),
        Some(2000),
    )
    .unwrap() as u32;

    let adjustment_interval_default = i64::from(sampling_interval) * 10;
    let adjustment_interval = parse_integer_in_range_optional(
        intervals,
        "adjust",
        "timing.intervals.adjust",
        i64::from(sampling_interval)..=i64::MAX,
        Some(adjustment_interval_default),
        Some(adjustment_interval_default),
    )
    .unwrap() as u64;

    let burst_samples = parse_integer_in_range_optional(
        timing,
        "burst-samples",
        "timing.burst-samples",
        1..=64,
        None,
        None,
    )
    .map(|v| v as u32);

    const I16_MAX: i64 = i16::MAX as i64;
    let down_events = parse_integer_in_range_optional(
        timing,
        "down-events",
        "timing.down-events",
        0..=I16_MAX,
        Some(10),
        Some(10),
    )
    .unwrap() as i16;

    let ramp_rates = nested_table(timing, "ramp-rates");
    let ramp_rate = parse_float_in_range_optional(
        ramp_rates,
        "normal",
        "timing.ramp-rates.normal",
        0.0..=f64::MAX,
        Some(1.0),
        Some(1.0),
    )
    .unwrap() as f32;

    let ramp_rate_burst = parse_float_in_range_optional(
        ramp_rates,
        "burst",
        "timing.ramp-rates.burst",
        0.0..=f64::MAX,
        Some((200.0 * ramp_rate) as f64),
        Some((200.0 * ramp_rate) as f64),
    )
    .unwrap() as f32;

    let ramp_rate_burst = if ramp_rate_burst <= ramp_rate {
        warn!(
            "timing.ramp-rates.burst must, if bursting is active, be greater than timing.ramp-rates.normal (if you want to turn bursting off, set timing.burst-samples = 0), replaced with the default value of 200 * timing.ramp-rates.normal"
        );
        200.0 * ramp_rate
    } else {
        ramp_rate_burst
    };

    TimingConfig {
        sampling_interval: Duration::from_micros(sampling_interval.try_into().unwrap()),
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

    parse_integer_in_range_optional(
        freq_threshs,
        "adjust",
        "frequency-thresholds.adjust",
        1..=i64::from(u32::MAX),
        Some(10),
        Some(10),
    )
    .unwrap() as u32
}

fn parse_load_target_config(config: &Table) -> LoadTargetConfig {
    let load_threshs = config.get("load-target").and_then(|t| t.as_table());

    let up_thresh = parse_float_in_range_optional(
        load_threshs,
        "upper",
        "load-target.upper",
        0.0..1.0,
        Some(0.95),
        Some(0.95),
    )
    .unwrap();

    let down_thresh = parse_float_in_range_optional(
        load_threshs,
        "lower",
        "load-target.lower",
        0.0..1.0,
        Some((up_thresh - 0.15).max(0.0)),
        Some((up_thresh - 0.15).max(0.0)),
    )
    .unwrap();

    let down_thresh = if down_thresh > up_thresh {
        warn!("load-target.lower can't be greater than load-target.upper, clamping");
        up_thresh
    } else {
        down_thresh
    };

    LoadTargetConfig {
        up_thresh: up_thresh as f32,
        down_thresh: down_thresh as f32,
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
    if highest_pair.1 > 10000 || highest_pair.0 > 10000 {
        Err(IoError::new(
            ErrorKind::InvalidInput,
            format!(
                "safe point with frequency {} MHz and voltage {} mV is unrealistic",
                highest_pair.1, highest_pair.0,
            ),
        ))?;
    }
    Ok(())
}

fn parse_temperature_config(config: &Table) -> TemperatureConfig {
    let temperature = config.get("temperature").and_then(|t| t.as_table());

    let throttling_temp = parse_integer_in_range_optional(
        temperature,
        "throttling",
        "temperature.throttling",
        0..=100,
        Some(85),
        None,
    )
    .map(|v| v as u32);

    let throttling_recovery_temp = throttling_temp.and_then(|max_recovery| {
        let max_allowed = i64::from(max_recovery.saturating_sub(1));
        parse_integer_in_range_optional(
            temperature,
            "throttling_recovery",
            "temperature.throttling_recovery",
            1..=max_allowed,
            None,
            None,
        )
        .map(|v| v as u32)
    });

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
        .and_then(|v| v.as_bool().ok_or("must be a boolean true or false"))
        .unwrap_or_else(|s| {
            warn!("gpu-usage.fix-metrics {s}, replaced with the default value of true");
            true
        });

    let gpu_metric_fix_flush_every = parse_integer_in_range_optional(
        gpu_usage,
        "flush-every",
        "gpu-usage.flush-every",
        1..=i64::from(u32::MAX),
        Some(10),
        Some(10),
    )
    .unwrap() as u32;

    let gpu_freq_fix = gpu_usage
        .and_then(|t| t.get("fix-freq").or_else(|| t.get("fix_freq")))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let gpu_usage_method = match gpu_usage
        .and_then(|t| t.get("method"))
        .and_then(|v| v.as_str())
    {
        Some("busy-flag") => GpuUsageMethod::BusyFlag,
        Some("process") => GpuUsageMethod::Process,
        Some("kernel") => GpuUsageMethod::Kernel,
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
        fix_freq: gpu_freq_fix,
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

fn parse_dbus_config(config: &Table) -> DbusConfig {
    let dbus = config.get("dbus").and_then(|t| t.as_table());

    let enabled = dbus
        .and_then(|t| t.get("enabled"))
        .ok_or("is missing")
        .and_then(|v| v.as_bool().ok_or("must be a boolean true or false"))
        .unwrap_or_else(|s| {
            warn!("dbus.enabled {s}, replaced with the default value of false");
            false
        });

    DbusConfig { enabled }
}

fn parse_frequency_range_config(config: &Table) -> FrequencyRangeConfig {
    let freq_range = config
        .get("frequency-range")
        .or_else(|| config.get("frequency_range"))
        .and_then(|t| t.as_table());

    let min = parse_integer_in_range_optional(
        freq_range,
        "min",
        "frequency-range.min",
        0..=i64::from(u32::MAX),
        None,
        None,
    )
    .map(|v| v as u32);

    let max = parse_integer_in_range_optional(
        freq_range,
        "max",
        "frequency-range.max",
        0..=i64::from(u32::MAX),
        None,
        None,
    )
    .map(|v| v as u32);

    FrequencyRangeConfig { min, max }
}

fn nested_table<'a>(table: Option<&'a Table>, key: &str) -> Option<&'a Table> {
    table
        .and_then(|t| t.get(key))
        .and_then(|value| value.as_table())
}

fn integer_value(table: Option<&Table>, key: &str) -> std::result::Result<i64, &'static str> {
    table
        .and_then(|t| t.get(key))
        .ok_or("is missing")
        .and_then(|value| value.as_integer().ok_or("must be an integer"))
}

fn number_value(table: Option<&Table>, key: &str) -> std::result::Result<f64, &'static str> {
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

macro_rules! impl_parse_in_range_optional {
    ($func_name:ident, $value_getter:ident, $type:ty) => {
        fn $func_name<R: std::ops::RangeBounds<$type> + std::fmt::Debug>(
            table: Option<&Table>,
            key: &str,
            config_key: &str,
            range: R,
            missing_value: Option<$type>,
            default: Option<$type>,
        ) -> Option<$type> {
            let is_missing = table.and_then(|t| t.get(key)).is_none();
            if is_missing {
                warn!(
                    "{} is missing {}",
                    config_key,
                    missing_value
                        .map(|d| format!(", using default {d}"))
                        .unwrap_or_else(|| ", disabled".to_string())
                );
                return missing_value;
            }
            match $value_getter(table, key) {
                Ok(v) if range.contains(&v) => Some(v),
                Ok(v) => {
                    warn!(
                        "{} = {} must be between {} and {}, using default value of {}",
                        config_key,
                        v,
                        range_min(&range),
                        range_max(&range),
                        default
                            .map(|d| format!("{d}"))
                            .unwrap_or_else(|| "none".to_string())
                    );
                    default
                }
                Err(s) => {
                    warn!(
                        "{} = {}, using default value of {}",
                        config_key,
                        s,
                        default
                            .map(|d| format!("{d}"))
                            .unwrap_or_else(|| "none".to_string())
                    );
                    default
                }
            }
        }
    };
}

impl_parse_in_range_optional!(parse_integer_in_range_optional, integer_value, i64);
impl_parse_in_range_optional!(parse_float_in_range_optional, number_value, f64);

fn range_min<T, R>(range: &R) -> String
where
    T: std::fmt::Display,
    R: std::ops::RangeBounds<T>,
{
    match range.start_bound() {
        std::ops::Bound::Included(m) => format!("{} (included)", m),
        std::ops::Bound::Excluded(m) => format!("{} (excluded)", m),
        std::ops::Bound::Unbounded => "unbounded".into(),
    }
}

fn range_max<T, R>(range: &R) -> String
where
    T: std::fmt::Display,
    R: std::ops::RangeBounds<T>,
{
    match range.end_bound() {
        std::ops::Bound::Included(m) => format!("{} (included)", m),
        std::ops::Bound::Excluded(m) => format!("{} (excluded)", m),
        std::ops::Bound::Unbounded => "unbounded".into(),
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
        assert_eq!(cfg.timing.burst_samples, None);
        assert_eq!(cfg.timing.down_events, 10);
        assert!((cfg.load_target.up_thresh - 0.95).abs() < f32::EPSILON);
        assert!((cfg.load_target.down_thresh - 0.80).abs() < f32::EPSILON);
        assert_eq!(cfg.frequency_thresholds.significant_change, 10);
        assert_eq!(cfg.temperature.throttling_temp, Some(85));
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

    #[test]
    fn parse_integer_in_range_or_default_returns_value_when_in_range() {
        let config_text = r#"
            [timing.intervals]
            sample = 5000
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.timing.sampling_interval.as_micros(), 5000);
    }

    #[test]
    fn parse_integer_in_range_or_default_returns_default_when_missing() {
        let config_text = "";
        let config = parse_config(config_text);
        assert_eq!(config.timing.sampling_interval.as_micros(), 2000);
    }

    #[test]
    fn parse_integer_in_range_or_default_returns_default_when_out_of_range() {
        let config_text = r#"
            [timing.intervals]
            sample = 0
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.timing.sampling_interval.as_micros(), 2000);
    }

    #[test]
    fn parse_optional_integer_in_range_or_default_returns_value_when_in_range() {
        let config_text = r#"
            [timing]
            burst-samples = 50
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.timing.burst_samples, Some(50));
    }

    #[test]
    fn parse_optional_integer_clamped_returns_value_when_in_range() {
        let config_text = r#"
            [temperature]
            throttling = 100
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.temperature.throttling_temp, Some(100));
    }

    #[test]
    fn parse_optional_integer_disable_when_out_of_range() {
        let config_text = r#"
            [temperature]
            throttling = 200
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.temperature.throttling_temp, None);
    }

    #[test]
    fn parse_optional_integer_clamped_disables_when_negative() {
        let config_text = r#"
            [temperature]
            throttling = -10
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.temperature.throttling_temp, None);
    }

    #[test]
    fn parse_optional_integer_clamped_returns_default_when_missing() {
        let config_text = "";
        let config = parse_config(config_text);
        assert_eq!(config.temperature.throttling_temp, Some(85));
    }

    #[test]
    fn parse_frequency_range_returns_none_when_missing() {
        let config_text = "";
        let config = parse_config(config_text);
        assert_eq!(config.frequency_range.min, None);
        assert_eq!(config.frequency_range.max, None);
    }

    #[test]
    fn parse_frequency_range_with_explicit_values() {
        let config_text = r#"
            [frequency-range]
            min = 500
            max = 1800
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.frequency_range.min, Some(500));
        assert_eq!(config.frequency_range.max, Some(1800));
    }

    #[test]
    fn parse_frequency_range_with_only_min() {
        let config_text = r#"
            [frequency-range]
            min = 700
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.frequency_range.min, Some(700));
        assert_eq!(config.frequency_range.max, None);
    }

    #[test]
    fn parse_frequency_range_with_only_max() {
        let config_text = r#"
            [frequency-range]
            max = 1500
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.frequency_range.min, None);
        assert_eq!(config.frequency_range.max, Some(1500));
    }

    #[test]
    fn parse_frequency_range_with_underscore_section_name() {
        let config_text = r#"
            [frequency_range]
            min = 600
            max = 1700
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.frequency_range.min, Some(600));
        assert_eq!(config.frequency_range.max, Some(1700));
    }

    #[test]
    fn parse_gpu_usage_underscore_section_and_key_aliases() {
        let config_text = r#"
            [gpu_usage]
            fix_metric = false
            flush-every = 7
            method = "process"
        "#;
        let config = parse_config(config_text);
        assert_eq!(config.gpu_usage.fix_metrics, false);
        assert_eq!(config.gpu_usage.flush_every, 7);
        assert!(matches!(config.gpu_usage.method, GpuUsageMethod::Process));
    }
}
