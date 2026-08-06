# Cyan Skillfish GPU Governor

Adaptive GPU governor for the AMD Cyan Skillfish APU.

It continuously tracks GPU load, maintains a target frequency, and adjusts GPU frequency when the deviation is large enough. It also supports burst behavior for sustained load and optional thermal throttling.

This version can set frequency/voltage using either:
- the SMU API (thanks to [bc250collective](https://github.com/bc250-collective/))
- kernel sysfs controls

## What It Does

- Samples GPU load and computes a moving target frequency.
- Applies frequency changes only when meaningful (unless burst mode forces faster response).
- Optionally throttles with temperature limits.
- Optionally exposes a D-Bus interface to toggle a high-performance mode.

## Usage

```bash
cyan-skillfish-governor-smu [-v|--verbose] [CONFIG]
```

- `CONFIG` is an optional TOML path.
- If `CONFIG` is omitted, internal defaults are used.

## Installation

Prebuilt/community packaging references:

- AUR: https://aur.archlinux.org/packages/cyan-skillfish-governor-smu
- COPR (Fedora/Bazzite): https://copr.fedorainfracloud.org/coprs/filippor/bazzite/
- ARCHIVE: https://github.com/filippor/cyan-skillfish-governor/releases

<details>
<summary>Bazzite</summary>

Bazzite can consume the same COPR package source. On mutable setups, use the Fedora steps below.

On rpm-ostree based setups, layer the package and reboot:

```bash
sudo rpm-ostree install cyan-skillfish-governor-smu
systemctl reboot
```

Configuration file location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

</details>

<details>
<summary>Fedora</summary>

Enable the COPR repository and install:

```bash
sudo dnf copr enable filippor/bazzite
sudo dnf install cyan-skillfish-governor-smu
```

Configuration file location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

</details>

<details>
<summary>Arch</summary>

Install from AUR package `cyan-skillfish-governor-smu` with your preferred AUR helper:

```bash
paru -S cyan-skillfish-governor-smu
```


Configuration file location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

</details>

<details>
<summary>Generic Linux</summary>

If your distribution is not listed above, you can install from a release archive.

1. Download the latest release archive from GitHub Releases.

2. Extract and enter the directory:

```bash
tar -xf cyan-skillfish-governor-*.tar.gz
cd cyan-skillfish-governor-*
```

3. Make sure the binary is available in the extracted directory (for example by using a release archive that already contains `cyan-skillfish-governor-smu`, or by building it with `cargo build --release`).

4. Run the installer script:

```bash
chmod +x scripts/install.sh
sudo ./scripts/install.sh
```

Installed configuration location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

</details>

<details>
<summary>Build from sources</summary>

Clone the repository (smu branch), then build:

```bash
git clone --branch smu https://github.com/filippor/cyan-skillfish-governor.git
cd cyan-skillfish-governor
```

Build the binary with:

```bash
cargo build --release
```

Then use the binary at:

```bash
./target/release/cyan-skillfish-governor-smu
```

For a manual run test with an explicit config file:

```bash
./target/release/cyan-skillfish-governor-smu ./config.toml
```
</details>

## Building Debian package

```bash
cargo install cargo-deb
```

From the root directory of the cloned respository:
```bash
cargo deb
```

This produces a .deb package in `target/debian/cyan-skillfish-governor-smu_<version>_amd64.deb`

Install the package:
```bash
sudo dpkg -i target/debian/cyan-skillfish-governor-smu_<version>_amd64.deb
```

## General recommendation:

Before enabling at boot, test one manual start and check logs:

```bash
systemctl start cyan-skillfish-governor-smu
```

After that, run a real GPU workload (for example a benchmark or a game) for a few minutes and re-check service logs to confirm expected behavior under load.
check log
```bash
systemctl status cyan-skillfish-governor-smu
sudo journalctl -u cyan-skillfish-governor-smu -n 100 --no-pager
```
If everything looks good, then enable it:

```bash
systemctl enable cyan-skillfish-governor-smu
```

after configuration change restart the service with 
```bash
systemctl restart cyan-skillfish-governor-smu
```

## Configuration

Top-level keys:

- `gpu-usage` (also accepts legacy `gpu_usage`)
  - `fix-metrics` (bool, default: `true`): enable GPU usage metrics patching.
  - `fix-freq` (bool, default: `false`): patch the `current_gfxclk_frequency` field in `gpu_metrics` with the real value read from the SMU. Fixes incorrect frequency reporting on 8-core. Can be enabled independently of `fix-metrics`.
  - `method` (`"busy-flag"` , `"kernel"` or `"process"`, default: `"busy-flag"`): how load is sampled proces is more CPU intensive scan all process that use GPU, kernel require patched kernel.
  - `flush-every` (integer, default: `10`): flush patched metrics every N update cycles.

- `gpu`
  - `set-method` (`"smu"` or `"kernel"`, default: `"smu"`): backend used to apply frequency/voltage.

- `dbus`
  - `enabled` (bool, default: `false`): enable D-Bus performance-mode service.

- `frequency-range` (optional)
  - `min` (optional integer, MHz): initial minimum frequency limit. Default: hardware minimum.
  - `max` (optional integer, MHz): initial maximum frequency limit. Default: hardware maximum.
  - Both can be omitted for full range. Can be overridden at runtime via D-Bus.

- `timing`
  - `intervals` (microseconds)
    - `sample` (default: `2000`): sampling period. Used by `gpu-usage.method = "busy-flag"`.
    - `adjust` (default: `sample * 10`): control-loop period.
  - `burst-samples` (optional integer `1..=64`, default: disabled): number of consecutive busy samples needed to enter burst mode.
    - `0`, negative, out-of-range, or missing value disables burst mode.
  - `down-events` (integer, default: `10`): number of low-load events (below `load-target.lower`) required before stepping down.
  - `ramp-rates` (MHz/ms)
    - `normal` (default: `1.0`): normal ramp rate.
    - `burst` (default: `200 * normal`): burst ramp rate. Must be greater than `normal`.

- `frequency-thresholds`
  - `adjust` (MHz, default: `10`): minimum proposed frequency delta required to apply a non-burst change.

- `load-target` (fraction)
  - `upper` (default: `0.95`): load above which target frequency increases.
  - `lower` (default: `upper - 0.15`): load below which target frequency decreases.

- `temperature` (degrees C)
  - `throttling` (optional integer `0..=110`, default when missing: `85`): above this temperature, max allowed frequency is reduced.
  - `throttling_recovery` (optional): below this temperature, max frequency is restored.
    - Must be at least `1` and strictly less than `throttling`.
    - Missing value keeps recovery disabled.

- `safe-points`
  - Array of `{ frequency, voltage }` tables.
  - `frequency` in MHz, `voltage` in mV.
  - Must be non-empty when provided.
  - For increasing frequency, voltage must not decrease.
  - If missing entirely, conservative built-in defaults are used.

### Example Configuration

Use [default-config.toml](default-config.toml) as a baseline profile.

## Configuration note 
At 25 FPS with heavy CPU load:
GPU actually renders in ~18 ms per frame
GPU waits ~22 ms for CPU
GPU utilization shows 45% busy (not 100%)

## Performance Mode Script

The configuration option `dbus.enabled` must be set to `true`.

Performance mode:
- Sets frequency to max by default,
- Reduces load-check overhead (and skips load calculation entirely when `gpu-usage.fix-metrics` is disabled),
- Keeps thermal throttling active.

The script communicates with the governor via D-Bus (see [D-Bus Interface](#dbus-interface-complete-reference) section).

Prerequisites:
- Governor service must be running and D-Bus enabled.
- `busctl` (preferred) or `dbus-send` available on the system.

### Script modes

1. Toggle and set frequency:

```bash
cyan-skillfish-performance-mode --on
cyan-skillfish-performance-mode --fixed-frequency 1200
cyan-skillfish-performance-mode --range 500 1500
cyan-skillfish-performance-mode --off
cyan-skillfish-performance-mode --status
```

2. Wrap a command (auto-enable then auto-disable on exit):

```bash
cyan-skillfish-performance-mode mangohud %command%
cyan-skillfish-performance-mode --fixed-frequency 1200 mangohud %command%
cyan-skillfish-performance-mode --range 700 1500 some-game
```

3. Steam launch option example:

```bash
cyan-skillfish-performance-mode %command%
cyan-skillfish-performance-mode --fixed-frequency 1200 %command%
```

If needed, you can pass `--` before the wrapped command:

```bash
cyan-skillfish-performance-mode --fixed-frequency 1200 -- mangohud %command%
```

In wrapper mode, the script installs a cleanup trap, so performance mode is disabled when the wrapped process exits (including Ctrl+C / TERM paths handled by the script).

## D-Bus Interface Complete Reference

D-Bus service exposed when `dbus.enabled = true`:

- **Service**: `com.cyanskillfish.Governor`
- **Object**: `/com/cyanskillfish/Governor`

### PerformanceMode Interface

**Interface**: `com.cyanskillfish.Governor.PerformanceMode` (available to all authenticated users)

#### Methods

- `SetFixedFrequency(frequency: u32)` — Set fixed frequency in MHz (enables performance mode)
- `SetRange(min: u32, max: u32)` — Set runtime current frequency range (`0` keeps bound open)
- `SetLoadTarget(min: f64, max: f64)` — Set runtime load target bounds
- `SetTemperatureThresholds(throttling: u32, recovery: u32)` — Set runtime temperature thresholds (`0,0` disables both)

#### Properties

- `Enabled` (bool, read/write) — Get or set performance mode enabled state
- `LoadTargetMin` / `LoadTargetMax` (f64, read/write) — Current load target bounds used by the governor; validate together and keep `min <= max`
- `TemperatureThrottling` / `TemperatureRecovery` (u32, read/write) — Current temperature thresholds in Celsius; `0` disables both, and recovery must stay below throttling

### Range Interface

**Interface**: `com.cyanskillfish.Governor.Range`

Range values are exposed on dedicated child objects:

- Current range object: `/com/cyanskillfish/Governor/Range/Current` (read/write)
  - `min` (u32, read/write)
  - `max` (u32, read/write)
- Allowed range object: `/com/cyanskillfish/Governor/Range/Allowed` (read-only)
  - `min` (u32, read-only)
  - `max` (u32, read-only)
- Initial range object: `/com/cyanskillfish/Governor/Range/Initial` (read-only)
  - `min` (u32, read-only)
  - `max` (u32, read-only)

For current range, use `0` for an open bound and keep the pair valid (`min <= max` when both are non-zero).


### TestMode Interface

**Interface**: `com.cyanskillfish.Governor.TestMode` (root-only, requires authorization)

> **Security Note**: The TestMode interface is restricted to root access only 
#### Methods

- `SetTestMode(frequency: u32, voltage: u32)` — Set specific frequency and voltage and disable automatic adjustment. Thermal throttling remains active.

### Examples

```bash
# Performance Mode operations (available to all authenticated users)
busctl --system set-property com.cyanskillfish.Governor /com/cyanskillfish/Governor com.cyanskillfish.Governor.PerformanceMode Enabled b true
busctl --system call com.cyanskillfish.Governor /com/cyanskillfish/Governor com.cyanskillfish.Governor.PerformanceMode SetFixedFrequency u 1200
busctl --system call com.cyanskillfish.Governor /com/cyanskillfish/Governor com.cyanskillfish.Governor.PerformanceMode SetRange uu 500 1500
busctl --system call com.cyanskillfish.Governor /com/cyanskillfish/Governor com.cyanskillfish.Governor.PerformanceMode SetLoadTarget dd 0.80 0.95
busctl --system call com.cyanskillfish.Governor /com/cyanskillfish/Governor com.cyanskillfish.Governor.PerformanceMode SetTemperatureThresholds uu 85 80
busctl --system get-property com.cyanskillfish.Governor /com/cyanskillfish/Governor/Range/Allowed com.cyanskillfish.Governor.Range min
busctl --system get-property com.cyanskillfish.Governor /com/cyanskillfish/Governor/Range/Allowed com.cyanskillfish.Governor.Range max
busctl --system set-property com.cyanskillfish.Governor /com/cyanskillfish/Governor com.cyanskillfish.Governor.PerformanceMode Enabled b false

# Test Mode operations (root-only)
sudo busctl --system call com.cyanskillfish.Governor /com/cyanskillfish/Governor com.cyanskillfish.Governor.TestMode SetTestMode uu 1500 1000
```

## Troubleshooting

If the service does not behave as expected, run the governor directly with verbose logging and an explicit config path:

```bash
sudo cyan-skillfish-governor-smu --verbose /etc/cyan-skillfish-governor-smu/config.toml
```

If you are running from a local build tree instead of the installed binary:

```bash
./target/release/cyan-skillfish-governor-smu --verbose ./config.toml
```

This helps isolate whether issues come from systemd startup or from configuration/runtime behavior.
