use cyan_skillfish_governor_smu::Bc250Smu;
use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
    let profile = std::env::args()
        .nth(1)
        .ok_or("usage: set_memory_profile <1|2|3>")?
        .parse::<u32>()?;
    if !matches!(profile, 1..=3) {
        return Err("profile must be 1, 2, or 3".into());
    }

    let smu = Bc250Smu::new("0000:00:00.0", true, false, 100)?;
    smu.check_test_message()?;
    smu.q3_set_perf_profile_index(profile)?;
    println!("Memory fabric performance profile set to {profile}");
    Ok(())
}
