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

use std::io::Write;

use clap::{Parser, Subcommand, ValueEnum};
use nanocore::asset;
use nanocore::client::{Error, Nanocore, RetryConfig};
use nanocore::midi::{self, MidiTransport};
use nanocore::midicc::{self, Block};
use nanocore::models_data;
use nanocore::params_data;
use nanocore::protocol;
use nanocore::{Frame, Transport};

// ─── CLI definition ─────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "nanocore-cli",
    version,
    about = "Control and inspect a Nanocore (TPE-S10) guitar effects processor.",
    long_about = None,
    after_help = "Modules: fx1 fx2 amp cab mod delay reverb eq \
                  (`delay` may be shortened to `del`, `reverb` to `rev`)"
)]
struct Cli {
    /// MIDI input port index (see `ports`)
    #[arg(long, global = true, value_name = "N")]
    in_port: Option<usize>,
    /// MIDI output port index (see `ports`)
    #[arg(long, global = true, value_name = "N")]
    out_port: Option<usize>,
    /// Port-name substring used for auto-detection [default: nanocore]
    #[arg(long, global = true, value_name = "NAME")]
    hint: Option<String>,
    /// MIDI channel for CC/PC commands (0–15; the device treats 0 as omni)
    #[arg(long, global = true, value_name = "0-15", default_value_t = 0)]
    ch: u8,
    /// Initial reply timeout in ms (doubles per retry, 8× hard cap)
    #[arg(long, global = true, value_name = "MS")]
    timeout: Option<u64>,
    /// Maximum retries for a transaction [default: 3]
    #[arg(long, global = true, value_name = "N")]
    retries: Option<u32>,

    // Protocol-probe knobs — hidden from the main help; they only matter for
    // `debug raw`, `debug replay`, `debug monitor`, `debug probe`.
    /// Only treat these reply types as an answer (comma-separated hex)
    #[arg(long, global = true, hide = true, value_name = "HH,HH")]
    expect: Option<String>,
    /// Frame flags byte (hex) [default: 00]
    #[arg(long, global = true, hide = true, value_name = "HH")]
    flags: Option<String>,
    /// Fire and forget: do not wait for a reply
    #[arg(long, global = true, hide = true)]
    no_wait: bool,
    /// Send without the F0…F7 SysEx wrapper
    #[arg(long, global = true, hide = true)]
    no_sysex: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    // ── presets ────────────────────────────────────────────────────────────
    /// Manage presets
    Preset {
        #[command(subcommand)]
        cmd: PresetCmd,
    },

    // ── sound engine ──────────────────────────────────────────────────────
    /// Control the effect modules (fx1 fx2 amp cab mod delay reverb eq)
    Module {
        #[command(subcommand)]
        cmd: ModuleCmd,
    },
    /// Set or show the effect-chain order (8 positions, left → right)
    Chain {
        #[command(subcommand)]
        cmd: Option<ChainCmd>,
    },
    /// Show a readable snapshot of the current device state
    ///
    /// Best-effort for now: the full snapshot decode (active preset, per-module
    /// on/off + model, volume, tuner) is still in progress; what can be decoded
    /// today is printed, the rest is marked (TBD).
    Show,
    /// Set the USB output volume (0–100)
    Volume { percent: u8 },
    /// Control the tuner
    Tuner { state: Option<Power> },

    // ── device info ───────────────────────────────────────────────────────
    /// List the available MIDI ports
    Ports,
    /// List the built-in model tables, or query them from the device
    Models {
        /// Omit to list both tables
        region: Option<RegionArg>,
        /// Query the real device instead of the built-in table (read-only)
        #[arg(long)]
        live: bool,
    },

    // ── assets (amp models & IRs) ─────────────────────────────────────────
    /// Read, check or write the raw amp/IR blobs stored in device slots
    Asset {
        #[command(subcommand)]
        cmd: AssetCmd,
    },

    // ── protocol probe (raw / undocumented) ───────────────────────────────
    /// Low-level MIDI / protocol tools (raw frames, SysEx, debugging)
    Debug {
        #[command(subcommand)]
        cmd: DebugCmd,
    },
}

#[derive(Subcommand, Debug)]
enum PresetCmd {
    /// Select a preset by index (0–63, device shows index+1)
    Set { index: u8 },
    /// Step to the next preset
    Next,
    /// Step to the previous preset
    Prev,
    /// List the names of all 64 preset slots (read-only)
    List,
    /// Show the currently active preset (pending snapshot decode)
    Show,
}

