//! Frame encoding/decoding.

use std::fmt;

/// Frame magic: `7D 4E 43` (`}NC`).
pub const MAGIC: [u8; 3] = [0x7D, 0x4E, 0x43];

/// Byte 5 of every observed frame is constant.
const CONST5: u8 = 0x02;

/// MIDI SysEx start / end bytes. The device transports Nanocore frames as
/// SysEx packets, so `F0 ... F7` surrounds each one on the wire.
pub const SYSEX_START: u8 = 0xF0;
pub const SYSEX_END: u8 = 0xF7;

/// Strip a SysEx wrapper: leading `F0`s and trailing `F7`s.
///
/// Only the *edges* are trimmed — an `F7` inside a binary payload is data.
fn strip_sysex_wrapping(bytes: &[u8]) -> &[u8] {
    let mut s = bytes;
    while s.first() == Some(&SYSEX_START) {
        s = &s[1..];
    }
    while s.last() == Some(&SYSEX_END) {
        s = &s[..s.len() - 1];
    }
    s
}

/// Direction byte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 0x70: app -> device.
    Tx,
    /// 0x71: device -> app (reply).
    Rx,
    /// 0x72: device -> app (unsolicited event).
    Event,
}

impl Direction {
    pub fn from_byte(b: u8) -> Option<Self> {
        match b {
            0x70 => Some(Direction::Tx),
            0x71 => Some(Direction::Rx),
            0x72 => Some(Direction::Event),
            _ => None,
        }
    }
    pub fn to_byte(self) -> u8 {
        match self {
            Direction::Tx => 0x70,
            Direction::Rx => 0x71,
            Direction::Event => 0x72,
        }
    }
}

/// A decoded Nanocore message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub direction: Direction,
    /// Byte 4 (observed 0x00 normal, 0x02 live, 0x22 preset write, 0x40 RX).
    pub flags: u8,
    /// Little-endian counter in bytes 6..8 (echoed by the device).
    pub seq: u16,
    /// Message type (byte 8).
    pub kind: u8,
    /// Bytes after the type byte.
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(direction: Direction, flags: u8, seq: u16, kind: u8, payload: Vec<u8>) -> Self {
        Self { direction, flags, seq, kind, payload }
    }

    /// True if every payload byte is a legal MIDI data byte (high bit clear).
    ///
    /// The transport is a SysEx stream, so any payload byte >= 0x80 is
    /// interpreted as a status byte and the message is discarded. Float
    /// parameters therefore have to be encoded 7-bit safe — see
    /// [`crate::protocol::encode_param_value`].
    pub fn is_seven_bit_safe(&self) -> bool {
        self.payload.iter().all(|b| b & 0x80 == 0)
    }

    /// Encode into the on-wire byte sequence.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(9 + self.payload.len());
        out.extend_from_slice(&MAGIC);
        out.push(self.direction.to_byte());
        out.push(self.flags);
        out.push(CONST5);
        out.push((self.seq & 0xFF) as u8);
        out.push((self.seq >> 8) as u8);
        out.push(self.kind);
        out.extend_from_slice(&self.payload);
        out
    }

    /// Decode a single frame. The input may contain trailing bytes
    /// (e.g. the next frame); only this frame is consumed.
    ///
    /// On the wire frames are wrapped in SysEx (`F0 7D 4E 43 ... F7`), so a
    /// leading `F0` / trailing `F7` belonging to the MIDI layer is stripped.
    pub fn decode(bytes: &[u8]) -> Result<(Frame, usize), DecodeError> {
        let bytes = strip_sysex_wrapping(bytes);
        if bytes.len() < 9 {
            return Err(DecodeError::TooShort);
        }
        if bytes[0..3] != MAGIC {
            return Err(DecodeError::BadMagic([bytes[0], bytes[1], bytes[2]]));
        }
        let direction = Direction::from_byte(bytes[3]).ok_or(DecodeError::BadDirection(bytes[3]))?;
        let flags = bytes[4];
        let seq = (bytes[6] as u16) | ((bytes[7] as u16) << 8);
        let kind = bytes[8];
        let mut payload = bytes[9..].to_vec();
        while payload.last() == Some(&SYSEX_END) {
            payload.pop();
        }
        Ok((Frame { direction, flags, seq, kind, payload }, bytes.len()))
    }

    /// Split a byte stream into consecutive frames.
    ///
    /// Frames carry no explicit length, but each one is terminated by `F7`
    /// (SysEx end) and starts with the `7D 4E 43` magic, so we split on the
    /// magic and cut at the terminator.
    pub fn split_stream(data: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        let mut i = 0;
        while i + 3 <= data.len() {
            if data[i..i + 3] == MAGIC {
                // A terminator is an F7 that is followed by the end of the
                // buffer, another F0, or the next frame's magic — an F7 in
                // the middle of a binary payload is just data.
                let end = (i..data.len())
                    .find(|&j| {
                        data[j] == SYSEX_END
                            && (j + 1 >= data.len()
                                || data[j + 1] == SYSEX_START
                                || data[j + 1..].starts_with(&MAGIC))
                    })
                    .unwrap_or(data.len());
                if let Ok((f, _)) = Frame::decode(&data[i..end]) {
                    frames.push(f);
                }
                i = end + 1;
            } else {
                i += 1;
            }
        }
        frames
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeError {
    TooShort,
    BadMagic([u8; 3]),
    BadDirection(u8),
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DecodeError::TooShort => write!(f, "frame too short"),
            DecodeError::BadMagic(b) => write!(f, "bad magic {:02x?}", b),
            DecodeError::BadDirection(b) => write!(f, "bad direction byte 0x{:02x}", b),
        }
    }
}

