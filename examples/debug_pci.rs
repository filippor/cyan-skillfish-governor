use std::fs;

fn main() {
    println!("Checking PCI devices...");
    
    // List all PCI devices
    match fs::read_dir("/sys/bus/pci/devices/") {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    println!("Found: {}", entry.file_name().to_string_lossy());
                }
            }
        }
        Err(e) => println!("Error reading PCI devices: {}", e),
    }
    
    println!("\nChecking default device 0000:00:00.0...");
    let default_path = "/sys/bus/pci/devices/0000:00:00.0/config";
    match fs::metadata(default_path) {
        Ok(meta) => println!("✓ Device exists, permissions: {:?}", meta.permissions()),
        Err(e) => println!("✗ Device not found: {}", e),
    }
    
    // Try to open the config file
    println!("\nTrying to open config file...");
    match std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(default_path)
    {
        Ok(_) => println!("✓ Successfully opened config file"),
        Err(e) => println!("✗ Failed to open config file: {}", e),
    }
}
