use clap::Parser;
use cyan_skillfish_governor_smu::Bc250Smu;
use signal_hook::consts::{SIGINT, SIGTERM};
use std::collections::BTreeSet;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::os::fd::FromRawFd;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const CACHE_LINE_BYTES: f64 = 64.0;
const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
const PERF_TYPE_RAW: u32 = 4;
const PERF_FLAG_FD_CLOEXEC: usize = 8;
const DEMAND_DRAM_REFILLS: u64 = 0x843;
const PREFETCH_DRAM_REFILLS: u64 = 0x85a;
const SAMPLE_INTERVAL: Duration = Duration::from_millis(250);
const WARMUP: Duration = Duration::from_secs(2);

#[derive(Parser)]
#[command(about = "Measure single-core and multi-core memory bandwidth at a fixed fabric profile")]
struct Args {
    /// Memory fabric performance profile to measure
    #[arg(value_parser = clap::value_parser!(u32).range(1..=3))]
    profile: u32,

    /// Duration of each workload in seconds
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u64).range(4..))]
    seconds: u64,

    /// Cache size passed to each stress-ng STREAM worker
    #[arg(long, default_value = "8M")]
    stream_l3_size: String,
}

struct ProfileRestore<'a>(&'a Bc250Smu);

impl Drop for ProfileRestore<'_> {
    fn drop(&mut self) {
        match self.0.q3_set_perf_profile_index(3) {
            Ok(_) => eprintln!("Restored memory fabric profile 3"),
            Err(error) => eprintln!("WARNING: failed to restore profile 3: {error}"),
        }
    }
}

struct PerfCounter(File);

impl PerfCounter {
    fn open(cpu: u32, config: u64) -> Result<Self, Box<dyn Error>> {
        let attr = PerfEventAttr {
            type_: PERF_TYPE_RAW,
            size: size_of::<PerfEventAttr>() as u32,
            config,
            ..Default::default()
        };
        let fd = unsafe {
            nix::libc::syscall(
                nix::libc::SYS_perf_event_open,
                &attr,
                -1,
                cpu as i32,
                -1,
                PERF_FLAG_FD_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(format!(
                "cannot open CPU {cpu} performance counter: {} (run this example as root)",
                std::io::Error::last_os_error()
            )
            .into());
        }
        Ok(Self(unsafe { File::from_raw_fd(fd as i32) }))
    }

    fn read(&self) -> std::io::Result<u64> {
        let mut value = [0; size_of::<u64>()];
        (&self.0).read_exact(&mut value)?;
        Ok(u64::from_ne_bytes(value))
    }
}

#[repr(C)]
#[derive(Default)]
struct PerfEventAttr {
    type_: u32,
    size: u32,
    config: u64,
    sample_period: u64,
    sample_type: u64,
    read_format: u64,
    flags: u64,
    wakeup_events: u32,
    bp_type: u32,
    config1: u64,
    config2: u64,
}

struct BandwidthCounters {
    counters: Vec<PerfCounter>,
}

impl BandwidthCounters {
    fn new(cpus: Vec<u32>) -> Result<Self, Box<dyn Error>> {
        let mut counters = Vec::with_capacity(cpus.len() * 2);
        for &cpu in &cpus {
            counters.push(PerfCounter::open(cpu, DEMAND_DRAM_REFILLS)?);
            counters.push(PerfCounter::open(cpu, PREFETCH_DRAM_REFILLS)?);
        }
        Ok(Self { counters })
    }

    fn read(&self) -> Result<Vec<u64>, Box<dyn Error>> {
        self.counters
            .chunks_exact(2)
            .map(|pair| Ok(pair[0].read()? + pair[1].read()?))
            .collect()
    }
}

#[derive(Clone, Copy)]
struct Sample {
    total_gib: f64,
    max_cpu_gib: f64,
}

struct Measurement {
    samples: Vec<Sample>,
    status: ExitStatus,
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.0.try_wait().ok().flatten().is_none() {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let interrupted = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, Arc::clone(&interrupted))?;
    signal_hook::flag::register(SIGTERM, Arc::clone(&interrupted))?;

    require_command("stress-ng")?;
    require_command("taskset")?;

    let online_cpus = parse_cpu_list(&std::fs::read_to_string("/sys/devices/system/cpu/online")?)?;
    let physical_cpus = physical_cpu_representatives(&online_cpus)?;
    let counters = BandwidthCounters::new(online_cpus)?;

    let smu = Bc250Smu::new("0000:00:00.0", true, true, 500)?;
    smu.check_test_message()?;
    smu.q3_set_perf_profile_index(args.profile)?;
    let _restore = ProfileRestore(&smu);
    println!("Pinned memory fabric profile {}", args.profile);
    thread::sleep(Duration::from_secs(1));

    let duration = Duration::from_secs(args.seconds);
    let single_cpu = physical_cpus[0];
    println!("\nSingle-core test on CPU {single_cpu}...");
    let single = run_stream(
        &[single_cpu],
        duration,
        &args.stream_l3_size,
        &counters,
        &interrupted,
    )?;
    report("Single-core", &single)?;

    if interrupted.load(Ordering::Relaxed) {
        return Err("measurement interrupted".into());
    }

    println!(
        "\nMulti-core test on physical CPUs {}...",
        format_cpu_list(&physical_cpus)
    );
    let multi = run_stream(
        &physical_cpus,
        duration,
        &args.stream_l3_size,
        &counters,
        &interrupted,
    )?;
    report("Multi-core", &multi)?;

