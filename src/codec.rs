/// Decode a u32 value (identity function for consistency)
pub fn decode_u32(value: u32) -> u32 {
    value
}

/// Pack a u32 value (identity function for consistency)
pub fn pack_u32(value: u32) -> u32 {
    value
}

/// Convert millivolts to VID encoding
/// Formula: VID = (1.55 - (mV / 1000.0)) / 0.00625
pub fn mv_to_vid(mv: u32) -> u32 {
    let volts = mv as f64 / 1000.0;
    let vid = (1.55 - volts) / 0.00625;
    vid.round() as u32
}

/// Convert VID encoding to millivolts
/// Formula: mV = ((VID * -0.00625) + 1.55) * 1000.0
pub fn vid_to_mv(vid: u32) -> u32 {
    let volts = (vid as f64 * -0.00625) + 1.55;
    let mv = volts * 1000.0;
    mv.round() as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vid_conversion() {
        // Test some known values
        let test_values = [
            (1000, mv_to_vid(1000)),
            (1125, mv_to_vid(1125)),
            (1200, mv_to_vid(1200)),
            (1325, mv_to_vid(1325)),
            (1550, mv_to_vid(1550)),
        ];

        for (mv, vid) in test_values {
            let converted_back = vid_to_mv(vid);
            println!("{} mV -> VID {} -> {} mV", mv, vid, converted_back);
            // Allow small rounding error
            assert!((converted_back as i32 - mv as i32).abs() <= 1,
                    "Conversion failed: {} mV -> VID {} -> {} mV", 
                    mv, vid, converted_back);
        }
    }

    #[test]
    fn test_specific_values() {
        // Test the example from your code: 1125 mV
        let vid_1125 = mv_to_vid(1125);
        let back_to_mv = vid_to_mv(vid_1125);
        println!("1125 mV -> VID: {} -> {} mV", vid_1125, back_to_mv);
        assert_eq!(back_to_mv, 1125);

        // Test boundary values
        // 1550 mV should give VID = 0
        assert_eq!(mv_to_vid(1550), 0);
        assert_eq!(vid_to_mv(0), 1550);
        
        // Lower voltages give higher VID values
        // 1000 mV should give VID = 88
        let vid_1000 = mv_to_vid(1000);
        println!("1000 mV -> VID: {}", vid_1000);
        assert_eq!(vid_1000, 88);
    }
}
