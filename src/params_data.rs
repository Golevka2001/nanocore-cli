//! 内嵌的 Nanocore 参数表。
//!
//! 与 `data/params.json` 保持镜像（schema `nanocore.params/v2`）；
//! 同步由 `tests/data_sync.rs` 强制保证 —— 修改 JSON 时须一并修改本表，
//! 二者不一致时 `cargo test` 会失败。
//! 该 JSON 解析自 OEM MIDI CC 参考表，后者转录自 OEM 的
//! `NANOCORE-MIDI-Control-User-Guide.pdf` —— 经人工校对的唯一事实来源，无二手转录。
//!
//! ## 到达同一参数的两条路径
//!
//! * **标准 MIDI CC** —— 有文档记载、7 比特、单向：[`ParamInfo::cc`]。
//! * **SysEx float32** —— App 协议 `0x6D`/子命令 `0x07`：
//!   `00 07 00 [b3] 01 [b5] [b6] <7-bit-safe float32>`，
//!   其中 `b5` = [`block_slot`]，`b6` = [`ParamInfo::index`]。
//!
//! ## `b6` 按效果类型区分，而非按块区分
//!
//! [`ParamInfo::index`] 是**块当前效果类型内**的索引。
//! FX1 噪声门接受 `b6 = 0,1`，而 FX1 压缩器接受 `b6 = 0..5`，
//! 两者都在 `b5 = 0` —— 因此整块一维的平铺列表是错误的。
//! 这正是本表以 `(block, effect type)` 为键的原因。
//!
//! ## 依据来源
//!
//! * `b5` = 块槽位：设备字符串表 ID 14-21 依次对应块 FX1,FX2,AMP,CAB,DEL,MOD,REV,EQ -> 0..7；
//!   抓包结果一致（FX1 写 `b5=0x00`，FX2 写 `b5=0x01`，音箱启用帧为 `ENABLE slot=2`）。
//! * `b6` = 参数索引：压缩器抓包（其文件名按调节顺序列出全部六个参数）解码为 `b6 = 0..5`，
//!   与指南 CC51-56 一致 —— 两个独立来源互相印证。
//! * `b3` 不参与寻址；参见 [`crate::protocol::DEFAULT_B3`]。
//!
//! ## 已知空白（出自指南本身，非猜测）
//!
//! OEM 指南未给出 FX1 类型 1/2（Auto Gate、Gate + Compressor）和 FX2 类型 5-7（Boost 1-3）的参数名称，
//! 也没有抓包覆盖它们 —— 因此 [`params`] 对这些情况返回 `None`，而非编造名称。
#![allow(dead_code)]

/// 一个块 / 效果类型的单个参数。
pub struct ParamInfo {
    /// SysEx 写入中的 `b6` —— 当前块效果类型内的索引。
    pub index: u8,
    /// 显示名称（OEM 指南）。
    pub name: &'static str,
    /// 控制同一参数的标准 MIDI CC 编号。
    pub cc: u8,
}

/// `(块键, b5)`，按设备字符串表列出的顺序排列。
pub const BLOCKS: &[(&str, u8)] = &[
    ("fx1", 0),
    ("fx2", 1),
    ("amp", 2),
    ("cab", 3),
    ("delay", 4),
    ("mod", 5),
    ("reverb", 6),
    ("eq", 7),
];

