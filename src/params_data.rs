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
//!   实际写入一致（FX1 写 `b5=0x00`，FX2 写 `b5=0x01`，音箱启用帧为 `ENABLE slot=2`）。
//! * `b6` = 参数索引：压缩器按调节顺序解码为 `b6 = 0..5`，
//!   与指南 CC51-56 一致 —— 两个独立来源互相印证。
//! * `b3` 不参与寻址；它是 [`crate::protocol`] 里 `0x6D sub 0x07` 消息的**还原掩码**
//!   （记录哪些值字节的 bit7 被剥掉了），见 [`crate::protocol::param_write`]。
//!
//! ## 已知空白（出自指南本身，非猜测）
//!
//! OEM 指南未给出 FX1 类型 1/2（Auto Gate、Gate + Compressor）和 FX2 类型 5-7（Boost 1-3）的参数名称，
//! 也无从得知 —— 因此 [`params`] 对这些情况返回 `None`。
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

/// `1=fx1 2=fx2 … 8=eq` 这样一份槽位说明，由 [`BLOCKS`] 生成。
///
/// 帮助文本里这份映射出现过两次手写副本，彼此可能漂移；`module` 的
/// `BlockArg` 是第三份。从常量生成让它们不可能不一致——新增或改名只需
/// 改 `BLOCKS` 一处。
pub fn slot_legend() -> String {
    let mut s = String::new();
    for (i, (key, _)) in BLOCKS.iter().enumerate() {
        if i > 0 {
            s.push(' ');
        }
        s.push_str(&format!("{}={key}", i + 1));
    }
    s
}

/// `(块键, 效果类型 ID, 参数列表)`。仅包含参数列表已知的类型；
/// 缺失的类型即文档记载的空白。
pub const TYPED: &[(&str, u8, &[ParamInfo])] = &[
("fx1", 0, &[
    ParamInfo { index: 0, name: "Threshold", cc: 50 },
    ParamInfo { index: 1, name: "Release", cc: 50 },
]),
("fx1", 3, &[
    ParamInfo { index: 0, name: "Threshold", cc: 51 },
    ParamInfo { index: 1, name: "Ratio", cc: 52 },
    ParamInfo { index: 2, name: "Knee", cc: 53 },
    ParamInfo { index: 3, name: "Attack", cc: 54 },
    ParamInfo { index: 4, name: "Release", cc: 55 },
    ParamInfo { index: 5, name: "Makeup", cc: 56 },
]),
("fx2", 0, &[
    ParamInfo { index: 0, name: "Gain", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 1, &[
    ParamInfo { index: 0, name: "Gain", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 2, &[
    ParamInfo { index: 0, name: "Gain", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 3, &[
    ParamInfo { index: 0, name: "Gain", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 4, &[
    ParamInfo { index: 0, name: "Gain", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 8, &[
    ParamInfo { index: 0, name: "Pitch", cc: 68 },
    ParamInfo { index: 1, name: "Fine", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
]),
("fx2", 9, &[
    ParamInfo { index: 0, name: "Freq", cc: 68 },
    ParamInfo { index: 1, name: "Env", cc: 69 },
    ParamInfo { index: 2, name: "Q", cc: 70 },
    ParamInfo { index: 3, name: "Mix", cc: 71 },
]),
("fx2", 10, &[
    ParamInfo { index: 0, name: "Freq", cc: 68 },
    ParamInfo { index: 1, name: "Sweep", cc: 69 },
    ParamInfo { index: 2, name: "Rate", cc: 70 },
    ParamInfo { index: 3, name: "Shape", cc: 71 },
    ParamInfo { index: 4, name: "Mix", cc: 72 },
]),
("mod", 0, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
]),
("mod", 1, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Feedback", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
]),
("mod", 2, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Feedback", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
]),
("mod", 3, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Wave", cc: 70 },
    ParamInfo { index: 3, name: "Smooth", cc: 71 },
    ParamInfo { index: 4, name: "Mix", cc: 72 },
]),
("mod", 4, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Wave", cc: 70 },
    ParamInfo { index: 3, name: "Mix", cc: 71 },
]),
("delay", 0, &[
    ParamInfo { index: 0, name: "Delay", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Age", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
]),
("delay", 1, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mix", cc: 76 },
]),
("delay", 2, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Filter", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
]),
("delay", 3, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Interval", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
]),
("delay", 4, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mix", cc: 76 },
]),
("delay", 5, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Bit Depth", cc: 76 },
    ParamInfo { index: 3, name: "LoFi Mix", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
]),
("delay", 6, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Smear", cc: 76 },
    ParamInfo { index: 3, name: "Filter", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
]),
("delay", 7, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mod Depth", cc: 76 },
    ParamInfo { index: 3, name: "Mod Rate", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
]),
// reverb 的键是**真实 variant**，不是手册的产品序号 —— 两者对 0..3 一致，
// 但末三项错位（产品序号 4,5,6 = spring/shimmer/cloud 对应 variant 7,5,6）。
// 写 CC47=0..6 回读 variant 得 0,1,2,3,5,6,7。
// 每类只有 5 个参数 —— CC 指南那 6 个里的第 6 项（CC90）设备并不暴露。
("reverb", 0, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
]),
("reverb", 1, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
]),
("reverb", 2, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "LF Damp", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
]),
("reverb", 3, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "HF Damp", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
]),
// variant 5 = shimmer（CC47=4）
("reverb", 5, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Pitch", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
]),
// variant 6 = cloud（CC47=5，无 PreDelay，含 Mod Depth / Mod Rate）
("reverb", 6, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "Tone", cc: 86 },
    ParamInfo { index: 2, name: "Mod Depth", cc: 87 },
    ParamInfo { index: 3, name: "Mod Rate", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
]),
// variant 7 = spring（CC47=6；手册 5-7：Decay / PreDelay / Crossover / Mod / Mix）
("reverb", 7, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Crossover", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
]),
];

