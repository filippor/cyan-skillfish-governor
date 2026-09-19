use super::{TempStrategy, millidegrees_to_celsius};
use crate::app_error::{AppError, Result};
use log::warn;
use std::{
    io::Error as IoError,
    path::{Path, PathBuf},
};

/// Anything outside this range is treated as an unusable reading rather than
/// passed on -- a bogus value here would feed straight into thermal throttling.
const TEMP_MIN_MILLIDEGREES: i64 = -40_000;
const TEMP_MAX_MILLIDEGREES: i64 = 150_000;

/// How many reads in a row have to fail before this strategy gives up.
///
/// One failure means nothing: the value only drives thermal throttling, and a
/// reading a few hundred milliseconds stale serves that just as well. Giving up
/// is the expensive outcome, not the error -- the caller then falls back to the
/// DRM ioctl, opening a render node for the rest of the run, which is the thing
/// `temp-read = "sysfs"` exists to avoid.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Reads the GPU temperature from the amdgpu hwmon `temp1_input` attribute.
///
/// Holds only a path, and opens the file per read, so no DRM client is kept
/// open for temperature. Carries the last good reading so a transient failure
/// can be ridden out instead of ending the strategy.
pub(super) struct SysfsTempStrategy {
    path: PathBuf,
    last_good: i64,
    failures: u32,
}

impl SysfsTempStrategy {
    /// Locate and probe the attribute, returning an error if it is unusable.
    ///
    /// The hwmon attribute is plain upstream amdgpu -- `temp1_input` is
    /// registered for every ASIC except multi-AID parts, and Cyan Skillfish
    /// already implements `AMDGPU_PP_SENSOR_EDGE_TEMP` in a stock kernel -- so
    /// this normally succeeds. It is probed once anyway rather than trusted, so
    /// a kernel that does not expose it costs a warning instead of a governor
    /// that will not start.
    ///
    /// The probe read is kept rather than discarded: it seeds `last_good`, so a
    /// failure on the very first control cycle already has something sensible
    /// to fall back on.
    pub(super) fn probe(sysfs_path: &Path) -> Result<Self> {
        let path = find_hwmon_temp_input(sysfs_path)?;
        let last_good = read_hwmon_millidegrees(&path)?;
        Ok(Self {
            path,
            last_good,
            failures: 0,
        })
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Build a strategy straight from a known attribute path, skipping the
    /// hwmon lookup. Tests use this to point the strategy at a scratch file.
    #[cfg(test)]
    fn probe_from_path(path: PathBuf) -> Self {
        let last_good = read_hwmon_millidegrees(&path).expect("test file must be readable");
        Self {
            path,
            last_good,
            failures: 0,
        }
    }
}

impl TempStrategy for SysfsTempStrategy {
    /// A failed read reuses the last good value and is reported only as a
    /// warning. An error is returned -- meaning "replace me" -- only once
    /// `MAX_CONSECUTIVE_FAILURES` reads in a row have failed. A successful read
    /// resets the count, so isolated errors never accumulate towards that.
    fn read_temperature(&mut self) -> Result<u32> {
        match read_hwmon_millidegrees(&self.path) {
            Ok(millidegrees) => {
                self.last_good = millidegrees;
                self.failures = 0;
                Ok(millidegrees_to_celsius(millidegrees))
            }
            Err(e) => {
                self.failures += 1;
                if self.failures < MAX_CONSECUTIVE_FAILURES {
                    warn!(
                        "gpu-usage.temp-read = \"sysfs\": {e}; reusing the last good reading ({} failure(s) in a row, giving up at {MAX_CONSECUTIVE_FAILURES})",
                        self.failures
                    );
                    return Ok(millidegrees_to_celsius(self.last_good));
                }
                Err(IoError::other(format!(
                    "{MAX_CONSECUTIVE_FAILURES} reads in a row failed, last error: {e}"
                ))
                .into())
            }
        }
    }
}

/// Read one temperature from an amdgpu hwmon `temp1_input`, rejecting anything
/// that is missing, unreadable, not an integer, or out of plausible range.
fn read_hwmon_millidegrees(path: &Path) -> Result<i64> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| IoError::other(format!("{}: {e}", path.display())))?;
    let millidegrees: i64 = raw.trim().parse().map_err(|_| {
        IoError::other(format!(
            "{} did not contain an integer: {:?}",
            path.display(),
            raw.trim()
        ))
    })?;

    if !(TEMP_MIN_MILLIDEGREES..=TEMP_MAX_MILLIDEGREES).contains(&millidegrees) {
        return Err(IoError::other(format!(
            "{} reported an implausible temperature: {} millidegrees C",
            path.display(),
            millidegrees
        ))
        .into());
    }

    Ok(millidegrees)
}

