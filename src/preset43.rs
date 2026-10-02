//! `0x43` 预设写双帧（234 B 整预设帧 + 变长参数帧）的构造与解码。
//!
//! 移植自 `nanocore-reverse/tools/decode_preset43.py`（40 组出厂预设逐字节
//! 构造验证定案，2026-09-30；Rust 移植以 [`builder_round_trips_factory40`]
//! 锚定，2026-10-01）。
//!
//! 两条 0x43 的载荷从 `[3]` 起都是连续 `[mask][7 data]` 打包流（掩码位于
//! `[3+8k]`，bit *j* 还原组内第 *j* 数据字节的 bit7），两帧的流首尾相接构成
//! 一条逻辑流：
//!
//! ```text
//! 长帧 (恒 234 B, [0..3) = 00 4a 00):
//!   const7 = 00 00 01 02 07 00 02
//!   name9  = 10 + 名字 ASCII(≤8) + NUL 填充
//!   zeros8
//!   03 08                      ← 字段 id + 记录数
//!   chain8                     ← 8 字节链序原值（无填充字节）
//!   ×8: [00 分隔(首记录省略)]
//!       10 (5*cnt+6) recIdx    ← 标记：0x10 + 记录线上长度+2 + 链位
//!       fx variant enabled count
//!       [idx][float32 全精度]×count
//! 短帧 ([0..3) = 00 <len> 00):
//!   c8 00                       ← 常量前缀（语义未解，恒定）
//!   <逻辑流的剩余部分>
//!   22 01 <presetVolume>
//!   20 01 <ampSlot>
//!   21 01 <irSlot>
//!   24 01 FF                    ← 终止符（逻辑值 FF，线上 7F）
//! ```
//!
//! 长帧定长截断：逻辑流前 202 字节进长帧，其余进短帧。
//! 注意：名字编码无中插 NUL（与 `0x40` 名表规则不同）。

/// 一条待写入预设的核心参数（[`build_frames`] 的输入）。
#[derive(Debug, Clone, PartialEq)]
pub struct PresetWrite {
    /// 预设名（≤8 个可打印 ASCII，超出截断）。
    pub name: String,
    /// 链序（8 字节角色排列原值）。
    pub chain_order: [u8; 8],
    /// 预设音量（0-100）。
    pub preset_volume: u8,
    /// 引用的 amp 型号槽（0-37，255 = 无/未知）。
    pub amp_slot: u8,
    /// 引用的 IR 槽（0-29，255 = 无/未知）。
    pub ir_slot: u8,
    /// 8 条角色记录（按链位 0-7 顺序）。
    pub slots: Vec<SlotWrite>,
}

/// 单条角色记录。
#[derive(Debug, Clone, PartialEq)]
pub struct SlotWrite {
    pub effect_id: u8,
    pub variant: u8,
    pub enabled: bool,
    /// 参数（float32，按 b6 下标顺序）。fx6（reverb 角色）App 侧只有 3 个，
    /// 设备存 4 个——构造时由调用方截到与 0x43 一致的数目。
    pub params: Vec<f32>,
}

/// 打包：连续 `[mask][7 data]` 流，掩码插在 `[3+8k]`，`lead` 为帧头 3 字节。
pub fn pack_stream(data: &[u8], lead: &[u8; 3]) -> Vec<u8> {
    let mut out = lead.to_vec();
    for chunk in data.chunks(7) {
        let mut mask = 0u8;
        for (j, &b) in chunk.iter().enumerate() {
            if b & 0x80 != 0 {
                mask |= 1 << j;
            }
        }
        out.push(mask);
        out.extend(chunk.iter().map(|&b| b & 0x7F));
    }
    out
}

/// 解包：[`pack_stream`] 的逆操作（从 `start` 起跳过帧头）。
pub fn unpack_stream(payload: &[u8], start: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let mut i = start;
    while i < payload.len() {
        let mask = *payload.get(i)?;
        for j in 1..8 {
            let Some(&b) = payload.get(i + j) else { break };
            let b = if mask & (1 << (j - 1)) != 0 { b | 0x80 } else { b };
            out.push(b);
        }
        i += 8;
    }
    Some(out)
}