/// `(块键, 效果类型 ID, 显示名)`，与 `data/params.json` 的 `effect_types` 镜像。
///
/// 快照每条记录都带一个 `variant` 字节（渲染成 `type=N`），它就是这里表的键 ——
/// 设备用它决定该槽位用哪套参数表。没有这张表时输出只能印一个数字。
pub const EFFECT_TYPES: &[(&str, &[(u8, &str)])] = &[
("fx1", &[
    (0, "gate"),
    (1, "auto gate"),
    (2, "NG-Comp"),
    (3, "compressor"),
]),
// fx2 的名字是设备/App 的 pedal 名；`Drive 1-5` / `Boost 1-3` 是 CC 指南的合并
// 命名，设备上不显示。id 与手册 `N-11` 的产品编号一致（0 起始）。
("fx2", &[
    (0, "Scream"),
    (1, "Klone"),
    (2, "OCD"),
    (3, "DS2"),
    (4, "PiFUZZ"),
    (5, "Range"),
    (6, "XAC"),
    (7, "Fiman"),
    (8, "Pitch"),
    (9, "Envelope Wah"),
    (10, "Wah"),
]),
("mod", &[
    (0, "Chorus"),
    (1, "Phaser"),
    (2, "Flanger"),
    (3, "Tremolo"),
    (4, "Vibrato"),
]),
("delay", &[
    (0, "BBD"),
    (1, "Digital"),
    (2, "Duck"),
    (3, "Ice"),
    (4, "Reverse"),
    (5, "LoFi"),
    (6, "Smear"),
    (7, "Mod Delay"),
]),
// reverb 的 id 不连续，且**名称顺序与 id 顺序不一致**——这是本表最容易踩的坑。
//
// 写 CC47 = 0..6，逐次回读 0x63 快照的 variant 字节：
//
//     CC47 = 0,1,2,3,4,5,6,7  ->  variant = 0,1,2,3,5,6,7,7
//
// variant 4 空缺，>=7 钳在 7。名字则由两条独立证据交叉确定：
//   * CC 指南 `CC47 / ID` 表：4=闪烁(shimmer) 5=云雾(cloud) 6=弹簧(spring)
//   * 用户在设备/App 上核对：预设 #14 Psych 的 REV 是 spring，其 variant = 7
//
// 两者一致给出下表。注意**设备显示的类型编号不是 variant+1**：variant 7 是
// 设备上的第 5 个类型（手册 `5-7 spring`），因为手册那栏是产品顺序，与存储
// 槽位不同 —— 见 `type_number`。
("reverb", &[
    (0, "Room"),
    (1, "Plate"),
    (2, "Hall"),
    (3, "Concert"),
    (5, "Shimmer"),
    (6, "Cloud"),
    (7, "Spring"),
]),
("eq", &[
    (0, "EQ3"),
    (1, "EQ6"),
    (2, "EQ8"),
]),
];

