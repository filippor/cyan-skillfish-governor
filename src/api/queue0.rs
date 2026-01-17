use crate::{Bc250Smu, Result};
use crate::codec::{decode_u32, pack_u32, mv_to_vid, vid_to_mv};

impl Bc250Smu {
    // Queue 0 methods - General SMU control


    pub fn get_smu_version(&self) -> Result<u32> {
        self.send_message(0, 0x02, 0, None, None, Some(decode_u32), true)
    }

    pub fn get_driver_if_version(&self) -> Result<u32> {
        self.send_message(0, 0x03, 0, None, None, Some(decode_u32), true)
    }

    /// Set driver table DRAM address (high 32 bits)
    pub fn set_driver_table_dram_addr_high(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x04, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Set driver table DRAM address (low 32 bits)
    pub fn set_driver_table_dram_addr_low(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x05, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Transfer table from SMU to DRAM
    pub fn transfer_table_smu2dram(&self) -> Result<()> {
        self.send_message(0, 0x06, 0, None, None, None, true)?;
        Ok(())
    }

    /// Transfer table from DRAM to SMU
    pub fn transfer_table_dram2smu(&self) -> Result<()> {
        self.send_message(0, 0x07, 0, None, None, None, true)?;
        Ok(())
    }

    pub fn request_core_pstate(&self, pstate: u8, core_mask: u8) -> Result<()> {
        let param = ((pstate as u32 & 0xF) << 16) | (core_mask as u32 & 0xFF);
        self.send_message(0, 0x0B, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

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

    /// Request GFXCLK (functionality unclear)
    pub fn request_gfxclk(&self) -> Result<()> {
        self.send_message(0, 0x0E, 0, None, None, None, true)?;
        Ok(())
    }

    pub fn query_gfxclk(&self) -> Result<u32> {
        self.send_message(0, 0x0F, 0, None, None, Some(decode_u32), true)
    }

    pub fn query_vddcr_soc_clock(&self, index: u16) -> Result<u32> {
        let param = (index as u32 & 0xFFFF) << 16;
        self.send_message(0, 0x11, param, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Query DF (Data Fabric) P-state
    pub fn query_df_pstate(&self) -> Result<u32> {
        self.send_message(0, 0x13, 0, None, None, Some(decode_u32), true)
    }

    /// Configure S3 power-off register address (high 32 bits)
    pub fn configure_s3_pwroff_register_addr_high(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x16, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Configure S3 power-off register address (low 32 bits)
    pub fn configure_s3_pwroff_register_addr_low(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x17, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Request active WGP (Work Group Processor)
    pub fn request_active_wgp(&self) -> Result<()> {
        self.send_message(0, 0x18, 0, None, None, None, true)?;
        Ok(())
    }

    /// Set minimum deep sleep GFXCLK frequency
    pub fn set_min_deep_sleep_gfxclk_freq(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x19, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Set maximum deep sleep DFLL GFX divider
    pub fn set_max_deep_sleep_dfll_gfx_div(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x1A, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Start telemetry reporting
    pub fn start_telemetry_reporting(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x1B, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Stop telemetry reporting
    pub fn stop_telemetry_reporting(&self) -> Result<()> {
        self.send_message(0, 0x1C, 0, None, None, None, true)?;
        Ok(())
    }

    /// Clear telemetry maximum values
    pub fn clear_telemetry_max(&self) -> Result<()> {
        self.send_message(0, 0x1D, 0, None, None, None, true)?;
        Ok(())
    }

    pub fn query_active_wgp(&self) -> Result<u32> {
        self.send_message(0, 0x1E, 0, None, None, Some(decode_u32), true)
    }

    pub fn get_gfx_frequency(&self) -> Result<u32> {
        self.send_message(0, 0x37, 0, None, None, Some(decode_u32), true)
    }

    pub fn get_gfx_vid(&self) -> Result<u32> {
        let vid = self.send_message(0, 0x38, 0, None, None, Some(decode_u32), true)?;
        Ok(vid_to_mv(vid))
    }

    pub fn force_gfx_freq(&self, freq_mhz: u32) -> Result<()> {
        self.send_message(0, 0x39, freq_mhz, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn unforce_gfx_freq(&self) -> Result<()> {
        self.send_message(0, 0x3A, 0, None, None, None, true)?;
        Ok(())
    }

    pub fn force_gfx_vid(&self, mv: u32) -> Result<()> {
        let vid = mv_to_vid(mv);
        self.send_message(0, 0x3B, vid, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn unforce_gfx_vid(&self) -> Result<()> {
        self.send_message(0, 0x3C, 0, None, None, None, false)?;
        Ok(())
    }

    pub fn get_enabled_smu_features(&self) -> Result<u32> {
        self.send_message(0, 0x3D, 0, None, None, Some(decode_u32), true)
    }

    pub fn set_core_enable_mask(&self, mask: u8) -> Result<()> {
        self.send_message(0, 0x2C, (mask & 0xFF) as u32, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// GFX CAC (Current Aware Control) weight operation
    /// 
    /// CAC weights are not well documented. Related to AMD patents.
    pub fn gfx_cac_weight_operation(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x2F, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// L3 cache CAC weight operation
    pub fn l3_cac_weight_operation(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x30, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Core CAC weight packing
    pub fn pack_core_cac_weight(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x31, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Set driver table VMID
    pub fn set_driver_table_vmid(&self, value: u32) -> Result<()> {
        self.send_message(0, 0x34, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_soft_min_cclk(&self, core_id: u8, freq_mhz: u16) -> Result<u32> {
        let param = ((core_id as u32 & 0xFF) << 20) | (freq_mhz as u32 & 0xFFFF);
        self.send_message(0, 0x35, param, None, Some(pack_u32), Some(decode_u32), true)
    }

    pub fn set_soft_max_cclk(&self, core_id: u8, freq_mhz: u16) -> Result<u32> {
        let param = ((core_id as u32 & 0xFF) << 20) | (freq_mhz as u32 & 0xFFFF);
        self.send_message(0, 0x36, param, None, Some(pack_u32), Some(decode_u32), true)
    }
}