    println!("\nRecommended profile {} capacities:", args.profile);
    println!(
        "  core:  {:.2} GiB/s (single-core median)",
        median(
            single
                .samples
                .iter()
                .map(|sample| sample.max_cpu_gib)
                .collect()
        )
    );
    println!(
        "  total: {:.2} GiB/s (multi-core median)",
        median(
            multi
                .samples
                .iter()
                .map(|sample| sample.total_gib)
                .collect()
        )
    );
    Ok(())
}

fn run_stream(
    cpus: &[u32],
    duration: Duration,
    stream_l3_size: &str,
    counters: &BandwidthCounters,
    interrupted: &AtomicBool,
) -> Result<Measurement, Box<dyn Error>> {
    let mut child = ChildGuard(spawn_stream(cpus, duration, stream_l3_size)?);
    let started = Instant::now();
    let mut previous_time = Instant::now();
    let mut previous = counters.read()?;
    let mut samples = Vec::new();

    let status = loop {
        thread::sleep(SAMPLE_INTERVAL);
        let now = Instant::now();
        let current = counters.read()?;
        let elapsed = now.duration_since(previous_time).as_secs_f64();
        if started.elapsed() >= WARMUP {
            let per_cpu: Vec<f64> = previous
                .iter()
                .zip(&current)
                .map(|(&old, &new)| {
                    new.saturating_sub(old) as f64 * CACHE_LINE_BYTES / elapsed / GIB
                })
                .collect();
            samples.push(Sample {
                total_gib: per_cpu.iter().sum(),
                max_cpu_gib: per_cpu.iter().copied().reduce(f64::max).unwrap_or(0.0),
            });
        }
        previous = current;
        previous_time = now;

        if interrupted.load(Ordering::Relaxed) {
            child.0.kill()?;
            break child.0.wait()?;
        }
        if let Some(status) = child.0.try_wait()? {
            break status;
        }
    };

    Ok(Measurement { samples, status })
}

fn spawn_stream(
    cpus: &[u32],
    duration: Duration,
    stream_l3_size: &str,
) -> Result<Child, Box<dyn Error>> {
    let seconds = format!("{}s", duration.as_secs());
    Ok(Command::new("taskset")
        .args(["-c", &format_cpu_list(cpus), "stress-ng", "--stream"])
        .arg(cpus.len().to_string())
        .args([
            "--stream-l3-size",
            stream_l3_size,
            "--timeout",
            &seconds,
            "--metrics-brief",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?)
}

fn report(label: &str, measurement: &Measurement) -> Result<(), Box<dyn Error>> {
    if !measurement.status.success() {
        return Err(format!("{label} stress-ng exited with {}", measurement.status).into());
    }
    if measurement.samples.is_empty() {
        return Err(format!("{label} test produced no post-warm-up samples").into());
    }

    let total: Vec<f64> = measurement
        .samples
        .iter()
        .map(|sample| sample.total_gib)
        .collect();
    let max_cpu: Vec<f64> = measurement
        .samples
        .iter()
        .map(|sample| sample.max_cpu_gib)
        .collect();
    println!("{label} bandwidth ({} samples):", total.len());
    println!(
        "  aggregate: median {:.2}, average {:.2}, peak {:.2} GiB/s",
        median(total.clone()),
        average(&total),
        maximum(&total)
    );
    println!(
        "  max CPU:   median {:.2}, average {:.2}, peak {:.2} GiB/s",
        median(max_cpu.clone()),
        average(&max_cpu),
        maximum(&max_cpu)
    );
    Ok(())
}

fn require_command(command: &str) -> Result<(), Box<dyn Error>> {
    if Command::new(command)
        .arg("--help")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_err()
    {
        return Err(format!("required command not found: {command}").into());
    }
    Ok(())
}

fn physical_cpu_representatives(online: &[u32]) -> Result<Vec<u32>, Box<dyn Error>> {
    let mut cores = BTreeSet::new();
    let mut cpus = Vec::new();
    for &cpu in online {
        let topology = format!("/sys/devices/system/cpu/cpu{cpu}/topology");
        let package = std::fs::read_to_string(format!("{topology}/physical_package_id"))?
            .trim()
            .parse::<u32>()?;
        let core = std::fs::read_to_string(format!("{topology}/core_id"))?
            .trim()
            .parse::<u32>()?;
        if cores.insert((package, core)) {
            cpus.push(cpu);
        }
    }
    if cpus.is_empty() {
        return Err("no online physical CPU cores found".into());
    }
    Ok(cpus)
}

fn parse_cpu_list(list: &str) -> Result<Vec<u32>, Box<dyn Error>> {
    let mut cpus = Vec::new();
    for range in list.trim().split(',') {
        let (start, end) = range
            .split_once('-')
            .map_or((range, range), |(start, end)| (start, end));
        let start = start.parse::<u32>()?;
        let end = end.parse::<u32>()?;
        if start > end {
            return Err("invalid online CPU range".into());
        }
        cpus.extend(start..=end);
    }
    Ok(cpus)
}

fn format_cpu_list(cpus: &[u32]) -> String {
    cpus.iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    }
}

fn average(values: &[f64]) -> f64 {
    values.iter().sum::<f64>() / values.len() as f64
}

fn maximum(values: &[f64]) -> f64 {
    values.iter().copied().reduce(f64::max).unwrap_or(0.0)
}
