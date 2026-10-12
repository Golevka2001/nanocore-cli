//! 设备生成的"紧凑状态"元数据尾。
//!
//! 官方 app 发送的大多数消息以如下形式的尾结尾：
//!
//! ```text
//! DA 00 0E <u16 id>... 5A "statusByte" 5E "clockTimestamp" ... 5D "From Nanocore" ...
//! ```
//!
//! * `0x5A`、`0x5E`、`0x5D` 是形如 `0x50 | len` 的字符串标记。
//! * 这些字符串由**设备固件**生成，
//!   app 将其原样回传。
//!
//! 设备在写入命令时是否要求携带该尾尚不明确；
//! 下面内置了一份设备实际发出的尾。

/// 元数据尾的起始标记（`statusByte` 字符串标记）。
pub const STATUS_MARKER: [u8; 11] = [
    0x5A, b's', b't', b'a', b't', b'u', b's', b'B', b'y', b't', b'e',
];

/// 设备生成的状态块的起始头部字节。
pub const TAIL_HEADER: [u8; 3] = [0xDA, 0x00, 0x0E];

/// 查找元数据尾起始的字节偏移。
pub fn find_tail(payload: &[u8]) -> Option<usize> {
    if let Some(i) = payload.windows(3).position(|w| w == TAIL_HEADER) {
        return Some(i);
    }
    payload
        .windows(STATUS_MARKER.len())
        .position(|w| w == STATUS_MARKER)
        .map(|i| i.saturating_sub(1))
}

/// 返回移除元数据尾之后的"核心"载荷。
pub fn core_payload(payload: &[u8]) -> &[u8] {
    match find_tail(payload) {
        Some(off) => &payload[..off],
        None => payload,
    }
}

/// 提取原始尾（从可检测到的 `DA 00 0E` 块开始的全部内容）。
pub fn extract_tail(payload: &[u8]) -> Option<&[u8]> {
    let start = payload.windows(3).position(|w| w == [0xDA, 0x00, 0x0E])?;
    Some(&payload[start..])
}
