//! 批量资源传输：音箱模型与 IR/箱体载荷。
//!
//! Nanocore 将每个音箱模型（约 12,242 字节）和每个 IR（约 4,096 字节）存储为不透明的 blob。
//! 应用通过一系列小型"数据块"请求读出其中一个；
//! 设备对每个请求应答一个固定 239 字节的载荷，解码后还原出 blob 的 200 字节。
//!
//! # 线上格式
//!
//! 请求（TX），8 字节载荷：
//!
//! ```text
//! 00 04 00 [B3] [SLOT] [ADDR_lo] [ADDR_hi] [LEN]
//! ```
//!
//! 应答（RX），239 字节载荷：
//!
//! ```text
//! 00 00 [C2] [B3<<1 | LEAD_HI<<6] 00 [SLOT] [ADDR_lo] [ADDR_hi] 52 2f | data...
//! ```
//!
//! `ADDR_lo`/`ADDR_hi` 是一个 7 比特对（`addr = hi*128 + lo`），
//! 应答会原样回显请求中的槽位和地址。
//!
//! # 载荷编解码（7 比特安全，无损）
//!
//! SysEx 禁止出现 ≥ 0x80 的字节，因此 blob 按如下方式打包：
//!
//! * 数据的字节 0 = 解码后的字节 0，
//!   其 bit 7 由应答字节 3 的 bit 6 携带（回显时为 `B3<<1`，因此实际是 `B3` 的 bit 5）。
//! * 之后按 8 字节分组：`[mask][d0..d6]`；解码字节 `i` 为 `d_i | ((mask >> i) & 1) << 7`。
//! * 最后一组可以不满（`[mask][d0..dk]`）。
//!
//! 每个完整数据块恰好产出 200 字节，
//! 因此 blob 的第 `c` 个数据块为 `blob[200c .. 200c+200]`。
//!
//! # 数据块顺序
//!
//! 应用发出的请求是**乱序**的；载荷顺序是 **ADDR 升序**。
//! 拼接前务必按 `addr` 对应答排序。
//!
//! 本模块只覆盖**写入**侧：`0x49`…`0x57` 的 IR 会话与 `0x30`…`0x37` 的 amp 会话。
//!
//! 读取侧（`0x39` / `0x59` 的分块读）已按 ToneCommand 的实际行为删除——
//! 官方 App 既不回读资产也不做内容校验。

/// 音箱模型区域传输所用的消息类型。
pub const REGION_AMP: u8 = 0x39;
/// IR/箱体区域传输所用的消息类型。
pub const REGION_IR: u8 = 0x59;
/// IR 的 blob 大小（4,096 字节 = 1,024 个 `f32le` 抽头）。
pub const IR_TOTAL: u32 = 4096;
/// 音箱模型的 blob 大小（资源包清单中的 `ampAssets` `length`）。
pub const AMP_TOTAL: u32 = 12242;

/// AMP 区域可寻址的槽位数（30 个音箱仿真 + 8 个 FX2 踏板效果）。
pub const AMP_SLOTS: u8 = 38;
/// IR 区域可寻址的槽位数。
pub const IR_SLOTS: u8 = 30;