impl std::error::Error for DecodeError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Frame {
        Frame::new(
            Direction::Tx,
            0x00,
            0x1234,
            0x6D,
            vec![0x00, 0x02, 0x00, 0x00, 0x09, 0x0C],
        )
    }

    #[test]
    fn direction_bytes_round_trip() {
        for (b, d) in [
            (0x70, Direction::Tx),
            (0x71, Direction::Rx),
            (0x72, Direction::Event),
        ] {
            assert_eq!(Direction::from_byte(b), Some(d));
            assert_eq!(d.to_byte(), b);
        }
        assert_eq!(Direction::from_byte(0x00), None);
    }

    #[test]
    fn encode_decode_round_trip() {
        let f = sample();
        let wire = f.encode();
        // Header: magic | dir | flags | const 0x02 | LE seq | type.
        assert_eq!(
            &wire[..9],
            &[0x7D, 0x4E, 0x43, 0x70, 0x00, 0x02, 0x34, 0x12, 0x6D]
        );
        let (got, used) = Frame::decode(&wire).expect("decode");
        assert_eq!(used, wire.len());
        assert_eq!(got, f);
    }

    #[test]
    fn decode_strips_the_sysex_wrapper() {
        let f = sample();
        let mut wire = vec![SYSEX_START];
        wire.extend_from_slice(&f.encode());
        wire.push(SYSEX_END);
        let (got, _) = Frame::decode(&wire).expect("decode");
        assert_eq!(got, f);
    }

    #[test]
    fn decode_rejects_short_bad_magic_and_bad_direction() {
        assert_eq!(Frame::decode(&MAGIC), Err(DecodeError::TooShort));

        let mut bad_magic = vec![0x7D, 0x4E, 0x44];
        bad_magic.resize(9, 0);
        assert_eq!(
            Frame::decode(&bad_magic),
            Err(DecodeError::BadMagic([0x7D, 0x4E, 0x44]))
        );

        let mut bad_dir = MAGIC.to_vec();
        bad_dir.push(0x00); // not a direction byte
        bad_dir.resize(9, 0);
        assert_eq!(Frame::decode(&bad_dir), Err(DecodeError::BadDirection(0x00)));
    }

    #[test]
    fn seven_bit_safety_flag() {
        assert!(sample().is_seven_bit_safe());
        assert!(!Frame::new(Direction::Tx, 0, 0, 0x6D, vec![0x80]).is_seven_bit_safe());
    }

    #[test]
    fn split_stream_reads_consecutive_framed_messages() {
        let f1 = sample();
        let f2 = Frame::new(Direction::Rx, 0x02, 0x0007, 0x63, vec![0xAA, 0xBB]);
        let mut wire = Vec::new();
        wire.extend_from_slice(&f1.encode());
        wire.push(SYSEX_END);
        wire.extend_from_slice(&f2.encode());
        wire.push(SYSEX_END);
        assert_eq!(Frame::split_stream(&wire), vec![f1, f2]);
    }

    /// An `F7` inside a binary payload is data, not a frame terminator —
    /// unless it is followed by the end of the stream, an `F0`, or a magic.
    #[test]
    fn split_stream_treats_payload_f7_as_data() {
        let f = Frame::new(Direction::Tx, 0, 1, 0x6D, vec![0xF7, 0x01]);
        let mut wire = f.encode();
        wire.push(SYSEX_END);
        assert_eq!(Frame::split_stream(&wire), vec![f]);
    }

    #[test]
    fn split_stream_skips_garbage_before_the_magic() {
        let f = sample();
        let mut wire = vec![0x00, 0xFF, 0x00];
        wire.extend_from_slice(&f.encode());
        wire.push(SYSEX_END);
        assert_eq!(Frame::split_stream(&wire), vec![f]);
    }
}