/// `(块键, 效果类型 ID, 参数列表)`。仅包含参数列表已知的类型；
/// 缺失的类型即文档记载的空白。
pub const TYPED: &[(&str, u8, &[ParamInfo])] = &[
("fx1", 0, &[
    ParamInfo { index: 0, name: "Gate Threshold", cc: 50 },
    ParamInfo { index: 1, name: "Release", cc: 50 },
]),
("fx1", 3, &[
    ParamInfo { index: 0, name: "Compressor Threshold", cc: 51 },
    ParamInfo { index: 1, name: "Ratio", cc: 52 },
    ParamInfo { index: 2, name: "Knee", cc: 53 },
    ParamInfo { index: 3, name: "Attack", cc: 54 },
    ParamInfo { index: 4, name: "Release", cc: 55 },
    ParamInfo { index: 5, name: "Makeup", cc: 56 },
]),
("fx2", 0, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 1, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 2, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 3, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 4, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 8, &[
    ParamInfo { index: 0, name: "Pitch", cc: 68 },
    ParamInfo { index: 1, name: "Fine", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
    ParamInfo { index: 3, name: "Level", cc: 71 },
]),
("fx2", 9, &[
    ParamInfo { index: 0, name: "Freq", cc: 68 },
    ParamInfo { index: 1, name: "Env", cc: 69 },
    ParamInfo { index: 2, name: "Q", cc: 70 },
    ParamInfo { index: 3, name: "Mix", cc: 71 },
    ParamInfo { index: 4, name: "Level", cc: 72 },
]),
("fx2", 10, &[
    ParamInfo { index: 0, name: "Freq", cc: 68 },
    ParamInfo { index: 1, name: "Sweep", cc: 69 },
    ParamInfo { index: 2, name: "Rate", cc: 70 },
    ParamInfo { index: 3, name: "Shape", cc: 71 },
    ParamInfo { index: 4, name: "Mix", cc: 72 },
    ParamInfo { index: 5, name: "Level", cc: 73 },
]),
("mod", 0, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Level", cc: 70 },
]),
("mod", 1, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Feedback", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
    ParamInfo { index: 3, name: "Level", cc: 71 },
]),
("mod", 2, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Feedback", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
    ParamInfo { index: 3, name: "Level", cc: 71 },
]),
("mod", 3, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Wave", cc: 70 },
    ParamInfo { index: 3, name: "Smooth", cc: 71 },
    ParamInfo { index: 4, name: "Mix", cc: 72 },
    ParamInfo { index: 5, name: "Level", cc: 73 },
]),
("mod", 4, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Wave", cc: 70 },
    ParamInfo { index: 3, name: "Mix", cc: 71 },
    ParamInfo { index: 4, name: "Level", cc: 72 },
]),
("delay", 0, &[
    ParamInfo { index: 0, name: "Delay", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Age", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
    ParamInfo { index: 4, name: "Level", cc: 78 },
]),
("delay", 1, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mix", cc: 76 },
    ParamInfo { index: 3, name: "Level", cc: 77 },
]),
("delay", 2, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Filter", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
    ParamInfo { index: 4, name: "Level", cc: 78 },
]),
("delay", 3, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Interval", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
    ParamInfo { index: 4, name: "Level", cc: 78 },
]),
("delay", 4, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mix", cc: 76 },
    ParamInfo { index: 3, name: "Level", cc: 77 },
]),
("delay", 5, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Bit Depth", cc: 76 },
    ParamInfo { index: 3, name: "LoFi Mix", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
    ParamInfo { index: 5, name: "Level", cc: 79 },
]),
("delay", 6, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Smear", cc: 76 },
    ParamInfo { index: 3, name: "Filter", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
    ParamInfo { index: 5, name: "Level", cc: 79 },
]),
("delay", 7, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mod Depth", cc: 76 },
    ParamInfo { index: 3, name: "Mod Rate", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
    ParamInfo { index: 5, name: "Level", cc: 79 },
]),
("reverb", 0, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 1, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 2, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "LF Damp", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 3, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "HF Damp", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 4, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Pitch", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 5, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "Tone", cc: 86 },
    ParamInfo { index: 2, name: "Mod Depth", cc: 87 },
    ParamInfo { index: 3, name: "Mod Rate", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 6, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Crossover", cc: 88 },
    ParamInfo { index: 4, name: "Mod", cc: 89 },
    ParamInfo { index: 5, name: "Mix", cc: 90 },
]),
];

/// 参数列表不依赖效果类型的块的 `(块键, b5, 参数列表)`。
pub const FIXED: &[(&str, u8, &[ParamInfo])] = &[
("amp", 2, &[
    ParamInfo { index: 0, name: "Amp Gain", cc: 60 },
    ParamInfo { index: 1, name: "Bass", cc: 61 },
    ParamInfo { index: 2, name: "Mid", cc: 62 },
    ParamInfo { index: 3, name: "Treble", cc: 63 },
    ParamInfo { index: 4, name: "Amp Level", cc: 64 },
]),
("cab", 3, &[
    ParamInfo { index: 0, name: "Cab Low Cut", cc: 65 },
    ParamInfo { index: 1, name: "Cab High Cut", cc: 66 },
    ParamInfo { index: 2, name: "Cab Level", cc: 67 },
]),
];