/// 参数列表不依赖效果类型的块的 `(块键, b5, 参数列表)`。
pub const FIXED: &[(&str, u8, &[ParamInfo])] = &[
// 手册「AMP 参数说明」写的是 Gain / Bass / Mid / Treble / Level —— 无块名前缀，
// 设备与 App 上也是这么显示的。`cab` 同理，只有 Low Cut / High Cut 两个，
// 设备实存也是 2 个（此前表里多一个 `Level`(CC67)，设备并不存）。
("amp", 2, &[
    ParamInfo { index: 0, name: "Gain", cc: 60 },
    ParamInfo { index: 1, name: "Bass", cc: 61 },
    ParamInfo { index: 2, name: "Mid", cc: 62 },
    ParamInfo { index: 3, name: "Treble", cc: 63 },
    ParamInfo { index: 4, name: "Level", cc: 64 },
]),
("cab", 3, &[
    ParamInfo { index: 0, name: "Low Cut", cc: 65 },
    ParamInfo { index: 1, name: "High Cut", cc: 66 },
]),
];

/// `(EQ 类型 ID, 频段参数列表)`。
/// 每种 EQ = 频段 + 末位 `Level`。
///
/// 参数个数 = 频段数 + 1（3/6/8 段 -> 4/7/9），在 3 段下写 `b6=3` 被接受
/// 且只改第 4 个值。手册的 EQ 表只列频段，`Level` 没有量程。
pub const EQ_BANDS: &[(u8, &[ParamInfo])] = &[
(0, &[
    ParamInfo { index: 0, name: "Low", cc: 91 },
    ParamInfo { index: 1, name: "Mid", cc: 92 },
    ParamInfo { index: 2, name: "High", cc: 93 },
    ParamInfo { index: 3, name: "Level", cc: 94 },
]),
(1, &[
    ParamInfo { index: 0, name: "120 Hz", cc: 91 },
    ParamInfo { index: 1, name: "250 Hz", cc: 92 },
    ParamInfo { index: 2, name: "500 Hz", cc: 93 },
    ParamInfo { index: 3, name: "1 kHz", cc: 94 },
    ParamInfo { index: 4, name: "2 kHz", cc: 95 },
    ParamInfo { index: 5, name: "4 kHz", cc: 96 },
    ParamInfo { index: 6, name: "Level", cc: 97 },
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
    ParamInfo { index: 8, name: "Level", cc: 34 },
]),
];

/// 块键（`fx1`、`fx2`、`amp`、`cab`、`delay`、`mod`、`reverb`、`eq`）对应的 `b5`。
pub fn block_slot(key: &str) -> Option<u8> {
    BLOCKS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}

/// 某块某效果类型的显示名。
///
/// amp 与 cab 不在此表：它们的 `variant` 是 38 / 30 项型号表的槽位号，
/// 名字在 [`crate::models_data`]。
pub fn effect_type_name(block: &str, type_id: u8) -> Option<&'static str> {
    EFFECT_TYPES
        .iter()
        .find(|(k, _)| *k == block)
        .and_then(|(_, ts)| ts.iter().find(|(t, _)| *t == type_id))
        .map(|(_, n)| *n)
}

