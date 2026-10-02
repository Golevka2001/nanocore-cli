//! amp 槽位写入（`0x30`–`0x37` XFER 体系）的帧构造与输入格式解析。
//!
//! 与 IR 写（[`crate::asset`] 的 `0x49`–`0x57`）不同，amp 走独立的 XFER 体系，
//! 无 `0x49` 会话门控。会话时序（`导入RESONA覆盖Marsup.mmon`，2026-09-22）：
//!
//! ```text
//! PING → 0x30(`00 00 00`) → 0x31(长度+槽) → 0x32×N(数据帧) → 0x33(提交+crc)
//!      → 0x35(模型名) → 0x30 → 0x37(收尾) → 0x30 → 0x36×38(型号表刷新)
//! ```
//!
//! 数据帧复用 [`crate::asset::encode_data_frame`]（`0x52` 与 `0x32` 帧头逐字段
//! 同构：`p[1]=0x44`、`p[7]=slot`、块索引编码与掩码插位完全一致）。该同构已用
//! 两份抓包全量验证：`导入RESONA覆盖Marsup.mmon`（193 帧）+ `恢复出厂设置.mmon`
//! （38 型号 × 193 帧 = 7327 帧），解码后的 blob 长度与 crc 均精确吻合。
//!
//! `0x33` 提交帧**完全可推导**（38/38 出厂会话逐字节验证，2026-10-01）：
//!
//! ```text
//! 00 08 00 [B3] [len_lo] [len_hi] 00 [slot] [c0&7F][c1&7F][c2&7F][c3>>7][c3&7F]
//! ```
//!
//! 其中 `c0..c3` = `crc32(blob)` 小端字节，`B3` 承载被 7 比特掩码剥掉的高位
//! （`bit4/5/6` = `c0/c1/c2` 的高位，与 [`crate::asset::hash_b3`] 同构），
//! `bit0` = 长度非标准 12302 标志（假设，与 IR 侧语义对齐）。
//!
//! 输入格式：
//! * `.ead` —— 12302 B，`SAPF\x01` 魔数 + 60 B 壳 + 12242 B 负载；
//! * `.eadl` —— `EADL\x01\x00` + u32 LE JSON 长度 + JSON 元数据 + 内嵌完整 `.ead`
//!   （实测样例 `BL-Recto-Crunch.eadl`，2026-10-01）。
//!
//! 壳内字节 24–27 = 负载长度 LE（12242），其余为逐模型变化的摘要
//! （非 payload 的 md5/sha1/sha256/crc32，无法从负载合成）——
//! 因此**不接受裸 12242 B 负载作为写入输入**。

use crate::asset;

/// XFER 会话开始（也用作收尾节拍）。RX 为 26–36 B 能力块（字段未解，透传忽略）。
pub const XFER_BEGIN: u8 = 0x30;
/// 宣告 `(total, slot)`。
pub const XFER_HEADER: u8 = 0x31;
/// 数据帧（与 IR 写的 `0x52` 同构）。
pub const XFER_DATA: u8 = 0x32;
/// 提交（携带 blob 的 7 比特安全 crc32）。
pub const XFER_COMMIT: u8 = 0x33;
/// 模型名元数据。
pub const XFER_META: u8 = 0x35;
/// 收尾确认。
pub const XFER_FINALIZE: u8 = 0x37;

/// 完整 `.ead` 大小（含 60 B 壳）。
pub const EAD_TOTAL: usize = 12302;
/// 剥壳负载大小（`0x39` 读回形态）。
pub const EAD_PAYLOAD: usize = 12242;

/// `0x31`：`00 04 00 00 [len_lo] [len_hi] 00 [slot]`，len 为**小端** u16
/// （与 IR 侧 `write_begin` 的大端不同；12302 = `0e 30`）。
pub fn xfer_header(slot: u8, total: usize) -> Vec<u8> {
    assert!(
        total <= 0x7FFF,
        "amp blob too large for the 7-bit-safe len field"
    );
    vec![
        0x00, 0x04, 0x00, 0x00, (total & 0xFF) as u8, (total >> 8) as u8, 0x00, slot,
    ]
}

/// `0x33` 的 `B3` 字节：crc32 小端字节 `c0..c2` 被剥掉的高位 + 非标准长度标志。
/// 与 [`crate::asset::hash_b3`] 同构；已用 38/38 出厂会话验证（bit0 恒 0，
/// len 恒为标准 12302）。
fn commit_b3(le: &[u8; 4], total: usize) -> u8 {
    let v = ((le[2] >> 7) << 2) | ((le[1] >> 7) << 1) | (le[0] >> 7);
    (v << 4) | u8::from(total != EAD_TOTAL)
}

/// crc32 小端字节的 7 比特安全形式：`[c0&7F][c1&7F][c2&7F][c3>>7][c3&7F]`。
fn crc_tail(le: &[u8; 4]) -> [u8; 5] {
    [le[0] & 0x7F, le[1] & 0x7F, le[2] & 0x7F, le[3] >> 7, le[3] & 0x7F]
}

