use crate::app_error::Result;
use crate::governor::Governor;
use log::{error, info};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, Sender};
use zbus::blocking::connection::Builder as ConnectionBuilder;
use zbus::fdo;

/// Commands sent from D-Bus to the main loop
#[derive(Debug, Clone)]
pub enum PerformanceModeCommand {
    Enable,
    Disable,
    SetTestMode(u32, u32),
    SetFixedFrequency(u32),
    SetParameters {
        min_freq: u32,
        max_freq: u32,
        load_min: f64,
        load_max: f64,
        throttling_temp: Option<u32>,
        recovery_temp: Option<u32>,
    },
    SetRange(u32, u32),
    SetLoadTarget(f64, f64),
    SetTemperatureThresholds(u32, u32),
}

const SERVICE_NAME: &str = "com.cyanskillfish.Governor";
const OBJECT_PATH: &str = "/com/cyanskillfish/Governor";
const INTERFACE_NAME: &str = "com.cyanskillfish.Governor.PerformanceMode";
const TEST_MODE_INTERFACE_NAME: &str = "com.cyanskillfish.Governor.TestMode";
const RANGE_INTERFACE_NAME: &str = "com.cyanskillfish.Governor.Range";
const CURRENT_RANGE_OBJECT_PATH: &str = "/com/cyanskillfish/Governor/Range/Current";
const ALLOWED_RANGE_OBJECT_PATH: &str = "/com/cyanskillfish/Governor/Range/Allowed";
const INITIAL_RANGE_OBJECT_PATH: &str = "/com/cyanskillfish/Governor/Range/Initial";

struct PerformanceModeIface {
    state: Arc<Mutex<Governor>>,
    tx: Sender<PerformanceModeCommand>,
}

struct CurrentRangeIface {
    state: Arc<Mutex<Governor>>,
    tx: Sender<PerformanceModeCommand>,
}

#[derive(Clone, Copy)]
enum RangeKind {
    Allowed,
    Initial,
}

struct ReadOnlyRangeIface {
    state: Arc<Mutex<Governor>>,
    kind: RangeKind,
}

struct TestModeIface {
    tx: Sender<PerformanceModeCommand>,
}

#[zbus::interface(name = "com.cyanskillfish.Governor.PerformanceMode")]
impl PerformanceModeIface {
    fn set_fixed_frequency(&self, frequency: u32) {
        self.send_command(PerformanceModeCommand::SetFixedFrequency(frequency));
    }

    fn set_range(&self, min: u32, max: u32) -> fdo::Result<()> {
        self.apply_range(min, max)
    }

    fn set_load_target(&self, min: f64, max: f64) -> fdo::Result<()> {
        self.apply_load_target(min, max)
    }

    fn set_temperature_thresholds(&self, throttling: u32, recovery: u32) -> fdo::Result<()> {
        let throttling_opt = (throttling > 0).then_some(throttling);
        let recovery_opt = (recovery > 0).then_some(recovery);
        self.apply_temperature_thresholds(throttling_opt, recovery_opt)
    }

    fn set_parameters(
        &self,
        min_freq: u32,
        max_freq: u32,
        load_min: f64,
        load_max: f64,
        throttling_temp: u32,
        recovery_temp: u32,
    ) {
        self.send_command(PerformanceModeCommand::SetParameters {
            min_freq,
            max_freq,
            load_min,
            load_max,
            throttling_temp: (throttling_temp > 0).then_some(throttling_temp),
            recovery_temp: (recovery_temp > 0).then_some(recovery_temp),
        });
    }

    #[zbus(property)]
    fn set_load_target_min(&self, value: f64) -> zbus::Result<()> {
        let current_max = self.with_state(|state| state.load_target().1);
        Self::fdo_to_zbus(self.apply_load_target(value, current_max))
    }

    #[zbus(property)]
    fn load_target_min(&self) -> f64 {
        self.with_state(|state| state.load_target().0)
    }

    #[zbus(property)]
    fn set_load_target_max(&self, value: f64) -> zbus::Result<()> {
        let current_min = self.with_state(|state| state.load_target().0);
        Self::fdo_to_zbus(self.apply_load_target(current_min, value))
    }

    #[zbus(property)]
    fn load_target_max(&self) -> f64 {
        self.with_state(|state| state.load_target().1)
    }

