use crate::codec::{decode_u32, mv_to_vid, pack_u32, vid_to_mv};
use crate::error::Result;
use crate::Bc250Smu;

impl Bc250Smu {
    /// Send test message and verify the response increments the value.
    pub fn test_message(&self, value: u32) -> Result<bool> {
        let response = self.send_message(3, 0x01, value, None, Some(pack_u32), Some(decode_u32), true)?;
        
        if response != value + 1 {
            return Err(crate::error::SmuError::TestMessageFailed {
                expected: value + 1,
                actual: response,
            });
        }
        Ok(true)
    }

    /// Quick test with value 123
    pub fn check_test_message(&self) -> Result<bool> {
        self.test_message(123)
    }

    /// Return the current GFX frequency in MHz.
    pub fn query_gfxclk(&self) -> Result<u32> {
        self.send_message(0, 0x0F, 0, None, None, Some(decode_u32), true)
    }

    /// Return the current GFX frequency in MHz (alias of query_gfxclk).
    pub fn get_gfx_frequency(&self) -> Result<u32> {
        self.send_message(0, 0x37, 0, None, None, Some(decode_u32), true)
    }

    /// Return the current GFX VID in mV.
    pub fn get_gfx_vid(&self) -> Result<u32> {
        let vid = self.send_message(0, 0x38, 0, None, None, Some(decode_u32), true)?;
        Ok(vid_to_mv(vid))
    }

    /// Force GFX frequency; firmware interprets the argument as MHz.
    pub fn force_gfx_freq(&self, freq_mhz: u32) -> Result<()> {
        self.send_message(0, 0x39, freq_mhz, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Clear any forced GFX frequency settings.
    pub fn unforce_gfx_freq(&self) -> Result<()> {
        self.send_message(0, 0x3A, 0, None, None, None, true)?;
        Ok(())
    }

    /// Force GFX VID using millivolts input.
    pub fn force_gfx_vid(&self, mv: u32) -> Result<()> {
        let vid = mv_to_vid(mv);
        self.send_message(0, 0x3B, vid, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Clear any forced GFX VID settings.
    pub fn unforce_gfx_vid(&self) -> Result<()> {
        self.send_message(0, 0x3C, 0, None, None, None, false)?;
        Ok(())
    }

    /// Request a CPU P-state for cores specified in the mask.
    pub fn request_core_pstate(&self, pstate: u8, core_mask: u8) -> Result<()> {
        let param = ((pstate as u32 & 0xF) << 16) | (core_mask as u32 & 0xFF);
        self.send_message(0, 0x0B, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Return the current core P-state (status 0xFF if core_id > 7).
    pub fn query_core_pstate(&self, core_id: u8) -> Result<u32> {
        self.send_message(
            0,
            0x0C,
            core_id as u32,
            None,
            Some(pack_u32),
            Some(decode_u32),
            false,
        )
    }

    /// Set soft min CCLK for a core; returns the clamped frequency in MHz.
    pub fn set_soft_min_cclk(&self, core_id: u8, freq_mhz: u16) -> Result<u32> {
        let param = ((core_id as u32 & 0xFF) << 20) | (freq_mhz as u32 & 0xFFFF);
        self.send_message(0, 0x35, param, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Set soft max CCLK for a core; returns the clamped frequency in MHz.
    pub fn set_soft_max_cclk(&self, core_id: u8, freq_mhz: u16) -> Result<u32> {
        let param = ((core_id as u32 & 0xFF) << 20) | (freq_mhz as u32 & 0xFFFF);
        self.send_message(0, 0x36, param, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Set the CPU core enable mask (lower 8 bits).
    pub fn set_core_enable_mask(&self, mask: u8) -> Result<()> {
        self.send_message(0, 0x2C, (mask & 0xFF) as u32, None, Some(pack_u32), None, true)?;
        Ok(())
    }
}