#[derive(Subcommand, Debug)]
enum ModuleCmd {
    /// List the modules and their parameter tables (no device needed)
    List,
    /// FX1 slot (noise gate, compressor, …)
    Fx1 { #[command(subcommand)] action: Option<ModuleAction> },
    /// FX2 slot (second FX / drive-pedal-slot effects)
    Fx2 { #[command(subcommand)] action: Option<ModuleAction> },
    /// Amp slot (amp-sim models)
    Amp { #[command(subcommand)] action: Option<ModuleAction> },
    /// Cab / IR slot (cab / impulse-response models)
    Cab { #[command(subcommand)] action: Option<ModuleAction> },
    /// Modulation slot (chorus, phaser, flanger, …)
    Mod { #[command(subcommand)] action: Option<ModuleAction> },
    /// Delay slot (`del`)
    #[command(alias = "del")]
    Delay { #[command(subcommand)] action: Option<ModuleAction> },
    /// Reverb slot (`rev`)
    #[command(alias = "rev")]
    Reverb { #[command(subcommand)] action: Option<ModuleAction> },
    /// EQ slot
    Eq { #[command(subcommand)] action: Option<ModuleAction> },
}

#[derive(Subcommand, Debug)]
enum ModuleAction {
    /// Switch the module on
    On,
    /// Switch the module off
    Off,
    /// Select the model / effect type
    ///
    /// `amp` and `cab` take a model index 0–29 (CC43 / CC44); the other modules
    /// take their effect-type id (CC40-CC48). Indices 30–37 of the amp table are
    /// the FX2 drive pedals and are NOT reachable here — use `debug variant`.
    Model { id: u8 },
    /// List the module's parameters (no device needed)
    Params {
        /// Effect-type id for blocks whose parameters depend on it
        /// (e.g. `delay --type 1` = Digital)
        #[arg(long, value_name = "N")]
        r#type: Option<u8>,
    },
    /// Set a named module parameter (0.0–1.0, device-normalised)
    ///
    /// Omit VALUE to read the current value — the read path is pending the
    /// effect-data / snapshot decode, so for now a write requires VALUE.
    Param {
        /// Parameter name (e.g. `threshold`) or its b6 index
        key: String,
        /// Parameter value 0.0–1.0; omit to read (pending)
        value: Option<f32>,
        /// Effect-type id for blocks whose parameters depend on it
        #[arg(long, value_name = "N")]
        r#type: Option<u8>,
    },
}

#[derive(Subcommand, Debug)]
enum ChainCmd {
    /// Write the chain order (exactly 8 roles/indices, each module once)
    Set {
        /// 8 positions, each module exactly once (`fx1 fx2 amp cab delay mod reverb eq`)
        positions: Vec<String>,
    },
    /// Read the current chain order from the device
    Show,
}

#[derive(Subcommand, Debug)]
enum AssetCmd {
    /// List the asset regions and their slot ranges (no device needed)
    List,
    /// Fetch the blob, print its crc32, optionally save it
    Read {
        region: RegionArg,
        slot: u8,
        /// Write the blob to this file
        out: Option<String>,
    },
    /// Fetch and compare byte-for-byte against a reference file
    ///
    /// Use e.g. the `ir/slot_00.bin` inside a `.npreset` bundle as reference.
    Verify {
        region: RegionArg,
        slot: u8,
        /// Reference file to compare against
        reference: String,
        /// Write the blob to this file
        out: Option<String>,
    },
    /// Write a blob into a slot and verify by reading it back (IR only)
    ///
    /// WARNING (experimental): an interrupted write session can leave the
    /// device's preset-name table wiped (all slots show the placeholder
    /// "preset"); root cause under investigation. Keep the session alive
    /// until this command prints its final OK/MISMATCH line.
    Write {
        region: RegionArg,
        slot: u8,
        /// File holding the blob (must be the standard IR size, 4096 bytes)
        file: String,
        /// Slot name to write along with the blob (default: keep the current one)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
#[command(arg_required_else_help = true)]
enum DebugCmd {
    /// Ping the device
    Ping,
    /// Read the current-preset state snapshot
    Snapshot,
    /// Print every frame the device emits (ctrl-c to stop)
    Monitor,
    /// Raw Control Change (decimal; 0x.. prefix for hex)
    Cc {
        /// CC number 0–127
        cc: String,
        /// CC value 0–127
        value: u8,
    },
    /// Raw slot enable (SysEx; prefer `module <m> on/off` for module switching)
    Enable {
        slot: u8,
        state: Option<Power>,
    },
    /// Raw SysEx parameter write (b3/b5/b6 as hex — see the protocol notes)
    Setparam {
        b3: String,
        b5: String,
        b6: String,
        /// Parameter value 0.0–1.0
        value: f32,
    },
    /// Amp model select via SysEx (same table as `module amp model`)
    ///
    /// Unlike the CC path this reaches slots 30–37 (the FX2 drive pedals).
    Variant { index: u8 },
    /// Send a raw frame: first hex byte is the message type, rest the payload
    Raw {
        /// Hex bytes, e.g. `raw 6d 00 02 00 00 09 01`
        bytes: Vec<String>,
    },
    /// Replay byte-exact frames (header included) and print replies for 2 s
    Replay { bytes: Vec<String> },
    /// Preset-switch discovery flow: select → snapshot → query → 3 s tail
    Probe { index: Option<u8> },
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum BlockArg {
    Fx1,
    Fx2,
    Amp,
    Cab,
    Mod,
    #[value(alias = "del")]
    Delay,
    #[value(alias = "rev")]
    Reverb,
    Eq,
}

impl BlockArg {
    fn midicc(self) -> Block {
        match self {
            BlockArg::Fx1 => Block::Fx1,
            BlockArg::Fx2 => Block::Fx2,
            BlockArg::Amp => Block::Amp,
            BlockArg::Cab => Block::Cab,
            BlockArg::Mod => Block::Mod,
            BlockArg::Delay => Block::Delay,
            BlockArg::Reverb => Block::Reverb,
            BlockArg::Eq => Block::Eq,
        }
    }

    /// The key used by `params_data`.
    fn key(self) -> &'static str {
        match self {
            BlockArg::Fx1 => "fx1",
            BlockArg::Fx2 => "fx2",
            BlockArg::Amp => "amp",
            BlockArg::Cab => "cab",
            BlockArg::Mod => "mod",
            BlockArg::Delay => "delay",
            BlockArg::Reverb => "reverb",
            BlockArg::Eq => "eq",
        }
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum RegionArg {
    Amp,
    Ir,
}

impl RegionArg {
    fn asset_region(self) -> u8 {
        match self {
            RegionArg::Amp => asset::REGION_AMP,
            RegionArg::Ir => asset::REGION_IR,
        }
    }

    fn name(self) -> &'static str {
        match self {
            RegionArg::Amp => "amp",
            RegionArg::Ir => "ir",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
enum Power {
    On,
    Off,
}

impl Power {
    fn on(self) -> bool {
        matches!(self, Power::On)
    }
}

/// Connection options lifted off the global flags.
struct Conn {
    in_port: Option<usize>,
    out_port: Option<usize>,
    hint: String,
    channel: u8,
    timeout_ms: Option<u64>,
    retries: Option<u32>,
    expect: Vec<u8>,
    flags: u8,
    wait: bool,
    no_sysex: bool,
}

impl Conn {
    fn from_cli(cli: &Cli) -> Self {
        let expect = cli
            .expect
            .as_deref()
            .map(|v| {
                v.split(',')
                    .filter_map(|p| u8::from_str_radix(p.trim().trim_start_matches("0x"), 16).ok())
                    .collect()
            })
            .unwrap_or_default();
        Self {
            in_port: cli.in_port,
            out_port: cli.out_port,
            hint: cli.hint.clone().unwrap_or_else(|| midi::DEFAULT_HINT.into()),
            channel: cli.ch,
            timeout_ms: cli.timeout,
            retries: cli.retries,
            expect,
            flags: cli
                .flags
                .as_deref()
                .and_then(|v| u8::from_str_radix(v.trim().trim_start_matches("0x"), 16).ok())
                .unwrap_or(0x00),
            wait: !cli.no_wait,
            no_sysex: cli.no_sysex,
        }
    }
}

// ─── entry point ────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();
    let conn = Conn::from_cli(&cli);

    let result = match cli.command {
        // No device needed at all.
        Command::Ports => return cmd_list(),
        Command::Chain { cmd: None } => return print_chain_help(),
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
        Command::Tuner { state: Some(state) } => send_standard(
            &mut dev,
            &midicc::global::tuner(conn.channel, Power::on(state)),
        ),
        Command::Debug { cmd } => cmd_debug(&mut dev, conn, cmd),
        _ => unreachable!("command handled without opening a device"),
    }
}

// ─── per-command helpers ────────────────────────────────────────────────────

#[allow(clippy::needless_pass_by_value)]
fn cmd_list() {
    match midi::input_ports() {
        Ok(ports) if ports.is_empty() => println!("inputs:  (none)"),
        Ok(ports) => {
            println!("inputs:");
            for (i, n) in &ports {
                println!("  #{i} {n}");
            }
        }
        Err(e) => println!("inputs:  <error: {e}>"),
    }
    match midi::output_ports() {
        Ok(ports) if ports.is_empty() => println!("outputs: (none)"),
        Ok(ports) => {
            println!("outputs:");
            for (i, n) in &ports {
                println!("  #{i} {n}");
            }
        }
        Err(e) => println!("outputs: <error: {e}>"),
    }
}

fn cmd_models(region: Option<RegionArg>, live: bool, conn: &Conn) -> Result<(), String> {
    if live {
        let mut dev = open_device(conn)?;
        return query_models_live(&mut dev);
    }
    match region {
        Some(RegionArg::Amp) => print_models(models_data::AMP),
        Some(RegionArg::Ir) => print_models(models_data::CAB),
        None => {
            println!("AMP models (0-37, device slot = index+1):");
            print_models(models_data::AMP);
            println!("\nCAB / IR models (0-29, device slot = index+1):");
            print_models(models_data::CAB);
        }
    }
    Ok(())
}

// ── module ──────────────────────────────────────────────────────────────────

fn cmd_module(cmd: ModuleCmd, conn: &Conn) -> Result<(), String> {
    match cmd {
        ModuleCmd::List => cmd_module_list(),
        ModuleCmd::Fx1 { action } => module_action(BlockArg::Fx1, action, conn),
        ModuleCmd::Fx2 { action } => module_action(BlockArg::Fx2, action, conn),
        ModuleCmd::Amp { action } => module_action(BlockArg::Amp, action, conn),
        ModuleCmd::Cab { action } => module_action(BlockArg::Cab, action, conn),
        ModuleCmd::Mod { action } => module_action(BlockArg::Mod, action, conn),
        ModuleCmd::Delay { action } => module_action(BlockArg::Delay, action, conn),
        ModuleCmd::Reverb { action } => module_action(BlockArg::Reverb, action, conn),
        ModuleCmd::Eq { action } => module_action(BlockArg::Eq, action, conn),
    }
}

fn module_action(
    block: BlockArg,
    action: Option<ModuleAction>,
    conn: &Conn,
) -> Result<(), String> {
    match action {
        None => print_module_help(block),
        Some(ModuleAction::Params { r#type }) => cmd_params(block, r#type),
        Some(ModuleAction::Param { key, value, r#type }) => {
            let Some(value) = value else {
                return Err(format!(
                    "reading a parameter value is not implemented yet (needs \
                     effect-data / snapshot read); pass a value to write:\n  \
                     nanocore-cli module {} param {key} <0..1>",
                    block.key()
                ));
            };
            let mut dev = open_device(conn)?;
            cmd_param(&mut dev, conn, block.key(), &key, value, r#type)
        }
        Some(ModuleAction::Model { id }) => {
            report_model_selection(block.midicc(), id)?;
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::block_type(conn.channel, block.midicc(), id))
        }
        Some(ModuleAction::On) => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::block_enable(conn.channel, block.midicc(), true))
        }
        Some(ModuleAction::Off) => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::block_enable(conn.channel, block.midicc(), false))
        }
    }
}

fn cmd_module_list() -> Result<(), String> {
    println!("modules (b5 slot — the device string-table order):");
    for &(key, slot) in params_data::BLOCKS {
        let summary = match params_data::params(key, None) {
            Some(ps) => format!("type-independent: {} parameter(s)", ps.len()),
            None => "type-dependent: `module <m> params --type N` to list".into(),
        };
        println!("  {key:<7} b5={slot}  {summary}");
    }
    println!();
    println!("usage: nanocore-cli module <module> <action>");
    println!("       module <module> on | off");
    println!("       module <module> model <N>");
    println!("       module <module> params [--type N]");
    println!("       module <module> param <name|b6> <0..1> [--type N]");
    Ok(())
}

fn print_module_help(block: BlockArg) -> Result<(), String> {
    let key = block.key();
    let slot = params_data::block_slot(key)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".into());
    println!("{key} (b5={slot}, CC on/off {}, CC type {})", block.midicc().onoff_cc(), block.midicc().type_cc());
    println!("  actions: on | off | model <N> | params [--type N] | param <name> [value] [--type N]");
    Ok(())
}

fn cmd_params(block: BlockArg, r#type: Option<u8>) -> Result<(), String> {
    let key = block.key();
    let slot = params_data::block_slot(key)
        .ok_or_else(|| format!("unknown module {key:?}"))?;
    let ps = params_data::params(key, r#type).ok_or_else(|| {
        format!(
            "module {key:?} is type-dependent: pass `--type N`, and note that some \
             types have no published parameter list (fx1 types 1-2, fx2 types 5-7)"
        )
    })?;
    println!(
        "{key} (b5={slot}){}:",
        match r#type {
            Some(t) => format!(", effect type {t}"),
            None => String::new(),
        }
    );
    for p in ps {
        println!("  b6={:<2} {:<24} CC{}", p.index, p.name, p.cc);
    }
    Ok(())
}

fn cmd_param(
    dev: &mut Nanocore<MidiTransport>,
    conn: &Conn,
    block: &str,
    key: &str,
    value: f32,
    r#type: Option<u8>,
) -> Result<(), String> {
    let slot = params_data::block_slot(block)
        .ok_or_else(|| format!("unknown module {block:?} (fx1 fx2 amp cab delay mod reverb eq)"))?;
    let info = match key.parse::<u8>() {
        Ok(i) => params_data::param_by_index(block, r#type, i),
        Err(_) => params_data::param_by_name(block, r#type, key),
    };
    let info = info.ok_or_else(|| {
        format!(
            "no parameter {key:?} for module {block:?}{}. \
             Run `nanocore-cli module {block} params` to list them; type-dependent \
             modules additionally need `--type N`. Some types have no published \
             parameter list (fx1 types 1-2, fx2 types 5-7).",
            match r#type {
                Some(t) => format!(" type {t}"),
                None => String::new(),
            }
        )
    })?;
    if !(0.0..=1.0).contains(&value) {
        return Err(format!("value must be 0.0-1.0 (device-normalised), got {value}"));
    }
    println!(
        "param {block} [b5={slot}] b6={} {} (CC{}) = {value}",
        info.index, info.name, info.cc
    );
    let (k, p) = protocol::param_write(protocol::DEFAULT_B3, slot, info.index, value);
    transact(dev, conn, k, p, &[])
}

/// Validate + describe an Amp/Cab **model** selection made over standard CC.
///
/// `CC43` (Amp) and `CC44` (Cab) are documented as "Type CC" but the OEM guide
/// prints no type table for either block and claims model selection is
/// unassigned. Hardware testing shows they *are* model selectors over the
/// 30-entry tables, and that indices `>= 30` are silently ignored — so reject
/// them here instead of letting the write disappear without effect.
///
/// For every other module this is a no-op: their "type" is a real effect-type ID
/// with its own table in the OEM MIDI CC reference.
fn report_model_selection(block: Block, id: u8) -> Result<(), String> {
    let (label, count, name) = match block {
        Block::Amp => (
            "amp",
            midicc::AMP_MODEL_COUNT,
            models_data::lookup_amp(id).map(|m| format!("{} ({})", m.name, m.code)),
        ),
        Block::Cab => (
            "cab",
            midicc::CAB_MODEL_COUNT,
            models_data::lookup_cab(id).map(|m| format!("{} ({})", m.name, m.code)),
        ),
        _ => return Ok(()),
    };
    if id >= count {
        return Err(format!(
            "{label} model index must be 0-{} (device has {count} models); \
             {id} would be silently ignored — drive pedals 30-37 belong to FX2 and \
             need `debug variant`, not the {label} module",
            count - 1
        ));
    }
    match name {
        Some(n) => println!("module {label} model {id} -> {n}   (standard CC{})", block.type_cc()),
        None => println!("module {label} model {id} (not in embedded table; device has 0-{})", count - 1),
    }
    Ok(())
}

// ── preset ──────────────────────────────────────────────────────────────────

fn cmd_preset(
    dev: &mut Nanocore<MidiTransport>,
    conn: &Conn,
    cmd: PresetCmd,
) -> Result<(), String> {
    match cmd {
        PresetCmd::Set { index } => {
            send_standard(dev, &midicc::program_change(conn.channel, index))
        }
        PresetCmd::Next => send_standard(dev, &midicc::global::next_preset(conn.channel)),
        PresetCmd::Prev => send_standard(dev, &midicc::global::prev_preset(conn.channel)),
        PresetCmd::List => {
            let entries = dev.read_preset_names()?;
            for e in &entries {
                println!(
                    "  #{:<2} {:<12} {}",
                    e.slot,
                    e.name,
                    if e.named { "" } else { "(empty)" }
                );
            }
            Ok(())
        }
        PresetCmd::Show => Err(
            "the active preset is not decoded yet (full snapshot decode pending). \
             Try `preset list` for the slot names."
                .into(),
        ),
    }
}

// ── chain ───────────────────────────────────────────────────────────────────

fn print_chain_help() {
    println!("Effect chain order (`chainOrder`) — the order effects are routed in.");
    println!();
    println!("  Left-to-right order, using these role names (or their 0-7 indices):");
    for (i, (cli, disp)) in protocol::CHAIN_ROLES.iter().enumerate() {
        println!("    {i}  {cli:<8} ({disp})");
    }
    println!();
    println!("  nanocore-cli chain set <8 roles|indices>   write the chain order");
    println!("  nanocore-cli chain show                    read the current order");
    println!();
    println!("  examples:");
    println!("    nanocore-cli chain set delay eq fx1 fx2 amp cab mod reverb");
    println!("    nanocore-cli chain set 4 7 0 1 2 3 5 6          # same thing");
    println!();
    println!("  NOTE: this order is NOT the CC module order — in the chain, DEL comes");
    println!("  before MOD (CC25 is Mod, CC26 is Delay). This is a live edit to the");
    println!("  current preset; reload a factory preset afterwards to get back.");
}

/// Parse and validate the 8 chain positions.
///
/// Accepts role names (`delay`, `del`, `mod`, ...) or 0-7 indices, in any mix.
/// Rejects anything that is not a permutation of `0..8` so a bad order can
/// never reach the device.
fn parse_chain_positions(tokens: &[String]) -> Result<Vec<u8>, String> {
    let n = protocol::CHAIN_LEN;
    if tokens.len() != n {
        return Err(format!(
            "need exactly {n} chain positions, got {}.\nroles: {}",
            tokens.len(),
            protocol::CHAIN_ROLES
                .iter()
                .map(|(c, _)| *c)
                .collect::<Vec<_>>()
                .join(" ")
        ));
    }
    let mut order: Vec<u8> = Vec::with_capacity(n);
    for (i, tok) in tokens.iter().enumerate() {
        let v = match tok.parse::<u8>() {
            Ok(v) if (v as usize) < n => v,
            Ok(v) => return Err(format!("position {}: index {v} out of range (0-{})", i + 1, n - 1)),
            Err(_) => protocol::chain_role_index(tok).ok_or_else(|| {
                format!(
                    "position {}: unknown role {tok:?}. roles: {}",
                    i + 1,
                    protocol::CHAIN_ROLES
                        .iter()
                        .map(|(c, _)| *c)
                        .collect::<Vec<_>>()
                        .join(" ")
                )
            })?,
        };
        order.push(v);
    }
    let mut sorted = order.clone();
    sorted.sort_unstable();
    if sorted != (0..n as u8).collect::<Vec<_>>() {
        return Err(format!(
            "order must contain each module exactly once; got [{}] = {}",
            order.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", "),
            render_chain(&order)
        ));
    }
    Ok(order)
}

/// Write a pre-validated chain order and verify via the device's own echo.
fn chain_write_cmd(
    dev: &mut Nanocore<MidiTransport>,
    kind: u8,
    payload: Vec<u8>,
) -> Result<(), String> {
    let order = protocol::parse_chain_write(&payload)
        .ok_or("internal: device frame does not decode back to the chain")?;

    let ack = dev
        .transact_with_flags(0x00, kind, payload, &[])
        .map_err(|e| match e {
            Error::Timeout { attempts, elapsed } => {
                format!("no reply to the chain write after {attempts} attempt(s) in {elapsed:?}")
            }
            Error::Transport(t) => t.to_string(),
        })?;
    println!("rx (ack):");
    describe(&ack);

    match protocol::parse_chain_ack(&ack.payload) {
        Some(echoed) if echoed == order => {
            println!("ack echoes: {}", render_chain(&echoed));
            println!("OK: device accepted the chain order we sent.");
            Ok(())
        }
        Some(echoed) => {
            println!(
                "WARN: ack echoes a different order (sent {}, ack {})",
                render_chain(&order),
                render_chain(&echoed)
            );
            Ok(())
        }
        None => {
            println!(
                "note: acknowledged, but the ack did not decode as a chain echo \
                 (payload {} bytes): {}",
                ack.payload.len(),
                hex(&ack.payload)
            );
            Ok(())
        }
    }
}

/// Read the current chain order off a device snapshot.
///
/// The `chainOrder` tail is one of the few snapshot areas that is decoded, so
/// this works today; the rest of `show` is still marked (TBD).
fn cmd_chain_show(dev: &mut Nanocore<MidiTransport>) -> Result<(), String> {
    let s = dev.read_snapshot().map_err(|e| e.to_string())?;
    match protocol::parse_chain_order(&s.payload) {
        Some(order) => {
            println!("chain (left -> right): {}", render_chain(&order));
            Ok(())
        }
        None => Err("snapshot did not contain a decodable chainOrder".into()),
    }
}

/// Show a readable snapshot of the current device state.
///
/// Best-effort: only the decoded parts (chainOrder, payload shape) are printed;
/// the rest is marked (TBD) until the `0x63` snapshot decode is completed.
fn cmd_show(dev: &mut Nanocore<MidiTransport>) -> Result<(), String> {
    let s = dev.read_snapshot().map_err(|e| e.to_string())?;
    println!("Current device state (partial — full snapshot decode pending):");
    match protocol::parse_chain_order(&s.payload) {
        Some(order) => println!("  chain:    {}", render_chain(&order)),
        None => println!("  chain:    (not decodable from this snapshot)"),
    }
    println!("  preset:   (TBD) active preset index/name not decoded yet");
    println!("  modules:  (TBD) per-module on/off + model not decoded yet");
    println!("  volume:   (TBD) USB output volume not decoded yet");
    println!("  tuner:    (TBD) tuner state not decoded yet");
    println!(
        "  data:     snapshot {} bytes, {} float candidate(s)",
        s.payload.len(),
        protocol::scan_snapshot_floats(&s.payload).len()
    );
    Ok(())
}

// ── asset ───────────────────────────────────────────────────────────────────

fn cmd_asset(cmd: AssetCmd, conn: &Conn) -> Result<(), String> {
    match cmd {
        AssetCmd::List => {
            println!("asset regions (device blob storage):");
            println!(
                "  amp   region 0x{:02x} — slots 0-37 (30 amp sims + 8 FX2 drive pedals), {} bytes each",
                asset::REGION_AMP,
                asset::AMP_TOTAL
            );
            println!(
                "  ir    region 0x{:02x} — slots 0-29, {} bytes each (write supported)",
                asset::REGION_IR,
                asset::IR_TOTAL
            );
            println!("  operations: read | verify | write (ir only)");
            Ok(())
        }
        AssetCmd::Read { region, slot, out } => {
            let mut dev = open_device(conn)?;
            asset_read(&mut dev, region, slot, out, None)
        }
        AssetCmd::Verify { region, slot, reference, out } => {
            let want = std::fs::read(&reference)
                .map_err(|e| format!("cannot read {reference}: {e}"))?;
            let mut dev = open_device(conn)?;
            asset_read(&mut dev, region, slot, out, Some(&want))
        }
        AssetCmd::Write { region, slot, file, name } => {
            if region != RegionArg::Ir {
                return Err("asset write currently supports ir slots only".into());
            }
            let blob = std::fs::read(&file)
                .map_err(|e| format!("cannot read {file}: {e}"))?;
            if blob.len() as u32 != asset::IR_TOTAL {
                return Err(format!(
                    "IR blob must be exactly {} bytes (got {})",
                    asset::IR_TOTAL,
                    blob.len()
                ));
            }
            println!("asset write ir slot {slot} — {} bytes", blob.len());
            eprintln!(
                "warning: asset write is experimental — do not interrupt the session; \
                 an interrupted write has been observed to wipe the preset-name table \n\
                 (recoverable via a factory reset in the official app)"
            );
            let mut dev = open_device(conn)?;
            let back = dev.write_ir_slot(slot, &blob, name.as_deref(), |msg| {
                eprint!("\r{msg}");
            })?;
            eprintln!();
            match back == blob {
                true => {
                    let crc = asset::crc32(&back);
                    println!("OK: read-back is byte-identical ({} bytes, crc32 {crc:08x}).", back.len());
                    Ok(())
                }
                false => {
                    let at = back
                        .iter()
                        .zip(blob.iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(0);
                    println!(
                        "MISMATCH after write: first difference at byte {at} \
                         (device 0x{:02x}, file 0x{:02x})",
                        back[at], blob[at]
                    );
                    std::process::exit(3);
                }
            }
        }
    }
}

/// Shared body of `asset read` / `asset verify`.
fn asset_read(
    dev: &mut Nanocore<MidiTransport>,
    region: RegionArg,
    slot: u8,
    out: Option<String>,
    reference: Option<&[u8]>,
) -> Result<(), String> {
    let region_byte = region.asset_region();
    let Some((total, plan)) = asset::read_plan(region_byte, slot) else {
        return Err(format!("no read plan for region {}", region.name()));
    };
    println!(
        "asset {} slot {} (read-only, {} chunk request(s), expecting {} bytes)...",
        region.name(),
        slot,
        plan.len(),
        total
    );
    let blob = dev
        .read_asset(&plan, total, |done, n| {
            eprint!("\r  chunk {done}/{n}");
        })
        .map_err(|e| e.to_string())?;
    eprintln!();
    println!("read {} bytes", blob.len());

    let crc = asset::crc32(&blob);
    println!("crc32 = {crc:08x}");

    if let Some(path) = &out {
        match std::fs::write(path, &blob) {
            Ok(()) => println!("written to {path}"),
            Err(e) => eprintln!("warning: cannot write {path}: {e}"),
        }
    }
    if let Some(want) = reference {
        if blob.len() != want.len() {
            println!(
                "MISMATCH: device returned {} bytes, reference has {}",
                blob.len(),
                want.len()
            );
            std::process::exit(3);
        }
        match blob.iter().zip(want.iter()).position(|(a, b)| a != b) {
            None => println!(
                "OK: byte-identical to the reference ({} bytes, crc32 {crc:08x}).",
                blob.len()
            ),
            Some(at) => {
                println!(
                    "MISMATCH: first difference at byte {at}: device 0x{:02x}, reference 0x{:02x}",
                    blob[at], want[at]
                );
                std::process::exit(3);
            }
        }
    }
    Ok(())
}

// ── models ──────────────────────────────────────────────────────────────────

fn print_models(rows: &[models_data::ModelInfo]) {
    for m in rows {
        println!(
            "  {:>2}  {:<10} {:<34} [{}]",
            m.index, m.code, m.name, m.category
        );
    }
}

/// Query the real device for every amp / drive model name (slots 0-37) and
/// cross-check against the embedded table. Read-only: sends `0x36` queries
/// and parses the `0x36` replies.
///
/// Slots whose live name differs from the factory table are flagged
/// `[CUSTOM]` (e.g. an imported NAM file overwrote them); the slot the device
/// is currently playing is additionally flagged `[active]`. Both are counted
/// separately from real table mismatches.
fn query_models_live(dev: &mut Nanocore<MidiTransport>) -> Result<(), String> {
    println!("Querying device model list (amp/drive slots 0-37, read-only)...");
    let mut mismatches = 0usize;
    let mut custom = 0usize;
    for i in 0u8..=37 {
        let reply = dev.query_model(i).map_err(|e| match e {
            Error::Timeout { attempts, elapsed } => {
                format!("timeout on slot {i} after {attempts} attempt(s) in {elapsed:?}")
            }
            Error::Transport(t) => t.to_string(),
        })?;
        match protocol::parse_model_name(&reply.payload) {
            Some(mn) => {
                // Name comparison only: the reply's `active` byte flags the
                // slot the device is currently using, which crosses factory /
                // custom lines and must not decide "matches" by itself.
                let matches_factory = models_data::lookup_amp(i)
                    .map(|em| em.name == mn.name || em.code.eq_ignore_ascii_case(&mn.name))
                    .unwrap_or(false);
                let tag = if mn.active { " [active]" } else { "" };
                if matches_factory {
                    let em = models_data::lookup_amp(i).unwrap();
                    println!("  {i:>2}  {:<18} (matches embedded {}){tag}", mn.name, em.code);
                } else if mn.active || mn.name != models_data::lookup_amp(i).map(|e| e.name).unwrap_or("") {
                    match models_data::lookup_amp(i) {
                        Some(em) => println!(
                            "  {i:>2}  {:<18} [CUSTOM]{tag} (embedded: {} / {})",
                            mn.name, em.name, em.code
                        ),
                        None => println!(
                            "  {i:>2}  {:<18} [CUSTOM]{tag} (no factory entry)",
                            mn.name
                        ),
                    }
                    custom += 1;
                } else {
                    mismatches += 1;
                }
            }
            None => {
                println!("  {i:>2}  <no name in reply>");
                mismatches += 1;
            }
        }
    }
    if custom > 0 {
        println!("{custom} slot(s) carry a custom model.");
    }
    if mismatches == 0 {
        println!("OK: all 38 slots match the embedded models.json table.");
    } else {
        println!("WARN: {mismatches} slot(s) did not match the embedded table.");
    }
    Ok(())
}

// ── debug ───────────────────────────────────────────────────────────────────

fn cmd_debug(
    dev: &mut Nanocore<MidiTransport>,
    conn: &Conn,
    cmd: DebugCmd,
) -> Result<(), String> {
    match cmd {
        DebugCmd::Ping => {
            let (k, p) = protocol::ping();
            transact(dev, conn, k, p, &[nanocore::msg_type::PING])
        }
        DebugCmd::Snapshot => {
            let (k, p) = protocol::request_snapshot();
            transact(dev, conn, k, p, &[nanocore::msg_type::SNAPSHOT])
        }
        DebugCmd::Monitor => {
            println!("monitoring... (ctrl-c to stop)");
            loop {
                let raw = dev.transport().receive().map_err(|e| e.to_string())?;
                if !raw.is_empty() {
                    println!("raw[{}]: {}", raw.len(), hex(&raw));
                    for f in Frame::split_stream(&raw) {
                        describe(&f);
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
                let _ = std::io::stdout().flush();
            }
        }
        DebugCmd::Cc { cc, value } => {
            // CC numbers default to **decimal** so they can be copied straight
            // out of the OEM MIDI CC reference; `0x..` selects hex.
            let cc_num = match cc.strip_prefix("0x") {
                Some(hex) => u8::from_str_radix(hex, 16)
                    .map_err(|_| format!("bad hex CC number {cc:?}"))?,
                None => cc
                    .parse()
                    .map_err(|_| format!("CC number must be decimal 0-127 ({cc:?}); use 0x.. for hex"))?,
            };
            if cc_num > 127 {
                return Err(format!("CC number must be 0-127, got {cc_num}"));
            }
            if value > 127 {
                return Err(format!("CC value must be 0-127, got {value}"));
            }
            send_standard(dev, &midicc::control_change(conn.channel, cc_num, value))
        }
        DebugCmd::Enable { slot, state } => {
            let (k, p) = protocol::set_enabled(slot, state.map(Power::on).unwrap_or(true));
            transact(dev, conn, k, p, &[])
        }
        DebugCmd::Setparam { b3, b5, b6, value } => {
            let p = |s: &str| u8::from_str_radix(s.trim_start_matches("0x"), 16)
                .map_err(|_| format!("{s:?} must be hex"));
            let (b3, b5, b6) = (p(&b3)?, p(&b5)?, p(&b6)?);
            let (k, payload) = protocol::param_write(b3, b5, b6, value);
            transact(dev, conn, k, payload, &[])
        }
        DebugCmd::Variant { index } => {
            match models_data::lookup_amp(index) {
                Some(m) => println!("variant {index} -> {} ({})", m.name, m.code),
                None => println!("variant {index} (unknown slot; device has 0-37)"),
            }
            let (k, p) = protocol::select_variant(index);
            transact(dev, conn, k, p, &[])
        }
        DebugCmd::Raw { bytes } => {
            let bytes = parse_hex(&bytes)?;
            if bytes.len() < 2 {
                return Err("raw needs at least a message type".into());
            }
            if bytes[1..].iter().any(|b| b & 0x80 != 0) {
                eprintln!(
                    "warning: payload contains bytes >= 0x80; they are illegal MIDI data \n\
                     bytes and the device will discard the message (7-bit-safe encoding?)"
                );
            }
            transact(dev, conn, bytes[0], bytes[1..].to_vec(), &conn.expect.clone())
        }
        DebugCmd::Replay { bytes } => {
            let bytes = parse_hex(&bytes)?;
            println!("replay[{}]: {}", bytes.len(), hex(&bytes));
            dev.transport().send(&bytes).map_err(|e| e.to_string())?;
            let deadline = std::time::Instant::now() + std::time::Duration::from_millis(2000);
            let mut got = 0;
            while std::time::Instant::now() < deadline {
                let raw = dev.transport().receive().map_err(|e| e.to_string())?;
                if !raw.is_empty() {
                    for f in Frame::split_stream(&raw) {
                        println!("rx:");
                        describe(&f);
                        got += 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            println!("{got} frame(s) in 2s");
            Ok(())
        }
        DebugCmd::Probe { index } => probe(dev, conn, index),
    }
}

// ── probe ───────────────────────────────────────────────────────────────────

/// Run the preset-switch flow documented in the protocol notes:
/// select preset -> read snapshot (0x63) -> query (0x41), then dump every
/// frame the device emits for a few seconds.
fn probe(
    dev: &mut Nanocore<MidiTransport>,
    conn: &Conn,
    index: Option<u8>,
) -> Result<(), String> {
    let idx = index.unwrap_or(0);

    /// One discovery step: label, (msg type, payload), expected reply types.
    type Step<'a> = (&'a str, (u8, Vec<u8>), Vec<u8>);
    let steps: Vec<Step> = vec![
        ("1 select preset", protocol::select_preset(idx), vec![]),
        ("2 read snapshot", protocol::request_snapshot(), vec![nanocore::msg_type::SNAPSHOT]),
        ("3 query", (nanocore::msg_type::QUERY, vec![0x00, 0x04, 0x00, 0x08]), vec![]),
        ("4 preset list", protocol::query_preset_list(0, 8), vec![nanocore::msg_type::PRESET_LIST]),
    ];

    for (label, (kind, payload), expect) in steps {
        println!("--- step {label} ---");
        println!("tx: type=0x{kind:02x} payload[{}] {}", payload.len(), hex(&payload));
        match dev.transact_with_flags(conn.flags, kind, payload, &expect) {
            Ok(f) => {
                println!("rx:");
                describe(&f);
            }
            Err(e) => println!("  -> {e}"),
        }
    }

    println!("--- tail: everything the device emits for 3s ---");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let raw = dev.transport().receive().map_err(|e| e.to_string())?;
        if !raw.is_empty() {
            println!("raw[{}]: {}", raw.len(), hex(&raw));
            for f in Frame::split_stream(&raw) {
                describe(&f);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    Ok(())
}

// ─── shared plumbing ────────────────────────────────────────────────────────

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

fn describe(frame: &Frame) {
    println!(
        "  dir={:?} flags=0x{:02x} seq=0x{:04x} type=0x{:02x} payload[{}]",
        frame.direction,
        frame.flags,
        frame.seq,
        frame.kind,
        frame.payload.len()
    );
    println!("  payload: {}", hex(&frame.payload));
    if frame.kind == 0x06
        && let Some(ep) = protocol::parse_effect_data(&frame.payload)
    {
        println!("  effect data: slot {} params {:?}", ep.slot, ep.params);
    }
    if frame.kind == 0x40
        && let Some(entries) = protocol::parse_preset_list(&frame.payload)
    {
        println!("  presets ({}):", entries.len());
        for e in &entries {
            println!(
                "    #{:<2} {:<12} {}",
                e.slot,
                e.name,
                if e.named { "" } else { "(empty)" }
            );
        }
    }
    if let Some(order) = protocol::parse_chain_order(&frame.payload) {
        println!("  chainOrder: {order:?}");
    }
    if frame.kind == 0x63 {
        let floats = protocol::scan_snapshot_floats(&frame.payload);
        println!(
            "  snapshot: {} bytes, {} float candidates",
            frame.payload.len(),
            floats.len()
        );
    }
}

fn open_device(conn: &Conn) -> Result<Nanocore<MidiTransport>, String> {
    let transport = match (conn.in_port, conn.out_port) {
        (Some(i), Some(o)) => MidiTransport::open(i, o).map_err(|e| e.to_string()),
        _ => {
            let ins = midi::input_ports().map_err(|e| e.to_string())?;
            let outs = midi::output_ports().map_err(|e| e.to_string())?;
            let i = conn
                .in_port
                .unwrap_or_else(|| midi::find_port(&ins, &conn.hint).unwrap_or(0));
            let o = conn
                .out_port
                .unwrap_or_else(|| midi::find_port(&outs, &conn.hint).unwrap_or(0));
            eprintln!(
                "input  #{} {:?}\noutput #{} {:?}",
                i,
                ins.get(i).map(|p| p.1.as_str()).unwrap_or("?"),
                o,
                outs.get(o).map(|p| p.1.as_str()).unwrap_or("?")
            );
            MidiTransport::open(i, o).map_err(|e| e.to_string())
        }
    }?;
    let mut dev = Nanocore::with_config(transport, retry_config(conn));
    if conn.no_sysex {
        dev.transport().set_sysex(false);
    }
    Ok(dev)
}

fn retry_config(conn: &Conn) -> RetryConfig {
    let mut c = RetryConfig::default();
    if let Some(ms) = conn.timeout_ms {
        c.initial_timeout = std::time::Duration::from_millis(ms);
        c.max_timeout = std::time::Duration::from_millis(ms * 4);
        c.hard_limit = std::time::Duration::from_millis(ms * 8);
    }
    if let Some(n) = conn.retries {
        c.max_retries = n;
    }
    c
}

/// Emit a standard MIDI message (CC / PC) as bare 3-byte packets — no SysEx
/// wrapper. Flips the transport out of SysEx mode for the duration of the send.
fn send_standard(dev: &mut Nanocore<MidiTransport>, bytes: &[u8]) -> Result<(), String> {
    dev.transport().set_sysex(false);
    dev.transport().send(bytes).map_err(|e| e.to_string())
}

fn parse_hex(parts: &[String]) -> Result<Vec<u8>, String> {
    parts
        .iter()
        .map(|h| {
            u8::from_str_radix(h.trim().trim_start_matches("0x"), 16)
                .map_err(|_| format!("bad hex byte {h:?}"))
        })
        .collect()
}

/// Generic SysEx transact: send, wait for a (type-filtered) reply, describe it.
fn transact(
    dev: &mut Nanocore<MidiTransport>,
    conn: &Conn,
    kind: u8,
    payload: Vec<u8>,
    expect: &[u8],
) -> Result<(), String> {
    println!("tx: type=0x{kind:02x} payload[{}] {}", payload.len(), hex(&payload));
    if payload.iter().any(|b| b & 0x80 != 0) {
        eprintln!(
            "warning: payload contains bytes >= 0x80; they are illegal MIDI data \n\
             bytes and the device will discard the message (7-bit-safe encoding?)"
        );
    }
    if !conn.wait {
        dev.send(kind, payload).map_err(|e| e.to_string())?;
        println!("sent (no wait)");
        return Ok(());
    }
    match dev.transact_with_flags(conn.flags, kind, payload, expect) {
        Ok(reply) => {
            println!("rx:");
            describe(&reply);
            Ok(())
        }
        Err(Error::Timeout { attempts, elapsed }) => {
            Err(format!("timeout after {attempts} attempt(s) in {elapsed:?}"))
        }
        Err(Error::Transport(e)) => Err(e.to_string()),
    }
}

/// Render a chain order as `DEL FX1 FX2 ...` using the device display names.
fn render_chain(order: &[u8]) -> String {
    order
        .iter()
        .map(|i| protocol::chain_role_name(*i).unwrap_or("??").to_string())
        .collect::<Vec<_>>()
        .join(" ")
}