/// CRC-32（IEEE 802.3——与 zlib 及资源包清单使用的多项式相同）。
///
/// 用于在不加依赖的前提下算出 `0x33` 提交帧与 `0x53` 哈希帧里的 crc，
/// 以及 `.npreset` 清单记录的 `crc32`。
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &b in data {
        crc ^= b as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ─── 写入侧（仅 IR 槽位；音箱路径使用 `0x32`，尚未建模） ───

/// IR 数据帧写入所用的消息类型。
pub const WRITE_IR: u8 = 0x52;
/// 消息类型：开始一次槽位写入。
pub const WRITE_BEGIN: u8 = 0x51;
/// 消息类型：写入槽位的身份哈希。
pub const WRITE_HASH: u8 = 0x53;
/// 消息类型：写入槽位名称。
pub const WRITE_NAME: u8 = 0x55;
/// 消息类型：提交槽位写入。
pub const WRITE_COMMIT: u8 = 0x57;
/// 消息类型：查询槽位名称 + 哈希（也用于*校验*一次写入）。
pub const SLOT_QUERY: u8 = 0x56;

/// 一个完整写入帧携带的原始字节数（64，与读取方向相同）。
pub const WRITE_CHUNK_BYTES: usize = 64;

/// 将 blob 的一段切片编码为 `0x52` 数据帧载荷。
///
/// 布局：
/// 15/16 个槽位与资源包清单的 crc32 一致）：
///
/// ```text
/// [00 LEN+4 00 H3 H4 CNT 00 SLOT] [3 lead bytes] [M][7 data] × 9 (last partial)
/// ```
///
/// * `LEN+4` —— 数据帧 p[1] = 本帧携带的数据字节数 + 4。整块（64 B）恒为
///   `0x44`；非整块尾帧来自短 IR 写入（28 B → `0x20`、
///   52 B → `0x38`）与 amp 写（14 B → `0x12`）四点共同确证。
///   **设备会对 p[1] 与实际载荷不符的帧回状态 `01` 拒绝**。
/// * 3 个前导字节的最高位存放在 `H3` 的 bit 4/5/6（`d0←bit4` 等）；
/// * 每个 `M` 字节是其后 **7 个数据字节** 的最低位在前的最高位掩码；
/// * `H3` bit 0、`H4` bit 6 与 `CNT` 共同携带数据块索引
///   （`ci = CNT*4 + (H3&1)*2 + (H4>>6)`）。
pub fn encode_data_frame(chunk: &[u8], chunk_index: usize, slot: u8) -> Vec<u8> {
    let n = chunk.len();
    assert!(
        (1..=WRITE_CHUNK_BYTES).contains(&n),
        "chunk size out of range"
    );
    let lead = n.min(3);
    let groups = (n - lead).div_ceil(7);
    let mut out = vec![0u8; 8 + n + groups];
    out[0] = 0x00;
    out[1] = (n as u8) + 4;
    out[2] = 0x00;
    let mut h3 = ((chunk_index >> 1) & 1) as u8;
    for (j, &byte) in chunk.iter().take(lead).enumerate() {
        h3 |= ((byte >> 7) & 1) << (4 + j);
    }
    out[3] = h3;
    out[4] = ((chunk_index & 1) as u8) << 6;
    out[5] = (chunk_index / 4) as u8;
    out[7] = slot;

    let mut w = 8;
    for j in 0..lead {
        out[w + j] = chunk[j] & 0x7f;
    }
    w += lead;
    let mut k = lead;
    while k < n {
        let grp = (k + 7).min(n) - k;
        let mut mask = 0u8;
        for j in 0..grp {
            mask |= ((chunk[k + j] >> 7) & 1) << j;
            out[w + 1 + j] = chunk[k + j] & 0x7f;
        }
        out[w] = mask;
        w += 1 + grp;
        k += grp;
    }
    out
}

/// 将 `0x52` 数据帧载荷解码回 blob 字节（[`encode_data_frame`] 的逆操作）。
/// 可用于测试和校验。
pub fn decode_data_frame(payload: &[u8]) -> Vec<u8> {
    let d = &payload[8..];
    let mut out = Vec::with_capacity(WRITE_CHUNK_BYTES);
    for (j, &byte) in d.iter().take(3).enumerate() {
        out.push(byte | (((payload[3] >> (4 + j)) & 1) << 7));
    }
    let mut i = 3;
    while i < d.len() {
        let end = (i + 8).min(d.len());
        let group = &d[i..end];
        i = end;
        let mask = group[0];
        for (j, &byte) in group[1..].iter().enumerate() {
            out.push(byte | (((mask >> j) & 1) << 7));
        }
    }
    out
}

/// `0x51` 开始写入：在数据帧之前宣告 `(slot, total)`。
pub fn write_begin(slot: u8, total: u32) -> Vec<u8> {
    vec![
        0x00, 0x04, 0x00, 0x00, 0x00, (total >> 8) as u8, total as u8, slot,
    ]
}

/// 槽位身份哈希：`[SLOT, c0 & 0x7F, c1 & 0x7F, c2 & 0x7F]`，
/// 其中 `c0..c3` 是 `crc32(blob)` 的小端字节。
///
/// `0x53` 帧按此布局编解码。
/// （每一帧的哈希都等于该槽位 `0x52` 序列所写 blob 的此值）。
/// 设备会在提交时重新计算该哈希，不匹配时静默拒绝写入，
/// 因此它必须由正在写入的 blob 推导——
/// 复用之前查询到的*不同*内容的哈希会使提交成为空操作。
pub fn identity_hash(slot: u8, blob: &[u8]) -> [u8; 4] {
    let le = crc32(blob).to_le_bytes();
    [slot, le[0] & 0x7F, le[1] & 0x7F, le[2] & 0x7F]
}

/// `0x53` 身份哈希帧——完全可由 `(slot, blob)` 推导。
///
/// 载荷以 7 比特安全形式携带完整的 `crc32(blob)`：
///
/// ```text
/// 00 08 00 B3 00 len_hi len_lo SLOT | c0&7F c1&7F c2&7F | c3>>7 | c3&7F
/// ```
///
/// `B3` **不是**独立字段——它补全 CRC：
///
/// ```text
/// B3 = ((c2 >> 7) << 6 | (c1 >> 7) << 5 | (c0 >> 7) << 4) | (len != 4096)
/// ```
///
/// * bit 4-6 = `c0..c2` 被 7 比特掩码丢弃的最高位
///   （30 个槽位均适用；
///   取值错误时设备会应答一切，然后**静默丢弃提交**），
/// * bit 0 = "资源短于标准 4096 字节 IR"
///   。
pub fn write_hash(slot: u8, total: u32, blob: &[u8]) -> Vec<u8> {
    let le = crc32(blob).to_le_bytes();
    let b3 = hash_b3(total, &le);
    vec![
        0x00, 0x08, 0x00, b3, 0x00, (total >> 8) as u8, total as u8, slot,
        le[0] & 0x7F,
        le[1] & 0x7F,
        le[2] & 0x7F,
        le[3] >> 7,
        le[3] & 0x7F,
    ]
}

/// `0x53` 帧的 `B3` 字节：从掩码后的 CRC 字节 `c0..c2` 剥离出的三个最高位，
/// 加上短资源标志（参见 [`write_hash`]）。
fn hash_b3(total: u32, le: &[u8; 4]) -> u8 {
    let v = ((le[2] >> 7) << 2) | ((le[1] >> 7) << 1) | (le[0] >> 7);
    (v << 4) | u8::from(total != IR_TOTAL)
}

/// `0x55` 槽位名称：`00 [strlen+1] 00 00 [SLOT] [name fragments]`。
///
/// 长度前缀计的是 `strlen + 1`（一个隐式 NUL 终止符）：
/// `Og412`（5 个字符，未发送 NUL）→ `06`；
/// `Bog412A`（7 个字符 + 显式 NUL 片段 = 8 字节）→ `08`。
///
/// `name` 为设备的 NUL 片段形式（例如 `Bog412A` 对应 `Bog412` `00` `A`）：
/// 前 6 个字符、一个 NUL，然后是剩余部分。
pub fn write_name(slot: u8, name: &[u8]) -> Vec<u8> {
    // 片段长度减去插入的 NUL（若有）= 纯名称长度。
    let strlen = name.iter().filter(|&&b| b != 0x00).count();
    let mut v = vec![0x00, (strlen + 1) as u8, 0x00, 0x00, slot];
    v.extend_from_slice(name);
    v
}

/// `0x57` 提交：`00 01 00 00 [SLOT]`。
pub fn write_commit(slot: u8) -> Vec<u8> {
    vec![0x00, 0x01, 0x00, 0x00, slot]
}

/// `0x49` 打开写入会话：`00 02 00 00 00 40`。
///
/// 出现在每个写入会话的开头，而只读事务中从来没有，
/// 因此它是设备接受写入的门控。
pub const SESSION_OPEN: u8 = 0x49;

pub fn session_open() -> Vec<u8> {
    vec![0x00, 0x02, 0x00, 0x00, 0x00, 0x40]
}

/// `0x56` 槽位查询：`00 01 00 00 [SLOT]`。
/// 应答携带该槽位的 4 字节身份哈希及其 NUL 片段形式的名称。
pub fn slot_query(slot: u8) -> Vec<u8> {
    vec![0x00, 0x01, 0x00, 0x00, slot]
}

/// 将纯 ASCII 名称切分为设备的 NUL 片段形式：
/// 前 6 个字符、一个 NUL，然后是剩余部分（设备只接受这种形式：
/// 超过 6 个字符的名称总是在该处切分）。
pub fn encode_name(name: &[u8]) -> Vec<u8> {
    let b = name;
    if b.len() <= 6 {
        return b.to_vec();
    }
    let mut out = b[..6].to_vec();
    out.push(0x00);
    out.extend_from_slice(&b[6..]);
    out
}

/// 从 `0x56` 应答中提取 `(hash, name)`
/// （`00 00 1e … [hash x4] 1a 1a 00 00 [name fragments] …`）。
pub fn parse_slot_query(payload: &[u8]) -> Option<([u8; 4], Vec<u8>)> {
    if payload.len() < 24 || payload[17] != 0x1a || payload[18] != 0x1a {
        return None;
    }
    let hash = [payload[13], payload[14], payload[15], payload[16]];
    let mut name = Vec::new();
    for &b in &payload[21..] {
        if b != 0x00 && (0x20..=0x7e).contains(&b) {
            name.push(b);
        }
    }
    Some((hash, name))
}

/// 一个 IR 槽（= CAB 型号）的身份，含**当前是否被预设引用**。
///
/// 需先走工厂恢复的前置序列，再连发 30 条 `0x56`：
/// 30 个槽全部应答 39 B，第 7 字节**恰好一个为 `0x01`** —— 那个就是当前预设
/// 引用的 CAB 型号。位置与 `0x36` 的 amp active 标志一致。
///
/// 例如 `irSlot=24` 的预设，第 7 字节为 `0x01`
/// 的正是槽 24（`AC3021`）；写 `CC44=5` 后 active 移到槽 5（`Fd59BM`），
/// 再写回 `CC44=24` 复原。
///
/// 这条路之前没被找到，是因为它只出现在工厂恢复的**前置序列**里 —— 单发
/// `0x56` 也能应答，但没人知道要查的是这一条。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IrSlot {
    /// 槽位号（= `0x56` 请求的第 5 字节，0..29）。
    pub index: u8,
    /// 当前预设是否引用这个槽。
    pub active: bool,
    /// 型号名（设备侧 8 字符定宽字段，以 NUL 结尾）。
    pub name: Vec<u8>,
}

