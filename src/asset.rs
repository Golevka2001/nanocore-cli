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
//! # 来源与可信度
//!
//! 下面的地址计划原样取自 `factory-preset-export.mmon`（完整出厂导出）的抓包。
//! 对同一区域中存放相同大小 blob 的每个槽位，这些计划完全一致
//! （已验证：15 个 IR 槽位共用一份计划，19 个音箱槽位共用另一份；
//! 较小的两份计划是较大那份的前缀）。
//!
//! 这些地址的推导规律尚不清楚——它们不是线性字节偏移（相邻数据块的步进为 72 或 200，
//! 且最后一个数据块的地址取决于 blob 长度）。
//! 因此本模块通过重放抓包中的计划来寻址 blob；这正是实测中设备所接受的方式。
//!
//! 端到端验证：解码所有抓包应答可逐字节复现资源包中的 `ir/slot_NN.bin`，
//! 并与清单中的 `crc32` 一致（16/16 个 IR 资源），
//! 同时对 19/19 个音箱资源与清单 `crc32` 一致。

/// 音箱模型区域传输所用的消息类型。
pub const REGION_AMP: u8 = 0x39;
/// IR/箱体区域传输所用的消息类型。
pub const REGION_IR: u8 = 0x59;

/// 应答载荷中打包数据前面的帧头字节数。
pub const PAYLOAD_HEADER: usize = 10;
/// 应答字节 3 中携带解码字节 0 bit 7 的位。
pub const LEAD_HI_BIT: u8 = 0x40;
/// 一个完整数据块携带的解码字节数。
pub const CHUNK_BYTES: usize = 200;
/// 应用对除最后一个数据块外的每个数据块发送的 `LEN` 字节。
pub const FULL_CHUNK_LEN: u8 = 72;

/// 出厂 IR 的 blob 大小（4,096 字节 = 1,024 个 `f32le` 抽头）。
pub const IR_TOTAL: u32 = 4096;
/// 出厂音箱模型的 blob 大小（资源包清单中的 `ampAssets` `length`）。
pub const AMP_TOTAL: u32 = 12242;

/// AMP 区域可寻址的槽位数（30 个音箱仿真 + 8 个 FX2 踏板效果）。
pub const AMP_SLOTS: u8 = 38;
/// IR 区域可寻址的槽位数。
pub const IR_SLOTS: u8 = 30;

/// 4,096 字节 IR 每个数据块索引对应的 `(LEN 字节, 地址)`。
///
/// 原样取自抓包；导出中所有 15 个该大小的 IR 槽位共用。
pub const IR_PLAN: &[(u8, u16)] = &[
    (0x08, 0),
    (0x0a, 72),
    (0x0a, 144),
    (0x08, 344),
    (0x08, 416),
    (0x0a, 488),
    (0x0a, 560),
    (0x08, 760),
    (0x08, 832),
    (0x08, 904),
    (0x0a, 976),
    (0x0a, 1048),
    (0x08, 1248),
    (0x08, 1320),
    (0x0a, 1392),
    (0x0a, 1464),
    (0x0a, 1536),
    (0x08, 1736),
    (0x08, 1808),
    (0x0a, 1880),
    (0x02, 1952),
];

/// 12,242 字节音箱模型每个数据块索引对应的 `(B3, 地址)`。
///
/// 原样取自抓包；导出中所有 19 个该大小的音箱槽位共用。
/// 其前 19 项与 [`IR_PLAN`] 完全相同。
pub const AMP_PLAN: &[(u8, u16)] = &[
    (0x08, 0),
    (0x0a, 72),
    (0x0a, 144),
    (0x08, 344),
    (0x08, 416),
    (0x0a, 488),
    (0x0a, 560),
    (0x08, 760),
    (0x08, 832),
    (0x08, 904),
    (0x0a, 976),
    (0x0a, 1048),
    (0x08, 1248),
    (0x08, 1320),
    (0x0a, 1392),
    (0x0a, 1464),
    (0x0a, 1536),
    (0x08, 1736),
    (0x08, 1808),
    (0x0a, 1880),
    (0x0a, 1952),
    (0x08, 2152),
    (0x08, 2224),
    (0x0a, 2296),
    (0x0a, 2368),
    (0x0a, 2440),
    (0x08, 2640),
    (0x08, 2712),
    (0x0a, 2784),
    (0x0a, 2856),
    (0x08, 3056),
    (0x08, 3128),
    (0x08, 3200),
    (0x0a, 3272),
    (0x0a, 3344),
    (0x08, 3544),
    (0x08, 3616),
    (0x0a, 3688),
    (0x0a, 3760),
    (0x08, 3960),
    (0x08, 4032),
    (0x08, 4104),
    (0x0a, 4176),
    (0x0a, 4248),
    (0x08, 4448),
    (0x08, 4520),
    (0x0a, 4592),
    (0x0a, 4664),
    (0x0a, 4736),
    (0x08, 4936),
    (0x08, 5008),
    (0x0a, 5080),
    (0x0a, 5152),
    (0x08, 5352),
    (0x08, 5424),
    (0x0a, 5496),
    (0x0a, 5568),
    (0x0a, 5640),
    (0x08, 5840),
    (0x08, 5912),
    (0x0a, 5984),
    (0x02, 6056),
];

