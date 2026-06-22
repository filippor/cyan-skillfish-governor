use crate::app_error::Result;
use crate::governor::Governor;
use log::{error, info};
use std::sync::Arc;
use std::sync::Mutex;
use zbus::blocking::connection::Builder as ConnectionBuilder;
use zbus::fdo;

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
}

struct CurrentRangeIface {
    state: Arc<Mutex<Governor>>,
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
    state: Arc<Mutex<Governor>>,
}

#[zbus::interface(name = "com.cyanskillfish.Governor.PerformanceMode")]
impl PerformanceModeIface {
    fn set_fixed_frequency(&self, frequency: u32) -> fdo::Result<()> {
        let mut gov = self
            .state
            .lock()
            .map_err(|_| fdo::Error::Failed("state lock poisoned".into()))?;
        gov.apply_fixed_frequency_command(frequency)
            .map_err(|err| fdo::Error::InvalidArgs(err.to_string()))?;
        Ok(())
    }

    fn set_range(&self, min: u32, max: u32) -> fdo::Result<()> {
        let mut gov = self
            .state
            .lock()
            .map_err(|_| fdo::Error::Failed("state lock poisoned".into()))?;
        gov.apply_range_command(min, max)
            .map_err(|err| fdo::Error::InvalidArgs(err.to_string()))?;
        Ok(())
    }

    fn set_load_target(&self, min: f64, max: f64) -> fdo::Result<()> {
        let mut gov = self
            .state
            .lock()
            .map_err(|_| fdo::Error::Failed("state lock poisoned".into()))?;
        gov.apply_load_target_command(min, max)
            .map_err(|err| fdo::Error::InvalidArgs(err.to_string()))?;
        Ok(())
    }

    fn set_temperature_thresholds(&self, throttling: u32, recovery: u32) -> fdo::Result<()> {
        let mut gov = self
            .state
            .lock()
            .map_err(|_| fdo::Error::Failed("state lock poisoned".into()))?;
        gov.apply_temperature_thresholds_command(throttling, recovery)
            .map_err(|err| fdo::Error::InvalidArgs(err.to_string()))?;
        Ok(())
    }

    fn set_parameters(
        &self,
        min_freq: u32,
        max_freq: u32,
        load_min: f64,
        load_max: f64,
        throttling_temp: u32,
        recovery_temp: u32,
    ) -> fdo::Result<()> {
        let mut gov = self
            .state
            .lock()
            .map_err(|_| fdo::Error::Failed("state lock poisoned".into()))?;
        gov.apply_parameters_command(
            min_freq,
            max_freq,
            load_min,
            load_max,
            throttling_temp,
            recovery_temp,
        )
        .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Ok(())
    }

    #[zbus(property)]
    fn set_load_target_min(&self, value: f64) -> zbus::Result<()> {
        let mut gov = self.state.lock().map_err(|_| {
            zbus::Error::FDO(Box::new(fdo::Error::Failed("state lock poisoned".into())))
        })?;
        let current_max = gov.load_target().1;
        gov.apply_load_target_command(value, current_max)
            .map_err(|err| zbus::Error::FDO(Box::new(fdo::Error::InvalidArgs(err.to_string()))))
    }

    #[zbus(property)]
    fn load_target_min(&self) -> f64 {
        self.state
            .lock()
            .map(|state| state.load_target().0)
            .unwrap_or(0.0)
    }

    #[zbus(property)]
    fn set_load_target_max(&self, value: f64) -> zbus::Result<()> {
        let mut gov = self.state.lock().map_err(|_| {
            zbus::Error::FDO(Box::new(fdo::Error::Failed("state lock poisoned".into())))
        })?;
        let current_min = gov.load_target().0;
        gov.apply_load_target_command(current_min, value)
            .map_err(|err| zbus::Error::FDO(Box::new(fdo::Error::InvalidArgs(err.to_string()))))
    }

    #[zbus(property)]
    fn load_target_max(&self) -> f64 {
        self.state
            .lock()
            .map(|state| state.load_target().1)
            .unwrap_or(1.0)
    }

    #[zbus(property)]
    fn set_temperature_throttling(&self, value: u32) -> zbus::Result<()> {
        let mut gov = self.state.lock().map_err(|_| {
            zbus::Error::FDO(Box::new(fdo::Error::Failed("state lock poisoned".into())))
        })?;
        let current_recovery = gov.temperature_thresholds().1;
        gov.apply_temperature_thresholds_command(value, current_recovery.unwrap_or(0))
            .map_err(|err| zbus::Error::FDO(Box::new(fdo::Error::Failed(err.to_string()))))?;
        Ok(())
    }