impl IrSlot {
    /// 型号名的字符串形式。
    ///
    /// 设备存的是**定宽码**（如 `AC3021`），不是完整产品名（`VOX AC30 2x12`）；
    /// 完整名在 [`crate::models_data`]，按槽位号查。
    pub fn name_str(&self) -> String {
        String::from_utf8_lossy(&self.name).into_owned()
    }
}

/// 解析一条 `0x56` 应答为 [`IrSlot`]。
///
/// 应答固定 39 B，例如槽 24（`AC3021`，active）：
///
/// ```text
/// 00 00 1e 00 00 18 01 01 00 10 00 12 00 7b 04 20
/// 60 1a 1a 00 00 41 43 33 30 32 31 00 32 00 00 00
/// 00 00 00 00 00 00 00
/// ```
///
/// * `[5]` = 槽位号
/// * `[7]` = active 标志（同 `0x36`）
/// * `[17..21]` = `1a 1a 00 00` 名称标记，名字紧随其后
///
/// 名字字段是**定宽 8 字节**，不足处补 NUL，其后可能还有别的字节：
/// 槽 24 的实际字节是 `41 43 33 30 32 31 00 32`，NUL 之后还有一个 `0x32`。
/// 因此名字在**第一个 NUL 处截断**（否则 `AC3021` 会变成 `AC30212`）。
pub fn parse_ir_slot(payload: &[u8]) -> Option<IrSlot> {
    if payload.len() < 24 || payload[17] != 0x1a || payload[18] != 0x1a {
        return None;
    }
    let field = &payload[21..];
    let end = field.iter().position(|&b| b == 0x00).unwrap_or(field.len());
    let name: Vec<u8> = field[..end]
        .iter()
        .copied()
        .filter(|b| (0x20..=0x7e).contains(b))
        .collect();
    Some(IrSlot {
        index: payload[5],
        active: payload[7] != 0,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// 资源包中 `ir/slot_00.bin` 的前 64 字节（已知明文）。
    const IR0_HEAD: &[u8] = &[
        0x2d, 0x88, 0x8c, 0x3c, 0x6f, 0x4a, 0x25, 0x3e, 0x1b, 0x4f, 0xea, 0x3e, 0xa0, 0xfd, 0x30,
        0x3f, 0x14, 0x8c, 0x39, 0x3f, 0xb6, 0x4b, 0x1d, 0x3f, 0x03, 0x23, 0xba, 0x3e, 0x7c, 0x7d,
        0xc6, 0xbb, 0xe1, 0x42, 0xa3, 0xbe, 0xbb, 0xd4, 0xdc, 0xbe, 0x42, 0x69, 0xb7, 0xbe, 0x05,
        0x6a, 0x36, 0xbe, 0x90, 0xcf, 0x59, 0x3c, 0x05, 0xbf, 0xcb, 0x3d, 0x6f, 0x30, 0x9d, 0x3d,
        0x95, 0x46, 0x04, 0x3c,
    ];

    /// IR 槽位 0 的 `0x52` 写入帧 #0。
    const IR0_WRITE_FRAME0: &[u8] = &[
        0x00, 0x44, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x2d, 0x08, 0x0c, 0x00, 0x3c, 0x6f, 0x4a,
        0x25, 0x3e, 0x1b, 0x4f, 0x0d, 0x6a, 0x3e, 0x20, 0x7d, 0x30, 0x3f, 0x14, 0x09, 0x0c, 0x39,
        0x3f, 0x36, 0x4b, 0x1d, 0x3f, 0x44, 0x03, 0x23, 0x3a, 0x3e, 0x7c, 0x7d, 0x46, 0x7b, 0x3b,
        0x61, 0x42, 0x23, 0x3e, 0x3b, 0x54, 0x33, 0x5c, 0x3e, 0x42, 0x69, 0x37, 0x3e, 0x05, 0x1c,
        0x6a, 0x36, 0x3e, 0x10, 0x4f, 0x59, 0x3c, 0x46, 0x05, 0x3f, 0x4b, 0x3d, 0x6f, 0x30, 0x1d,
        0x02, 0x3d, 0x15, 0x46, 0x04, 0x3c,
    ];

    #[test]
    fn encode_reproduces_the_captured_write_frame() {
        let got = encode_data_frame(IR0_HEAD, 0, 0);
        assert_eq!(got.len(), IR0_WRITE_FRAME0.len(), "payload length");
        assert_eq!(got, IR0_WRITE_FRAME0.to_vec());
    }

    #[test]
    fn decode_data_frame_inverts_encode() {
        // 完整数据块。
        assert_eq!(decode_data_frame(&encode_data_frame(IR0_HEAD, 0, 0)), IR0_HEAD);
        // 一个较短的最后一个数据块（52 字节，如 3,764 字节资源的末块）。
        let tail: Vec<u8> = (0..52).map(|i| (i * 37 + 129) as u8).collect();
        let enc = encode_data_frame(&tail, 58, 15);
        assert_eq!(decode_data_frame(&enc), tail);
    }

    #[test]
    fn p1_encodes_chunk_len_plus_four() {
        // p[1] = 数据字节数 + 4：整块 64 → 0x44；非整块尾帧来自短 IR
        // （28 B → 0x20、52 B → 0x38）与 amp 写（14 B → 0x12）确证。
        // 设备对 p[1] 与载荷不符的帧回状态 01 拒绝。
        for (len, p1) in [(64usize, 0x44u8), (28, 0x20), (52, 0x38), (14, 0x12), (1, 0x05)] {
            let chunk: Vec<u8> = (0..len).map(|i| (i * 91 + 7) as u8).collect();
            let enc = encode_data_frame(&chunk, 0, 0);
            assert_eq!(enc[1], p1, "len {len}");
        }
    }

    #[test]
    fn name_encoding_fragments_after_six_characters() {
        assert_eq!(encode_name(b"Bog412A"), b"Bog412\x00A");
        assert_eq!(encode_name(b"Fd59BM410"), b"Fd59BM\x00410");
        assert_eq!(encode_name(b"Short1"), b"Short1");
    }

    #[test]
    fn control_frames_carry_slot_and_length() {
        assert_eq!(write_begin(0, 4096), vec![0x00, 0x04, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00]);
        assert_eq!(write_begin(2, 4096), vec![0x00, 0x04, 0x00, 0x00, 0x00, 0x10, 0x00, 0x02]);
        assert_eq!(write_commit(2), vec![0x00, 0x01, 0x00, 0x00, 0x02]);
        assert_eq!(slot_query(3), vec![0x00, 0x01, 0x00, 0x00, 0x03]);

        // 以标准 CRC-32 校验向量（crc32("123456789") = 0xCBF43926，
        // 在 `crc32_matches_the_standard_vector` 中断言）为基准，
        // 而不是磁盘上的某个 blob，这样期望值不会随被测公式的漂移而改变。
        // 小端字节 `26 39 f4 cb` -> 掩码后为 `26 39 74`；
        // c3 = 0xCB 拆分为进位 `01` | 低位 `4b`。
        // 9 字节的 blob 属于"短"资源（!= 4096），因此 B3 bit 0 = 1，
        // 而 c2 的最高位（0xf4）落在 B3 的 bit 6 -> 0x41。
        // 同一推导：
        // crc32(ir/slot_00.bin) = 0xdab13b3e 得到 identity [00 3e 3b 31] 和 B3 0x40。
        assert_eq!(identity_hash(7, b"123456789"), [0x07, 0x26, 0x39, 0x74]);
        assert_eq!(
            write_hash(7, 9, b"123456789"),
            vec![0x00, 0x08, 0x00, 0x41, 0x00, 0x00, 0x09, 0x07, 0x26, 0x39, 0x74, 0x01, 0x4b]
        );

        // 名称长度前缀计 strlen + 1（隐式终止符）：Og412 -> 06。
        assert_eq!(write_name(14, b"Og412")[1], 0x06);
        assert_eq!(write_name(0, b"Bog412\x00A")[1], 0x08);
        assert_eq!(write_name(0, b"Bog412\x00A").len(), 5 + 8);
    }

    /// B3 位布局的合成校验：crc32("123456789") = 0xCBF43926，
    /// 小端 `26 39 cb 4c` -> c0..c2 的最高位 = 0,0,1 -> v = 4。
    #[test]
    fn hash_b3_bit_layout() {
        let le = crc32(b"123456789").to_le_bytes();
        assert_eq!(hash_b3(IR_TOTAL, &le), 0x40);
        assert_eq!(hash_b3(2780, &le), 0x41);
    }


    #[test]
    fn ir_slot_active_flag_is_byte_seven() {
        // 槽 24 = `AC3021`，active。名字字段是 8 字节定宽 `41 43 33 30 32 31
        // 00 32` —— NUL 之后还有一个 `0x32`，扫到载荷末尾会把它吃进来。
        let active = [
            0x00, 0x00, 0x1e, 0x00, 0x00, 24, 0x01, 0x01, 0x00, 0x10, 0x00, 0x12, 0x00, 0x7b,
            0x04, 0x20, 0x60, 0x1a, 0x1a, 0x00, 0x00, b'A', b'C', b'3', b'0', b'2', b'1', 0x00,
            b'2', 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        // 槽 5 = `Fd59BM`，非 active。
        let other = [
            0x00, 0x00, 0x1e, 0x00, 0x00, 5, 0x01, 0x00, 0x00, 0x10, 0x00, 0x1e, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x1a, 0x1a, 0x00, 0x00, b'F', b'd', b'5', b'9', b'B', b'M', 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        let a = parse_ir_slot(&active).expect("active 槽应能解析");
        assert_eq!((a.index, a.active, a.name_str().as_str()), (24, true, "AC3021"));
        let o = parse_ir_slot(&other).expect("非 active 槽也应能解析");
        assert_eq!((o.index, o.active, o.name_str().as_str()), (5, false, "Fd59BM"));
    }
}