/// `"amp"` / `"ir"`（也包括 `"cab"`）→ 区域消息类型。
pub fn region_from_name(name: &str) -> Option<u8> {
    match name.to_ascii_lowercase().as_str() {
        "amp" | "a" => Some(REGION_AMP),
        "ir" | "cab" | "c" => Some(REGION_IR),
        _ => None,
    }
}

/// 区域消息类型 → 短标签。
pub fn region_name(region: u8) -> &'static str {
    match region {
        REGION_AMP => "amp",
        REGION_IR => "ir",
        _ => "?",
    }
}

/// 读取计划中的一个数据块请求。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRequest {
    /// 区域消息类型（[`REGION_AMP`] / [`REGION_IR`]）。
    pub region: u8,
    /// 从 0 开始的固件槽位。
    pub slot: u8,
    /// `B3` 字段。含义未确定；按抓包原样重放。
    pub b3: u8,
    /// 7 比特对地址。含义未确定；按抓包原样重放。
    pub addr: u16,
    /// `LEN` 字段：除最后一个数据块外均为 [`FULL_CHUNK_LEN`]；
    /// 最后一个数据块为 blob 中剩余的字节数。
    pub len: u8,
}

impl ChunkRequest {
    /// 要发送的 8 字节载荷（帧类型为 [`REGION_AMP`] / [`REGION_IR`]）。
    pub fn payload(&self) -> Vec<u8> {
        vec![
            0x00,
            0x04,
            0x00,
            self.b3,
            self.slot,
            (self.addr & 0x7f) as u8,
            (self.addr >> 7) as u8,
            self.len,
        ]
    }
}

/// 某区域在标准（出厂）大小下的 blob 总字节数。
pub fn total_for(region: u8) -> Option<u32> {
    match region {
        REGION_AMP => Some(AMP_TOTAL),
        REGION_IR => Some(IR_TOTAL),
        _ => None,
    }
}

/// 某区域可寻址的槽位数；未知区域返回 `None`。
///
/// 与型号表一一对应：AMP 区域的 38 个槽即 [`models_data::AMP`]（30 个音箱
/// 仿真 + 8 个 FX2 踏板效果），IR 区域的 30 个槽即 [`models_data::CAB`]。
pub fn slots_for(region: u8) -> Option<u8> {
    match region {
        REGION_AMP => Some(AMP_SLOTS),
        REGION_IR => Some(IR_SLOTS),
        _ => None,
    }
}

/// 为一个槽位构建完整读取计划。
///
/// 返回 `(blob_len, requests)`。这些请求必须按序应答；参见 [`assemble`]。
/// 区域未知或槽位超出 [`slots_for`] 时返回 `None`——计划永远不会指向
/// 设备上不存在的槽位。
pub fn read_plan(region: u8, slot: u8) -> Option<(u32, Vec<ChunkRequest>)> {
    let (plan, total, slots) = match region {
        REGION_AMP => (AMP_PLAN, AMP_TOTAL, AMP_SLOTS),
        REGION_IR => (IR_PLAN, IR_TOTAL, IR_SLOTS),
        _ => return None,
    };
    if slot >= slots {
        return None;
    }
    let n = plan.len();
    let mut out = Vec::with_capacity(n);
    for (i, &(b3, addr)) in plan.iter().enumerate() {
        let len = if i + 1 == n {
            let rem = total - CHUNK_BYTES as u32 * (n as u32 - 1);
            // LEN 是一个 7 比特字段；抓包得到的计划从不需要截断。
            rem.min(0x7f) as u8
        } else {
            FULL_CHUNK_LEN
        };
        out.push(ChunkRequest { region, slot, b3, addr, len });
    }
    Some((total, out))
}