/// Locate the amdgpu hwmon temperature input for this device.
///
/// Resolved once rather than per read: the hwmon index is stable for the life
/// of the bound device, and a readdir on every control loop would be wasteful.
fn find_hwmon_temp_input(sysfs_path: &Path) -> Result<PathBuf> {
    let hwmon_root = sysfs_path.join("hwmon");
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&hwmon_root)
        .map_err(|e| IoError::other(format!("{}: {e}", hwmon_root.display())))?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("temp1_input"))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();

    candidates.into_iter().next().ok_or_else(|| {
        AppError::from(format!(
            "no hwmon temp1_input under {}",
            hwmon_root.display()
        ))
    })
}

#[cfg(test)]
mod tests {
    use super::{MAX_CONSECUTIVE_FAILURES, TEMP_MAX_MILLIDEGREES, read_hwmon_millidegrees};
    use crate::gpu::TempStrategy;
    use std::path::PathBuf;

    /// Write `contents` to a uniquely named file and hand back its path.
    ///
    /// The crate has no dev-dependencies and this is the only test that needs a
    /// file on disk, so a counter plus the pid is cheaper than pulling in a
    /// temporary-file crate.
    fn temp_file_with(contents: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static COUNTER: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "cs-governor-hwmon-test-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&path, contents).expect("failed to write test file");
        path
    }

    #[test]
    fn read_hwmon_accepts_a_plausible_reading() {
        let path = temp_file_with("45000\n");

        assert_eq!(read_hwmon_millidegrees(&path).unwrap(), 45000);

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_hwmon_rejects_an_implausible_reading() {
        let path = temp_file_with(&format!("{}\n", TEMP_MAX_MILLIDEGREES + 1));

        assert!(read_hwmon_millidegrees(&path).is_err());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_hwmon_rejects_a_non_integer_reading() {
        let path = temp_file_with("not a number\n");

        assert!(read_hwmon_millidegrees(&path).is_err());

        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn read_hwmon_rejects_a_missing_file() {
        let path = std::env::temp_dir().join("cs-governor-hwmon-test-does-not-exist");
        let _ = std::fs::remove_file(&path);

        assert!(read_hwmon_millidegrees(&path).is_err());
    }

    /// A transient failure must not end the strategy: it reuses the last good
    /// reading and only gives up after MAX_CONSECUTIVE_FAILURES in a row.
    #[test]
    fn a_transient_failure_reuses_the_last_good_reading() {
        let path = temp_file_with("45000\n");
        let mut strategy = super::SysfsTempStrategy::probe_from_path(path.clone());

        assert_eq!(strategy.read_temperature().unwrap(), 45);

        std::fs::write(&path, "garbage\n").unwrap();
        for _ in 1..MAX_CONSECUTIVE_FAILURES {
            assert_eq!(
                strategy.read_temperature().unwrap(),
                45,
                "a failed read should reuse the last good value"
            );
        }
        assert!(
            strategy.read_temperature().is_err(),
            "the {MAX_CONSECUTIVE_FAILURES}th consecutive failure should give up"
        );

        let _ = std::fs::remove_file(&path);
    }

    /// A good read in between must reset the count, so isolated errors never
    /// accumulate towards giving up.
    #[test]
    fn a_good_read_resets_the_failure_count() {
        let path = temp_file_with("45000\n");
        let mut strategy = super::SysfsTempStrategy::probe_from_path(path.clone());

        for _ in 0..(MAX_CONSECUTIVE_FAILURES * 3) {
            std::fs::write(&path, "garbage\n").unwrap();
            assert!(strategy.read_temperature().is_ok());
            std::fs::write(&path, "46000\n").unwrap();
            assert_eq!(strategy.read_temperature().unwrap(), 46);
        }

        let _ = std::fs::remove_file(&path);
    }
}
