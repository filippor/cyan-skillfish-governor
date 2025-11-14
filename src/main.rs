use std::{
    collections::BTreeMap,
    fs::File,
    io::{Error as IoError, ErrorKind, Write},
    os::fd::AsRawFd,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use libdrm_amdgpu_sys::{AMDGPU::DeviceHandle, PCI::BUS_INFO};
use toml::Table;


// cyan_skillfish.gfx1013.mmGRBM_STATUS
const GRBM_STATUS_REG: u32 = 0x2004;
// cyan_skillfish.gfx1013.mmGRBM_STATUS.GUI_ACTIVE
const GPU_ACTIVE_BIT: u8 = 31;

struct Config {  
    sampling_interval: Duration,
    adjustment_interval: Duration,
    finetune_interval: Duration,
    ramp_rate: f32,
    ramp_rate_burst: f32,
    burst_samples: Option<u32>,
    significant_change: u16,
    small_change: u16,
    up_thresh : f32,
    down_thresh : f32,
    throttling_temp : Option<u32>,
    throttling_recovery_temp : Option<u32>,
}

struct GPU {
    // Other fields
    reader: GPUReader,
    writer: GPUWriter,
}

struct GPUReader {
    dev_handle: DeviceHandle,
    samples: u64,
    min_freq : u16,
    max_freq : u16,
}

struct GPUWriter {
    pp_file: File,
    safe_points: BTreeMap<u16, u16>,  
}


fn main() -> Result<(), Box<dyn std::error::Error>> {
    let (config,safe_points) = parse_config(std::env::args()
        .nth(1)
        .map(std::fs::read_to_string)
        .unwrap_or(Ok("".to_string())))?;

    
    let temp_check_period = config.adjustment_interval * 10;
    let mut gpu = GPU::new(safe_points)?;
    
    let (send, mut recv) = watch::channel(gpu.reader.min_freq);
    
    let jh_gov: JoinHandle<Result<(), IoError>> = std::thread::spawn(move || {
        let mut curr_freq: u16 = gpu.reader.min_freq;
        let mut target_freq = gpu.reader.min_freq;
        let mut max_freq = gpu.reader.max_freq;
        
        let mut last_adjustment = Instant::now();
        let mut last_finetune = Instant::now();
        let mut last_temp_check = Instant::now();
        

        let burst_freq_step = (config.ramp_rate_burst * config.sampling_interval.as_millis() as f32).clamp(1.0, (gpu.reader.max_freq - gpu.reader.min_freq).into()) as u16;
        let freq_step = (config.ramp_rate * config.sampling_interval.as_millis() as f32) as u16;

        loop {
            let (average_load, burst_length) = gpu.reader.poll_and_get_load()?;        

            let burst = config.burst_samples
                .map_or(false, |burst_samples| burst_length >= burst_samples);
            
            if burst || last_adjustment.elapsed() >= config.adjustment_interval {
                //Temperature Management
                let temp = gpu.reader.read_temperature()?;  
                if let Some(max_temp) = config.throttling_temp && last_temp_check.elapsed()>temp_check_period{                  
                    if (temp > max_temp) && (max_freq >= gpu.reader.min_freq + freq_step) {
                        last_temp_check = Instant::now();
                        max_freq -= freq_step;
                        println!("throttling temp {temp} freq {max_freq}");
                    } else if let Some(recovery_temp) = config.throttling_recovery_temp 
                        && temp < recovery_temp && max_freq != gpu.reader.max_freq{
                            max_freq = gpu.reader.max_freq;
                            println!("recover throttling temp {temp} freq {max_freq}");
                    } 
                }
                

                if burst {

                    target_freq += burst_freq_step;
                } else if average_load > config.up_thresh {
                    target_freq += freq_step;
                } else if average_load < config.down_thresh {
                    target_freq -= freq_step;
                }
                target_freq = target_freq.clamp(gpu.reader.min_freq, max_freq);

                let hit_bounds = target_freq == gpu.reader.min_freq || target_freq == max_freq;
                let big_change = curr_freq.abs_diff(target_freq) >= config.significant_change;
                let finetune = (last_finetune.elapsed()>= config.finetune_interval)
                    && curr_freq.abs_diff(target_freq) >= config.small_change;
                     
                if curr_freq != target_freq && (
                    burst || 
                    hit_bounds || 
                    big_change || 
                    finetune
                ) {
                    send.send(target_freq);
                    curr_freq = target_freq;
                    last_finetune = Instant::now();
                }
                last_adjustment = Instant::now();
            }            
            std::thread::sleep(config.sampling_interval);          
        }
    });
    let jh_set: JoinHandle<Result<(), IoError>> = std::thread::spawn(move || {
        loop {
            gpu.writer.change_freq(recv.wait())?;       
        }
    });

    let () = jh_set.join().unwrap()?;
    let () = jh_gov.join().unwrap()?;
    Ok(())
}

impl GPUReader{
    pub fn poll_and_get_load(&mut self)->Result<(f32,u32), IoError>{
        let res = self.dev_handle
            .read_mm_registers(GRBM_STATUS_REG)
            .map_err(IoError::from_raw_os_error)?;
        let gui_busy = (res & (1 << GPU_ACTIVE_BIT)) > 0;
        self.samples <<= 1;
        if gui_busy {
            self.samples |= 1;
        }

        let average_load = (self.samples.count_ones() as f32)/ 64.0;
        let burst_length = (!self.samples).trailing_zeros();
        Ok((average_load, burst_length)) 
               
    }
    pub fn read_temperature(&mut self)->Result<u32, IoError>{
 
         let temp = self.dev_handle
            .sensor_info(libdrm_amdgpu_sys::AMDGPU::SENSOR_INFO::SENSOR_TYPE::GPU_TEMP)
            .map_err(IoError::from_raw_os_error)?;
        Ok((temp/1000) as u32)
       
    }
}
impl GPUWriter {
     pub fn change_freq(&mut self, freq : u16)->Result<(), IoError>{
        let vol = *self.safe_points
                .range(freq..)
                .next()
                .ok_or(IoError::other(
                    "tried to set a frequency beyond max safe point",
                ))?
                .1;
        self.pp_file.write_all(format!("vc 0 {freq} {vol}").as_bytes())?;
        self.pp_file.write_all("c".as_bytes())?;
        Ok(())  
     }
}



fn parse_config(path : Result<String,std::io::Error>) -> Result<(Config, BTreeMap<u16, u16>),Box<dyn std::error::Error>>{
    let config = path?.parse::<Table>()?;

    let timing = config.get("timing").and_then(|t| t.as_table());
    let intervals = timing
        .and_then(|t| t.get("intervals"))
        .and_then(|t| t.as_table());
    // us
    let sampling_interval: u16 = intervals
        .and_then(|t| t.get("sample"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u16::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u16::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!("timing.intervals.sample {s}, replaced with the default value of 2 ms");
            2000
        });
    // us
    let adjustment_interval = intervals
        .and_then(|t| t.get("adjust"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            (v >= i64::from(sampling_interval))
                .then_some(v)
                .ok_or("must be at least as high as timing.intervals.sample")
        })
        .and_then(|v| {
            u64::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u64::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.intervals.adjust {s}, replaced with the default of \
                10 * timing.intervals.sample"
            );
            10 * u64::from(sampling_interval)
        });
    // us
    let finetune_interval = intervals
        .and_then(|t| t.get("finetune"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            (v >= i64::from(sampling_interval))
                .then_some(v)
                .ok_or("must be at least as high as timing.intervals.sample")
        })
        .and_then(|v| {
            u64::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u64::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.intervals.finetune {s}, replaced with the default of \
                50_000 * timing.intervals.adjust"
            );
            50_000 * u64::from(sampling_interval)
        });

    // samples
    let burst_samples = match timing
        .and_then(|t| t.get("burst-samples"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
    {
        Err(s) => {
            println!(
                "timing.burst-samples {s}, replaced with the default of \
            48"
            );
            Some(48)
        }
        Ok(0) => None,
        Ok(v @ 1..=64) => Some(v as u32),
        Ok(65..) => {
            println!("timing.burst-samples can be at most 64, clamping");
            Some(64)
        }
        Ok(i64::MIN..0) => {
             println!("timing.burst-samples is negative Disabling burst");
            None
        }
    };

    let ramp_rates = timing
        .and_then(|t| t.get("ramp-rates"))
        .and_then(|t| t.as_table());
    // MHz/ms
    let ramp_rate = ramp_rates
        .and_then(|t| t.get("normal"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            v.is_sign_positive()
                .then_some(v)
                .ok_or("must have positive sign")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "timing.ramp-rates.normal {s}, replaced with the default value of \
                1 MHz/ms"
            );
            1.0
        });
    // MHz/ms
    let ramp_rate_burst = ramp_rates
        .and_then(|t| t.get("burst"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            v.is_sign_positive()
                .then_some(v)
                .ok_or("must have positive sign")
        })
        .map(|v| v as f32)
        .and_then(|v| {
            (v > ramp_rate || burst_samples.is_none()).then_some(v).ok_or(
                "must, if bursting is active, be greater than timing.ramp-rates.normal \
                (if you want to turn bursting off, set timing.burst-samples = 0)",
            )
        })
        .unwrap_or_else(|s| {
            println!(
                "timing.ramp-rates.burst {s}, replaced with the default value of \
                200 * timing.ramp-rates.normal"
            );
            200.0 * ramp_rate
        });

    let freq_threshs = config
        .get("frequency-thresholds")
        .and_then(|t| t.as_table());
    // MHz
    let small_change = freq_threshs
        .and_then(|t| t.get("finetune"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u16::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u16::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "frequency-thresholds.finetune {s}, replaced with the default of \
                10 MHz"
            );
            10
        });
    // MHz
    let significant_change = freq_threshs
        .and_then(|t| t.get("adjust"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        .and_then(|v| v.is_positive().then_some(v).ok_or("must be positive"))
        .and_then(|v| {
            u16::try_from(v).map_err(|_| &*format!("cannot be greater than {}", u16::MAX).leak())
        })
        .unwrap_or_else(|s| {
            println!(
                "frequency-thresholds.adjust {s}, replaced with the default of \
                10 * frequency-thresholds.finetune"
            );
            10 * small_change
        });

    let load_threshs = config.get("load-target").and_then(|t| t.as_table());
    // fraction
    let up_thresh = load_threshs
        .and_then(|t| t.get("upper"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            (0.0..1.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be fractional")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "load-target.upper {s}, replaced with the default value of \
                0.95"
            );
            0.95
        });
    // fraction
    let down_thresh = load_threshs
        .and_then(|t| t.get("lower"))
        .ok_or("is missing")
        .and_then(|v| {
            v.as_float()
                .or_else(|| v.as_integer().map(|v| v as f64))
                .ok_or("must be a number")
        })
        .and_then(|v| {
            (0.0..1.0)
                .contains(&v)
                .then_some(v)
                .ok_or("must be fractional")
        })
        .map(|v| v as f32)
        .unwrap_or_else(|s| {
            println!(
                "load-target.lower {s}, replaced with the default value of \
                load-target.upper - 0.15"
            );
            (up_thresh - 0.15).max(0.0)
        });
    let down_thresh = if down_thresh > up_thresh {
        println!("load-target.lower can't be greater than load-target.upper, clamping");
        up_thresh
    } else {
        down_thresh
    };

    // MHz, mV
    let safe_points: BTreeMap<u16, u16> = if let Some(array) = config.get("safe-points") {
        let array = array.as_array().ok_or(IoError::new(
            ErrorKind::InvalidInput,
            "safe-points must be an array",
        ))?;
        if array.is_empty() {
            Err(IoError::new(
                ErrorKind::InvalidInput,
                "safe-points must not be empty",
            ))?;
        }
        let mut safe_points = BTreeMap::new();
        for (i, t) in array.iter().enumerate() {
            let t = t.as_table().ok_or_else(|| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!("safe-points[{i}] must be a table"),
                )
            })?;

            // MHz
            let frequency = t
                .get("frequency")
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].frequency must exist"),
                    )
                })?
                .as_integer()
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].frequency must be an integer"),
                    )
                })?;
            let frequency = u16::try_from(frequency).map_err(|_| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "safe-points[{i}].frequency must be between 0 and {} inclusive",
                        u16::MAX
                    ),
                )
            })?;

            // mV
            let voltage = t
                .get("voltage")
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].voltage must exist"),
                    )
                })?
                .as_integer()
                .ok_or_else(|| {
                    IoError::new(
                        ErrorKind::InvalidInput,
                        format!("safe-points[{i}].voltage must be an integer"),
                    )
                })?;
            let voltage = u16::try_from(voltage).map_err(|_| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "safe-points[{i}].voltage must be between 0 and {} inclusive",
                        u16::MAX
                    ),
                )
            })?;

            if safe_points.insert(frequency, voltage).is_some() {
                Err(IoError::new(
                    ErrorKind::InvalidInput,
                    format!("multiple supposedly safe voltages for {frequency} MHz"),
                ))?;
            }
        }
        let mut highest_pair = (0, 0);
        for (frequency, voltage) in &safe_points {
            let pair = (*voltage, *frequency);
            if pair < highest_pair {
                Err(IoError::new(
                    ErrorKind::InvalidInput,
                    format!(
                        "supposedly safe voltage {} mV for {} MHz is higher than \
                        {voltage} mV for {frequency} MHz",
                        highest_pair.0, highest_pair.1,
                    ),
                ))?;
            } else {
                highest_pair = pair;
            }
        }
        safe_points
    } else {
        println!(
            "safe-points undefined, using conservative defaults:\n\
            * 350 MHz @ 700 mV\n\
            * 2000 MHz @ 1000 mV"
        );
        BTreeMap::from([(350, 700), (2000, 1000)])
    };
    
    let temperature = config.get("temperature").and_then(|t| t.as_table());
    let throttling_temp = match temperature
        .and_then(|t| t.get("throttling"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
    {
        Err(s) => {
            println!(
                "temperature.throttling {s}, disabled"
            );
            None
        }
        Ok(v @ 0..=110) => Some(v as u32),
        Ok(111..) => {
            println!("temperature.throttling can be at most 110, clamping");
            Some(110)
        }
        Ok(i64::MIN..0)=> {
                 println!("temperature.throttling is negative disable throttling");
                 None
            },
    };
    let throttling_recovery_temp = if let Some(max_recovery) = throttling_temp {
         match temperature
        .and_then(|t| t.get("throttling_recovery"))
        .ok_or("is missing")
        .and_then(|v| v.as_integer().ok_or("must be an integer"))
        {
            Err(s) => {
                println!(
                    "temperature.throttling_recovery {s}, disabled"
                );
                None
            }
            Ok(0) => None,
            Ok(v @ 1..=i64::MAX) => {
                if v>=max_recovery as i64 {
                    let tmp = max_recovery -1;
                    println!("temperature.throttling_recovery can be at most temperature.throttling -1 ({tmp}), clamping");
                    Some(max_recovery -1)
                }else{
                    Some(v as u32)
                }
            }
            Ok(i64::MIN..0) => {
                 println!("temperature.throttling_recovery is negative disable recovery");
                 None
            },
        }
    }else{
        None
    };
    
  
    Ok((
         Config { 
            sampling_interval: Duration::from_micros(u64::from(sampling_interval)), 
            finetune_interval:Duration::from_micros(u64::from(finetune_interval)), 
            ramp_rate: ramp_rate, 
            burst_samples: burst_samples,
            ramp_rate_burst : ramp_rate_burst,
            up_thresh : up_thresh,
            down_thresh : down_thresh,
            adjustment_interval : Duration::from_micros(adjustment_interval),
            significant_change : significant_change,
            small_change: small_change,
            throttling_temp : throttling_temp,
            throttling_recovery_temp : throttling_recovery_temp,
        },
         safe_points
    ))
} 

impl GPU{
    fn new (safe_points: BTreeMap<u16, u16>) -> Result<GPU, Box<dyn std::error::Error>>{
        
        let location = BUS_INFO {
            domain: 0,
            bus: 1,
            dev: 0,
            func: 0,
        };
        let sysfs_path = location.get_sysfs_path();
        let vendor = std::fs::read_to_string(sysfs_path.join("vendor"))?;
        let device = std::fs::read_to_string(sysfs_path.join("device"))?;
        if !((vendor == "0x1002\n") && (device == "0x13fe\n")) {
            Err(IoError::other(
                "Cyan Skillfish GPU not found at expected PCI bus location",
            ))?;
        }
        let card = File::open(location.get_drm_render_path()?)?;
        let (dev_handle, _, _) =
            DeviceHandle::init(card.as_raw_fd()).map_err(IoError::from_raw_os_error)?;

        let info = dev_handle
            .device_info()
            .map_err(IoError::from_raw_os_error)?;

        let pp_file = std::fs::OpenOptions::new().write(true).open(
            dev_handle
                .get_sysfs_path()
                .map_err(IoError::from_raw_os_error)?
                .join("pp_od_clk_voltage"),
        )?;
            // given in kHz, we need MHz
        let min_engine_clock = info.min_engine_clock / 1000;
        let max_engine_clock = info.max_engine_clock / 1000;
        let mut min_freq = *safe_points.first_key_value().unwrap().0;
        if u64::from(min_freq) < min_engine_clock {
            eprintln!("GPU minimum frequency higher than lowest safe frequency, clamping");
            min_freq = u16::try_from(min_engine_clock)?;
        }
        let mut max_freq = *safe_points.last_key_value().unwrap().0;
        if u64::from(max_freq) > max_engine_clock {
            eprintln!("GPU maximum frequency lower than highest safe frequency, clamping");
            max_freq = u16::try_from(max_engine_clock)?;
        }
        
        Ok(GPU { 
            reader : GPUReader { 
                dev_handle: dev_handle, 
                samples: 0, 
                min_freq:min_freq,
                max_freq:max_freq,
            },
            writer: GPUWriter { 
                pp_file: pp_file,
                safe_points:safe_points
            }
        })
    }
}