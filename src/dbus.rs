use crate::app_error::Result;
use crate::config::GovernorParams;
use log::{error, info};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use zbus::blocking::connection::Builder as ConnectionBuilder;
use zbus::fdo;

/// Commands sent from D-Bus to the main loop
#[derive(Debug, Clone)]
pub enum PerformanceModeCommand {
    Enable,
    Disable,
    SetTestMode(u32, u32),
    SetFixedFrequency(u32),
    SetRange(u32, u32),
    SetLoadTarget(f64, f64),
    SetTemperatureThresholds(u32, u32),
}

const SERVICE_NAME: &str = "com.cyanskillfish.Governor";
const OBJECT_PATH: &str = "/com/cyanskillfish/Governor";
const INTERFACE_NAME: &str = "com.cyanskillfish.Governor.PerformanceMode";
const TEST_MODE_INTERFACE_NAME: &str = "com.cyanskillfish.Governor.TestMode";

#[derive(Debug, Clone, Copy)]
struct PerformanceModeState {
    current_range_min: u32,
    current_range_max: u32,
    allowed_range_min: u32,
    allowed_range_max: u32,
    initial_range_min: u32,
    initial_range_max: u32,
    load_target_min: f64,
    load_target_max: f64,
    throttling_temp: Option<u32>,
    throttling_recovery_temp: Option<u32>,
}

impl PerformanceModeState {
    fn from_params(params: &GovernorParams) -> Self {
        Self {
            current_range_min: *params.initial_frequency_range.start(),
            current_range_max: *params.initial_frequency_range.end(),
            allowed_range_min: *params.allowed_frequency_range.start(),
            allowed_range_max: *params.allowed_frequency_range.end(),
            initial_range_min: *params.initial_frequency_range.start(),
            initial_range_max: *params.initial_frequency_range.end(),
            load_target_min: f64::from(params.down_thresh),
            load_target_max: f64::from(params.up_thresh),
            throttling_temp: params.temperature.throttling_temp,
            throttling_recovery_temp: params.temperature.throttling_recovery_temp,
        }
    }
}

struct PerformanceModeIface {
    enabled: Arc<AtomicBool>,
    state: Arc<Mutex<PerformanceModeState>>,
    tx: Sender<PerformanceModeCommand>,
    allowed_min: u32,
    allowed_max: u32,
}

struct TestModeIface {
    enabled: Arc<AtomicBool>,
    tx: Sender<PerformanceModeCommand>,
}

fn dispatch_command(tx: &Sender<PerformanceModeCommand>, command: PerformanceModeCommand) {
    if let Err(err) = tx.send(command) {
        error!("failed to notify governor main loop from D-Bus handler: {err}");
    }
}

impl PerformanceModeIface {
    fn send_command(&self, command: PerformanceModeCommand) {
        dispatch_command(&self.tx, command);
    }

    fn with_state<R>(&self, f: impl FnOnce(&PerformanceModeState) -> R) -> R {
        let state = self.state.lock().expect("D-Bus state lock poisoned");
        f(&state)
    }

    fn with_state_mut_infallible<R>(&self, f: impl FnOnce(&mut PerformanceModeState) -> R) -> R {
        let mut state = self.state.lock().expect("D-Bus state lock poisoned");
        f(&mut state)
    }

