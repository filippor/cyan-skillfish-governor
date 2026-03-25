# Cyan Skillfish GPU governor
GPU governor for the AMD Cyan Skillfish APU.
Continously maintains a target frequency, and adjusts the actual GPU frequency when the deviation is too great.
If the CPU is continously busy for too long, ramps up the target frequency rapidly.

this version can set voltage/frequency using either the smu api (thanks to the work of [bc250collective](https://github.com/bc250-collective/)) or the kernel sysfs path.

Takes a TOML config file path as its only argument.
Keys are:
* `gpu-usage`
  * `fix-metrics` : boolean default true fix gpu usage metrics
  * `method` : 'process' or 'busy-flag' default 'busy-flag' choose the method to get the gpu usage sample busy-flag or total time from process
  * `flush-every` : integer default 10 flush patched gpu metrics to disk every N update cycles
* `gpu`
  * `set-method`: 'smu' or 'kernel' default 'smu' choose the frequency/voltage control backend
* `timing`
  * `intervals`: in µs
    * `sample`: how often to sample GPU load used only for gpu-usage = 'busy-flag'
      (it's a single bit, so needs to be sampled more often than you'd think)
    * `adjust`: how often to consider adjusting the frequency
  * `burst-samples`: while the GPU has been busy for this many samples in a row,
    enter "burst mode", increasing the frequency at the `timing.ramp_rates.burst` rate.
    Set to 0 to disable burst mode. This work only for gpu-usage = 'busy-flag'
  * `down-events`: number of event below `load-target.low` to step down
  * `ramp_rates`: how quickly to increase/decrease GPU frequency, in MHz/ms
    * `normal`: ramp rate for normal adjustments
    * `burst`: ramp rate in burst mode
* `frequency-thresholds`: in MHz
  * `adjust`: how large a proposed adjustment must be to actually be carried out
* `load-target`: as a fraction
  * `upper`: GPU load above which target frequency is increased
  * `lower`: GPU load below which target frequency is decreased
* `temperature` in °C
  * `throttling` if temperature is greather  start reducing max frequency
  * `throttling_recovery` if temperaure is lower restore max frequency
* `safe-points`: known safe/stable power points, array of tables with two keys:
  * `frequency`: GPU frequency in MHz
  * `voltage`: GPU supply voltage in mV

See also [default-config.toml](default-config.toml).