    #[zbus(property)]
    fn set_temperature_throttling(&self, value: u32) -> zbus::Result<()> {
        let current_recovery = self.with_state(|state| state.temperature_thresholds().1);

        if value == 0 {
            return Self::fdo_to_zbus(self.apply_temperature_thresholds(None, None));
        }

        Self::fdo_to_zbus(self.apply_temperature_thresholds(Some(value), current_recovery))
    }

    #[zbus(property)]
    fn temperature_throttling(&self) -> u32 {
        self.with_state(|state| state.temperature_thresholds().0.unwrap_or(0))
    }

    #[zbus(property)]
    fn set_temperature_recovery(&self, value: u32) -> zbus::Result<()> {
        let current_throttling = self.with_state(|state| state.temperature_thresholds().0);

        if value == 0 {
            return Self::fdo_to_zbus(self.apply_temperature_thresholds(None, None));
        }

        Self::fdo_to_zbus(self.apply_temperature_thresholds(current_throttling, Some(value)))
    }

    #[zbus(property)]
    fn temperature_recovery(&self) -> u32 {
        self.with_state(|state| state.temperature_thresholds().1.unwrap_or(0))
    }

    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.with_state(|state| state.performance_mode_enabled())
    }

    #[zbus(property)]
    fn set_enabled(&self, value: bool) {
        let command = if value {
            PerformanceModeCommand::Enable
        } else {
            PerformanceModeCommand::Disable
        };

        self.send_command(command);
    }
}

#[zbus::interface(name = "com.cyanskillfish.Governor.TestMode")]
impl TestModeIface {
    fn set_test_mode(&self, frequency: u32, voltage: u32) {
        self.send_command(PerformanceModeCommand::SetTestMode(frequency, voltage));
    }
}

#[zbus::interface(name = "com.cyanskillfish.Governor.Range")]
impl CurrentRangeIface {
    #[zbus(property)]
    fn min(&self) -> u32 {
        self.with_state(|state| state.current_range().0)
    }

    #[zbus(property)]
    fn set_min(&self, value: u32) -> zbus::Result<()> {
        Self::fdo_to_zbus(self.validate_range_bound("min", value))?;

        let current_max = self.with_state(|state| state.current_range().1);
        if current_max != 0 && value > current_max {
            return Err(zbus::Error::FDO(Box::new(fdo::Error::InvalidArgs(
                format!("Invalid range: min {} > max {}", value, current_max),
            ))));
        }

        self.send_command(PerformanceModeCommand::SetRange(value, current_max));
        Ok(())
    }

    #[zbus(property)]
    fn max(&self) -> u32 {
        self.with_state(|state| state.current_range().1)
    }

    #[zbus(property)]
    fn set_max(&self, value: u32) -> zbus::Result<()> {
        Self::fdo_to_zbus(self.validate_range_bound("max", value))?;

        let current_min = self.with_state(|state| state.current_range().0);
        if current_min != 0 && value != 0 && current_min > value {
            return Err(zbus::Error::FDO(Box::new(fdo::Error::InvalidArgs(
                format!("Invalid range: min {} > max {}", current_min, value),
            ))));
        }

        self.send_command(PerformanceModeCommand::SetRange(current_min, value));
        Ok(())
    }
}

#[zbus::interface(name = "com.cyanskillfish.Governor.Range")]
impl ReadOnlyRangeIface {
    #[zbus(property)]
    fn min(&self) -> u32 {
        self.with_state(|state| match self.kind {
            RangeKind::Allowed => state.allowed_range().0,
            RangeKind::Initial => *state.startup_initial_range().start(),
        })
    }

    #[zbus(property)]
    fn max(&self) -> u32 {
        self.with_state(|state| match self.kind {
            RangeKind::Allowed => state.allowed_range().1,
            RangeKind::Initial => *state.startup_initial_range().end(),
        })
    }
}

fn dispatch_command(tx: &Sender<PerformanceModeCommand>, command: PerformanceModeCommand) {
    if let Err(err) = tx.send(command) {
        error!("failed to notify governor main loop from D-Bus handler: {err}");
    }
}

impl PerformanceModeIface {
    fn fdo_to_zbus<T>(result: fdo::Result<T>) -> zbus::Result<T> {
        result.map_err(|err| zbus::Error::FDO(Box::new(err)))
    }

    fn send_command(&self, command: PerformanceModeCommand) {
        dispatch_command(&self.tx, command);
    }