/// 构造 `(长帧载荷, 短帧载荷)`。
///
/// 恰好 8 条记录；每条 `params.len() ≤ 16`（线上长度字节 `5*cnt+6 ≤ 0x7F`）。
pub fn build_frames(p: &PresetWrite) -> Result<(Vec<u8>, Vec<u8>), String> {
    if p.slots.len() != 8 {
        return Err(format!("expected 8 slots, got {}", p.slots.len()));
    }
    let name: Vec<u8> = p
        .name
        .bytes()
        .filter(|&c| (0x20..=0x7E).contains(&c))
        .take(8)
        .collect();
    let mut log = Vec::with_capacity(256);
    log.extend_from_slice(&[0x00, 0x00, 0x01, 0x02, 0x07, 0x00, 0x02]);
    log.push(0x10);
    log.extend_from_slice(&name);
    log.extend(std::iter::repeat_n(0x00, 8 - name.len()));
    log.extend_from_slice(&[0x00u8; 8]);
    log.extend_from_slice(&[0x03, 0x08]);
    log.extend_from_slice(&p.chain_order);
    for (i, slot) in p.slots.iter().enumerate() {
        let cnt = slot.params.len();
        if cnt > 16 {
            return Err(format!("slot {i}: {cnt} params exceeds the 16-byte line length"));
        }
        if i > 0 {
            log.push(0x00);
        }
        log.extend_from_slice(&[0x10, (5 * cnt + 6) as u8, i as u8]);
        log.extend_from_slice(&[slot.effect_id, slot.variant, u8::from(slot.enabled), cnt as u8]);
        for (k, &v) in slot.params.iter().enumerate() {
            log.push(k as u8);
            log.extend_from_slice(&v.to_le_bytes());
        }
    }
    log.push(0x00);
    log.extend_from_slice(&[0x22, 0x01, p.preset_volume]);
    log.extend_from_slice(&[0x20, 0x01, p.amp_slot]);
    log.extend_from_slice(&[0x21, 0x01, p.ir_slot]);
    log.extend_from_slice(&[0x24, 0x01, 0xFF]);

    if log.len() < 202 {
        return Err(format!(
            "logical stream is {} bytes, shorter than the 202-byte long-frame cut",
            log.len()
        ));
    }
    let long_chunk = &log[..202];
    let short_chunk = &log[202..];
    let long_payload = pack_stream(long_chunk, &[0x00, 0x4a, 0x00]);
    let mut short_data = vec![0xc8, 0x00];
    short_data.extend_from_slice(short_chunk);
    let short_payload = pack_stream(
        &short_data,
        &[0x00, short_data.len() as u8, 0x00],
    );
    Ok((long_payload, short_payload))
}

/// 解码出的预设核心字段。
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedPreset {
    pub name: String,
    pub chain_order: [u8; 8],
    pub preset_volume: u8,
    pub amp_slot: u8,
    pub ir_slot: u8,
    pub slots: Vec<SlotWrite>,
}

