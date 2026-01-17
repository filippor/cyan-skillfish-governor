/// Decode a u32 value (identity function for consistency)
pub fn decode_u32(value: u32) -> u32 {
    value
}

/// Pack a u32 value (identity function for consistency)
pub fn pack_u32(value: u32) -> u32 {
    value
}

/// Pack a signed 16-bit value into u32 (sign-extended to 32 bits)
pub fn pack_s16(value: i16) -> u32 {
    let bytes = value.to_le_bytes();
    u32::from_le_bytes([bytes[0], bytes[1], 0, 0])
}

/// Pack a 32-bit float into u32
pub fn pack_f32(value: f32) -> u32 {
    u32::from_le_bytes(value.to_le_bytes())
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
    fn test_pack_s16() {
        assert_eq!(pack_s16(0), 0x00000000);
        assert_eq!(pack_s16(100), 0x00000064);
        assert_eq!(pack_s16(-5), 0x0000FFFB);
    }

    #[test]
    fn test_pack_f32() {
        // 0.1 as f32
        let packed = pack_f32(0.1);
        let unpacked = f32::from_le_bytes(packed.to_le_bytes());
        assert!((unpacked - 0.1).abs() < 0.0001);
    }

    #[test]
    fn test_vid_conversion() {
        let test_values = [
            (1000, mv_to_vid(1000)),
            (1125, mv_to_vid(1125)),
            (1200, mv_to_vid(1200)),
            (1325, mv_to_vid(1325)),
            (1550, mv_to_vid(1550)),
        ];

        for (mv, vid) in test_values {
            let converted_back = vid_to_mv(vid);
            assert!((converted_back as i32 - mv as i32).abs() <= 1);
        }
    }
}