/// 设备为该 `variant` 显示的类型编号（渲染成 `type=N` 的那个 `N`）。
///
/// 对 6 个 id 连续的块（fx1 / fx2 / mod / delay / eq）它就是 `variant + 1`。
/// **reverb 不是**：手册 REV 那栏是**产品顺序**（`5-7 spring`），而存储的
/// `variant` 是另一套编号，两者对末三项错开 —— variant 7 = spring，在设备上
/// 是第 5 个类型。
///
/// **amp / cab 返回 `None`**：它们的 `variant` 恒为 0（内置预设与
/// 40 组预设 manifest 皆然），型号存在各自 30 项的表里、当前值不在 `0x63`
/// 快照中。`variant + 1` 会印出 `type=1` 这个凭空编号。
pub fn type_number(block: &str, variant: u8) -> Option<u8> {
    match block {
        "amp" | "cab" => None,
        "reverb" => {
            // 手册产品顺序 room..cloud 的位置（**1 起始**，与 `variant + 1` 同一
            // 基准 —— 两套编号必须同基准，否则 Room 会印出 `type=0`）。
            // index 0 恒为 0，不参与映射：variant 4 设备不用，落进来时回退到
            // `variant + 1`，不编一个产品位出来。
            const PRODUCT_POS: [u8; 8] = [1, 2, 3, 4, 0, 6, 7, 5];
            Some(
                PRODUCT_POS
                    .get(variant as usize)
                    .copied()
                    .filter(|&p| p != 0)
                    .unwrap_or(variant + 1),
            )
        }
        _ => Some(variant + 1),
    }
}


/// 一个参数的显示量程（官方用户手册「效果列表」节）。
///
/// 快照与 CC 传的都是 `[0,1]` 归一化值，量程是把它们还原成设备/手册上印的那个
/// 数字的依据：线性，`实际值 = lo + 归一化值 × (hi - lo)`。
///
/// **量程按 `(块, 效果类型)` 而非按参数名**：手册里 `Decay` 有 0.5\~12.0s（Room）、
/// 0.0\~12.0s（Spring/Shimmer）、1.0\~100.0s（Cloud）三种；`Rate` 有四种；
/// `Threshold` 也有 -100\~0（门）与 -60\~0（压缩）两种。按名字索引必然取错。
///
/// 手册未给量程的参数不在表内（返回 `None`）——例如多数类型末尾的 `Level`、
/// `auto gate` 的全部参数。
pub struct Range {
    pub name: &'static str,
    pub lo: f32,
    pub hi: f32,
    /// 手册写的单位（`dB` / `ms` / `Hz` / `st` / `:1` / `bit`；无则空串）。
    pub unit: &'static str,
}

