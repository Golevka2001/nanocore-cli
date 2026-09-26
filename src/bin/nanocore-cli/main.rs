//! `nanocore-cli` — 通过 MIDI 控制和检查 Nanocore（TPE-S10）吉他效果处理器。
//!
//! 命令面遵循设备自身的概念模型——预设（preset）包含效果模块（module），
//! 参数隶属于各自的模块：
//!
//! ```text
//! nanocore-cli preset set 12                 select preset 12
//! nanocore-cli preset next
//! nanocore-cli module amp off                mute the amp module
//! nanocore-cli module amp model 5            pick amp model 5 (Fender 65 Darkface)
//! nanocore-cli module delay param mix 0.3    set a parameter on the delay module
//! nanocore-cli module delay params           list that module's parameters
//! nanocore-cli chain set delay eq fx1 ...    re-route the effect chain
//! nanocore-cli show                          current device state (best-effort)
//! nanocore-cli asset read ir 0 --out out.bin pull the raw IR blob out of slot 0
//! ```
//!
//! 所有涉及原始/未公开帧的操作都归入 `debug`；这些命令面向协议研究，
//! 而非演奏。底层旋钮（`--expect`、`--flags`、`--no-wait`、`--no-sysex`）
//! 为全局隐藏参数，由所有设备命令共享。
//!
//! 模块布局：[`cli`] 定义 clap 命令树，[`common`] 提供共享的基础辅助代码，
//! [`device`] 汇集设备级命令（show / volume / tuner），其余模块各对应一个
//! 命令族（preset / module / chain / asset / models / debug / probe）。
//!
//! 本模块只做分发：设备由各命令族自行打开，并且都在打开之前完成参数校验
//! ——手误不该触碰 MIDI，也不该为一个必然失败的调用占用端口。

mod asset;
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
use crate::chain::cmd_chain;
use crate::cli::{Cli, Command, Conn};
use crate::common::cmd_list;
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

    let cli = Cli::parse();
    let conn = match Conn::from_cli(&cli) {
        Ok(conn) => conn,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(2);
        }
    };

    let result = match cli.command {
        // 完全不需要设备。
        Command::Ports => return cmd_list(),
        Command::Chain { cmd: None } => return chain::print_chain_help(),
        // 需要设备：由各命令族自行打开。
        Command::Preset { cmd } => cmd_preset(cmd, &conn),
        Command::Module { cmd } => cmd_module(cmd, &conn),
        Command::Chain { cmd: Some(cmd) } => cmd_chain(cmd, &conn),
        Command::Show => cmd_show(&conn),
        Command::Volume { percent } => cmd_volume(percent, &conn),
        Command::Tuner { state } => cmd_tuner(state, &conn),
        Command::Models { table, live } => cmd_models(table, live, &conn),
        Command::Asset { cmd } => cmd_asset(cmd, &conn),
        Command::Debug { cmd } => cmd_debug(cmd, &conn),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}
