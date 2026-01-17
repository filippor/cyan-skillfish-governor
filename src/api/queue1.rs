use crate::{Bc250Smu, Result};

impl Bc250Smu {
    // Queue 1 methods - Very limited, mostly undocumented
    
    /// Queue 1 message 0x08 (functionality unknown)
    pub fn q1_msg_0x08(&self) -> Result<u32> {
        self.send_message(1, 0x08, 0, None, None, None, true)
    }

    /// Queue 1 message 0x10 (functionality unknown)
    pub fn q1_msg_0x10(&self) -> Result<u32> {
        self.send_message(1, 0x10, 0, None, None, None, true)
    }
}