    fn with_state<R>(&self, f: impl FnOnce(&Governor) -> R) -> R {
        let state = self.state.lock().expect("D-Bus state lock poisoned");
        f(&state)
    }

    fn apply_range(&self, min: u32, max: u32) -> fdo::Result<()> {
        let (allowed_min, allowed_max) = self.with_state(|state| state.allowed_range());

        if min != 0 && !(allowed_min..=allowed_max).contains(&min) {
            return Err(fdo::Error::InvalidArgs(format!(
                "min {} out of allowed range {}..={} MHz",
                min, allowed_min, allowed_max
            )));
        }

        if max != 0 && !(allowed_min..=allowed_max).contains(&max) {
            return Err(fdo::Error::InvalidArgs(format!(
                "max {} out of allowed range {}..={} MHz",
                max, allowed_min, allowed_max
            )));
        }

        if min != 0 && max != 0 && min > max {
            return Err(fdo::Error::InvalidArgs(format!(
                "invalid range: min {} > max {}",
                min, max
            )));
        }

        self.send_command(PerformanceModeCommand::SetRange(min, max));
        Ok(())
    }

    fn apply_load_target(&self, min: f64, max: f64) -> fdo::Result<()> {
        if !min.is_finite() || !max.is_finite() {
            return Err(fdo::Error::InvalidArgs(
                "load target values must be finite numbers".into(),
            ));
        }
        if !(0.0..=1.0).contains(&min) || !(0.0..=1.0).contains(&max) {
            return Err(fdo::Error::InvalidArgs(
                "load target values must be between 0.0 and 1.0".into(),
            ));
        }
        if min > max {
            return Err(fdo::Error::InvalidArgs(
                "load target min cannot be greater than max".into(),
            ));
        }

        self.send_command(PerformanceModeCommand::SetLoadTarget(min, max));
        Ok(())
    }

    fn apply_temperature_thresholds(
        &self,
        throttling: Option<u32>,
        recovery: Option<u32>,
    ) -> fdo::Result<()> {
        let (next_throttling, next_recovery) = match (throttling, recovery) {
            (None, None) => (None, None),
            (Some(0), _) | (_, Some(0)) => (None, None),
            (Some(throttling), Some(recovery)) => {
                if !(1..=110).contains(&throttling) {
                    return Err(fdo::Error::InvalidArgs(
                        "temperature throttling must be between 1 and 110 Celsius, or 0 to disable"
                            .into(),
                    ));
                }
                if recovery >= throttling {
                    return Err(fdo::Error::InvalidArgs(
                        "temperature recovery must be greater than 0 and lower than throttling"
                            .into(),
                    ));
                }
                (Some(throttling), Some(recovery))
            }
            _ => {
                return Err(fdo::Error::InvalidArgs(
                    "temperature throttling and recovery must be set together or both disabled"
                        .into(),
                ));
            }
        };

        self.send_command(PerformanceModeCommand::SetTemperatureThresholds(
            next_throttling.unwrap_or(0),
            next_recovery.unwrap_or(0),
        ));
        Ok(())
    }
}

impl CurrentRangeIface {
    fn fdo_to_zbus<T>(result: fdo::Result<T>) -> zbus::Result<T> {
        result.map_err(|err| zbus::Error::FDO(Box::new(err)))
    }

    fn with_state<R>(&self, f: impl FnOnce(&Governor) -> R) -> R {
        let state = self.state.lock().expect("D-Bus state lock poisoned");
        f(&state)
    }

    fn validate_range_bound(&self, label: &str, value: u32) -> fdo::Result<()> {
        let (allowed_min, allowed_max) = self.with_state(|state| state.allowed_range());
        if value != 0 && !(allowed_min..=allowed_max).contains(&value) {
            return Err(fdo::Error::InvalidArgs(format!(
                "{} {} out of allowed range {}..={} MHz",
                label, value, allowed_min, allowed_max
            )));
        }
        Ok(())
    }

    fn send_command(&self, command: PerformanceModeCommand) {
        dispatch_command(&self.tx, command);
    }
}

impl ReadOnlyRangeIface {
    fn with_state<R>(&self, f: impl FnOnce(&Governor) -> R) -> R {
        let state = self.state.lock().expect("D-Bus state lock poisoned");
        f(&state)
    }
}

impl TestModeIface {
    fn send_command(&self, command: PerformanceModeCommand) {
        dispatch_command(&self.tx, command);
    }
}

/// D-Bus service handler
pub struct DbusService;

