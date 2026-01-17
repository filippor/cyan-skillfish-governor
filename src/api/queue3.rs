use crate::{Bc250Smu, Result};
use crate::codec::{decode_u32, pack_u32, pack_s16, pack_f32, mv_to_vid};

impl Bc250Smu {
    // Queue 3 methods - Advanced control and monitoring

    /// Queue 3 message 0x04 (functionality unknown)
    pub fn q3_msg_0x04(&self) -> Result<u32> {
        self.send_message(3, 0x04, 0, None, None, None, true)
    }

    /// Queue 3 message 0x0A (functionality unknown)
    pub fn q3_msg_0x0a(&self) -> Result<u32> {
        self.send_message(3, 0x0A, 0, None, None, None, true)
    }

    /// Queue 3 message 0x0B (functionality unknown)
    pub fn q3_msg_0x0b(&self) -> Result<u32> {
        self.send_message(3, 0x0B, 0, None, None, None, true)
    }

    /// Queue 3 message 0x0C (functionality unknown)
    pub fn q3_msg_0x0c(&self) -> Result<u32> {
        self.send_message(3, 0x0C, 0, None, None, None, true)
    }

    /// Queue 3 message 0x0D (functionality unknown)
    pub fn q3_msg_0x0d(&self) -> Result<u32> {
        self.send_message(3, 0x0D, 0, None, None, None, true)
    }

    /// Queue 3 message 0x0E (functionality unknown)
    pub fn q3_msg_0x0e(&self) -> Result<u32> {
        self.send_message(3, 0x0E, 0, None, None, None, true)
    }

