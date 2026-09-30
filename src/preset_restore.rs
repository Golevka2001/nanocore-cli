//! 出厂预设恢复数据：从官方 App"恢复出厂设置"抓包中提取的预设写序列。
//!
//! 来源：`nanocore-reverse/data/captures/恢复出厂设置.mmon` 的 TX 侧预设写阶段——
//! 从首条 `0x42` 到末条 `0x46` 的全部 TX 帧：40 组 × [`0x42` + `0x43`×2 + `0x44` + `0x46`]
//! 共 200 帧写帧，外加官方客户端按节奏穿插的 18 条 `PING` 心跳（0x68，约每 10 帧一条）。
//! 该序列已对照 2026-09-30 的第二次恢复出厂抓包逐字节验证为跨会话稳定
//! （见 `docs/protocol/presets.md` 的"写流程稳定性"）。
//!
//! **PING 必须原样重放**：实测剔除 PING 的重放会在组 25 的 `0x46` 处确定性卡死
//! （设备对该帧完全沉默），而官方序列在同一位置之前恰好有一条心跳——
//! 设备似乎依赖心跳推进写会话状态机。
//!
//! 用途：当设备的预设表被清空（见资产写会话事故，`docs/protocol/assets.md`）
//! 时，重放本序列可完整修复 40 个出厂预设的名字与内容，无需官方 App。
//!
//! 数据格式（自定义紧凑二进制，`include_bytes!` 嵌入、零依赖解析）：
//!
//! ```text
//! 记录 := [kind:1][flags:1][len:u16 LE][payload:len]   共 218 条
//! ```
//!
//! `flags` 按抓包原样保存（观测到 `0x00`/`0x02`/`0x20`/`0x22`，随上下文变化），
//! 重放时逐帧使用，不自行推断。

/// 槽位配置（开门帧，8 B）。
pub const KIND_SLOT_CONFIG: u8 = 0x42;
/// 整预设帧（234 B）/ 变长参数体（32–101 B）。
pub const KIND_PRESET_WRITE: u8 = 0x43;
/// 收尾帧（10 B）。
pub const KIND_BULK_DATA: u8 = 0x44;
/// 落槽定位帧（5 B，末字节 = 槽位 0..40）。
pub const KIND_SLOT_SEEK: u8 = 0x46;
/// 心跳帧（3 B，官方按节奏穿插，实测不可省略）。
pub const KIND_PING: u8 = 0x68;

/// 每组写帧数：`0x42` + `0x43`×2 + `0x44` + `0x46`。
pub const FRAMES_PER_GROUP: usize = 5;
/// 出厂预设组数（槽位 0–39）。
pub const GROUP_COUNT: usize = 40;
/// 写帧总数（不含 PING）。
pub const WRITE_FRAME_COUNT: usize = GROUP_COUNT * FRAMES_PER_GROUP;
/// 总帧数（含 18 条 PING）。
pub const FRAME_COUNT: usize = WRITE_FRAME_COUNT + 18;

/// 一条待重放的预设写帧。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PresetRestoreFrame {
    pub kind: u8,
    pub flags: u8,
    pub payload: Vec<u8>,
}

/// 解析内嵌数据。结构不符时返回 `Err`（数据损坏的兜底，正常不会发生）。
pub fn frames() -> Result<Vec<PresetRestoreFrame>, String> {
    let data = include_bytes!("../data/factory_preset_restore.bin");
    parse(data)
}

/// 解析原始数据（供测试与校验工具复用）。
pub fn parse(data: &[u8]) -> Result<Vec<PresetRestoreFrame>, String> {
    let mut out = Vec::with_capacity(FRAME_COUNT);
    let mut rest = data;
    while !rest.is_empty() {
        if rest.len() < 4 {
            return Err(format!("truncated header at frame {} of {}", out.len(), FRAME_COUNT));
        }
        let kind = rest[0];
        let flags = rest[1];
        let len = u16::from_le_bytes([rest[2], rest[3]]) as usize;
        let payload = rest
            .get(4..4 + len)
            .ok_or_else(|| format!("truncated payload at frame {}", out.len()))?
            .to_vec();
        rest = &rest[4 + len..];
        out.push(PresetRestoreFrame { kind, flags, payload });
    }
    if out.len() != FRAME_COUNT {
        return Err(format!("expected {FRAME_COUNT} frames, got {}", out.len()));
    }
    Ok(out)
}