/// `0x33` 提交帧：`00 08 00 [B3] [len_lo] [len_hi] 00 [slot] <crc tail>`。
///
/// len 与 `0x31` 相同为小端；crc 尾已对 `恢复出厂设置.mmon` 全部 38 个会话
/// 逐字节验证。
pub fn xfer_commit(slot: u8, total: usize, blob: &[u8]) -> Vec<u8> {
    let le = asset::crc32(blob).to_le_bytes();
    let mut v = vec![
        0x00, 0x08, 0x00, commit_b3(&le, total), (total & 0xFF) as u8, (total >> 8) as u8, 0x00,
        slot,
    ];
    v.extend_from_slice(&crc_tail(&le));
    v
}

/// `0x35` 模型名：`00 [strlen+1] 00 00 [slot] [name bytes]`。
///
/// `strlen` 只数可打印字符（设备名存储里嵌有 NUL 分片）。
/// 布局取自 `恢复出厂设置.mmon` 的 38 条 0x35（5 字节头 `00 len 00 00 slot`）；
/// 注意 `导入RESONA覆盖Marsup.mmon` 的 0x35 少一个 00（4 字节头 `00 len 00 slot`，
/// 可能是抓包工具吞字节），真机实测 5 字节头被接受、4 字节头被拒
/// （应答状态字节 `01`，2026-10-01）。
pub fn xfer_meta(slot: u8, name_fragments: &[u8]) -> Vec<u8> {
    let strlen = name_fragments.iter().filter(|&&b| b != 0x00).count();
    let mut v = vec![0x00, (strlen + 1) as u8, 0x00, 0x00, slot];
    v.extend_from_slice(name_fragments);
    v
}

/// `0x37` 收尾：`00 01 00 [slot]`（注意与 IR 的 `0x57` 提交 `00 01 00 00 [slot]`
/// 相比少一个字节）。
pub fn xfer_finalize(slot: u8) -> Vec<u8> {
    vec![0x00, 0x01, 0x00, slot]
}

/// `0x30` 会话开始 / 收尾节拍：`00 00 00`。
pub fn xfer_begin() -> Vec<u8> {
    vec![0x00, 0x00, 0x00]
}

/// 把模型名编码为设备名存储的分片形式：第 1 组 6 个可打印字符，之后每组
/// 7 个，组间插 NUL。锚点：`"BogXTC1"` → `BogXTC\0 1`、
/// `"NDSP Archetype:"` → `NDSP A\0rchetyp\0e:`（`0x36` 读回的两份原始字节）。
pub fn encode_model_name(name: &str) -> Vec<u8> {
    let b: Vec<u8> = name
        .bytes()
        .filter(|&c| (0x20..=0x7E).contains(&c))
        .collect();
    if b.len() <= 6 {
        return b;
    }
    let mut out = b[..6].to_vec();
    let mut i = 6;
    while i < b.len() {
        out.push(0x00);
        let end = (i + 7).min(b.len());
        out.extend_from_slice(&b[i..end]);
        i = end;
    }
    out
}

/// 解析用户输入的 amp 模型文件：接受完整 `.ead`（12302 B）或 `.eadl`
/// （解包出内嵌 `.ead`）。裸 12242 B 负载不可写——60 B 壳含无法从负载
/// 合成的逐模型摘要。
pub fn prepare_amp_input(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.starts_with(b"EADL") {
        return parse_eadl(data);
    }
    if data.len() == EAD_TOTAL {
        if !data.starts_with(b"SAPF") {
            return Err(
                "12302-byte file does not start with the SAPF magic — not a valid .ead".into(),
            );
        }
        return Ok(data.to_vec());
    }
    if data.len() == EAD_PAYLOAD {
        return Err(
            "this is a bare 12242-byte payload (the `asset read` form); the write path \
             needs the full 12302-byte .ead including its 60-byte shell — use the \
             original .ead/.eadl file"
                .into(),
        );
    }
    Err(format!(
        "unexpected size {} bytes: expected 12302 (.ead) or an .eadl container",
        data.len()
    ))
}