    pub fn set_cpu_gpu_vid(&self, kind: u16, mv: u32) -> Result<()> {
        let vid = mv_to_vid(mv);
        let param = ((kind as u32 & 0xFFFF) << 16) | (vid & 0xFFFF);
        self.send_message(3, 0x0F, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn unforce_cpu_gpu_vid(&self, kind: u16) -> Result<()> {
        let param = (kind as u32 & 0xFFFF) << 16;
        self.send_message(3, 0x10, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 3 message 0x11 (functionality unknown)
    pub fn q3_msg_0x11(&self) -> Result<u32> {
        self.send_message(3, 0x11, 0, None, None, None, true)
    }

    /// Queue 3 message 0x14 (functionality unknown)
    pub fn q3_msg_0x14(&self) -> Result<u32> {
        self.send_message(3, 0x14, 0, None, None, None, true)
    }

    /// Queue 3 message 0x15 (functionality unknown)
    pub fn q3_msg_0x15(&self) -> Result<u32> {
        self.send_message(3, 0x15, 0, None, None, None, true)
    }

    /// Queue 3 message 0x18 (functionality unknown)
    pub fn q3_msg_0x18(&self) -> Result<u32> {
        self.send_message(3, 0x18, 0, None, None, None, true)
    }

    /// Queue 3 message 0x19 (functionality unknown)
    pub fn q3_msg_0x19(&self) -> Result<u32> {
        self.send_message(3, 0x19, 0, None, None, None, true)
    }

    /// Queue 3 message 0x1A (functionality unknown)
    pub fn q3_msg_0x1a(&self) -> Result<u32> {
        self.send_message(3, 0x1A, 0, None, None, None, true)
    }

    /// Queue 3 message 0x1B (functionality unknown)
    pub fn q3_msg_0x1b(&self) -> Result<u32> {
        self.send_message(3, 0x1B, 0, None, None, None, true)
    }

    /// Set SoC clock for index (functionality unclear)
    pub fn q3_set_soc_clock_for_index(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x1D, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Set performance profile index (functionality unclear)
    pub fn q3_set_perf_profile_index(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x1E, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_max_temperature_cpu_gpu(&self, temp_c: u32) -> Result<()> {
        self.send_message(3, 0x20, temp_c, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 3 message 0x24 (functionality unknown)
    pub fn q3_msg_0x24(&self) -> Result<u32> {
        self.send_message(3, 0x24, 0, None, None, None, true)
    }

    pub fn set_oc_clk(&self, core_id: u8, freq_mhz: u16) -> Result<()> {
        let param = ((core_id as u32 & 0xFF) << 16) | (freq_mhz as u32 & 0xFFFF);
        self.send_message(3, 0x25, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn unset_oc_clk(&self, core_id: u8) -> Result<()> {
        let param = (core_id as u32 & 0xFF) << 16;
        self.send_message(3, 0x26, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Secure access message 0x27 (requires BIOS flag)
    pub fn q3_secure_0x27(&self) -> Result<u32> {
        self.send_message(3, 0x27, 0, None, None, None, true)
    }

    /// Write to DAT 8B08 secure (functionality unclear)
    pub fn q3_write_to_dat_8b08_secure(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x28, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Write to pointer at DAT (functionality unclear)
    pub fn q3_write_to_pointer_at_dat(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x29, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Secure access message 0x2A (requires BIOS flag)
    pub fn q3_secure_0x2a(&self) -> Result<u32> {
        self.send_message(3, 0x2A, 0, None, None, None, true)
    }

    /// Write into DAT 00008B0C (functionality unclear)
    pub fn q3_writes_into_dat_00008b0c(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x2B, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Secure access message 0x2C (requires BIOS flag)
    pub fn q3_secure_0x2c(&self) -> Result<u32> {
        self.send_message(3, 0x2C, 0, None, None, None, true)
    }

    /// Secure access message 0x2D (requires BIOS flag)
    pub fn q3_secure_0x2d(&self) -> Result<u32> {
        self.send_message(3, 0x2D, 0, None, None, None, true)
    }

    /// Secure access message 0x2E (requires BIOS flag)
    pub fn q3_secure_0x2e(&self) -> Result<u32> {
        self.send_message(3, 0x2E, 0, None, None, None, true)
    }

    /// Secure access message 0x2F (requires BIOS flag)
    pub fn q3_secure_0x2f(&self) -> Result<u32> {
        self.send_message(3, 0x2F, 0, None, None, None, true)
    }

    pub fn get_cpu_gpu_vid_offset(&self, selector: u8) -> Result<u32> {
        self.send_message(3, 0x30, selector as u32, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Return DAT 00015778 (functionality unknown)
    pub fn q3_return_dat_00015778(&self) -> Result<u32> {
        self.send_message(3, 0x34, 0, None, None, Some(decode_u32), true)
    }

    pub fn get_current_cpu_voltage(&self) -> Result<u32> {
        self.send_message(3, 0x36, 0, None, None, Some(decode_u32), true)
    }

    pub fn get_current_gpu_voltage(&self) -> Result<u32> {
        self.send_message(3, 0x37, 0, None, None, Some(decode_u32), true)
    }

    /// Get more clock assigned to state (functionality unclear)
    pub fn q3_get_more_clock_assigned_to_state(&self, value: u32) -> Result<u32> {
        self.send_message(3, 0x38, value, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Get other clock assigned to state (functionality unclear)
    pub fn q3_get_other_clock_assigned_to_state(&self, value: u32) -> Result<u32> {
        self.send_message(3, 0x39, value, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Get some clock assigned to state (functionality unclear)
    pub fn q3_get_some_clock_assigned_to_state(&self, value: u32) -> Result<u32> {
        self.send_message(3, 0x3A, value, None, Some(pack_u32), Some(decode_u32), true)
    }

    pub fn get_clk_assigned_to_p_state(&self, pstate: u8) -> Result<u32> {
        self.send_message(3, 0x3B, pstate as u32, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Enable SMU features (Queue 3 variant)
    pub fn q3_enable_smu_features(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x3C, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Disable SMU features (Queue 3 variant)
    pub fn q3_disable_smu_features(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x3D, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn get_cpu_temp_max(&self) -> Result<u32> {
        self.send_message(3, 0x40, 0, None, None, Some(decode_u32), true)
    }

    /// Read from performance profile table (functionality unclear)
    pub fn q3_read_from_perf_profile_table(&self, value: u32) -> Result<u32> {
        self.send_message(3, 0x41, value, None, Some(pack_u32), Some(decode_u32), true)
    }

    pub fn get_vddcrsoc_dpm_value(&self, index: u16) -> Result<u32> {
        let param = (index as u32 & 0xFFFF) << 16;
        self.send_message(3, 0x42, param, None, Some(pack_u32), Some(decode_u32), true)
    }

    pub fn get_core_freq(&self, core_id: u8) -> Result<u32> {
        self.send_message(3, 0x43, core_id as u32, None, Some(pack_u32), Some(decode_u32), true)
    }

    /// Return status 0xFE
    pub fn q3_return_status_0xfe_47(&self) -> Result<u32> {
        self.send_message(3, 0x47, 0, None, None, None, true)
    }

    /// Return status 0xFE
    pub fn q3_return_status_0xfe_48(&self) -> Result<u32> {
        self.send_message(3, 0x48, 0, None, None, None, true)
    }

    pub fn set_cpu_vid_offset(&self, offset: i8) -> Result<()> {
        if offset < -5 || offset > 5 {
            return Err(crate::error::SmuError::Io(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Offset must be in range -5 to 5"
                )
            ));
        }
        self.send_message(3, 0x49, offset as u32, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_gfx_vid_offset(&self, offset: i8) -> Result<()> {
        if offset < -5 || offset > 5 {
            return Err(crate::error::SmuError::Io(
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "Offset must be in range -5 to 5"
                )
            ));
        }
        self.send_message(3, 0x4A, offset as u32, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// CPU droop calibration (Queue 3 variant)
    pub fn q3_cpu_droop_calibration(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x4B, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn gfx_droop_calibration(&self, test_voltage_mv: u16, margin_mv: u16) -> Result<()> {
        let param = ((margin_mv as u32 & 0xFFFF) << 16) | (test_voltage_mv as u32 & 0xFFFF);
        self.send_message(3, 0x4C, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_cpu_vid_offset_large(&self, offset_v: f32) -> Result<()> {
        let packed = pack_f32(offset_v);
        self.send_message(3, 0x4D, packed, None, None, None, true)?;
        Ok(())
    }

    pub fn set_gpu_vid_offset_large(&self, offset_v: f32) -> Result<()> {
        let packed = pack_f32(offset_v);
        self.send_message(3, 0x4E, packed, None, None, None, true)?;
        Ok(())
    }

    /// Queue 3 message 0x4F (functionality unknown)
    pub fn q3_msg_0x4f(&self) -> Result<u32> {
        self.send_message(3, 0x4F, 0, None, None, None, true)
    }

    pub fn scale_vid_curve(&self, value: i16) -> Result<()> {
        let packed = pack_s16(value);
        self.send_message(3, 0x50, packed, None, None, None, true)?;
        Ok(())
    }

    /// Set CPU coefficient (functionality unclear)
    pub fn q3_set_cpu_coeff(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x51, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_cpu_clock_stretch_coeff(&self, coeff: u32) -> Result<()> {
        self.send_message(3, 0x52, coeff, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_ccx_clock_stretch_coeff(&self, coeff: u32) -> Result<()> {
        self.send_message(3, 0x53, coeff, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 3 message 0x54 (functionality unknown)
    pub fn q3_msg_0x54(&self) -> Result<u32> {
        self.send_message(3, 0x54, 0, None, None, None, true)
    }

    /// Queue 3 message 0x55 (functionality unknown)
    pub fn q3_msg_0x55(&self) -> Result<u32> {
        self.send_message(3, 0x55, 0, None, None, None, true)
    }

    /// Queue 3 message 0x56 (functionality unknown)
    pub fn q3_msg_0x56(&self) -> Result<u32> {
        self.send_message(3, 0x56, 0, None, None, None, true)
    }

    /// Queue 3 message 0x58 (functionality unknown)
    pub fn q3_msg_0x58(&self) -> Result<u32> {
        self.send_message(3, 0x58, 0, None, None, None, true)
    }

    /// Queue 3 message 0x59 (functionality unknown)
    pub fn q3_msg_0x59(&self) -> Result<u32> {
        self.send_message(3, 0x59, 0, None, None, None, true)
    }

    /// Queue 3 message 0x5A (functionality unknown)
    pub fn q3_msg_0x5a(&self) -> Result<u32> {
        self.send_message(3, 0x5A, 0, None, None, None, true)
    }

    /// Queue 3 message 0x5B (functionality unknown)
    pub fn q3_msg_0x5b(&self) -> Result<u32> {
        self.send_message(3, 0x5B, 0, None, None, None, true)
    }

    /// Something frequency related
    pub fn q3_something_freq_related_5c(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x5C, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Something frequency related
    pub fn q3_something_freq_related_5d(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x5D, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 3 message 0x5E (functionality unknown)
    pub fn q3_msg_0x5e(&self) -> Result<u32> {
        self.send_message(3, 0x5E, 0, None, None, None, true)
    }

    /// Write some CPU frequency
    pub fn q3_write_some_cpu_frequency(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x5F, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Something P-state related
    pub fn q3_something_pstate_related(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x60, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Set DAT 000133FC value
    pub fn q3_set_dat_000133fc_value(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x65, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Reset DAT 000133FC value to 0
    pub fn q3_reset_dat_000133fc_value(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x66, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Return zero (typically)
    pub fn q3_zero_return(&self) -> Result<u32> {
        self.send_message(3, 0x67, 0, None, None, Some(decode_u32), true)
    }

    /// Queue 3 message 0x6A (functionality unknown)
    pub fn q3_msg_0x6a(&self) -> Result<u32> {
        self.send_message(3, 0x6A, 0, None, None, None, true)
    }

    /// Queue 3 message 0x6B (functionality unknown)
    pub fn q3_msg_0x6b(&self) -> Result<u32> {
        self.send_message(3, 0x6B, 0, None, None, None, true)
    }

    /// Set temperature parameters
    pub fn q3_set_temperature_parameters(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x6C, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn force_clock_stretching_vid(&self, cpu_vid_mv: u16, ccx_vid_mv: u16) -> Result<()> {
        let param = ((ccx_vid_mv as u32 & 0xFFFF) << 16) | (cpu_vid_mv as u32 & 0xFFFF);
        self.send_message(3, 0x6D, param, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// CPU coefficients
    pub fn q3_cpu_coefficients(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x6E, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 3 message 0x6F (functionality unknown)
    pub fn q3_msg_0x6f(&self) -> Result<u32> {
        self.send_message(3, 0x6F, 0, None, None, None, true)
    }

    /// Queue 3 message 0x70 (functionality unknown)
    pub fn q3_msg_0x70(&self) -> Result<u32> {
        self.send_message(3, 0x70, 0, None, None, None, true)
    }

    /// Queue 3 message 0x71 (functionality unknown)
    pub fn q3_msg_0x71(&self) -> Result<u32> {
        self.send_message(3, 0x71, 0, None, None, None, true)
    }

    /// Queue 3 message 0x72 (functionality unknown)
    pub fn q3_msg_0x72(&self) -> Result<u32> {
        self.send_message(3, 0x72, 0, None, None, None, true)
    }

    /// Queue 3 message 0x73 (functionality unknown)
    pub fn q3_msg_0x73(&self) -> Result<u32> {
        self.send_message(3, 0x73, 0, None, None, None, true)
    }

    /// Queue 3 message 0x74 (functionality unknown)
    pub fn q3_msg_0x74(&self) -> Result<u32> {
        self.send_message(3, 0x74, 0, None, None, None, true)
    }

    /// Queue 3 message 0x75 (functionality unknown)
    pub fn q3_msg_0x75(&self) -> Result<u32> {
        self.send_message(3, 0x75, 0, None, None, None, true)
    }

    /// Queue 3 message 0x76 (functionality unknown)
    pub fn q3_msg_0x76(&self) -> Result<u32> {
        self.send_message(3, 0x76, 0, None, None, None, true)
    }

    pub fn set_cpu_max_current(&self, current_ma: u32) -> Result<()> {
        self.send_message(3, 0x77, current_ma, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn get_current_perf_sample(&self) -> Result<u32> {
        self.send_message(3, 0x7F, 0, None, None, Some(decode_u32), true)
    }

    pub fn get_sample_interval_max(&self) -> Result<u32> {
        self.send_message(3, 0x80, 0, None, None, Some(decode_u32), true)
    }

    /// Queue 3 message 0x85 (functionality unknown)
    pub fn q3_msg_0x85(&self) -> Result<u32> {
        self.send_message(3, 0x85, 0, None, None, None, true)
    }

    /// Queue 3 message 0x86 (functionality unknown)
    pub fn q3_msg_0x86(&self) -> Result<u32> {
        self.send_message(3, 0x86, 0, None, None, None, true)
    }

    /// Queue 3 message 0x87 (functionality unknown)
    pub fn q3_msg_0x87(&self) -> Result<u32> {
        self.send_message(3, 0x87, 0, None, None, None, true)
    }

    pub fn set_cpu_max_temperature(&self, temp_c: u32) -> Result<()> {
        self.send_message(3, 0x8B, temp_c, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_gpu_max_temperature(&self, temp_c: u32) -> Result<()> {
        self.send_message(3, 0x8C, temp_c, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn get_current_sample_interval(&self) -> Result<u32> {
        self.send_message(3, 0x8D, 0, None, None, Some(decode_u32), true)
    }

    pub fn set_vid_main_2_limit(&self, limit_mv: u32) -> Result<()> {
        self.send_message(3, 0x8E, limit_mv, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn set_max_cpu_boost_clk(&self, freq_mhz: u32) -> Result<()> {
        self.send_message(3, 0x8F, freq_mhz, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Queue 3 message 0x90 (functionality unknown)
    pub fn q3_msg_0x90(&self) -> Result<u32> {
        self.send_message(3, 0x90, 0, None, None, None, true)
    }

    /// Queue 3 message 0x91 (functionality unknown)
    pub fn q3_msg_0x91(&self) -> Result<u32> {
        self.send_message(3, 0x91, 0, None, None, None, true)
    }

    /// Queue 3 message 0x96 (functionality unknown)
    pub fn q3_msg_0x96(&self) -> Result<u32> {
        self.send_message(3, 0x96, 0, None, None, None, true)
    }

    /// Queue 3 message 0x98 (functionality unknown)
    pub fn q3_msg_0x98(&self) -> Result<u32> {
        self.send_message(3, 0x98, 0, None, None, None, true)
    }

    /// Modify P-state 0 parameter
    pub fn q3_modify_pstate_0_parameter(&self, value: u32) -> Result<()> {
        self.send_message(3, 0x99, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    pub fn disable_extra_cpu_gpu_voltage(&self, flag: bool) -> Result<()> {
        let arg = if flag { 1 } else { 0 };
        self.send_message(3, 0x9A, arg, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// Switch core bilinear model
    pub fn q3_switch_core_bilinear_model(&self) -> Result<u32> {
        self.send_message(3, 0x9B, 0, None, None, None, true)
    }

    /// Queue 3 message 0x9C (functionality unknown)
    pub fn q3_msg_0x9c(&self) -> Result<u32> {
        self.send_message(3, 0x9C, 0, None, None, None, true)
    }

    /// CPU related operation
    pub fn q3_cpu_related_a7(&self, value: u32) -> Result<()> {
        self.send_message(3, 0xA7, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }

    /// CPU related operation
    pub fn q3_cpu_related_a8(&self, value: u32) -> Result<()> {
        self.send_message(3, 0xA8, value, None, Some(pack_u32), None, true)?;
        Ok(())
    }
}
