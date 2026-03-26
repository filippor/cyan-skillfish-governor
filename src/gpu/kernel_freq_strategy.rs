use super::FreqStrategy;
use crate::app_error::Result;
use log::warn;
use std::{
    collections::BTreeMap,
    fs::File,
    io::{Error as IoError, Write},
    path::PathBuf,
};

pub(super) struct KernelFreqStrategy {
    pp_file: File,
    dpm_sclk: PathBuf,
    pp_od_clk_voltage_path: PathBuf,
}

impl KernelFreqStrategy {
    pub(super) fn new(gpu_sysfs_path: PathBuf) -> Result<Self> {
        let pp_od_clk_voltage_path = gpu_sysfs_path.join("pp_od_clk_voltage");
        let pp_file = std::fs::OpenOptions::new()
            .write(true)
            .open(&pp_od_clk_voltage_path)?;
        let dpm_sclk = gpu_sysfs_path.join("pp_dpm_sclk");
        Ok(Self {
            pp_file,
            dpm_sclk,
            pp_od_clk_voltage_path,
        })
    }
}

impl FreqStrategy for KernelFreqStrategy {
    fn change_freq(&mut self, freq: u32, vol: u32) -> Result<()> {
        let cmd = format!("vc 0 {freq} {vol}");
        self.pp_file.write_all(cmd.as_bytes()).map_err(|e| {
            IoError::other(format!("writing '{cmd}' to pp_od_clk_voltage failed: {e}"))
        })?;
        self.pp_file.write_all("c".as_bytes()).map_err(|e| {
            IoError::other(format!(
                "writing 'c' (commit) to pp_od_clk_voltage failed: {e}"
            ))
        })?;
        Ok(())
    }

    fn get_freq(&self) -> Result<u32> {
        let content = std::fs::read_to_string(&self.dpm_sclk)?;

        let line = content
            .lines()
            .find(|line| line.contains('*'))
            .ok_or(IoError::other("failed to find active pp_dpm_sclk level"))?;

        let freq_mhz = line
            .split_whitespace()
            .find_map(|token| token.strip_suffix("Mhz"))
            .ok_or(IoError::other(
                "failed to parse pp_dpm_sclk frequency token",
            ))?
            .parse::<u32>()
            .map_err(IoError::other)?;

        Ok(freq_mhz)
    }

    fn clamp_safe_points(&self, safe_points: BTreeMap<u32, u32>) -> Result<BTreeMap<u32, u32>> {
        let content = match std::fs::read_to_string(&self.pp_od_clk_voltage_path) {
            Ok(c) => c,
            Err(e) => {
                warn!("could not read pp_od_clk_voltage for SCLK limits ({e}), skipping clamp");
                return Ok(safe_points);
            }
        };

        let Some((sclk_min, sclk_max)) = content.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("SCLK:")?;
            let mut vals = rest
                .split_whitespace()
                .filter_map(|t| t.strip_suffix("Mhz")?.parse::<u32>().ok());
            Some((vals.next()?, vals.next()?))
        }) else {
            warn!("SCLK limits not found in pp_od_clk_voltage, skipping clamp");
            return Ok(safe_points);
        };

        let vddc_limits = content.lines().find_map(|line| {
            let rest = line.trim().strip_prefix("VDDC:")?;
            let mut vals = rest.split_whitespace().filter_map(|t| {
                t.strip_suffix("mV")
                    .or_else(|| t.strip_suffix("mv"))?
                    .parse::<u32>()
                    .ok()
            });
            Some((vals.next()?, vals.next()?))
        });

        if vddc_limits.is_none() {
            warn!("VDDC limits not found in pp_od_clk_voltage, skipping voltage clamp");
        }

        Ok(safe_points
            .into_iter()
            .fold(BTreeMap::new(), |mut acc, (freq, vol)| {
                let clamped_freq = freq.clamp(sclk_min, sclk_max);
                if clamped_freq != freq {
                    warn!(
                        "clamping safe point frequency {}Mhz -> {}Mhz (SCLK range {}-{}Mhz)",
                        freq, clamped_freq, sclk_min, sclk_max
                    );
                }

                let clamped_vol = if let Some((vddc_min, vddc_max)) = vddc_limits {
                    let clamped = vol.clamp(vddc_min, vddc_max);
                    if clamped != vol {
                        warn!(
                            "clamping safe point voltage {}mV -> {}mV (VDDC range {}-{}mV)",
                            vol, clamped, vddc_min, vddc_max
                        );
                    }
                    clamped
                } else {
                    vol
                };

                match acc.get_mut(&clamped_freq) {
                    Some(existing_vol) => {
                        if clamped_vol > *existing_vol {
                            *existing_vol = clamped_vol;
                        }
                    }
                    None => {
                        acc.insert(clamped_freq, clamped_vol);
                    }
                }

                acc
            }))
    }
}