/// `(块键, 效果类型 ID, 该类型的量程表)`，与 `data/params.json` 的 `ranges` 镜像。
pub const RANGES: &[(&str, u8, &[Range])] = &[
("fx1", 0, &[
    Range { name: "Threshold", lo: -100.0, hi: 0.0, unit: "dB" },
]),
("fx1", 2, &[
    Range { name: "Threshold", lo: -100.0, hi: 0.0, unit: "dB" },
    Range { name: "Threshold", lo: -60.0, hi: 0.0, unit: "dB" },
    Range { name: "Ratio", lo: 1.1, hi: 20.0, unit: ":1" },
    Range { name: "Knee", lo: 0.0, hi: 24.0, unit: "dB" },
    Range { name: "Attack", lo: 0.0, hi: 200.0, unit: "ms" },
    Range { name: "Release", lo: 0.0, hi: 1000.0, unit: "ms" },
    Range { name: "Makeup", lo: -24.0, hi: 24.0, unit: "dB" },
]),
("fx1", 3, &[
    Range { name: "Threshold", lo: -60.0, hi: 0.0, unit: "dB" },
    Range { name: "Ratio", lo: 1.1, hi: 20.0, unit: ":1" },
    Range { name: "Knee", lo: 0.0, hi: 24.0, unit: "dB" },
    Range { name: "Attack", lo: 0.0, hi: 200.0, unit: "ms" },
    Range { name: "Release", lo: 0.0, hi: 1000.0, unit: "ms" },
    Range { name: "Makeup", lo: -24.0, hi: 24.0, unit: "dB" },
]),
("fx2", 0, &[
    Range { name: "Drive Gain / Boost", lo: 0.0, hi: 1.0, unit: "" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Level", lo: 0.0, hi: 100.0, unit: "" },
]),
("fx2", 1, &[
    Range { name: "Drive Gain / Boost", lo: 0.0, hi: 1.0, unit: "" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Level", lo: 0.0, hi: 100.0, unit: "" },
]),
("fx2", 2, &[
    Range { name: "Drive Gain / Boost", lo: 0.0, hi: 1.0, unit: "" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Level", lo: 0.0, hi: 100.0, unit: "" },
]),
("fx2", 3, &[
    Range { name: "Drive Gain / Boost", lo: 0.0, hi: 1.0, unit: "" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Level", lo: 0.0, hi: 100.0, unit: "" },
]),
("fx2", 4, &[
    Range { name: "Drive Gain / Boost", lo: 0.0, hi: 1.0, unit: "" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Level", lo: 0.0, hi: 100.0, unit: "" },
]),
("fx2", 8, &[
    Range { name: "Pitch", lo: -12.0, hi: 12.0, unit: "st" },
    Range { name: "Fine", lo: -1.0, hi: 1.0, unit: "st" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("fx2", 9, &[
    Range { name: "Freq", lo: 10.0, hi: 20000.0, unit: "Hz" },
    Range { name: "Env", lo: 0.0, hi: 20000.0, unit: "Hz" },
    Range { name: "Q", lo: 0.1, hi: 20.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("fx2", 10, &[
    Range { name: "Freq", lo: 163.0, hi: 3500.0, unit: "Hz" },
    Range { name: "Sweep", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Rate", lo: 0.0, hi: 8.0, unit: "Hz" },
    Range { name: "Shape", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 0, &[
    Range { name: "Delay", lo: 1.0, hi: 2000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Age", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 1, &[
    Range { name: "Time", lo: 0.1, hi: 3000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 2, &[
    Range { name: "Time", lo: 0.1, hi: 3000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Filter", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 3, &[
    Range { name: "Time", lo: 0.1, hi: 3000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Interval", lo: -12.0, hi: 12.0, unit: "st" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 4, &[
    Range { name: "Time", lo: 0.1, hi: 3000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 5, &[
    Range { name: "Time", lo: 2.0, hi: 3000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Bit Depth", lo: 4.0, hi: 16.0, unit: "bit" },
    Range { name: "LoFi Mix", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 6, &[
    Range { name: "Time", lo: 0.1, hi: 3000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Smear", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Filter", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("delay", 7, &[
    Range { name: "Time", lo: 0.1, hi: 3000.0, unit: "ms" },
    Range { name: "Feedback", lo: 0.0, hi: 95.0, unit: "" },
    Range { name: "Mod Depth", lo: 0.0, hi: 20.0, unit: "ms" },
    Range { name: "Mod Rate", lo: 0.05, hi: 5.0, unit: "Hz" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("mod", 0, &[
    Range { name: "Rate", lo: 0.1, hi: 3.74, unit: "Hz" },
    Range { name: "Depth", lo: 0.0, hi: 1.0, unit: "ms" },
]),
("mod", 1, &[
    Range { name: "Rate", lo: 0.05, hi: 5.0, unit: "Hz" },
    Range { name: "Feedback", lo: -95.0, hi: 95.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("mod", 2, &[
    Range { name: "Rate", lo: 0.05, hi: 5.0, unit: "Hz" },
    Range { name: "Feedback", lo: -95.0, hi: 95.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("mod", 3, &[
    Range { name: "Rate", lo: 0.01, hi: 20.0, unit: "Hz" },
    Range { name: "Depth", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Wave", lo: 0.0, hi: 3.0, unit: "" },
    Range { name: "Smooth", lo: 0.0, hi: 40.0, unit: "Hz" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("mod", 4, &[
    Range { name: "Rate", lo: 0.01, hi: 20.0, unit: "Hz" },
    Range { name: "Depth", lo: 0.0, hi: 20.0, unit: "ms" },
    Range { name: "Wave", lo: 0.0, hi: 3.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("reverb", 0, &[
    Range { name: "Decay", lo: 0.5, hi: 12.0, unit: "s" },
    Range { name: "PreDelay", lo: 0.0, hi: 80.0, unit: "ms" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mod", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("reverb", 1, &[
    Range { name: "Decay", lo: 0.5, hi: 12.0, unit: "s" },
    Range { name: "PreDelay", lo: 0.0, hi: 80.0, unit: "ms" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mod", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("reverb", 2, &[
    Range { name: "Decay", lo: 0.5, hi: 12.0, unit: "s" },
    Range { name: "PreDelay", lo: 0.0, hi: 80.0, unit: "ms" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "LF Damp", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
("reverb", 3, &[
    Range { name: "Decay", lo: 0.5, hi: 12.0, unit: "s" },
    Range { name: "PreDelay", lo: 0.0, hi: 80.0, unit: "ms" },
    Range { name: "HF Damp", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mod", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
// variant 5 = shimmer（手册 6-7：Pre Delay 5~120ms，移调音程 -24~24st）
("reverb", 5, &[
    Range { name: "Decay", lo: 0.0, hi: 12.0, unit: "s" },
    Range { name: "PreDelay", lo: 5.0, hi: 120.0, unit: "ms" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Pitch", lo: -24.0, hi: 24.0, unit: "st" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
// variant 6 = cloud（手册 7-7：超长衰减 1~100s，无 PreDelay）
("reverb", 6, &[
    Range { name: "Decay", lo: 1.0, hi: 100.0, unit: "s" },
    Range { name: "Tone", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mod Depth", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mod Rate", lo: 0.02, hi: 0.25, unit: "Hz" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),
// variant 7 = spring（手册 5-7：分频点 800Hz~8kHz）
("reverb", 7, &[
    Range { name: "Decay", lo: 0.0, hi: 12.0, unit: "s" },
    Range { name: "PreDelay", lo: 5.0, hi: 80.0, unit: "ms" },
    Range { name: "Crossover", lo: 800.0, hi: 8000.0, unit: "Hz" },
    Range { name: "Mod", lo: 0.0, hi: 100.0, unit: "" },
    Range { name: "Mix", lo: 0.0, hi: 100.0, unit: "" },
]),

];

/// 参数表不随效果类型变化的块（amp / cab / eq）的量程。
///
/// 与 [`RANGES`] 分开是因为键不同：这里只按块键索引。
pub const FLAT_RANGES: &[(&str, &[Range])] = &[
("amp", &[
    Range { name: "Gain", lo: 0.0, hi: 1.0, unit: "" },
    Range { name: "Bass", lo: -10.0, hi: 10.0, unit: "dB" },
    Range { name: "Mid", lo: -10.0, hi: 10.0, unit: "dB" },
    Range { name: "Treble", lo: -10.0, hi: 10.0, unit: "dB" },
    Range { name: "Level", lo: -10.0, hi: 10.0, unit: "dB" },
]),
("cab", &[
    Range { name: "Low Cut", lo: 20.0, hi: 500.0, unit: "Hz" },
    Range { name: "High Cut", lo: 2000.0, hi: 20000.0, unit: "Hz" },
]),
];

/// EQ 每个频段的量程 —— 3 / 6 / 8 段都一样，手册对每段重复写了一遍。
pub const EQ_RANGE: Range = Range { name: "", lo: -15.0, hi: 15.0, unit: "dB" };

/// 某块某效果类型的量程表。
pub fn ranges(block: &str, type_id: Option<u8>) -> Option<&'static [Range]> {
    if block == "eq" {
        return None; // EQ 用 [`EQ_RANGE`]，逐频段同一值
    }
    if let Some(rs) = FLAT_RANGES.iter().find(|(k, _)| *k == block) {
        return Some(rs.1);
    }
    let t = type_id?;
    RANGES
        .iter()
        .find(|(k, t2, _)| *k == block && *t2 == t)
        .map(|(_, _, rs)| *rs)
}

/// 把归一化值换算成带单位的显示值。
///
/// 线性映射，与设备一致：手册量程 `lo~hi` 对应归一化 `0.0~1.0`。
/// 小数位按量程跨度选，避免 `0.300` 变成 `0.3 ms` 或 `20000.00 Hz`。
pub fn denormalize(range: &Range, normalized: f32) -> String {
    let v = range.lo + normalized.clamp(0.0, 1.0) * (range.hi - range.lo);
    let span = (range.hi - range.lo).abs();
    let digits = if span >= 1000.0 {
        0
    } else if span >= 10.0 {
        1
    } else {
        2
    };
    // 大跨度值（Hz 一类）小数位没有意义，且 `12800.0 Hz` 比 `12800 Hz` 宽一格。
    let body = if span >= 1000.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.digits$}")
    };
    if range.unit.is_empty() { body } else { format!("{body} {}", range.unit) }
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
    // 退回到后缀匹配，但仅在结果无歧义时使用：像 "gain" 这样的短名称可解析到 "Gain"，
    // 而若多个参数都以 "level" 结尾，则保持报错。
    let mut hits = all.iter().filter(|p| norm(p.name).ends_with(&want));
    if let Some(first) = hits.next()
        && hits.next().is_none()
    {
        return Some(first);
    }
    // 带块名前缀的旧写法仍然接受。显示名按设备/App 上的原名（`Gain` / `Level` /
    // `Low Cut`），但用户脚本里可能还写着 `Amp Gain` / `Cab Low Cut` —— 剥掉块名
    // 前缀后就能按普通名字查到。
    let block_norm = norm(block);
    if let Some(rest) = want.strip_prefix(block_norm.as_str())
        && let Some(p) = all.iter().find(|p| norm(p.name) == rest)
    {
        return Some(p);
    }
    None
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
    /// 两者都在 b5=0。压缩器的顺序由 CC51-56 锁定。
    #[test]
    fn compressor_param_order_matches_capture() {
        let comp = params("fx1", Some(3)).expect("fx1 compressor");
        assert_eq!(
            comp.iter().map(|p| p.name).collect::<Vec<_>>(),
            vec!["Threshold", "Ratio", "Knee", "Attack", "Release", "Makeup"]
        );
        assert_eq!(comp.iter().map(|p| p.index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4, 5]);
        let makeup = param_by_name("fx1", Some(3), "makeup").expect("makeup");
        assert_eq!(makeup.index, 5); // b6 = 5
        assert_eq!(makeup.cc, 56);

        // 噪声门是同一块中的另一类型，且只有两个参数。
        //
        // 名字是 `Threshold` 而非 `Gate Threshold`：产品手册 `1-4 gate` 与
        // `4-4 compressor` 的参数说明栏都写 `Threshold`，设备/App 上也显示
        // `Threshold`。此前 CLI 加了块名前缀（`Gate Threshold` /
        // `Compressor Threshold`），用户按设备上的名字找不到。
        let gate = params("fx1", Some(0)).expect("fx1 gate");
        assert_eq!(
            gate.iter().map(|p| p.name).collect::<Vec<_>>(),
            vec!["Threshold", "Release"],
            "fx1 gate 的首参在手册与设备上都叫 Threshold，不带块名前缀"
        );
        let comp = params("fx1", Some(3)).expect("fx1 compressor");
        assert_eq!(
            comp.first().map(|p| p.name),
            Some("Threshold"),
            "fx1 compressor 的首参同样叫 Threshold"
        );
    }

    #[test]
    fn lookup_by_name_is_forgiving() {
        assert_eq!(param_by_name("amp", None, "gain").unwrap().cc, 60);
        assert_eq!(param_by_name("amp", None, "Gain").unwrap().index, 0);
        assert_eq!(param_by_name("cab", None, "low cut").unwrap().cc, 65);
        // 显示名已改回手册原名（`Level` / `Low Cut`），但带块名前缀的旧写法
        // 仍能解析 —— 用户的脚本里可能还写着 `Amp-Level`。
        assert_eq!(param_by_name("amp", None, "Amp-Level").unwrap().cc, 64);
        assert_eq!(param_by_name("amp", None, "level").unwrap().cc, 64);
        assert_eq!(param_by_name("cab", None, "Cab Low Cut").unwrap().cc, 65);
        assert!(param_by_name("amp", None, "nope").is_none());
        // 即使完全匹配同时也是更长名称的后缀，完全匹配仍然优先：
        // delay 类型 5 同时有 "Mix" 和 "Lo-Fi Mix"。
        assert_eq!(param_by_name("delay", Some(5), "mix").unwrap().cc, 78);
        assert_eq!(param_by_name("delay", Some(5), "lofi mix").unwrap().cc, 77);
        // 无完全匹配时，后缀回退可以帮上忙。
        // 显示名是设备上的原名 `Gain`（不带块名前缀），旧写法仍能解析。
        assert_eq!(param_by_name("amp", None, "gain").unwrap().name, "Gain");
        // ……但歧义的后缀会被拒绝而非猜测：8 频段 EQ 同时有 1 kHz / 2 kHz / 4 kHz。
        assert!(param_by_name("eq", Some(2), "khz").is_none());
        assert_eq!(param_by_name("eq", Some(2), "1khz").unwrap().cc, 95);
    }

    #[test]
    fn typed_tables_resolve_by_type() {
        // 数目取设备实存：Digital 3 个、BBD 4 个。
        // CC 指南的表每行末尾多一列「电平」，设备不存那个 b6。
        assert_eq!(params("delay", Some(1)).unwrap().len(), 3); // Digital
        assert_eq!(params("delay", Some(0)).unwrap().len(), 4); // BBD
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
        // 频段 + 末位 Level：3/6/8 段 -> 4/7/9 个参数。
        assert_eq!(params("eq", Some(0)).unwrap().len(), 4);
        assert_eq!(params("eq", Some(1)).unwrap().len(), 7);
        assert_eq!(params("eq", Some(2)).unwrap().len(), 9);
        assert_eq!(eq_bands(0).unwrap()[0].name, "Low");
        assert_eq!(
            eq_bands(0).unwrap().last().unwrap().name,
            "Level",
            "EQ3 的第 4 个参数是 Level（写 b6=3 会被接受）"
        );
        assert!(eq_bands(9).is_none());
    }

    #[test]
    fn fixed_blocks_ignore_type() {
        assert_eq!(params("amp", None).unwrap().len(), 5);
        assert_eq!(params("amp", Some(7)).unwrap().len(), 5); // 类型被忽略
        // cab 只有 2 个参数：手册「CAB 参数说明」只列 Low Cut / High Cut，设备
        // 实存也是 2 个。
        assert_eq!(params("cab", Some(3)).unwrap().len(), 2);
    }
}

#[cfg(test)]
mod slot_legend_tests {
    use super::*;

    /// 帮助文本与 `module list` 都从这里取值，故槽位顺序只有一份定义。
    /// `debug enable <SLOT>` 按位置号取 b5（`slot - 1`），若 BLOCKS 顺序变了，
    /// 那个映射会静默指向别的模块——所以钉死 1 起的位置号。
    #[test]
    fn positions_are_one_based_and_follow_block_order() {
        assert_eq!(BLOCKS.len(), 8);
        for (i, (key, b5)) in BLOCKS.iter().enumerate() {
            assert_eq!(*b5, i as u8, "{key} 的 b5 应等于其下标");
        }
        assert_eq!(
            slot_legend(),
            "1=fx1 2=fx2 3=amp 4=cab 5=delay 6=mod 7=reverb 8=eq"
        );
    }
}