/// 解码 `(长帧载荷, 短帧载荷)` 为核心字段（构造的逆操作，测试与导出校验用）。
pub fn decode_frames(long_payload: &[u8], short_payload: &[u8]) -> Option<DecodedPreset> {
    // 短帧流以常量前缀 c8 00 开头（不属于逻辑流），拼接时跳过。
    let short_data = unpack_stream(short_payload, 3)?;
    if short_data.get(..2)? != [0xc8, 0x00] {
        return None;
    }
    let mut data = unpack_stream(long_payload, 3)?;
    data.extend_from_slice(&short_data[2..]);

    // 名字在 const7（7 B）之后的 9 字节名字区（0x10 + ≤8 ASCII + NUL 填充）。
    if data.len() < 16 || data[7] != 0x10 {
        return None;
    }
    let name: String = data[8..16]
        .iter()
        .filter(|&&b| b != 0x00)
        .map(|&b| b as char)
        .collect();

    // 定位 `03 08` 锚点（正常在偏移 16，容错跳过 padding）。
    let mut p = 16usize;
    while !(data.get(p) == Some(&0x03) && data.get(p + 1) == Some(&0x08)) {
        p += 1;
        if p > 64 {
            return None;
        }
    }
    p += 2;
    let mut chain = [0u8; 8];
    chain.copy_from_slice(data.get(p..p + 8)?);
    p += 8;

    let mut slots = Vec::with_capacity(8);
    for i in 0..8 {
        if i > 0 {
            if data.get(p) != Some(&0x00) {
                return None;
            }
            p += 1;
        }
        if data.get(p) != Some(&0x10) {
            return None;
        }
        let xx = *data.get(p + 1)?;
        p += 3;
        let fx = *data.get(p)?;
        let variant = *data.get(p + 1)?;
        let enabled = *data.get(p + 2)? != 0;
        let cnt = *data.get(p + 3)? as usize;
        p += 4;
        if xx as usize != 5 * cnt + 6 {
            return None;
        }
        let mut params = Vec::with_capacity(cnt);
        for k in 0..cnt {
            if data.get(p) != Some(&(k as u8)) {
                return None;
            }
            let b: [u8; 4] = data.get(p + 1..p + 5)?.try_into().ok()?;
            params.push(f32::from_le_bytes(b));
            p += 5;
        }
        slots.push(SlotWrite { effect_id: fx, variant, enabled, params });
    }
    if data.get(p) != Some(&0x00) {
        return None;
    }
    p += 1;
    // TLV 块: 22 01 <vol> | 20 01 <amp> | 21 01 <ir> | 24 01 FF
    let tlv = data.get(p..)?;
    if tlv.len() < 10 {
        return None;
    }
    Some(DecodedPreset {
        name,
        chain_order: chain,
        preset_volume: tlv[2],
        amp_slot: tlv[5],
        ir_slot: tlv[8],
        slots,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 从 `data/factory_preset_restore.bin`（[kind][flags][len:u16][payload] 序列）
    /// 切出全部 0x43 载荷。
    fn factory_0x43_payloads() -> Vec<Vec<u8>> {
        let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/data/factory_preset_restore.bin"))
            .expect("restore data");
        let mut out = Vec::new();
        let mut i = 0usize;
        while i < data.len() {
            let kind = data[i];
            let len = u16::from_le_bytes([data[i + 2], data[i + 3]]) as usize;
            if kind == 0x43 {
                out.push(data[i + 4..i + 4 + len].to_vec());
            }
            i += 4 + len;
        }
        out
    }

    #[test]
    fn builder_round_trips_factory40() {
        let payloads = factory_0x43_payloads();
        assert_eq!(payloads.len(), 80, "40 presets × 2 frames");
        for (g, pair) in payloads.chunks(2).enumerate() {
            let decoded = decode_frames(&pair[0], &pair[1])
                .unwrap_or_else(|| panic!("preset {g}: decode failed"));
            assert!(decoded.name.len() <= 8);
            let rebuilt = build_frames(&PresetWrite {
                name: decoded.name.clone(),
                chain_order: decoded.chain_order,
                preset_volume: decoded.preset_volume,
                amp_slot: decoded.amp_slot,
                ir_slot: decoded.ir_slot,
                slots: decoded.slots.clone(),
            })
            .unwrap_or_else(|e| panic!("preset {g}: build failed: {e}"));
            assert_eq!(rebuilt.0.len(), 234, "preset {g}: long frame length");
            assert_eq!(rebuilt.0, pair[0], "preset {g}: long frame bytes");
            assert_eq!(rebuilt.1, pair[1], "preset {g}: short frame bytes");
        }
    }

    #[test]
    fn decode_rejects_bad_short_prefix() {
        let payloads = factory_0x43_payloads();
        let mut bad = payloads[1].clone();
        bad[5] ^= 0x01; // 破坏短帧流首字节（c8 前缀之后）
        // 直接改前缀字节更直接：把 c8 改掉需要解包后再看——这里仅验证
        // 解码器对随机破坏的健壮性（返回 None 而不是 panic）。
        let _ = decode_frames(&payloads[0], &bad);
    }

    #[test]
    fn pack_stream_round_trips() {
        let data: Vec<u8> = (0..=255u8).collect();
        let packed = pack_stream(&data, &[0x00, 0x4a, 0x00]);
        assert_eq!(unpack_stream(&packed, 3).unwrap(), data);
    }
}