/// 校验结构性不变量：按顺序吞掉 PING 后，写帧必须恰好是
/// 40 组 × [`0x42` + `0x43`×2 + `0x44` + `0x46`]，槽位 0–39 递增，
/// 234 B 帧带链字段锚点，PING 的载荷为 `00 00 00`。
pub fn validate(frames: &[PresetRestoreFrame]) -> Result<(), String> {
    if frames.len() != FRAME_COUNT {
        return Err(format!("expected {FRAME_COUNT} frames, got {}", frames.len()));
    }
    let mut group = 0usize;
    let mut step = 0usize; // 组内步进 0..5
    for (i, f) in frames.iter().enumerate() {
        if f.kind == KIND_PING {
            if f.payload != [0x00, 0x00, 0x00] {
                return Err(format!("frame {i}: unexpected ping payload"));
            }
            continue;
        }
        let expected = [KIND_SLOT_CONFIG, KIND_PRESET_WRITE, KIND_PRESET_WRITE, KIND_BULK_DATA, KIND_SLOT_SEEK];
        if f.kind != expected[step] {
            return Err(format!("frame {i}: expected kind 0x{:02x}, got 0x{:02x}", expected[step], f.kind));
        }
        match step {
            0 => {
                if f.payload.len() != 8 {
                    return Err(format!("frame {i}: slot-config is {} bytes", f.payload.len()));
                }
            }
            1 => {
                if f.payload.len() != 234 || f.payload[31] != 0x03 || f.payload[32] != 0x08 {
                    return Err(format!("frame {i}: 234-byte frame lacks the chain-field anchor"));
                }
            }
            3 => {
                if f.payload.len() != 10 {
                    return Err(format!("frame {i}: bulk-data is {} bytes", f.payload.len()));
                }
            }
            4 if f.payload != [0x00, 0x01, 0x00, 0x00, group as u8] => {
                return Err(format!("frame {i}: slot-seek does not target slot {group}"));
            }
            _ => {}
        }
        step += 1;
        if step == FRAMES_PER_GROUP {
            step = 0;
            group += 1;
        }
    }
    if group != GROUP_COUNT {
        return Err(format!("expected {GROUP_COUNT} groups, got {group}"));
    }
    Ok(())
}

/// 提取第 `slot` 组 234 B 预设帧内的名字区（`[13..21)`，含 NUL 插入形态）。
pub fn preset_name_bytes(frames: &[PresetRestoreFrame], slot: usize) -> Option<&[u8]> {
    // 跳过 PING，定位第 slot 组的第二个写帧（0x43 整预设帧）。
    let mut group = 0usize;
    let mut step = 0usize;
    for f in frames {
        if f.kind == KIND_PING {
            continue;
        }
        if group == slot && step == 1 {
            if f.payload.len() < 21 {
                return None;
            }
            return Some(&f.payload[13..21]);
        }
        step += 1;
        if step == FRAMES_PER_GROUP {
            step = 0;
            group += 1;
        }
    }
    None
}

/// 把帧序列按组切分：每片以前一组 `0x46` 之后的帧开始、以本组 `0x46` 结束，
/// 片内原样保留穿插的 `PING`（重放时保持官方节奏位置）。
/// 返回 `GROUP_COUNT` 个切片；切片内写帧不是 5 帧或顺序不对则返回 `Err`。
pub fn group_slices(frames: &[PresetRestoreFrame]) -> Result<Vec<Vec<&PresetRestoreFrame>>, String> {
    let mut slices: Vec<Vec<&PresetRestoreFrame>> = Vec::with_capacity(GROUP_COUNT);
    let mut cur: Vec<&PresetRestoreFrame> = Vec::new();
    let mut step = 0usize;
    for f in frames {
        cur.push(f);
        if f.kind == KIND_PING {
            continue;
        }
        let expected = [KIND_SLOT_CONFIG, KIND_PRESET_WRITE, KIND_PRESET_WRITE, KIND_BULK_DATA, KIND_SLOT_SEEK];
        if f.kind != expected[step] {
            return Err(format!("unexpected kind 0x{:02x} at write-step {step}", f.kind));
        }
        step += 1;
        if step == FRAMES_PER_GROUP
            && f.kind != KIND_SLOT_SEEK
        {
            return Err("group does not end with a slot-seek frame".to_string());
        }
        if step == FRAMES_PER_GROUP {
            slices.push(std::mem::take(&mut cur));
            step = 0;
        }
    }
    if !cur.is_empty() {
        return Err("trailing frames after the last group".to_string());
    }
    if slices.len() != GROUP_COUNT {
        return Err(format!("expected {GROUP_COUNT} groups, got {}", slices.len()));
    }
    Ok(slices)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_data_passes_structure_validation() {
        let frames = frames().expect("embedded data must parse");
        validate(&frames).expect("embedded data must satisfy structural invariants");
    }

    #[test]
    fn pings_are_evenly_interleaved() {
        let frames = frames().unwrap();
        let pings = frames.iter().filter(|f| f.kind == KIND_PING).count();
        assert_eq!(pings, 18);
        // 第一条 PING 不晚于第 12 帧（官方节奏约每 10 帧一条）。
        let first = frames.iter().position(|f| f.kind == KIND_PING).unwrap();
        assert!(first < 12, "first ping at frame {first}");
    }

    #[test]
    fn factory_names_match_the_documented_list() {
        let frames = frames().unwrap();
        // 首尾两个出厂名抽验（名字区为 8 B，NUL 插在最后一个字符之前）。
        assert_eq!(&preset_name_bytes(&frames, 0).unwrap()[..6], b"ClnArp");
        assert_eq!(&preset_name_bytes(&frames, 39).unwrap()[..6], b"BassFu");
        // 每组名字区都有非零字符。
        for g in 0..GROUP_COUNT {
            let name = preset_name_bytes(&frames, g).unwrap();
            assert!(name.iter().any(|&b| b != 0), "group {g} has an empty name");
        }
    }

    #[test]
    fn parse_rejects_truncated_data() {
        let full = include_bytes!("../data/factory_preset_restore.bin");
        assert!(parse(&full[..100]).is_err());
    }
}