/// `(EQ 类型 ID, 频段参数列表)`。
pub const EQ_BANDS: &[(u8, &[ParamInfo])] = &[
(0, &[
    ParamInfo { index: 0, name: "Low", cc: 91 },
    ParamInfo { index: 1, name: "Mid", cc: 92 },
    ParamInfo { index: 2, name: "High", cc: 93 },
]),
(1, &[
    ParamInfo { index: 0, name: "120 Hz", cc: 91 },
    ParamInfo { index: 1, name: "250 Hz", cc: 92 },
    ParamInfo { index: 2, name: "500 Hz", cc: 93 },
    ParamInfo { index: 3, name: "1 kHz", cc: 94 },
    ParamInfo { index: 4, name: "2 kHz", cc: 95 },
    ParamInfo { index: 5, name: "4 kHz", cc: 96 },
]),
(2, &[
    ParamInfo { index: 0, name: "60 Hz", cc: 91 },
    ParamInfo { index: 1, name: "120 Hz", cc: 92 },
    ParamInfo { index: 2, name: "250 Hz", cc: 93 },
    ParamInfo { index: 3, name: "500 Hz", cc: 94 },
    ParamInfo { index: 4, name: "1 kHz", cc: 95 },
    ParamInfo { index: 5, name: "2 kHz", cc: 96 },
    ParamInfo { index: 6, name: "4 kHz", cc: 97 },
    ParamInfo { index: 7, name: "8 kHz", cc: 33 },
]),
];

/// 块键（`fx1`、`fx2`、`amp`、`cab`、`delay`、`mod`、`reverb`、`eq`）对应的 `b5`。
pub fn block_slot(key: &str) -> Option<u8> {
    BLOCKS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}

/// 某 EQ 类型的频段参数。
pub fn eq_bands(type_id: u8) -> Option<&'static [ParamInfo]> {
    EQ_BANDS.iter().find(|(t, _)| *t == type_id).map(|(_, ps)| *ps)
}

/// 块在其当前效果类型下的全部参数。
///
/// 当块的参数依赖效果类型、而该类型未知或没有已发布的参数列表时返回 `None`（见模块文档）。
pub fn params(block: &str, type_id: Option<u8>) -> Option<&'static [ParamInfo]> {
    if block == "eq" {
        return type_id.and_then(eq_bands);
    }
    if TYPED.iter().any(|(k, _, _)| *k == block) {
        let t = type_id?;
        return TYPED
            .iter()
            .find(|(k, t2, _)| *k == block && *t2 == t)
            .map(|(_, _, ps)| *ps);
    }
    FIXED.iter().find(|(k, _, _)| *k == block).map(|(_, _, ps)| *ps)
}

/// 按 `b6` 索引查找参数。
pub fn param_by_index(block: &str, type_id: Option<u8>, index: u8) -> Option<&'static ParamInfo> {
    params(block, type_id)?.iter().find(|p| p.index == index)
}

