//! 帧的编码与解码。

use std::fmt;

/// 帧魔数：`7D 4E 43`（`}NC`）。
pub const MAGIC: [u8; 3] = [0x7D, 0x4E, 0x43];

/// 观测到的所有帧中字节 5 均为常量。
const CONST5: u8 = 0x02;

/// MIDI SysEx 起始 / 结束字节。设备以 SysEx 包传输 Nanocore 帧，
/// 因此每一帧在线上都被 `F0 ... F7` 包裹。
pub const SYSEX_START: u8 = 0xF0;
pub const SYSEX_END: u8 = 0xF7;

/// 去除 SysEx 包装：前导的 `F0` 和结尾的 `F7`。
///
/// 只修剪*边缘*——二进制载荷内部出现的 `F7` 属于数据。
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

/// 方向字节。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// 0x70：app -> 设备。
    Tx,
    /// 0x71：设备 -> app（应答）。
    Rx,
    /// 0x72：设备 -> app（主动非请求的事件）。
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

/// 一条解码后的 Nanocore 消息。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub direction: Direction,
    /// 字节 4（观测值：0x00 正常、0x02 live、0x22 预设写入、0x40 RX）。
    pub flags: u8,
    /// 字节 6..8 中的小端计数器（由设备回显）。
    pub seq: u16,
    /// 消息类型（字节 8）。
    pub kind: u8,
    /// 类型字节之后的字节。
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(direction: Direction, flags: u8, seq: u16, kind: u8, payload: Vec<u8>) -> Self {
        Self { direction, flags, seq, kind, payload }
    }

    /// 载荷的每个字节是否均为合法的 MIDI 数据字节（最高位为 0）。
    ///
    /// 传输层是 SysEx 流，因此任何 >= 0x80 的载荷字节都会被解释为状态字节，
    /// 导致消息被丢弃。所以 float 参数必须以 7 比特安全方式编码——
    /// 见 [`crate::protocol::encode_param_value`]。
    pub fn is_seven_bit_safe(&self) -> bool {
        self.payload.iter().all(|b| b & 0x80 == 0)
    }

    /// 编码为线上字节序列。
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

    /// 解码单帧。输入末尾可能带有多余字节（如下一帧）；只消费本帧。
    ///
    /// 线上帧由 SysEx 包裹（`F0 7D 4E 43 ... F7`），
    /// 因此属于 MIDI 层的前导 `F0` / 结尾 `F7` 会被剥离。
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

    /// 将字节流切分为连续的帧。
    ///
    /// 帧本身不携带显式长度，但每帧以 `F7`（SysEx 结束）终止，
    /// 并以 `7D 4E 43` 魔数开头，因此按魔数切分、在终止符处截断。
    pub fn split_stream(data: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        let mut i = 0;
        while i + 3 <= data.len() {
            if data[i..i + 3] == MAGIC {
                // 终止符是指后随缓冲区结尾、另一个 F0 或下一帧魔数的 F7——
                // 二进制载荷中间的 F7 只是数据。
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
