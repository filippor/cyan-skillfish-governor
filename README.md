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

## Quick Start

1. Build the binary.

```bash
cargo build --release
```

2. Copy or install with your preferred packaging flow.

3. Run manually for a quick check:

```bash
./target/release/cyan-skillfish-governor-smu ./config.toml
```

4. Or install as a service using the installer script:

```bash
sudo ./scripts/install.sh
sudo systemctl enable --now cyan-skillfish-governor-smu
```

5. Check logs:

```bash
sudo journalctl -u cyan-skillfish-governor-smu -f
```

## Generic Installation (Release Archive + Script)

If your distribution is not listed below, you can install from a release archive.

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

5. Test first with manual start, then enable at boot:

```bash
sudo systemctl start cyan-skillfish-governor-smu
sudo systemctl status cyan-skillfish-governor-smu
sudo systemctl enable cyan-skillfish-governor-smu
```

Installed configuration location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

## Package Repositories

Prebuilt/community packaging references:

- AUR: https://aur.archlinux.org/packages/cyan-skillfish-governor-smu
- COPR (Fedora/Bazzite): https://copr.fedorainfracloud.org/coprs/filippor/bazzite/

If you use these packages, still review the runtime config and make sure `dbus.enabled` matches your intended performance-mode workflow.

## Installation By Distribution

General recommendation (all distributions):

Before enabling at boot, test one manual start and check logs:

```bash
sudo systemctl start cyan-skillfish-governor-smu
sudo systemctl status cyan-skillfish-governor-smu
sudo journalctl -u cyan-skillfish-governor-smu -n 100 --no-pager
```

After that, run a real GPU workload (for example a benchmark or a game) for a few minutes and re-check service logs to confirm expected behavior under load.

If everything looks good, then enable it:

```bash
sudo systemctl enable cyan-skillfish-governor-smu
```

### Arch Linux (AUR)

Install from AUR package `cyan-skillfish-governor-smu` with your preferred AUR helper:

```bash
paru -S cyan-skillfish-governor-smu
```

Then enable and start the service:

```bash
sudo systemctl enable --now cyan-skillfish-governor-smu
```

Configuration file location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

### Fedora (COPR)

Enable the COPR repository and install:

```bash
sudo dnf copr enable filippor/bazzite
sudo dnf install cyan-skillfish-governor-smu
```

Then enable and start the service:

```bash
sudo systemctl enable --now cyan-skillfish-governor-smu
```

Configuration file location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

### Bazzite

Bazzite can consume the same COPR package source. On mutable setups, use the Fedora steps above.

On rpm-ostree based setups, layer the package and reboot:

```bash
sudo rpm-ostree install cyan-skillfish-governor-smu
systemctl reboot
```

After reboot:

```bash
sudo systemctl enable --now cyan-skillfish-governor-smu
```

Configuration file location:

```bash
/etc/cyan-skillfish-governor-smu/config.toml
```

## Systemd Service

- Service name: `cyan-skillfish-governor-smu`
- Typical commands:

```bash
sudo systemctl start cyan-skillfish-governor-smu
sudo systemctl stop cyan-skillfish-governor-smu
sudo systemctl restart cyan-skillfish-governor-smu
sudo systemctl status cyan-skillfish-governor-smu
```

Installer behavior:
- Installs binary and config under `/etc/cyan-skillfish-governor-smu`.
- Installs performance script to `/usr/local/bin/cyan-skillfish-performance-mode`.
- Installs D-Bus policy at `/etc/dbus-1/system.d/com.cyan.SkillFishGovernor.conf`.

## Performance Mode Script

The repository includes `scripts/cyan-skillfish-performance-mode`.

It controls performance mode over system D-Bus using interface:
- Service: `com.cyan.SkillFishGovernor`
- Object: `/com/cyan/SkillFishGovernor`
- Interface: `com.cyan.SkillFishGovernor.PerformanceMode`

Prerequisites:
- Governor service must be running.
- `dbus.enabled = true` in configuration.
- `busctl` (preferred) or `dbus-send` available.

### Script modes

1. Toggle explicitly:

```bash
cyan-skillfish-performance-mode --on
cyan-skillfish-performance-mode --off
cyan-skillfish-performance-mode --status
```

2. Wrap a command (auto-enable then auto-disable on exit):

```bash
cyan-skillfish-performance-mode mangohud %command%
```

3. Steam launch option example:

```bash
cyan-skillfish-performance-mode %command%
```

In wrapper mode, the script installs a cleanup trap, so performance mode is disabled when the wrapped process exits (including Ctrl+C / TERM paths handled by the script).

## D-Bus Behavior

When enabled, the governor listens for:
- `Enable`
- `Disable`
- property `Enabled`

While performance mode is active:
- target frequency is forced to max frequency
- adaptive down/up logic is bypassed
- GPU usage metric fix flushing still occurs if enabled

## Configuration

Top-level keys:

- `gpu-usage` (also accepts legacy `gpu_usage`)
  - `fix-metrics` (bool, default: `true`): enable GPU usage metrics patching.
  - `method` (`"busy-flag"` or `"process"`, default: `"busy-flag"`): how load is sampled.
  - `flush-every` (integer, default: `10`): flush patched metrics every N update cycles.

- `gpu`
  - `set-method` (`"smu"` or `"kernel"`, default: `"smu"`): backend used to apply frequency/voltage.

- `dbus`
  - `enabled` (bool, default: `false`): enable D-Bus performance-mode service.

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

## Example Configuration

Use [default-config.toml](default-config.toml) as a baseline profile.

Recommended first checks:
- Start with `gpu.set-method = "smu"`.
- Keep `safe-points` conservative and monotonic.
- Set `dbus.enabled = true` only if you plan to use the performance mode script.

## Troubleshooting

- Script reports enable/disable failure:
  - Verify service is running: `sudo systemctl status cyan-skillfish-governor-smu`
  - Verify D-Bus is enabled in config: `dbus.enabled = true`
  - Verify D-Bus policy is installed: `/etc/dbus-1/system.d/com.cyan.SkillFishGovernor.conf`

- `--status` fails:
  - Install `busctl` or `dbus-send`
  - Confirm system bus access permissions/policy

- Governor starts but does not react as expected:
  - Run with `--verbose`
  - Review journal logs for config fallback warnings and validation messages

See [default-config.toml](default-config.toml) for a full example profile.
