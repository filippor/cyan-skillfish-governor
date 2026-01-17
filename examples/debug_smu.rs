use cyan_skillfish_governor_smu::{Bc250Smu, Result};

fn main() -> Result<()> {
    println!("Creating SMU instance...");
    let smu = Bc250Smu::new("0000:00:00.0", true, false, 100)?;
    println!("✓ SMU created successfully");

    println!("\nAttempting test message (value=123)...");
    match smu.test_message(123) {
        Ok(result) => println!("✓ Test message succeeded: {}", result),
        Err(e) => {
            println!("✗ Test message failed: {:?}", e);
            return Err(e);
        }
    }

    println!("\nAttempting to read GFX frequency...");
    match smu.get_gfx_frequency() {
        Ok(freq) => println!("✓ GFX frequency: {} MHz", freq),
        Err(e) => {
            println!("✗ Failed to read GFX frequency: {:?}", e);
            return Err(e);
        }
    }

    println!("\nAttempting to read GFX voltage...");
    match smu.get_gfx_vid() {
        Ok(vid) => println!("✓ GFX voltage: {} mV", vid),
        Err(e) => {
            println!("✗ Failed to read GFX voltage: {:?}", e);
            return Err(e);
        }
    }

    println!("\n✓ All operations successful!");
    Ok(())
}
