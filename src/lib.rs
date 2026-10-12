//! Nanocore (TPE-S10) 协议库。
//!
//! 帧布局：
//!
//! ```text
//! 7D 4E 43 | dir | flags | 02 | seq_lo | seq_hi | type | payload...
//! ```
//!
//! * `dir`   0x70 = TX（app -> 设备），0x71 = RX，0x72 = 主动（非请求）的事件
//! * `flags` 观测到的取值：0x00（正常）、0x02（live/流式）、0x22（0x43 预设写入）、0x40（设备 RX）
//! * 字节 5 恒为 `0x02`
//! * `seq` 是小端 u16 计数器，由设备回显
//! * `type` 是消息类型（如 0x6D 数据、0x63 快照、0x68 ping）
//!
//! 多条消息的载荷以设备生成的"紧凑状态"元数据尾（`DA 00 0E ... statusByte ... From Nanocore`）结尾。
//! 相关辅助函数见 [`tail`]。

pub mod frame;
pub mod protocol;
pub mod midicc;
pub mod models_data;
pub mod params_data;
pub mod asset;
pub mod amp_write;
pub mod preset43;
pub mod preset_bundle;
pub mod preset_restore;
pub mod tail;
pub mod transport;
pub mod client;

#[cfg(feature = "midi")]
pub mod midi;

pub use frame::{Frame, Direction, MAGIC};
pub use transport::Transport;
pub use client::Nanocore;

/// 消息类型（帧的字节 8）。
pub mod msg_type {
    pub const PING: u8 = 0x68;
    pub const DATA: u8 = 0x6D;
    pub const SNAPSHOT: u8 = 0x63;
    pub const PRESET_LIST: u8 = 0x40;
    pub const QUERY: u8 = 0x41;
    pub const EFFECT_DATA: u8 = 0x06;
    /// 型号列表枚举：查询单个 amp/drive 槽位名称（`0x36`）。
    pub const MODEL_LIST: u8 = 0x36;
    pub const SETTINGS: u8 = 0x65;
    pub const USB_VOLUME: u8 = 0x66;
    pub const PRESET_WRITE: u8 = 0x43;
    /// 设备发起的通知（主动非请求；链序写入后携带新的链序等信息）。
    pub const NOTIFY: u8 = 0x09;
}

/// `DATA` (0x6D) 子命令。
pub mod sub {
    /// 通用的"设置标量字段"（预设索引 / cab / variant / ...）。
    pub const SET_FIELD: u8 = 0x02;
    /// 启用 / 禁用一个效果槽位。
    pub const ENABLE: u8 = 0x03;
    /// 写入单个参数值（float32）。
    pub const PARAM_WRITE: u8 = 0x07;
    /// 改名会话的前置帧（载荷 3 字节 0）。
    pub const NAME_PREPARE: u8 = 0x01;
    /// 预设命名。
    pub const NAME: u8 = 0x0A;
    // 效果类型切换使用 `4 * (param_count + 1)`。
}

/// 设置页字段 id（`0x66` SET_FIELD 写、`0x65` 快照广播读；
/// 全部与设备实际行为一致）。
pub mod setting {
    /// 无线开关（0/1）。
    pub const WIRELESS: u8 = 0x01;
    /// 回放开关（0/1）。
    pub const PLAYBACK: u8 = 0x02;
    /// 输入增益（dB，7-bit 补码负数 + 符号标志字节）。
    pub const INPUT_GAIN: u8 = 0x04;
    /// USB 音量（0-100）。
    pub const USB_VOLUME: u8 = 0x08;
    /// 蓝牙音量（0-100）。
    pub const BT_VOLUME: u8 = 0x10;
    /// 输入通道（Ch1-Ch16 = 1-16，0 = 全通道）。
    pub const INPUT_CHANNEL: u8 = 0x20;
}

/// [`sub::SET_FIELD`] 及设置类消息使用的字段 id。
pub mod field {
    /// 预设音量（0-100），App"预设音量"滑条走此字段
    /// 。
    pub const PRESET_VOLUME: u8 = 0x04;
    /// 效果 variant / amp 型号。
    pub const VARIANT: u8 = 0x06;
    /// 音箱 / IR。
    pub const CAB: u8 = 0x07;
    /// USB 音量（0-100），以 0x66 设置消息发送。
    pub const USB_VOLUME: u8 = 0x08;
    /// 预设索引（从 0 开始）。
    pub const PRESET: u8 = 0x09;
}