/// 按名称查找参数（忽略大小写、空格和标点）。
pub fn param_by_name(block: &str, type_id: Option<u8>, name: &str) -> Option<&'static ParamInfo> {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let want = norm(name);
    let all = params(block, type_id)?;
    if let Some(p) = all.iter().find(|p| norm(p.name) == want) {
        return Some(p);
    }
    // 退回到后缀匹配，但仅在结果无歧义时使用：像 "gain" 这样的短名称可解析到 "Amp Gain"，
    // 而若多个参数都以 "level" 结尾，则保持报错。
    let mut hits = all.iter().filter(|p| norm(p.name).ends_with(&want));
    let first = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_slots_match_capture_evidence() {
        assert_eq!(block_slot("fx1"), Some(0));
        assert_eq!(block_slot("fx2"), Some(1));
        assert_eq!(block_slot("amp"), Some(2)); // `ENABLE slot=2` = AMP
        assert_eq!(block_slot("cab"), Some(3));
        assert_eq!(block_slot("eq"), Some(7));
        assert_eq!(block_slot("nope"), None);
    }

    /// b6 按效果类型区分：FX1 噪声门接受 0,1，FX1 压缩器接受 0..5，
    /// 两者都在 b5=0。压缩器的顺序由抓包和 CC51-56 共同锁定。
    #[test]
    fn compressor_param_order_matches_capture() {
        let comp = params("fx1", Some(3)).expect("fx1 compressor");
        assert_eq!(
            comp.iter().map(|p| p.name).collect::<Vec<_>>(),
            vec!["Compressor Threshold", "Ratio", "Knee", "Attack", "Release", "Makeup"]
        );
        assert_eq!(comp.iter().map(|p| p.index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4, 5]);
        let makeup = param_by_name("fx1", Some(3), "makeup").expect("makeup");
        assert_eq!(makeup.index, 5); // 抓包中 b6 = 5
        assert_eq!(makeup.cc, 56);

        // 噪声门是同一块中的另一类型，且只有两个参数。
        let gate = params("fx1", Some(0)).expect("fx1 gate");
        assert_eq!(gate.iter().map(|p| p.name).collect::<Vec<_>>(), vec!["Gate Threshold", "Release"]);
    }

    #[test]
    fn lookup_by_name_is_forgiving() {
        assert_eq!(param_by_name("amp", None, "gain").unwrap().cc, 60);
        assert_eq!(param_by_name("amp", None, "Amp Gain").unwrap().index, 0);
        assert_eq!(param_by_name("cab", None, "low cut").unwrap().cc, 65);
        assert_eq!(param_by_name("amp", None, "Amp-Level").unwrap().cc, 64);
        assert!(param_by_name("amp", None, "nope").is_none());
        // 即使完全匹配同时也是更长名称的后缀，完全匹配仍然优先：
        // delay 类型 5 同时有 "Mix" 和 "Lo-Fi Mix"。
        assert_eq!(param_by_name("delay", Some(5), "mix").unwrap().cc, 78);
        assert_eq!(param_by_name("delay", Some(5), "lofi mix").unwrap().cc, 77);
        // 无完全匹配时，后缀回退可以帮上忙。
        assert_eq!(param_by_name("amp", None, "gain").unwrap().name, "Amp Gain");
        // ……但歧义的后缀会被拒绝而非猜测：8 频段 EQ 同时有 1 kHz / 2 kHz / 4 kHz。
        assert!(param_by_name("eq", Some(2), "khz").is_none());
        assert_eq!(param_by_name("eq", Some(2), "1khz").unwrap().cc, 95);
    }

    #[test]
    fn typed_tables_resolve_by_type() {
        assert_eq!(params("delay", Some(1)).unwrap().len(), 4); // Digital
        assert_eq!(params("delay", Some(0)).unwrap().len(), 5); // BBD
        assert_eq!(params("delay", Some(1)).unwrap()[0].name, "Time");
        assert_eq!(params("delay", Some(0)).unwrap()[3].name, "Mix"); // index == b6
        // 依赖类型的块：类型未知或缺省 -> 无法解析
        assert!(params("delay", Some(99)).is_none());
        assert!(params("delay", None).is_none());
    }

    /// 指南未发布这些类型的参数名称；必须返回 None 而非编造。
    #[test]
    fn documented_gaps_stay_unknown() {
        // fx1 类型 1：无已发布的参数列表
        assert!(params("fx1", Some(1)).is_none());
        // fx1 类型 2：无已发布的参数列表
        assert!(params("fx1", Some(2)).is_none());
        // fx2 boost 类型 5-7 同为空白
        for t in 5u8..=7 { assert!(params("fx2", Some(t)).is_none()); }
    }

    #[test]
    fn eq_uses_band_tables_per_type() {
        assert_eq!(params("eq", Some(0)).unwrap().len(), 3);
        assert_eq!(params("eq", Some(1)).unwrap().len(), 6);
        assert_eq!(params("eq", Some(2)).unwrap().len(), 8);
        assert_eq!(eq_bands(0).unwrap()[0].name, "Low");
        assert!(eq_bands(9).is_none());
    }

    #[test]
    fn fixed_blocks_ignore_type() {
        assert_eq!(params("amp", None).unwrap().len(), 5);
        assert_eq!(params("amp", Some(7)).unwrap().len(), 5); // 类型被忽略
        assert_eq!(params("cab", Some(3)).unwrap().len(), 3);
    }
}
