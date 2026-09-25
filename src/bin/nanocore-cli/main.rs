//! `nanocore-cli` — control and inspect a Nanocore (TPE-S10) guitar effects
//! processor over MIDI.
//!
//! The command surface mirrors the device's own concept model — presets contain
//! effect modules, and parameters belong to their module:
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
//! Everything that speaks raw / undocumented frames lives under `debug`; those
//! commands exist for protocol work, not for making music. The low-level
//! knobs (`--expect`, `--flags`, `--no-wait`, `--no-sysex`) are global-hidden
//! and shared by all device commands.
//!
//! Module layout: [`cli`] holds the clap command tree, [`common`] the shared
//! plumbing, and the remaining modules one command family each
//! (preset / module / chain / asset / models / debug / probe).

mod asset;
mod chain;
mod cli;
mod common;
mod debug;
mod models;
mod module;
mod probe;
mod preset;

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

// ─── entry point ────────────────────────────────────────────────────────────

fn main() {
    // Dynamic shell completion: when invoked by a completion hook
    // (COMPLETE=<shell> set), compute candidates and exit before anything
    // else touches stdout. Inert otherwise.
    clap_complete::CompleteEnv::with_factory(|| Cli::command()).complete();

    let cli = Cli::parse();
    let conn = Conn::from_cli(&cli);

    let result = match cli.command {
        // No device needed at all.
        Command::Ports => return cmd_list(),
        Command::Completions { shell } => {
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            clap_complete::generate(shell, &mut cmd, name, &mut std::io::stdout());
            return;
        }
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
        // Everything else goes through the device.
        command => run(command, &conn),
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        std::process::exit(2);
    }
}

fn run(command: Command, conn: &Conn) -> Result<(), String> {
    // Commands that validate their arguments before the port opens, so a typo
    // can never reach MIDI.
    enum Preflight {
        Chain(u8, Vec<u8>),
    }
    let preflight: Option<Preflight> = match &command {
        Command::Chain { cmd: Some(ChainCmd::Set { positions }) } => {
            let order = parse_chain_positions(positions)?;
            let (kind, payload) = protocol::chain_write(&order)
                .ok_or("internal: chain_write rejected a valid order")?;
            println!("chain (left -> right): {}", render_chain(&order));
            println!(
                "                    as: [{}]",
                order.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")
            );
            println!("tx: type=0x{kind:02x} payload[{}] {}", payload.len(), hex(&payload));
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
