use crate::app_error::Result;
use log::{error, info};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use zbus::blocking::ConnectionBuilder;

/// Commands sent from D-Bus to the main loop
#[derive(Debug, Clone)]
pub enum PerformanceModeCommand {
    Enable,
    Disable,
    SetFixedFrequency(u32),
}

/// Commands sent from D-Bus to the main loop (Frequency Range)
#[derive(Debug, Clone)]
pub enum FrequencyRangeCommand {
    SetRange(u32, u32),
}

const SERVICE_NAME: &str = "com.cyan.SkillFishGovernor";
const OBJECT_PATH: &str = "/com/cyan/SkillFishGovernor";
const INTERFACE_NAME: &str = "com.cyan.SkillFishGovernor.PerformanceMode";

const FREQ_RANGE_INTERFACE: &str = "com.cyan.SkillFishGovernor.FrequencyRange";
const FREQ_RANGE_PATH: &str = "/com/cyan/SkillFishGovernor/FrequencyRange";

struct PerformanceModeIface {
    enabled: Arc<AtomicBool>,
    tx: Sender<PerformanceModeCommand>,
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

    #[zbus(property)]
    fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    #[zbus(property)]
    fn set_enabled(&self, value: bool) {
        self.send_mode_update(value);
    }
}

struct FrequencyRangeIface {
    available_min: u32,
    available_max: u32,
    tx: Sender<FrequencyRangeCommand>,
}

#[zbus::interface(name = "com.cyan.SkillFishGovernor.FrequencyRange")]
impl FrequencyRangeIface {
    fn set_range(&self, min: u32, max: u32) {

        if min > max && max != 0 {
            error!("Invalid range: min {} > max {}", min, max);
            return;
        }
        if min > self.available_max {
            error!("min frequency {} exceeds available maximum {}", min, self.available_max);
            return;
        }
        if max < self.available_min && max != 0 {
            error!("max frequency {} less than available minimum {}", max, self.available_min);
            return;
        }

        if let Err(e) = self.tx.send(FrequencyRangeCommand::SetRange(min, max)) {
            error!("Failed to send frequency range command: {}", e);
        }
    }

    #[zbus(property)]
    fn available_min(&self) -> u32 {
        self.available_min
    }

    #[zbus(property)]
    fn available_max(&self) -> u32 {
        self.available_max
    }
}

/// D-Bus service handler
pub struct DbusService;

impl DbusService {
    /// Start the D-Bus service in a background thread
    /// Returns a pair of receivers: first for `PerformanceModeCommand`, second for `FrequencyRangeCommand`.
    pub fn start(available_min: u32,available_max: u32,) -> Result<(
        Receiver<PerformanceModeCommand>,
        Receiver<FrequencyRangeCommand>,
    )> {
        let (tx, rx) = mpsc::channel();

        let (fr_tx, fr_rx) = mpsc::channel();

        std::thread::spawn(move || {
            if let Err(e) = Self::run_service(tx, fr_tx, available_min, available_max) {
                error!("D-Bus service error: {}", e);
            }
        });

        info!("D-Bus service thread started");
        Ok((rx, fr_rx))
    }

    fn run_service(tx: Sender<PerformanceModeCommand>,
        fr_tx: Sender<FrequencyRangeCommand>,
        available_min: u32,
        available_max: u32,
    ) -> Result<()> {

        let enabled = Arc::new(AtomicBool::new(false));
        let iface = PerformanceModeIface { enabled, tx };

        let fr_iface = FrequencyRangeIface {
            available_min,
            available_max,
            tx: fr_tx,
        };


        let _connection = ConnectionBuilder::system()
            .map_err(|err| format!("failed to create D-Bus system connection: {err}"))?
            .name(SERVICE_NAME)
            .map_err(|err| format!("failed to request D-Bus name {SERVICE_NAME}: {err}"))?
            .serve_at(OBJECT_PATH, iface)
            .map_err(|err| {
                format!("failed to export D-Bus object {OBJECT_PATH} ({INTERFACE_NAME}): {err}")
            })?
            .serve_at(FREQ_RANGE_PATH, fr_iface)
            .map_err(|err| {
                format!("failed to export D-Bus object {FREQ_RANGE_PATH} ({FREQ_RANGE_INTERFACE}): {err}")
            })?
            .build()
            .map_err(|err| format!("failed to finalize D-Bus connection: {err}"))?;

        info!(
            "D-Bus performance mode service ready: {} {} {}",
            SERVICE_NAME, OBJECT_PATH, INTERFACE_NAME
        );
        info!(
            "D-Bus frequency range service ready: {} {} {}",
            SERVICE_NAME, FREQ_RANGE_PATH, FREQ_RANGE_INTERFACE
        );

        loop {
            std::thread::park();
        }
    }
}