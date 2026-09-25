//! 设备生成的"紧凑状态"元数据尾。
//!
//! 官方 app 发送的大多数消息以如下形式的尾结尾：
//!
//! ```text
//! DA 00 0E <u16 id>... 5A "statusByte" 5E "clockTimestamp" ... 5D "From Nanocore" ...
//! ```
//!
//! * `0x5A`、`0x5E`、`0x5D` 是形如 `0x50 | len` 的字符串标记。
//! * 这些字符串由**设备固件**生成（在 `libapp.so` 和 `liblivtra.so` 中均不存在），
//!   app 将其原样回传。
//!
//! 设备在写入命令时是否要求携带该尾尚不明确；
//! 最稳妥的做法是复用从官方 app 抓包得到的尾。

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

#[cfg(test)]
mod tests {
    use super::*;

    /// 合成的尾：`DA 00 0E` 头部后跟一个状态字符串。
    fn synthetic_tail() -> Vec<u8> {
        let mut v = TAIL_HEADER.to_vec();
        v.extend_from_slice(&STATUS_MARKER);
        v.extend_from_slice(b"statusByte\x00From Nanocore\x00");
        v
    }

    #[test]
    fn finds_the_tail_header() {
        let mut p = vec![0x00, 0x02, 0x00];
        p.extend_from_slice(&synthetic_tail());
        assert_eq!(find_tail(&p), Some(3));
        assert_eq!(core_payload(&p), &[0x00, 0x02, 0x00]);
        assert_eq!(extract_tail(&p), Some(&p[3..]));
    }

    /// 没有 `DA 00 0E` 头部时，由标记本身锚定尾
    /// （早一个字节，即真实抓包中观测到的位置）。
    #[test]
    fn falls_back_to_the_status_marker() {
        let mut p = vec![0x01];
        p.extend_from_slice(&STATUS_MARKER);
        p.extend_from_slice(b"statusByte");
        assert_eq!(find_tail(&p), Some(0));
        assert!(core_payload(&p).is_empty());
        // 无头部块 -> 也没有原始尾。
        assert_eq!(extract_tail(&p), None);
    }

    #[test]
    fn payload_without_a_tail_is_returned_whole() {
        let p = vec![0x01, 0x02, 0x03];
        assert_eq!(find_tail(&p), None);
        assert_eq!(core_payload(&p), &p[..]);
        assert_eq!(extract_tail(&p), None);
    }

    /// 头部搜索不能被那些仅看起来像标记前导 `0x5A` 的载荷字节欺骗。
    #[test]
    fn marker_prefix_alone_is_not_a_tail() {
        let p = vec![0x5A, b's', b't'];
        assert_eq!(find_tail(&p), None);
    }
}
