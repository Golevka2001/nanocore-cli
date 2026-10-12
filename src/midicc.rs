//! Nanocore 的标准 MIDI CC / Program Change 控制面（固件 1.04 及以上）。
//!
//! 这些是**官方文档化**的控制消息，出自 OEM 的
//! `NANOCORE-MIDI-Control-User-Guide.pdf`。
//! 它们是普通的 3 字节 MIDI 消息，
//! *不*经过自定义的 `F0 7D 4E 43 …` SysEx 包装。
//!
//! 本模块与传输层无关：每个构造函数都返回要发送的原始字节。
//! 当 `set_sysex(false)` 生效时，`midir` 传输层会原样发送它们
//! （参见 `crate::midi::MidiTransport` 与 `nanocore-cli`）。
//!
//! ## 对 OEM 指南的更正：CC43 / CC44 *确实*可选择模型
//!
//! 指南的主 CC 映射表将 `CC43` / `CC44` 列为音箱/箱体的"类型 CC"，
//! 但之后只印出了 FX1/FX2/Mod/Delay/Reverb/EQ 的类型表，并声称
//! *"此处未分配模型/配置与 IR 槽位选择。"*
//!
//! 硬件实测（固件 1.04）表明该说法**对音箱和箱体块不成立**：
//! `CC43 = n` 选择音箱槽位 `n`，`CC44 = n` 选择箱体槽位 `n`，
//! 二者都作用于 [`crate::models_data`] 中的 30 项表。值 `>= 30` 会被忽略。
//! 使用 [`block_type`]。
//! 这意味着普通第三方 MIDI 控制器即可切换音箱/箱体模型——无需 SysEx。

/// 在 `channel`（0–15）通道上 Control Change 的 MIDI 状态字节。
#[inline]
pub fn cc_status(channel: u8) -> u8 {
    0xB0u8 | (channel & 0x0F)
}

/// 在 `channel`（0–15）通道上 Program Change 的 MIDI 状态字节。
#[inline]
pub fn pc_status(channel: u8) -> u8 {
    0xC0u8 | (channel & 0x0F)
}

/// 构造 3 字节的 Control Change 消息：`[0xB0|ch, cc, value]`。
///
/// `cc` 和 `value` 会被掩码到 7 比特——
/// `>= 0x80` 的字节在 MIDI 数据流中不合法，会被设备丢弃。
pub fn control_change(channel: u8, cc: u8, value: u8) -> Vec<u8> {
    vec![cc_status(channel), cc & 0x7F, value & 0x7F]
}

/// 构造 2 字节的 Program Change 消息：`[0xC0|ch, program]`。
///
/// 程序号从 0 开始，与设备预设索引一一对应
/// （PC `2` 调用设备显示为 `03` 的预设）。
pub fn program_change(channel: u8, program: u8) -> Vec<u8> {
    vec![pc_status(channel), program & 0x7F]
}

/// 可通过标准 CC 映射寻址的效果块。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    Fx1,
    Fx2,
    Amp,
    Cab,
    Mod,
    Delay,
    Reverb,
    Eq,
}

impl Block {
    /// 切换该块开/关的 CC（值 `0`=OFF，`127`=ON）。
    pub fn onoff_cc(self) -> u8 {
        match self {
            Block::Fx1 => 20,
            Block::Fx2 => 21,
            Block::Amp => 23,
            Block::Cab => 24,
            Block::Mod => 25,
            Block::Delay => 26,
            Block::Reverb => 27,
            Block::Eq => 28,
        }
    }

    /// 选择该块效果/模型类型的 CC（发送确切的 Type ID）。
    pub fn type_cc(self) -> u8 {
        match self {
            Block::Fx1 => 40,
            Block::Fx2 => 41,
            Block::Amp => 43,
            Block::Cab => 44,
            Block::Mod => 45,
            Block::Delay => 46,
            Block::Reverb => 47,
            Block::Eq => 48,
        }
    }

    /// 按 CLI / CC 映射名称解析块（不区分大小写）。
    pub fn from_name(name: &str) -> Option<Block> {
        match name.to_ascii_lowercase().as_str() {
            "fx1" => Some(Block::Fx1),
            "fx2" => Some(Block::Fx2),
            "amp" => Some(Block::Amp),
            "cab" => Some(Block::Cab),
            "mod" | "modulation" => Some(Block::Mod),
            "delay" => Some(Block::Delay),
            "rev" | "reverb" => Some(Block::Reverb),
            "eq" => Some(Block::Eq),
            _ => None,
        }
    }
}

/// 发送块开/关切换（标准 CC）。
pub fn block_enable(channel: u8, block: Block, on: bool) -> Vec<u8> {
    control_change(channel, block.onoff_cc(), if on { 127 } else { 0 })
}

/// 发送块类型/效果/模型选择（标准 CC）。
///
/// amp / cab 的实测行为：OEM 指南未给出这两块的类型表，并声称模型选择未分配，
/// 但硬件上 CC43 / CC44 确实在选槽位 —— `CC43 = 20` → `RoJC120`、`CC43 = 29` →
/// 用户导入的 NAM、`CC44 = 14` → `Og412`、`CC44 = 29` → `Mar412`；`= 30` 与 `= 37`
/// 均无变化。索引 `>= 30` 被静默忽略，调用方应先行校验。
/// `models_data::AMP[30..38]` 的踏板条目在此不可达，属 FX2 块。
pub fn block_type(channel: u8, block: Block, type_id: u8) -> Vec<u8> {
    control_change(channel, block.type_cc(), type_id)
}

/// 音箱块中可选模型的数量（有效索引 `0..=29`）；31–38 是 FX2 drive pedals。
pub const AMP_MODEL_COUNT: u8 = 30;

/// 箱体 / IR 块中可选模型的数量（有效索引 `0..=29`）。
pub const CAB_MODEL_COUNT: u8 = 30;

/// 全局控制（参见 `midi_cc_reference.md`）。
pub mod global {
    use super::control_change;

    /// 调音器开/关（CC80）。
    pub fn tuner(channel: u8, on: bool) -> Vec<u8> {
        control_change(channel, 80, if on { 127 } else { 0 })
    }
    /// 切换到上一个预设（CC81，值 127）。
    pub fn prev_preset(channel: u8) -> Vec<u8> {
        control_change(channel, 81, 127)
    }
    /// 切换到下一个预设（CC82，值 127）。
    pub fn next_preset(channel: u8) -> Vec<u8> {
        control_change(channel, 82, 127)
    }
}