/// 将一个应答载荷中的打包数据解码为 blob 字节。
pub fn decode_chunk(payload: &[u8]) -> Vec<u8> {
    if payload.len() <= PAYLOAD_HEADER {
        return Vec::new();
    }
    let d = &payload[PAYLOAD_HEADER..];
    let mut out = Vec::with_capacity(CHUNK_BYTES);
    let lead_hi = (payload[3] & LEAD_HI_BIT) << 1;
    out.push(d[0] | lead_hi);

    let mut i = 1;
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

/// 一个解码后的应答数据块。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// 设备在应答中报告的槽位。
    pub slot: u8,
    /// 设备为该数据块回显的地址。
    pub addr: u16,
    /// 解码后的字节（完整数据块为 200 字节）。
    pub data: Vec<u8>,
}

/// 将应答载荷解析为 `(slot, addr, 解码字节)`。
pub fn parse_chunk(payload: &[u8]) -> Option<Chunk> {
    if payload.len() <= PAYLOAD_HEADER {
        return None;
    }
    Some(Chunk {
        slot: payload[5],
        addr: payload[6] as u16 | ((payload[7] as u16) << 7),
        data: decode_chunk(payload),
    })
}

/// 将解码后的数据块拼接成 blob。
///
/// 数据块先按 `addr` 排序：应用发出的请求是乱序的，
/// 因此到达顺序不等于载荷顺序。
pub fn assemble(chunks: &[Chunk]) -> Vec<u8> {
    let mut sorted: Vec<&Chunk> = chunks.iter().collect();
    sorted.sort_by_key(|c| c.addr);
    let mut out = Vec::with_capacity(sorted.iter().map(|c| c.data.len()).sum());
    for c in sorted {
        out.extend_from_slice(&c.data);
    }
    out
}

