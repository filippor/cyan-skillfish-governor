use crate::app_error::Result;
use log::{error, info};
use std::ops::RangeInclusive;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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
    SetRange(u32, u32),
}

const SERVICE_NAME: &str = "com.cyan.SkillFishGovernor";
const OBJECT_PATH: &str = "/com/cyan/SkillFishGovernor";
const INTERFACE_NAME: &str = "com.cyan.SkillFishGovernor.PerformanceMode";
struct PerformanceModeIface {
    enabled: Arc<AtomicBool>,
    tx: Sender<PerformanceModeCommand>,
    allowed_min: u32,
    allowed_max: u32,
}

impl PerformanceModeIface {
    fn send_command(&self, command: PerformanceModeCommand) {
        if let Err(err) = self.tx.send(command) {
            error!("failed to notify governor main loop from D-Bus handler: {err}");
        }
    }

    fn send_mode_update(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Relaxed);
        let command = if enabled {
            PerformanceModeCommand::Enable
        } else {
            PerformanceModeCommand::Disable
        };

        self.send_command(command);
    }

    fn send_fixed_frequency_update(&self, frequency: u32) {
        self.enabled.store(true, Ordering::Relaxed);
        self.send_command(PerformanceModeCommand::SetFixedFrequency(frequency));
    }

    fn send_test_mode_update(&self, frequency: u32, voltage: u32) {
        self.enabled.store(true, Ordering::Relaxed);
        self.send_command(PerformanceModeCommand::SetTestMode(frequency, voltage));
    }
}

#[zbus::interface(name = "com.cyan.SkillFishGovernor.PerformanceMode")]
impl PerformanceModeIface {
    fn enable(&self) {
        self.send_mode_update(true);
    }

    fn disable(&self) {
        self.send_mode_update(false);
    }

    fn set_fixed_frequency(&self, frequency: u32) {
        self.send_fixed_frequency_update(frequency);
    }

    fn set_test_mode(&self, frequency: u32, voltage: u32) {
        self.send_test_mode_update(frequency, voltage);
    }

    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    #[zbus(property)]
    fn set_enabled(&self, value: bool) {
        self.send_mode_update(value);
    }
    fn set_range(&self, min: u32, max: u32) -> fdo::Result<()> {
        if max != 0 && min > max {
            return Err(fdo::Error::InvalidArgs(format!(
                "Invalid range: min {} > max {}",
                min, max
            )));
        }
        if min != 0 && !(self.allowed_min..=self.allowed_max).contains(&min) {
            return Err(fdo::Error::InvalidArgs(format!(
                "min {} out of allowed range {}..={} MHz",
                min, self.allowed_min, self.allowed_max
            )));
        }
        if max != 0 && !(self.allowed_min..=self.allowed_max).contains(&max) {
            return Err(fdo::Error::InvalidArgs(format!(
                "max {} out of allowed range {}..={} MHz",
                max, self.allowed_min, self.allowed_max
            )));
        }
        if let Err(e) = self.tx.send(PerformanceModeCommand::SetRange(min, max)) {
            error!("Failed to send frequency range command: {}", e);
        }
        Ok(())
    }
}

/// D-Bus service handler
pub struct DbusService;

impl DbusService {
    /// Start the D-Bus service in a background thread
    /// Returns a receiver for performance mode commands
    pub fn start(allowed_range: &RangeInclusive<u32>) -> Result<Receiver<PerformanceModeCommand>> {
        let (tx, rx) = mpsc::channel();
        let allowed_min = *allowed_range.start();
        let allowed_max = *allowed_range.end();

        std::thread::spawn(move || {
            if let Err(e) = Self::run_service(tx, allowed_min, allowed_max) {
                error!("D-Bus service error: {}", e);
            }
        });

        info!("D-Bus service thread started");
        Ok(rx)
    }

    fn run_service(
        tx: Sender<PerformanceModeCommand>,
        allowed_min: u32,
        allowed_max: u32,
    ) -> Result<()> {
        let enabled = Arc::new(AtomicBool::new(false));
        let iface = PerformanceModeIface {
            enabled,
            tx,
            allowed_min,
            allowed_max,
        };

        let _connection = ConnectionBuilder::system()
            .map_err(|err| format!("failed to create D-Bus system connection: {err}"))?
            .name(SERVICE_NAME)
            .map_err(|err| format!("failed to request D-Bus name {SERVICE_NAME}: {err}"))?
            .serve_at(OBJECT_PATH, iface)
            .map_err(|err| {
                format!("failed to export D-Bus object {OBJECT_PATH} ({INTERFACE_NAME}): {err}")
            })?
            .build()
            .map_err(|err| format!("failed to finalize D-Bus connection: {err}"))?;

        info!(
            "D-Bus performance mode service ready: {} {} {}",
            SERVICE_NAME, OBJECT_PATH, INTERFACE_NAME
        );

        loop {
            std::thread::park();
        }
    }
}
