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
//! nanocore-cli asset read ir 0 out.bin       pull the raw IR blob out of slot 0
//! ```
//!
//! 所有涉及原始/未公开帧的操作都归入 `debug`；这些命令面向协议研究，
//! 而非演奏。底层旋钮（`--expect`、`--flags`、`--no-wait`、`--no-sysex`）
//! 为全局隐藏参数，由所有设备命令共享。
//!
//! 模块布局：[`cli`] 定义 clap 命令树，[`common`] 提供共享的基础辅助代码，
//! 其余模块各对应一个命令族（preset / module / chain / asset / models / debug / probe）。

mod asset;
mod chain;
mod cli;
mod common;
mod debug;
mod models;
mod module;
mod preset;
mod probe;

use clap::{CommandFactory, Parser};

use nanocore::midicc;
use nanocore::protocol;

use crate::asset::cmd_asset;
use crate::chain::{chain_write_cmd, cmd_chain_show, cmd_show, parse_chain_positions};
use crate::cli::{ChainCmd, Cli, Command, Conn, Power};
use crate::common::{cmd_list, hex, open_device, render_chain, transact};
use crate::debug::cmd_debug;
use crate::models::cmd_models;
use crate::module::cmd_module;
use crate::preset::cmd_preset;

// ─── 入口 ────────────────────────────────────────────────────────────

fn main() {
    // 动态 shell 补全：当由补全钩子调用时（设置了 COMPLETE=<shell>），
    // 先计算候选项并在其他任何代码触碰 stdout 之前退出。否则不做任何事。
    clap_complete::CompleteEnv::with_factory(|| Cli::command()).complete();

    let cli = Cli::parse();
    let conn = Conn::from_cli(&cli);

    let result = match cli.command {
        // 完全不需要设备。
        Command::Ports => return cmd_list(),
        Command::Chain { cmd: None } => return chain::print_chain_help(),
        Command::Models { region, live } => cmd_models(region, live, &conn),
        Command::Module { cmd } => cmd_module(cmd, &conn),
        Command::Asset { cmd } => cmd_asset(cmd, &conn),
        Command::Tuner { state: None } => {
            eprintln!(
                "error: tuner state read is not implemented yet (needs snapshot decode); \
                 pass `tuner on` or `tuner off`"
            );
            std::process::exit(2);
        }
        // 其余命令都通过设备执行。
        command => run(command, &conn),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

fn run(command: Command, conn: &Conn) -> Result<(), String> {
    // 在打开端口前自行校验参数的命令，确保手误永远不会传到 MIDI。
    enum Preflight {
        Chain(u8, Vec<u8>),
    }
    let preflight: Option<Preflight> = match &command {
        Command::Chain {
            cmd: Some(ChainCmd::Set { positions }),
        } => {
            let order = parse_chain_positions(positions)?;
            let (kind, payload) = protocol::chain_write(&order)
                .ok_or("internal: chain_write rejected a valid order")?;
            println!("chain (left -> right): {}", render_chain(&order));
            println!(
                "                    as: [{}]",
                order
                    .iter()
                    .map(|v| v.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            println!(
                "tx: type=0x{kind:02x} payload[{}] {}",
                payload.len(),
                hex(&payload)
            );
            Some(Preflight::Chain(kind, payload))
        }
        _ => None,
    };

    let mut dev = open_device(conn)?;

    match command {
        Command::Preset { cmd } => cmd_preset(&mut dev, conn, cmd),
        Command::Chain { cmd: Some(cmd) } => match cmd {
            ChainCmd::Set { .. } => {
                let (kind, payload) = match preflight.expect("chain preflight") {
                    Preflight::Chain(k, p) => (k, p),
                };
                chain_write_cmd(&mut dev, kind, payload)
            }
            ChainCmd::Show => cmd_chain_show(&mut dev),
        },
        Command::Show => cmd_show(&mut dev),
        Command::Volume { percent } => {
            let (k, p) = protocol::set_usb_volume(percent);
            transact(&mut dev, conn, k, p, &[])
        }
        Command::Tuner { state: Some(state) } => common::send_standard(
            &mut dev,
            &midicc::global::tuner(conn.channel, Power::on(state)),
        ),
        Command::Debug { cmd } => cmd_debug(&mut dev, conn, cmd),
        _ => unreachable!("command handled without opening a device"),
    }
}