/// CRC-32（IEEE 802.3——与 zlib 及资源包清单使用的多项式相同）。
///
/// 用于在不引入依赖的情况下，
/// 将读回的 blob 与 `.npreset` 清单中记录的 `crc32` 比对。
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
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
/// 布局（从 `preset-import.mmon` 破解得出；1019/1019 帧逐字节验证通过，
/// 15/16 个槽位与资源包清单的 crc32 一致）：
///
/// ```text
/// [00 LEN+4 00 H3 H4 CNT 00 SLOT] [3 lead bytes] [M][7 data] × 9 (last partial)
/// ```
///
/// * `LEN+4` —— 数据帧 p[1] = 本帧携带的数据字节数 + 4。整块（64 B）恒为
///   `0x44`；非整块尾帧由 `恢复出厂设置.mmon` 的短 IR 写入（28 B → `0x20`、
///   52 B → `0x38`）与 amp 写（14 B → `0x12`）四点共同确证。
///   **设备会对 p[1] 与实际载荷不符的帧回状态 `01` 拒绝**（2026-10-01 真机实测）。
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
/// 可用于测试和验证抓包。
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
/// 已针对 `preset-import.mmon` 中的 `0x53` 帧验证 16/16 通过
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
///   （已对照 `factory-reset.mmon` 的全部 30 个槽位验证；
///   取值错误时设备会应答一切，然后**静默丢弃提交**），
/// * bit 0 = "资源短于标准 4096 字节 IR"
///   （在 2780 和 3764 字节的出厂 IR 上观察到）。
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
/// 在所有抓包帧中，长度前缀计的是 `strlen + 1`（一个隐式 NUL 终止符）：
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
/// 出现在每个导入/恢复出厂抓包的开头，而**任何**只读抓包中都没有，
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
/// 前 6 个字符、一个 NUL，然后是剩余部分（实测规则；在每条抓包写入中，
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

    /// IR 槽位 0 的抓包 `0x52` 写入帧 #0，
    /// 原样取自 `preset-import.mmon`。
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
        // p[1] = 数据字节数 + 4：整块 64 → 0x44；非整块尾帧由恢复出厂的短 IR
        // （28 B → 0x20、52 B → 0x38）与 amp 写（14 B → 0x12）确证。
        // 设备对 p[1] 与载荷不符的帧回状态 01 拒绝（2026-10-01 真机实测）。
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
        // 导入抓包也印证了同一推导：
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

    /// 全部 30 个出厂 IR 的 `B3` 字节，
    /// 与 `factory-reset.mmon` 的 `0x53` 帧交叉核对
    /// （其内容 = APK 内置的 `factory_presets/ir/NN.bin`）。需要已解包的 APK 目录树。
    #[ignore = "needs the extracted factory IR files (not part of this repo)"]
    #[test]
    fn hash_b3_matches_all_30_factory_irs() {
        let dir = std::path::Path::new("/tmp/apkx/assets/flutter_assets/assets/nanocore/factory_presets/ir");
        if !dir.is_dir() {
            eprintln!("skipping: APK IR tree not extracted");
            return;
        }
        // (slot, 期望的 B3)，取自抓包。
        let expect: [(u8, u8); 30] = [
            (0, 0x40), (1, 0x40), (2, 0x20), (3, 0x70), (4, 0x10), (5, 0x70),
            (6, 0x60), (7, 0x20), (8, 0x50), (9, 0x40), (10, 0x60), (11, 0x01),
            (12, 0x30), (13, 0x30), (14, 0x40), (15, 0x41), (16, 0x40),
            (17, 0x20), (18, 0x00), (19, 0x00), (20, 0x70), (21, 0x10),
            (22, 0x20), (23, 0x50), (24, 0x10), (25, 0x10), (26, 0x10),
            (27, 0x20), (28, 0x20), (29, 0x10),
        ];
        for (slot, want) in expect {
            let blob = std::fs::read(dir.join(format!("{slot:02}.bin"))).expect("ir file");
            let total = blob.len() as u32;
            let frame = write_hash(slot, total, &blob);
            assert_eq!(frame[3], want, "B3 mismatch for slot {slot}");
            assert_eq!(frame[4], 0x00);
            assert_eq!(u16::from_be_bytes([frame[5], frame[6]]) as usize, blob.len());
            // 帧中的 CRC 字节必须能解码回完整的 crc32。
            // B3 bit4 = c0 的最高位，bit5 = c1 的，bit6 = c2 的。
            let c0 = frame[8] | (((frame[3] >> 4) & 1) << 7);
            let c1 = frame[9] | (((frame[3] >> 5) & 1) << 7);
            let c2 = frame[10] | (((frame[3] >> 6) & 1) << 7);
            let c3 = frame[12] | (frame[11] << 7);
            assert_eq!([c0, c1, c2, c3], crc32(&blob).to_le_bytes());
        }
    }

    /// B3 位布局的合成校验：crc32("123456789") = 0xCBF43926，
    /// 小端 `26 39 cb 4c` -> c0..c2 的最高位 = 0,0,1 -> v = 4。
    #[test]
    fn hash_b3_bit_layout() {
        let le = crc32(b"123456789").to_le_bytes();
        assert_eq!(hash_b3(IR_TOTAL, &le), 0x40);
        assert_eq!(hash_b3(2780, &le), 0x41);
    }

    /// IR 槽位 0、地址 0 的真实应答的前 64 个载荷字节，
    /// 取自 `factory-preset-export.mmon` 抓包。
    const IR0_CHUNK0: &[u8] = &[
        0x00, 0x00, 0x4d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x2d, 0x03, 0x08, 0x0c, 0x3c,
        0x6f, 0x4a, 0x25, 0x3e, 0x34, 0x1b, 0x4f, 0x6a, 0x3e, 0x20, 0x7d, 0x30, 0x24, 0x3f, 0x14,
        0x0c, 0x39, 0x3f, 0x36, 0x4b, 0x10, 0x1d, 0x3f, 0x03, 0x23, 0x3a, 0x3e, 0x7c, 0x6e, 0x7d,
        0x46, 0x3b, 0x61, 0x42, 0x23, 0x3e, 0x4f, 0x3b, 0x54, 0x5c, 0x3e, 0x42, 0x69, 0x37, 0x71,
        0x3e, 0x05, 0x6a, 0x36,
    ];

    #[test]
    fn decode_real_chunk_matches_the_bundle() {
        let got = decode_chunk(IR0_CHUNK0);
        assert_eq!(
            &got[..32],
            &[
                0x2d, 0x88, 0x8c, 0x3c, 0x6f, 0x4a, 0x25, 0x3e, 0x1b, 0x4f, 0xea, 0x3e, 0xa0, 0xfd,
                0x30, 0x3f, 0x14, 0x8c, 0x39, 0x3f, 0xb6, 0x4b, 0x1d, 0x3f, 0x03, 0x23, 0xba, 0x3e,
                0x7c, 0x7d, 0xc6, 0xbb,
            ][..]
        );
    }

    #[test]
    fn lead_high_bit_comes_from_payload_byte_3() {
        // 相同数据，但字节 3 置位了 LEAD_HI 位，
        // 解码后的字节 0 仅翻转其最高位。
        let mut p = IR0_CHUNK0.to_vec();
        let plain = decode_chunk(&p);
        p[3] |= LEAD_HI_BIT;
        let hi = decode_chunk(&p);
        assert_eq!(plain[0], 0x2d);
        assert_eq!(hi[0], 0xad);
        assert_eq!(plain[1..], hi[1..]);
    }

    #[test]
    fn parse_chunk_reads_slot_and_addr() {
        let c = parse_chunk(IR0_CHUNK0).expect("parse");
        assert_eq!(c.slot, 0);
        assert_eq!(c.addr, 0);
        assert_eq!(c.data.len(), 47); // 载荷被截断 -> 字节更少
    }

    #[test]
    fn read_plan_matches_the_captured_requests() {
        let (total, plan) = read_plan(REGION_IR, 0).expect("plan");
        assert_eq!(total, 4096);
        assert_eq!(plan.len(), 21);
        assert_eq!(plan[0].payload(), vec![0x00, 0x04, 0x00, 0x08, 0x00, 0x00, 0x00, 0x48]);
        // 20 个完整数据块 + 剩余 96 字节。
        assert_eq!(plan[20].len, 96);
        assert!(plan[..20].iter().all(|r| r.len == FULL_CHUNK_LEN));

        let (amp_total, amp) = read_plan(REGION_AMP, 1).expect("plan");
        assert_eq!(amp_total, 12242);
        assert_eq!(amp.len(), 62);
        assert_eq!(61 * CHUNK_BYTES as u32 + 42, amp_total);
        assert_eq!(amp[61].len, 42);
        // 两个计划共享 (b3, addr) 前缀；仅区域不同。
        assert!(amp[..19]
            .iter()
            .zip(plan[..19].iter())
            .all(|(a, b)| a.b3 == b.b3 && a.addr == b.addr));
    }

    #[test]
    fn plan_carries_the_slot_through() {
        let (_, plan) = read_plan(REGION_AMP, 27).expect("plan");
        assert!(plan.iter().all(|r| r.slot == 27 && r.region == REGION_AMP));
        assert_eq!(plan[1].payload()[4], 27);
    }

    #[test]
    fn plan_rejects_slots_past_the_region_capacity() {
        assert!(read_plan(REGION_AMP, AMP_SLOTS - 1).is_some());
        assert!(read_plan(REGION_AMP, AMP_SLOTS).is_none());
        assert!(read_plan(REGION_IR, IR_SLOTS - 1).is_some());
        assert!(read_plan(REGION_IR, IR_SLOTS).is_none());
        assert!(read_plan(0x00, 0).is_none(), "unknown region");
    }

    /// 槽位容量必须与型号表逐一对应：AMP 区域 38 个槽 = `models_data::AMP`，
    /// IR 区域 30 个槽 = `models_data::CAB`。两边单独改动时此测试会失败。
    #[test]
    fn region_capacity_matches_the_model_tables() {
        assert_eq!(usize::from(AMP_SLOTS), crate::models_data::AMP.len());
        assert_eq!(usize::from(IR_SLOTS), crate::models_data::CAB.len());
        assert_eq!(slots_for(REGION_AMP), Some(AMP_SLOTS));
        assert_eq!(slots_for(REGION_IR), Some(IR_SLOTS));
        assert_eq!(slots_for(0x00), None);
    }

    #[test]
    fn assemble_orders_by_addr_not_arrival() {
        let mk = |addr: u16, b: u8| Chunk { slot: 0, addr, data: vec![b; 2] };
        // 特意按设备应答的乱序排列。
        let got = assemble(&[mk(144, 3), mk(0, 1), mk(72, 2)]);
        assert_eq!(got, vec![1, 1, 2, 2, 3, 3]);
    }
}
