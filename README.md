# Cyan Skillfish GPU governor
GPU governor for the AMD Cyan Skillfish APU.
Continously maintains a target frequency, and adjusts the actual GPU frequency when the deviation is too great.
If the CPU is continously busy for too long, ramps up the target frequency rapidly.
this is the original version 
check tt or smu branch for different version

Takes a TOML config file path as its only argument.
Keys are:
* `timing`
  * `intervals`: in µs
    * `sample`: how often to sample GPU load
      (it's a single bit, so needs to be sampled more often than you'd think)
    * `adjust`: how often to consider adjusting the frequency
    * `finetune` how long since the last adjustment to consider making a fine-tuning adjustment
  * `burst-samples`: while the GPU has been busy for this many samples in a row,
    enter "burst mode", increasing the frequency at the `timing.ramp_rates.burst` rate.
    Set to 0 to disable burst mode.
  * `ramp_rates`: how quickly to increase/decrease GPU frequency, in MHz/ms
    * `normal`: ramp rate for normal adjustments
    * `burst`: ramp rate in burst mode
* `frequency-thresholds`: in MHz
  * `adjust`: how large a proposed adjustment must be to actually be carried out
  * `finetune`: how large a fine-tuning adjustment must be to be actually carried out
* `load-target`: as a fraction
  * `upper`: GPU load above which target frequency is increased
  * `lower`: GPU load below which target frequency is decreased
* `safe-points`: known safe/stable power points, array of tables with two keys:
  * `frequency`: GPU frequency in MHz
  * `voltage`: GPU supply voltage in mV

See also `default-config.toml`. 
