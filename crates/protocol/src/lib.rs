//! Fixed-width, ABI-independent wire protocol for Remote Input Bridge.

use std::fmt;
use std::io::{self, Read, Write};

pub const MAGIC: [u8; 4] = *b"RIB1";
pub const VERSION: u8 = 1;
pub const FRAME_LEN: usize = 36;
pub const MAX_LINUX_KEY_CODE: u16 = 0x2ff;

pub mod flags {
    pub const EXTENDED_E0: u16 = 1 << 0;
    pub const EXTENDED_E1: u16 = 1 << 1;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum MessageType {
    Hello = 1,
    Ready = 2,
    KeyEvent = 3,
    ReleaseAll = 4,
    Ping = 5,
    Pong = 6,
    Error = 7,
    Goodbye = 8,
}

impl TryFrom<u8> for MessageType {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Ready),
            3 => Ok(Self::KeyEvent),
            4 => Ok(Self::ReleaseAll),
            5 => Ok(Self::Ping),
            6 => Ok(Self::Pong),
            7 => Ok(Self::Error),
            8 => Ok(Self::Goodbye),
            other => Err(ProtocolError::UnknownMessage(other)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum KeyState {
    None = 0,
    Up = 1,
    Down = 2,
    Repeat = 3,
}

impl TryFrom<u8> for KeyState {
    type Error = ProtocolError;
    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::None),
            1 => Ok(Self::Up),
            2 => Ok(Self::Down),
            3 => Ok(Self::Repeat),
            other => Err(ProtocolError::InvalidKeyState(other)),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Frame {
    pub message_type: MessageType,
    pub flags: u16,
    pub sequence: u64,
    pub key_code: u16,
    pub key_state: KeyState,
    pub timestamp_micros: u64,
    /// Message-specific value. HELLO uses capability bits; ERROR uses an error code.
    pub data: u32,
}

impl Frame {
    pub fn control(message_type: MessageType, sequence: u64, timestamp_micros: u64) -> Self {
        Self {
            message_type,
            flags: 0,
            sequence,
            key_code: 0,
            key_state: KeyState::None,
            timestamp_micros,
            data: 0,
        }
    }

    pub fn key(
        sequence: u64,
        key_code: u16,
        key_state: KeyState,
        flags: u16,
        timestamp_micros: u64,
    ) -> Self {
        Self {
            message_type: MessageType::KeyEvent,
            flags,
            sequence,
            key_code,
            key_state,
            timestamp_micros,
            data: 0,
        }
    }

    pub fn encode(&self) -> [u8; FRAME_LEN] {
        let mut out = [0u8; FRAME_LEN];
        out[0..4].copy_from_slice(&MAGIC);
        out[4] = VERSION;
        out[5] = self.message_type as u8;
        out[6..8].copy_from_slice(&self.flags.to_be_bytes());
        out[8..16].copy_from_slice(&self.sequence.to_be_bytes());
        out[16..18].copy_from_slice(&self.key_code.to_be_bytes());
        out[18] = self.key_state as u8;
        out[20..28].copy_from_slice(&self.timestamp_micros.to_be_bytes());
        out[28..32].copy_from_slice(&self.data.to_be_bytes());
        let checksum = fnv1a32(&out[..32]);
        out[32..36].copy_from_slice(&checksum.to_be_bytes());
        out
    }

    pub fn decode(bytes: &[u8; FRAME_LEN]) -> Result<Self, ProtocolError> {
        if bytes[0..4] != MAGIC {
            return Err(ProtocolError::BadMagic);
        }
        if bytes[4] != VERSION {
            return Err(ProtocolError::UnsupportedVersion(bytes[4]));
        }
        if bytes[19] != 0 {
            return Err(ProtocolError::ReservedBits);
        }
        let expected = u32::from_be_bytes(bytes[32..36].try_into().unwrap());
        if fnv1a32(&bytes[..32]) != expected {
            return Err(ProtocolError::BadChecksum);
        }
        let frame = Self {
            message_type: MessageType::try_from(bytes[5])?,
            flags: u16::from_be_bytes(bytes[6..8].try_into().unwrap()),
            sequence: u64::from_be_bytes(bytes[8..16].try_into().unwrap()),
            key_code: u16::from_be_bytes(bytes[16..18].try_into().unwrap()),
            key_state: KeyState::try_from(bytes[18])?,
            timestamp_micros: u64::from_be_bytes(bytes[20..28].try_into().unwrap()),
            data: u32::from_be_bytes(bytes[28..32].try_into().unwrap()),
        };
        frame.validate()?;
        Ok(frame)
    }

    pub fn validate(&self) -> Result<(), ProtocolError> {
        if self.flags & !(flags::EXTENDED_E0 | flags::EXTENDED_E1) != 0 {
            return Err(ProtocolError::ReservedBits);
        }
        if self.message_type == MessageType::KeyEvent {
            if self.key_code > MAX_LINUX_KEY_CODE {
                return Err(ProtocolError::InvalidKeyCode(self.key_code));
            }
            if self.key_state == KeyState::None {
                return Err(ProtocolError::InvalidKeyState(0));
            }
        } else if self.key_code != 0 || self.key_state != KeyState::None || self.flags != 0 {
            return Err(ProtocolError::UnexpectedPayload);
        }
        Ok(())
    }

    pub fn read_from(mut reader: impl Read) -> Result<Self, ReadError> {
        let mut buf = [0u8; FRAME_LEN];
        reader.read_exact(&mut buf).map_err(ReadError::Io)?;
        Self::decode(&buf).map_err(ReadError::Protocol)
    }

    pub fn write_to(&self, mut writer: impl Write) -> io::Result<()> {
        writer.write_all(&self.encode())?;
        writer.flush()
    }
}

fn fnv1a32(bytes: &[u8]) -> u32 {
    bytes.iter().fold(0x811c9dc5, |hash, byte| {
        (hash ^ u32::from(*byte)).wrapping_mul(0x01000193)
    })
}

#[derive(Debug, Eq, PartialEq)]
pub enum ProtocolError {
    BadMagic,
    UnsupportedVersion(u8),
    UnknownMessage(u8),
    InvalidKeyState(u8),
    InvalidKeyCode(u16),
    ReservedBits,
    UnexpectedPayload,
    BadChecksum,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ProtocolError {}

#[derive(Debug)]
pub enum ReadError {
    Io(io::Error),
    Protocol(ProtocolError),
}
impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ReadError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_key_frame_is_stable() {
        let frame = Frame::key(
            0x0102030405060708,
            28,
            KeyState::Down,
            flags::EXTENDED_E0,
            99,
        );
        let bytes = frame.encode();
        assert_eq!(bytes.len(), FRAME_LEN);
        assert_eq!(Frame::decode(&bytes).unwrap(), frame);
        assert_eq!(
            &bytes[..20],
            &[82, 73, 66, 49, 1, 3, 0, 1, 1, 2, 3, 4, 5, 6, 7, 8, 0, 28, 2, 0]
        );
    }

    #[test]
    fn partial_reads_are_supported() {
        struct Chunks {
            data: Vec<u8>,
            at: usize,
        }
        impl Read for Chunks {
            fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
                if self.at == self.data.len() {
                    return Ok(0);
                }
                let n = out.len().min(3).min(self.data.len() - self.at);
                out[..n].copy_from_slice(&self.data[self.at..self.at + n]);
                self.at += n;
                Ok(n)
            }
        }
        let expected = Frame::control(MessageType::Hello, 1, 2);
        let got = Frame::read_from(Chunks {
            data: expected.encode().to_vec(),
            at: 0,
        })
        .unwrap();
        assert_eq!(got, expected);
    }

    #[test]
    fn corruption_is_rejected() {
        let mut bytes = Frame::control(MessageType::Ping, 1, 2).encode();
        bytes[12] ^= 1;
        assert_eq!(Frame::decode(&bytes), Err(ProtocolError::BadChecksum));
    }
}