    fn with_state_mut<R>(&self, f: impl FnOnce(&mut PerformanceModeState) -> R) -> fdo::Result<R> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| fdo::Error::Failed("failed to lock D-Bus state".into()))?;
        Ok(f(&mut state))
    }

    fn validate_range_bound(&self, label: &str, value: u32) -> fdo::Result<()> {
        if value != 0 && !(self.allowed_min..=self.allowed_max).contains(&value) {
            return Err(fdo::Error::InvalidArgs(format!(
                "{} {} out of allowed range {}..={} MHz",
                label, value, self.allowed_min, self.allowed_max
            )));
        }
        Ok(())
    }

    fn send_mode_update(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        self.with_state_mut_infallible(|state| {
            if enabled {
                state.current_range_min = state.allowed_range_min;
                state.current_range_max = state.allowed_range_max;
            } else {
                state.current_range_min = state.initial_range_min;
                state.current_range_max = state.initial_range_max;
            }
        });
        let command = if enabled {
            PerformanceModeCommand::Enable
        } else {
            PerformanceModeCommand::Disable
        };

        self.send_command(command);
    }

    fn send_fixed_frequency_update(&self, frequency: u32) {
        self.enabled.store(true, Ordering::Relaxed);
        self.with_state_mut_infallible(|state| {
            state.current_range_min = state.initial_range_min;
            state.current_range_max = frequency;
        });
        self.send_command(PerformanceModeCommand::SetFixedFrequency(frequency));
    }

    fn send_load_target_update(&self, min: f64, max: f64) {
        self.send_command(PerformanceModeCommand::SetLoadTarget(min, max));
    }

    fn send_temperature_thresholds_update(&self, throttling: u32, recovery: u32) {
        self.send_command(PerformanceModeCommand::SetTemperatureThresholds(
            throttling,
            recovery,
        ));
    }

    fn update_current_range(&self, min: u32, max: u32) {
        self.with_state_mut_infallible(|state| {
            state.current_range_min = min;
            state.current_range_max = max;
        });
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

        {
            self.with_state_mut(|state| {
                state.load_target_min = min;
                state.load_target_max = max;
            })?;
        }

        self.send_load_target_update(min, max);
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
                        "temperature throttling must be between 1 and 110 Celsius, or 0 to clear"
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
                    "temperature throttling and recovery must be set together or both cleared"
                        .into(),
                ));
            }
        };

        self.with_state_mut(|state| {
            state.throttling_temp = next_throttling;
            state.throttling_recovery_temp = next_recovery;
        })?;

        self.send_temperature_thresholds_update(
            next_throttling.unwrap_or(0),
            next_recovery.unwrap_or(0),
        );
        Ok(())
    }
}

impl TestModeIface {
    fn send_command(&self, command: PerformanceModeCommand) {
        dispatch_command(&self.tx, command);
    }

    fn send_test_mode_update(&self, frequency: u32, voltage: u32) {
        self.enabled.store(true, Ordering::Relaxed);
        self.send_command(PerformanceModeCommand::SetTestMode(frequency, voltage));
    }
}

#[zbus::interface(name = "com.cyanskillfish.Governor.PerformanceMode")]
impl PerformanceModeIface {
    fn set_fixed_frequency(&self, frequency: u32) {
        self.send_fixed_frequency_update(frequency);
    }

    #[zbus(property)]
    fn set_current_range_min(&self, value: u32) -> fdo::Result<()> {
        self.validate_range_bound("min", value)?;

        let current_max = self.with_state(|state| state.current_range_max);

        if current_max != 0 && value > current_max {
            return Err(fdo::Error::InvalidArgs(format!(
                "Invalid range: min {} > max {}",
                value, current_max
            )));
        }

        self.update_current_range(value, current_max);
        self.send_command(PerformanceModeCommand::SetRange(value, current_max));
        Ok(())
    }

    #[zbus(property)]
    fn set_load_target_min(&self, value: f64) -> fdo::Result<()> {
        let current_max = self.with_state(|state| state.load_target_max);
        self.apply_load_target(value, current_max)
    }

    #[zbus(property)]
    fn load_target_min(&self) -> f64 {
        self.with_state(|state| state.load_target_min)
    }

    #[zbus(property)]
    fn set_load_target_max(&self, value: f64) -> fdo::Result<()> {
        let current_min = self.with_state(|state| state.load_target_min);
        self.apply_load_target(current_min, value)
    }

    #[zbus(property)]
    fn load_target_max(&self) -> f64 {
        self.with_state(|state| state.load_target_max)
    }

    #[zbus(property)]
    fn current_range_min(&self) -> u32 {
        self.with_state(|state| state.current_range_min)
    }

    #[zbus(property)]
    fn set_current_range_max(&self, value: u32) -> fdo::Result<()> {
        self.validate_range_bound("max", value)?;

        let current_min = self.with_state(|state| state.current_range_min);

        if current_min != 0 && value != 0 && current_min > value {
            return Err(fdo::Error::InvalidArgs(format!(
                "Invalid range: min {} > max {}",
                current_min, value
            )));
        }

        self.update_current_range(current_min, value);
        self.send_command(PerformanceModeCommand::SetRange(current_min, value));
        Ok(())
    }

    #[zbus(property)]
    fn current_range_max(&self) -> u32 {
        self.with_state(|state| state.current_range_max)
    }

    #[zbus(property)]
    fn allowed_range_min(&self) -> u32 {
        self.with_state(|state| state.allowed_range_min)
    }

