//! 共享基础辅助代码，按职责拆成四块；本文件只做聚合。
//!
//! 调用方一律 `crate::common::X` —— 拆分是**文件内部**的边界，
//! 不改变任何导入路径，因此命令模块一行都不用改。
//!
//! - `failure`：失败的表示（消息 + 退出码）与设备类型别名
//! - `device`：端口列表、端口号解析、打开设备与重试配置
//! - `tx`：裸 MIDI 发送、SysEx 事务、进度回显、hex 位置参数解析
//! - `render`：hexdump、帧描述、分节线、链序与快照渲染
//!
//! 依赖只朝一个方向：`failure` 无内部依赖，`render` 只依赖协议层，
//! `device` 与 `tx` 再各自依赖前两者。

mod device;
mod failure;
mod render;
mod tx;

pub use device::{cmd_list, open_device};
pub use failure::{Dev, Failure};
pub use render::{Bytes, describe, divider, hex, render_chain, render_snapshot};
pub use tx::{parse_hex, progress, send_standard, transact, transact_reply};
