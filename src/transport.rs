use nix::fcntl::{Flock, FlockArg};
use std::fs::File;
use std::ops::Deref;
use std::os::unix::fs::FileExt;
use std::path::PathBuf;
use crate::error::{Result, SmuError};

pub struct Bc250PciTransport {
    config_path: PathBuf,
    use_flock: bool,
    file: Option<File>,
}

impl Bc250PciTransport {
    pub fn new(bdf: &str, use_flock: bool) -> Self {
        let config_path = PathBuf::from(format!("/sys/bus/pci/devices/{}/config", bdf));
        Self {
            config_path,
            use_flock,
            file: None,
        }
    }

    pub fn open(&mut self) -> Result<()> {
        if self.file.is_none() {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.config_path)?;
            self.file = Some(file);
        }
        Ok(())
    }

    pub fn close(&mut self) {
        self.file = None;
    }

    pub fn read_config32(&self, offset: u64) -> Result<u32> {
        let file = self.file.as_ref().ok_or(SmuError::TransportNotOpened)?;
        
        if self.use_flock {
            let cloned = file.try_clone()?;
            let locked = Flock::lock(cloned, FlockArg::LockExclusive)
                .map_err(|(_, e)| std::io::Error::from_raw_os_error(e as i32))?;
            
            let mut buf = [0u8; 4];
            locked.deref().read_exact_at(&mut buf, offset)?;
            Ok(u32::from_le_bytes(buf))
        } else {
            let mut buf = [0u8; 4];
            file.read_exact_at(&mut buf, offset)?;
            Ok(u32::from_le_bytes(buf))
        }
    }

    pub fn write_config32(&self, offset: u64, value: u32) -> Result<()> {
        let file = self.file.as_ref().ok_or(SmuError::TransportNotOpened)?;
        
        if self.use_flock {
            let cloned = file.try_clone()?;
            let locked = Flock::lock(cloned, FlockArg::LockExclusive)
                .map_err(|(_, e)| std::io::Error::from_raw_os_error(e as i32))?;
            
            let buf = value.to_le_bytes();
            locked.deref().write_all_at(&buf, offset)?;
            Ok(())
        } else {
            let buf = value.to_le_bytes();
            file.write_all_at(&buf, offset)?;
            Ok(())
        }
    }

    pub fn read_smu_reg(&self, reg: u32) -> Result<u32> {
        self.write_config32(0xB8, reg)?;
        self.read_config32(0xBC)
    }

    pub fn write_smu_reg(&self, reg: u32, value: u32) -> Result<()> {
        self.write_config32(0xB8, reg)?;
        self.write_config32(0xBC, value)
    }
}

impl Drop for Bc250PciTransport {
    fn drop(&mut self) {
        self.close();
    }
}
