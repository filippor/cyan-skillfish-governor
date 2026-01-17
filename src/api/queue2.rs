use crate::{Bc250Smu, Result};
use crate::codec::{decode_u32, pack_u32};

impl Bc250Smu {
    // Queue 2 methods - Device information and feature control

    pub fn q2_get_constant(&self) -> Result<u32> {
        self.send_message(2, 0x03, 0, None, None, Some(decode_u32), true)
    }

    pub fn get_device_name_chunk(&self, index: u8) -> Result<u32> {
        self.send_message(2, 0x04, index as u32, None, Some(pack_u32), Some(decode_u32), true)
    }

    pub fn get_device_name(&self) -> Result<String> {
        let mut bytes = Vec::with_capacity(48);
        
        for i in 0..12 {
            let chunk = self.get_device_name_chunk(i)?;
            bytes.extend_from_slice(&chunk.to_le_bytes());
        }
        
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        String::from_utf8(bytes[..end].to_vec())
            .map_err(|e| crate::error::SmuError::Io(
                std::io::Error::new(std::io::ErrorKind::InvalidData, e)
            ))
    }

    pub fn enable_smu_features(&self, mask_low: u32, mask_high: Option<u32>) -> Result<()> {
        self.send_message(2, 0x05, mask_low, mask_high, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn disable_smu_features(&self, mask_low: u32, mask_high: Option<u32>) -> Result<()> {
        self.send_message(2, 0x06, mask_low, mask_high, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 2 message 0x07 (functionality unknown)
    pub fn q2_msg_0x07(&self) -> Result<u32> {
        self.send_message(2, 0x07, 0, None, None, None, true)
    }

    /// Queue 2 message 0x08 (functionality unknown)
    pub fn q2_msg_0x08(&self) -> Result<u32> {
        self.send_message(2, 0x08, 0, None, None, None, true)
    }

    /// Queue 2 message 0x09 (functionality unknown)
    pub fn q2_msg_0x09(&self) -> Result<u32> {
        self.send_message(2, 0x09, 0, None, None, None, true)
    }

    /// Queue 2 message 0x0A (functionality unknown)
    pub fn q2_msg_0x0a(&self) -> Result<u32> {
        self.send_message(2, 0x0A, 0, None, None, None, true)
    }

    /// Queue 2 message 0x0B (functionality unknown)
    pub fn q2_msg_0x0b(&self) -> Result<u32> {
        self.send_message(2, 0x0B, 0, None, None, None, true)
    }

    /// Queue 2 message 0x0C (functionality unknown)
    pub fn q2_msg_0x0c(&self) -> Result<u32> {
        self.send_message(2, 0x0C, 0, None, None, None, true)
    }

    /// Set some address (high 32 bits) - functionality unknown
    pub fn q2_set_addr_high(&self, value: u32) -> Result<()> {
        self.send_message(2, 0x0D, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Set some address (low 32 bits) - functionality unknown
    pub fn q2_set_addr_low(&self, value: u32) -> Result<()> {
        self.send_message(2, 0x0E, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 2 message 0x0F (functionality unknown)
    pub fn q2_msg_0x0f(&self) -> Result<u32> {
        self.send_message(2, 0x0F, 0, None, None, None, true)
    }

    /// Queue 2 message 0x10 (functionality unknown)
    pub fn q2_msg_0x10(&self) -> Result<u32> {
        self.send_message(2, 0x10, 0, None, None, None, true)
    }

    /// Queue 2 message 0x13 (functionality unknown)
    pub fn q2_msg_0x13(&self) -> Result<u32> {
        self.send_message(2, 0x13, 0, None, None, None, true)
    }

    /// Queue 2 message 0x14 (functionality unknown)
    pub fn q2_msg_0x14(&self) -> Result<u32> {
        self.send_message(2, 0x14, 0, None, None, None, true)
    }

    /// Queue 2 message 0x15 (functionality unknown)
    pub fn q2_msg_0x15(&self) -> Result<u32> {
        self.send_message(2, 0x15, 0, None, None, None, true)
    }

    /// Queue 2 message 0x16 (functionality unknown)
    pub fn q2_msg_0x16(&self) -> Result<u32> {
        self.send_message(2, 0x16, 0, None, None, None, true)
    }

    pub fn cpu_droop_calibration(&self, test_voltage_mv: u16, margin_mv: u16) -> Result<()> {
        let param = ((margin_mv as u32 & 0xFFFF) << 16) | (test_voltage_mv as u32 & 0xFFFF);
        self.send_message(2, 0x17, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 2 message 0x1A (functionality unknown)
    pub fn q2_msg_0x1a(&self) -> Result<u32> {
        self.send_message(2, 0x1A, 0, None, None, None, true)
    }

    /// Queue 2 message 0x20 (functionality unknown)
    pub fn q2_msg_0x20(&self) -> Result<u32> {
        self.send_message(2, 0x20, 0, None, None, None, true)
    }

    /// Queue 2 message 0x21 (functionality unknown)
    pub fn q2_msg_0x21(&self) -> Result<u32> {
        self.send_message(2, 0x21, 0, None, None, None, true)
    }

    /// Queue 2 message 0x22 (functionality unknown)
    pub fn q2_msg_0x22(&self) -> Result<u32> {
        self.send_message(2, 0x22, 0, None, None, None, true)
    }

    /// Queue 2 message 0x23 (functionality unknown)
    pub fn q2_msg_0x23(&self) -> Result<u32> {
        self.send_message(2, 0x23, 0, None, None, None, true)
    }

    /// Queue 2 message 0x29 (functionality unknown)
    pub fn q2_msg_0x29(&self) -> Result<u32> {
        self.send_message(2, 0x29, 0, None, None, None, true)
    }

    /// Probably power limit settings (functionality unknown)
    pub fn q2_power_limit_settings(&self) -> Result<u32> {
        self.send_message(2, 0x2C, 0, None, None, None, true)
    }

    /// Sibling of 0x2C but returns value (functionality unknown)
    pub fn q2_power_limit_sibling(&self) -> Result<u32> {
        self.send_message(2, 0x2D, 0, None, None, None, true)
    }

    /// Queue 2 message 0x2E (functionality unknown)
    pub fn q2_msg_0x2e(&self) -> Result<u32> {
        self.send_message(2, 0x2E, 0, None, None, None, true)
    }

    /// Queue 2 message 0x2F (functionality unknown)
    pub fn q2_msg_0x2f(&self) -> Result<u32> {
        self.send_message(2, 0x2F, 0, None, None, None, true)
    }

    /// Queue 2 message 0x30 (functionality unknown)
    pub fn q2_msg_0x30(&self) -> Result<u32> {
        self.send_message(2, 0x30, 0, None, None, None, true)
    }
}
