//! `nanocore-cli` — 通过 MIDI 控制和检查 Nanocore（TPE-S10）吉他效果处理器。
//!
//! 命令面遵循设备自身的概念模型——预设（preset）包含效果模块（module），
//! 参数隶属于各自的模块：
//!
//! ```text
//! nanocore-cli preset set 12                 select preset 12
//! nanocore-cli preset next
//! nanocore-cli module amp off                mute the amp module
//! nanocore-cli module amp model 6            pick amp model 6 (Fender 65 Darkface)
//! nanocore-cli module delay param mix 0.3    set a parameter on the delay module
//! nanocore-cli module delay params           list that module's parameters
//! nanocore-cli chain set delay eq fx1 ...    re-route the effect chain
//! nanocore-cli show                          current device state (best-effort)
//! nanocore-cli asset read ir 1 --out out.bin pull the raw IR blob out of slot 1
//! ```
//!
//! 所有用户可见的编号一律 **1 起**（预设号、型号/效果类型、链位、模块、槽位、
//! MIDI 通道、端口），与设备屏和官方 App 同号；例外是 `debug` 里直接对应线缆
//! 字节的参数（`b5`/`b6`、CC 号/值、裸帧）保持原始值。
//!
//! 所有涉及原始/未公开帧的操作都归入 `debug`；这些命令面向协议研究，
//! 而非演奏。协议层旋钮（`--expect`、`--flags`、`--no-wait`、`--no-sysex`）
//! 只挂在 `debug` 下。
//!
//! 模块布局：[`cli`] 定义 clap 命令树，[`common`] 提供共享的基础辅助代码，
//! [`device`] 汇集设备级命令（show / volume / tuner），其余模块各对应一个
//! 命令族（preset / module / chain / asset / models / debug / probe）。
//!
//! 本模块只做分发。设备由各命令族自行打开，都在打开之前完成参数校验。
//! 失败以 [`Failure`]（消息 + 退出码）返回，`main` 是进程唯一的退出点。

mod asset;
mod settings;
mod chain;
mod cli;
mod common;
mod debug;
mod device;
mod models;
mod module;
mod preset;
mod probe;

use clap::{CommandFactory, Parser};

use crate::asset::cmd_asset;
use crate::settings::cmd_settings;
use crate::chain::cmd_chain;
use crate::cli::{Cli, Command, Conn};
use crate::common::{Failure, cmd_list};
use crate::debug::cmd_debug;
use crate::device::{cmd_show, cmd_tuner, cmd_volume};
use crate::models::cmd_models;
use crate::module::cmd_module;
use crate::preset::cmd_preset;

// ─── 入口 ────────────────────────────────────────────────────────────

fn main() {
    // 动态 shell 补全：当由补全钩子调用时（设置了 COMPLETE=<shell>），
    // 先计算候选项并在其他任何代码触碰 stdout 之前退出。否则不做任何事。
    clap_complete::CompleteEnv::with_factory(Cli::command).complete();

    // 唯一的退出点：失败自带退出码（见 [`Failure`]）。
    if let Err(f) = run(Cli::parse()) {
        eprintln!("error: {}", f.message);
        std::process::exit(f.exit_code);
    }
}

/// 解析连接选项并分发命令。
///
/// 设备由各命令族自行打开，都在打开之前完成参数校验。
fn run(cli: Cli) -> Result<(), Failure> {
    let conn = Conn::from_cli(&cli)?;
    match cli.command {
        // 完全不需要设备。
        Command::Ports => {
            cmd_list();
            Ok(())
        }
        Command::Chain { cmd: None } => {
            chain::print_chain_help();
            Ok(())
        }
        // 需要设备。
        Command::Preset { cmd } => cmd_preset(cmd, &conn),
        Command::Module { cmd } => cmd_module(cmd, &conn),
        Command::Chain { cmd: Some(cmd) } => cmd_chain(cmd, &conn),
        Command::Show {
            no_name,
            no_settings,
            no_model,
        } => cmd_show(no_name, no_settings, no_model, &conn),
        Command::Volume { percent } => cmd_volume(percent, &conn),
        Command::Tuner { state } => cmd_tuner(state, &conn),
        Command::Models { table, live } => cmd_models(table, live, &conn),
        Command::Asset { cmd } => cmd_asset(cmd, &conn),
        Command::Settings { cmd } => cmd_settings(cmd, &conn),
        // 协议旋钮挂在各叶子变体上，由 `cmd_debug` 就地应用（见 `ProtocolKnobs`）。
        Command::Debug { cmd } => cmd_debug(cmd, &conn),
    }
}