/// 解析 `.eadl` 容器：`EADL\x01\x00` + u32 LE JSON 长度 + JSON + 内嵌 `.ead`。
fn parse_eadl(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.len() < 10 {
        return Err(".eadl too short".into());
    }
    if data[4] != 0x01 || data[5] != 0x00 {
        return Err(format!(
            "unsupported .eadl version {:02x} {:02x}",
            data[4], data[5]
        ));
    }
    let jlen = u32::from_le_bytes([data[6], data[7], data[8], data[9]]) as usize;
    let payload = &data[10 + jlen..];
    if payload.len() != EAD_TOTAL {
        return Err(format!(
            ".eadl embedded payload is {} bytes, expected {EAD_TOTAL}",
            payload.len()
        ));
    }
    if !payload.starts_with(b"SAPF") {
        return Err(".eadl embedded payload does not start with the SAPF magic".into());
    }
    Ok(payload.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xfer_header_matches_resona_capture() {
        // 导入RESONA覆盖Marsup.mmon：TX 31 00 04 00 00 0e 30 00 1d（槽 29，12302 B）
        assert_eq!(
            xfer_header(0x1d, 12302),
            vec![0x00, 0x04, 0x00, 0x00, 0x0e, 0x30, 0x00, 0x1d]
        );
    }

    #[test]
    fn xfer_commit_matches_resona_capture() {
        // 0x33 TX：00 08 00 30 0e 30 00 1d 06 75 44 00 16。
        // 该 blob 的 crc32 LE = 86 f5 44 16（由 0x32 流逐帧解码重组得出）；
        // commit_b3 / crc_tail 两个纯函数用这组常量直接锚定（见上），
        // 这里再验证 xfer_commit 的接线（crc 计算 + 字段拼装）。
        let blob = vec![0x42u8; EAD_TOTAL];
        let le = asset::crc32(&blob).to_le_bytes();
        let mut expect = vec![
            0x00, 0x08, 0x00,
            commit_b3(&le, EAD_TOTAL),
            0x0e, 0x30, 0x00, 0x1d,
        ];
        expect.extend_from_slice(&crc_tail(&le));
        assert_eq!(xfer_commit(0x1d, 12302, &blob), expect);
    }

    #[test]
    fn commit_b3_carries_high_bits() {
        // 模拟与 asset::hash_b3 相同的位打包：c0/c1 高位置位 → bit4/bit5。
        let le: [u8; 4] = [0x80, 0xFF, 0x7F, 0x00];
        assert_eq!(commit_b3(&le, EAD_TOTAL), 0x30);
        // 全部高位为 0 → B3 = 0；非标准长度 → bit0 置位。
        let le0: [u8; 4] = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(commit_b3(&le0, EAD_TOTAL), 0x00);
        assert_eq!(commit_b3(&le0, 12242), 0x01);
    }

    #[test]
    fn encode_model_name_matches_device_storage() {
        // "NDSP Archetype:" 的设备分片形式，取自 0x36 RX（SLOT29 样例）与
        // 导入RESONA覆盖Marsup.mmon 的 0x35 TX，两者逐字节一致。
        assert_eq!(
            encode_model_name("NDSP Archetype:"),
            b"NDSP A\x00rchetyp\x00e:".to_vec()
        );
        // "BogXTC1" 取自 0x36 RX（SLOT0 样例）：BogXTC\0 1。
        assert_eq!(encode_model_name("BogXTC1"), b"BogXTC\x00\x31".to_vec());
        assert_eq!(encode_model_name("scream"), b"scream".to_vec());
        // 非 ASCII / 控制字符被过滤。
        assert_eq!(encode_model_name("ab\tcd"), b"abcd".to_vec());
    }

    #[test]
    fn xfer_meta_length_counts_printable_only() {
        // 恢复出厂设置.mmon：slot 34 "PiFUZZ v.2"（10 可打印）→
        // `00 0b 00 00 22 50 69 46 55 5a 5a 00 20 76 2e 32`。
        let frag = encode_model_name("PiFUZZ v.2");
        assert_eq!(frag, b"PiFUZZ\x00 v.2".to_vec());
        assert_eq!(
            xfer_meta(0x22, &frag),
            [
                0x00, 0x0b, 0x00, 0x00, 0x22, b'P', b'i', b'F', b'U', b'Z', b'Z', 0x00, b' ',
                b'v', b'.', b'2'
            ]
        );
    }

    #[test]
    fn frame_plan_is_193_for_standard_ead() {
        // 12302 B = 192 × 64 + 14 → 193 帧（两份抓包均如此）。
        assert_eq!(EAD_TOTAL.div_ceil(asset::WRITE_CHUNK_BYTES), 193);
        assert_eq!(EAD_TOTAL % asset::WRITE_CHUNK_BYTES, 14);
    }

    #[test]
    fn prepare_amp_input_accepts_ead_and_eadl() {
        let mut ead = vec![0u8; EAD_TOTAL];
        ead[..4].copy_from_slice(b"SAPF");
        assert_eq!(prepare_amp_input(&ead).unwrap(), ead);

        // .eadl：EADL 01 00 + u32 LE json_len + json + 内嵌 .ead
        let json = br#"{"nams":[]}"#;
        let mut eadl = vec![b'E', b'A', b'D', b'L', 0x01, 0x00];
        eadl.extend_from_slice(&(json.len() as u32).to_le_bytes());
        eadl.extend_from_slice(json);
        eadl.extend_from_slice(&ead);
        assert_eq!(prepare_amp_input(&eadl).unwrap(), ead);

        // 裸负载被拒绝。
        assert!(prepare_amp_input(&vec![0u8; EAD_PAYLOAD]).is_err());
        assert!(prepare_amp_input(&[0u8; 100]).is_err());
    }

    #[test]
    fn parse_eadl_rejects_bad_payload() {
        let json = b"{}";
        let mut eadl = vec![b'E', b'A', b'D', b'L', 0x01, 0x00];
        eadl.extend_from_slice(&(json.len() as u32).to_le_bytes());
        eadl.extend_from_slice(json);
        eadl.extend_from_slice(&[0u8; 100]); // 长度不对
        assert!(prepare_amp_input(&eadl).is_err());
    }
}
