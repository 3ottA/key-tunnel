//! Explicit Set-1 Windows scan code to Linux input-event-code mapping.

pub fn to_linux(scan: u32, extended: bool) -> Option<u16> {
    if extended {
        return Some(match scan {
            0x1c => 96,
            0x1d => 97,
            0x35 => 98,
            0x37 => 99,
            0x38 => 100,
            0x47 => 102,
            0x48 => 103,
            0x49 => 104,
            0x4b => 105,
            0x4d => 106,
            0x4f => 107,
            0x50 => 108,
            0x51 => 109,
            0x52 => 110,
            0x53 => 111,
            0x5b => 125,
            0x5c => 126,
            0x5d => 127,
            0x10 => 165,
            0x19 => 163,
            0x20 => 113,
            0x21 => 115,
            0x22 => 164,
            0x24 => 114,
            0x2e => 116,
            0x30 => 166,
            0x32 => 172,
            0x65 => 217,
            0x66 => 158,
            0x67 => 159,
            0x6a => 173,
            0x6b => 128,
            0x6c => 167,
            0x6d => 208,
            0x6e => 168,
            0x6f => 207,
            _ => return None,
        });
    }
    Some(match scan {
        0x01 => 1,
        0x02 => 2,
        0x03 => 3,
        0x04 => 4,
        0x05 => 5,
        0x06 => 6,
        0x07 => 7,
        0x08 => 8,
        0x09 => 9,
        0x0a => 10,
        0x0b => 11,
        0x0c => 12,
        0x0d => 13,
        0x0e => 14,
        0x0f => 15,
        0x10 => 16,
        0x11 => 17,
        0x12 => 18,
        0x13 => 19,
        0x14 => 20,
        0x15 => 21,
        0x16 => 22,
        0x17 => 23,
        0x18 => 24,
        0x19 => 25,
        0x1a => 26,
        0x1b => 27,
        0x1c => 28,
        0x1d => 29,
        0x1e => 30,
        0x1f => 31,
        0x20 => 32,
        0x21 => 33,
        0x22 => 34,
        0x23 => 35,
        0x24 => 36,
        0x25 => 37,
        0x26 => 38,
        0x27 => 39,
        0x28 => 40,
        0x29 => 41,
        0x2a => 42,
        0x2b => 43,
        0x2c => 44,
        0x2d => 45,
        0x2e => 46,
        0x2f => 47,
        0x30 => 48,
        0x31 => 49,
        0x32 => 50,
        0x33 => 51,
        0x34 => 52,
        0x35 => 53,
        0x36 => 54,
        0x37 => 55,
        0x38 => 56,
        0x39 => 57,
        0x3a => 58,
        0x3b => 59,
        0x3c => 60,
        0x3d => 61,
        0x3e => 62,
        0x3f => 63,
        0x40 => 64,
        0x41 => 65,
        0x42 => 66,
        0x43 => 67,
        0x44 => 68,
        0x45 => 69,
        0x46 => 70,
        0x47 => 71,
        0x48 => 72,
        0x49 => 73,
        0x4a => 74,
        0x4b => 75,
        0x4c => 76,
        0x4d => 77,
        0x4e => 78,
        0x4f => 79,
        0x50 => 80,
        0x51 => 81,
        0x52 => 82,
        0x53 => 83,
        0x56 => 86,
        0x57 => 87,
        0x58 => 88,
        0x59 => 89,
        0x64 => 183,
        0x65 => 184,
        0x66 => 185,
        0x67 => 186,
        0x68 => 187,
        0x69 => 188,
        0x6a => 189,
        0x6b => 190,
        0x6c => 191,
        0x6d => 192,
        0x6e => 193,
        0x6f => 194,
        _ => return None,
    })
}

pub fn pause() -> u16 {
    119
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distinguishes_enter() {
        assert_eq!(to_linux(0x1c, false), Some(28));
        assert_eq!(to_linux(0x1c, true), Some(96));
    }
    #[test]
    fn distinguishes_modifiers() {
        assert_eq!(to_linux(0x1d, false), Some(29));
        assert_eq!(to_linux(0x1d, true), Some(97));
        assert_eq!(to_linux(0x38, false), Some(56));
        assert_eq!(to_linux(0x38, true), Some(100));
    }
    #[test]
    fn special_keys() {
        assert_eq!(to_linux(0x37, true), Some(99));
        assert_eq!(pause(), 119);
        assert_eq!(to_linux(0x64, false), Some(183));
    }
}