    #[zbus(property)]
    fn allowed_range_max(&self) -> u32 {
        self.with_state(|state| state.allowed_range_max)
    }

    #[zbus(property)]
    fn initial_range_min(&self) -> u32 {
        self.with_state(|state| state.initial_range_min)
    }

    #[zbus(property)]
    fn initial_range_max(&self) -> u32 {
        self.with_state(|state| state.initial_range_max)
    }

    #[zbus(property)]
    fn set_temperature_throttling(&self, value: u32) -> fdo::Result<()> {
        let current_recovery = self.with_state(|state| state.throttling_recovery_temp);

        if value == 0 {
            return self.apply_temperature_thresholds(None, None);
        }

        self.apply_temperature_thresholds(Some(value), current_recovery)
    }

    #[zbus(property)]
    fn temperature_throttling(&self) -> u32 {
        self.with_state(|state| state.throttling_temp.unwrap_or(0))
    }

    #[zbus(property)]
    fn set_temperature_recovery(&self, value: u32) -> fdo::Result<()> {
        let current_throttling = self.with_state(|state| state.throttling_temp);

        if value == 0 {
            return self.apply_temperature_thresholds(None, None);
        }

        self.apply_temperature_thresholds(current_throttling, Some(value))
    }

    #[zbus(property)]
    fn temperature_recovery(&self) -> u32 {
        self.with_state(|state| state.throttling_recovery_temp.unwrap_or(0))
    }

    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    #[zbus(property)]
    fn set_enabled(&self, value: bool) {
        self.send_mode_update(value);
    }

}

#[zbus::interface(name = "com.cyanskillfish.Governor.TestMode")]
impl TestModeIface {
    fn set_test_mode(&self, frequency: u32, voltage: u32) {
        self.send_test_mode_update(frequency, voltage);
    }
}

/// D-Bus service handler
pub struct DbusService;

pub struct DbusServiceHandle {
    pub command_rx: Receiver<PerformanceModeCommand>,
    pub enabled_state: Arc<AtomicBool>,
}

impl DbusService {
    /// Start the D-Bus service in a background thread
    /// Returns a receiver for performance mode commands
    pub fn start(params: &GovernorParams) -> Result<DbusServiceHandle> {
        let (tx, rx) = mpsc::channel();
        let allowed_min = *params.allowed_frequency_range.start();
        let allowed_max = *params.allowed_frequency_range.end();
        let state = Arc::new(Mutex::new(PerformanceModeState::from_params(params)));
        let enabled = Arc::new(AtomicBool::new(false));
        let enabled_state = Arc::clone(&enabled);

        std::thread::spawn(move || {
            if let Err(e) = Self::run_service(tx, enabled, state, allowed_min, allowed_max) {
                error!("D-Bus service error: {}", e);
            }
        });

        info!("D-Bus service thread started");
        Ok(DbusServiceHandle {
            command_rx: rx,
            enabled_state,
        })
    }

    fn run_service(
        tx: Sender<PerformanceModeCommand>,
        enabled: Arc<AtomicBool>,
        state: Arc<Mutex<PerformanceModeState>>,
        allowed_min: u32,
        allowed_max: u32,
    ) -> Result<()> {
        let perf_iface = PerformanceModeIface {
            enabled: enabled.clone(),
            state: state.clone(),
            tx: tx.clone(),
            allowed_min,
            allowed_max,
        };
        let test_iface = TestModeIface {
            enabled,
            tx,
        };

        let _connection = ConnectionBuilder::system()
            .map_err(|err| format!("failed to create D-Bus system connection: {err}"))?
            .name(SERVICE_NAME)
            .map_err(|err| format!("failed to request D-Bus name {SERVICE_NAME}: {err}"))?
            .serve_at(OBJECT_PATH, perf_iface)
            .map_err(|err| {
                format!("failed to export D-Bus object {OBJECT_PATH} ({INTERFACE_NAME}): {err}")
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
        if let Ok(state) = state.lock() {
            info!(
                "D-Bus exposed load target min/max: {:.2}/{:.2}, temperature throttling/recovery: {}/{}",
                state.load_target_min,
                state.load_target_max,
                state.throttling_temp.unwrap_or(0),
                state.throttling_recovery_temp.unwrap_or(0)
            );
        }

        loop {
            std::thread::park();
        }
    }
}
