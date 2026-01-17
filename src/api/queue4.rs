use crate::{Bc250Smu, Result};
use crate::codec::pack_u32;

impl Bc250Smu {
    // Queue 4 methods - Mostly undocumented functionality
    
    /// Queue 4 message 0x04 (functionality unknown)
    pub fn q4_msg_0x04(&self) -> Result<u32> {
        self.send_message(4, 0x04, 0, None, None, None, true)
    }

    /// Queue 4 message 0x05 (functionality unknown)
    pub fn q4_msg_0x05(&self) -> Result<u32> {
        self.send_message(4, 0x05, 0, None, None, None, true)
    }

    /// Queue 4 message 0x06 (functionality unknown)
    pub fn q4_msg_0x06(&self) -> Result<u32> {
        self.send_message(4, 0x06, 0, None, None, None, true)
    }

    /// Queue 4 message 0x07 (functionality unknown)
    pub fn q4_msg_0x07(&self) -> Result<u32> {
        self.send_message(4, 0x07, 0, None, None, None, true)
    }

    /// Queue 4 message 0x08 (functionality unknown)
    pub fn q4_msg_0x08(&self) -> Result<u32> {
        self.send_message(4, 0x08, 0, None, None, None, true)
    }

    /// Queue 4 message 0x09 (functionality unknown)
    pub fn q4_msg_0x09(&self) -> Result<u32> {
        self.send_message(4, 0x09, 0, None, None, None, true)
    }

    /// Frequency operation (functionality unknown)
    pub fn q4_freq_operation(&self, value: u32) -> Result<()> {
        self.send_message(4, 0x0A, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 4 message 0x0B (functionality unknown)
    pub fn q4_msg_0x0b(&self) -> Result<u32> {
        self.send_message(4, 0x0B, 0, None, None, None, true)
    }

    /// Queue 4 message 0x0D (functionality unknown)
    pub fn q4_msg_0x0d(&self) -> Result<u32> {
        self.send_message(4, 0x0D, 0, None, None, None, true)
    }

    /// Queue 4 message 0x10 (functionality unknown)
    pub fn q4_msg_0x10(&self) -> Result<u32> {
        self.send_message(4, 0x10, 0, None, None, None, true)
    }

    /// Queue 4 message 0x11 (functionality unknown)
    pub fn q4_msg_0x11(&self) -> Result<u32> {
        self.send_message(4, 0x11, 0, None, None, None, true)
    }
}