pub struct DbusServiceHandle {
    pub command_rx: Receiver<PerformanceModeCommand>,
}

impl DbusService {
    /// Start the D-Bus service in a background thread
    /// Returns a receiver for performance mode commands
    pub fn start(state: Arc<Mutex<Governor>>) -> Result<DbusServiceHandle> {
        let (tx, rx) = mpsc::channel();

        std::thread::spawn(move || {
            if let Err(e) = Self::run_service(tx, state) {
                error!("D-Bus service error: {}", e);
            }
        });

        info!("D-Bus service thread started");
        Ok(DbusServiceHandle { command_rx: rx })
    }

    fn run_service(tx: Sender<PerformanceModeCommand>, state: Arc<Mutex<Governor>>) -> Result<()> {
        let perf_iface = PerformanceModeIface {
            state: state.clone(),
            tx: tx.clone(),
        };
        let current_range_iface = CurrentRangeIface {
            state: state.clone(),
            tx: tx.clone(),
        };
        let allowed_range_iface = ReadOnlyRangeIface {
            state: state.clone(),
            kind: RangeKind::Allowed,
        };
        let initial_range_iface = ReadOnlyRangeIface {
            state: state.clone(),
            kind: RangeKind::Initial,
        };
        let test_iface = TestModeIface { tx };

        let _connection = ConnectionBuilder::system()
            .map_err(|err| format!("failed to create D-Bus system connection: {err}"))?
            .name(SERVICE_NAME)
            .map_err(|err| format!("failed to request D-Bus name {SERVICE_NAME}: {err}"))?
            .serve_at(OBJECT_PATH, perf_iface)
            .map_err(|err| {
                format!("failed to export D-Bus object {OBJECT_PATH} ({INTERFACE_NAME}): {err}")
            })?
            .serve_at(CURRENT_RANGE_OBJECT_PATH, current_range_iface)
            .map_err(|err| {
                format!(
                    "failed to export D-Bus object {CURRENT_RANGE_OBJECT_PATH} ({RANGE_INTERFACE_NAME}): {err}"
                )
            })?
            .serve_at(ALLOWED_RANGE_OBJECT_PATH, allowed_range_iface)
            .map_err(|err| {
                format!(
                    "failed to export D-Bus object {ALLOWED_RANGE_OBJECT_PATH} ({RANGE_INTERFACE_NAME}): {err}"
                )
            })?
            .serve_at(INITIAL_RANGE_OBJECT_PATH, initial_range_iface)
            .map_err(|err| {
                format!(
                    "failed to export D-Bus object {INITIAL_RANGE_OBJECT_PATH} ({RANGE_INTERFACE_NAME}): {err}"
                )
            })?
            .serve_at(OBJECT_PATH, test_iface)
            .map_err(|err| {
                format!("failed to export D-Bus object {OBJECT_PATH} ({TEST_MODE_INTERFACE_NAME}): {err}")
            })?
            .build()
            .map_err(|err| format!("failed to finalize D-Bus connection: {err}"))?;

        info!(
            "D-Bus performance mode service ready: {} {} {}",
            SERVICE_NAME, OBJECT_PATH, INTERFACE_NAME
        );
        info!(
            "D-Bus test mode service ready: {} {} {}",
            SERVICE_NAME, OBJECT_PATH, TEST_MODE_INTERFACE_NAME
        );
        info!(
            "D-Bus current range service ready: {} {} {}",
            SERVICE_NAME, CURRENT_RANGE_OBJECT_PATH, RANGE_INTERFACE_NAME
        );
        info!(
            "D-Bus allowed range service ready: {} {} {}",
            SERVICE_NAME, ALLOWED_RANGE_OBJECT_PATH, RANGE_INTERFACE_NAME
        );
        info!(
            "D-Bus initial range service ready: {} {} {}",
            SERVICE_NAME, INITIAL_RANGE_OBJECT_PATH, RANGE_INTERFACE_NAME
        );
        if let Ok(state) = state.lock() {
            let (load_min, load_max) = state.load_target();
            let (throttling, recovery) = state.temperature_thresholds();
            info!(
                "D-Bus exposed load target min/max: {:.2}/{:.2}, temperature throttling/recovery: {}/{}",
                load_min,
                load_max,
                throttling.unwrap_or(0),
                recovery.unwrap_or(0)
            );
        }

        loop {
            std::thread::park();
        }
    }
}