    #[zbus(property)]
    fn temperature_throttling(&self) -> u32 {
        self.state
            .lock()
            .map(|state| state.temperature_thresholds().0.unwrap_or(0))
            .unwrap_or(0)
    }

    #[zbus(property)]
    fn set_temperature_recovery(&self, value: u32) -> zbus::Result<()> {
        let mut gov = self.state.lock().map_err(|_| {
            zbus::Error::FDO(Box::new(fdo::Error::Failed("state lock poisoned".into())))
        })?;
        let current_throttling = gov.temperature_thresholds().0;
        gov.apply_temperature_thresholds_command(current_throttling.unwrap_or(0), value)
            .map_err(|err| zbus::Error::FDO(Box::new(fdo::Error::Failed(err.to_string()))))
    }

    #[zbus(property)]
    fn temperature_recovery(&self) -> u32 {
        self.state
            .lock()
            .map(|state| state.temperature_thresholds().1.unwrap_or(0))
            .unwrap_or(0)
    }

    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.state
            .lock()
            .map(|state| state.performance_mode_enabled())
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn set_enabled(&self, value: bool) -> zbus::Result<()> {
        let mut gov = self.state.lock().map_err(|_| {
            zbus::Error::FDO(Box::new(fdo::Error::Failed("state lock poisoned".into())))
        })?;
        gov.apply_enable_performance_mode_command(value);
        Ok(())
    }
}

#[zbus::interface(name = "com.cyanskillfish.Governor.TestMode")]
impl TestModeIface {
    fn set_test_mode(&self, frequency: u32, voltage: u32) -> fdo::Result<()> {
        let mut gov = self
            .state
            .lock()
            .map_err(|_| fdo::Error::Failed("state lock poisoned".into()))?;
        gov.apply_test_mode_command(frequency, voltage)
            .map_err(|err| fdo::Error::Failed(err.to_string()))?;
        Ok(())
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
        let current_max = self.with_state(|state| state.current_range().1);
        let mut gov = self.state.lock().map_err(|_| {
            zbus::Error::FDO(Box::new(fdo::Error::Failed("state lock poisoned".into())))
        })?;
        gov.apply_range_command(value, current_max)
            .map_err(|err| zbus::Error::FDO(Box::new(fdo::Error::InvalidArgs(err.to_string()))))?;
        Ok(())
    }

    #[zbus(property)]
    fn max(&self) -> u32 {
        self.with_state(|state| state.current_range().1)
    }

    #[zbus(property)]
    fn set_max(&self, value: u32) -> zbus::Result<()> {
        let current_min = self.with_state(|state| state.current_range().0);
        let mut gov = self.state.lock().map_err(|_| {
            zbus::Error::FDO(Box::new(fdo::Error::Failed("state lock poisoned".into())))
        })?;
        gov.apply_range_command(current_min, value)
            .map_err(|err| zbus::Error::FDO(Box::new(fdo::Error::InvalidArgs(err.to_string()))))?;
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

impl CurrentRangeIface {
    fn with_state<R>(&self, f: impl FnOnce(&Governor) -> R) -> R {
        let state = self.state.lock().expect("D-Bus state lock poisoned");
        f(&state)
    }
}

impl ReadOnlyRangeIface {
    fn with_state<R>(&self, f: impl FnOnce(&Governor) -> R) -> R {
        let state = self.state.lock().expect("D-Bus state lock poisoned");
        f(&state)
    }
}

/// D-Bus service handler
pub struct DbusService;

impl DbusService {
    /// Start the D-Bus service in a background thread
    pub fn start(state: Arc<Mutex<Governor>>) -> Result<()> {
        std::thread::spawn(move || {
            if let Err(e) = Self::run_service(state) {
                error!("D-Bus service error: {}", e);
            }
        });

        info!("D-Bus service thread started");
        Ok(())
    }

    fn run_service(state: Arc<Mutex<Governor>>) -> Result<()> {
        let perf_iface = PerformanceModeIface {
            state: state.clone(),
        };
        let current_range_iface = CurrentRangeIface {
            state: state.clone(),
        };
        let allowed_range_iface = ReadOnlyRangeIface {
            state: state.clone(),
            kind: RangeKind::Allowed,
        };
        let initial_range_iface = ReadOnlyRangeIface {
            state: state.clone(),
            kind: RangeKind::Initial,
        };
        let test_iface = TestModeIface {
            state: state.clone(),
        };

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
