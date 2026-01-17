use cyan_skillfish_governor_direct::{Bc250Smu, Result};

fn main() -> Result<()> {
    let smu = Bc250Smu::new("0000:00:00.0", true, false, 100)?;

    smu.check_test_message()?;
    println!("SMU communication verified!");

    smu.force_gfx_vid(1000)?;
    println!("Set GPU voltage to 1125 mV");

    smu.force_gfx_freq(1500)?;
    println!("Set GPU frequency to 2350 MHz");

    // Read back current settings
    let freq = smu.get_gfx_frequency()?;
    let vid = smu.get_gfx_vid()?;
    println!("Current GPU: {} MHz @ {} mV", freq, vid);

    Ok(())
}
