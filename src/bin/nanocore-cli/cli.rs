//! CLI definition: the clap command tree and the `Conn` options bundle.

use clap::{Parser, Subcommand, ValueEnum};

use nanocore::asset;
use nanocore::midi;
use nanocore::midicc::Block;

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
pub struct Cli {
    /// MIDI input port index (see `ports`)
    #[arg(long, global = true, value_name = "N")]
    pub in_port: Option<usize>,
    /// MIDI output port index (see `ports`)
    #[arg(long, global = true, value_name = "N")]
    pub out_port: Option<usize>,
    /// Port-name substring used for auto-detection [default: nanocore]
    #[arg(long, global = true, value_name = "NAME")]
    pub hint: Option<String>,
    /// MIDI channel for CC/PC commands (0–15; the device treats 0 as omni)
    #[arg(long, global = true, value_name = "0-15", default_value_t = 0)]
    pub ch: u8,
    /// Initial reply timeout in ms (doubles per retry, 8× hard cap)
    #[arg(long, global = true, value_name = "MS")]
    pub timeout: Option<u64>,
    /// Maximum retries for a transaction [default: 3]
    #[arg(long, global = true, value_name = "N")]
    pub retries: Option<u32>,

    // Protocol-probe knobs — hidden from the main help; they only matter for
    // `debug raw`, `debug replay`, `debug monitor`, `debug probe`.
    /// Only treat these reply types as an answer (comma-separated hex)
    #[arg(long, global = true, hide = true, value_name = "HH,HH")]
    pub expect: Option<String>,
    /// Frame flags byte (hex) [default: 00]
    #[arg(long, global = true, hide = true, value_name = "HH")]
    pub flags: Option<String>,
    /// Fire and forget: do not wait for a reply
    #[arg(long, global = true, hide = true)]
    pub no_wait: bool,
    /// Send without the F0…F7 SysEx wrapper
    #[arg(long, global = true, hide = true)]
    pub no_sysex: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
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
pub enum PresetCmd {
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
pub enum ModuleCmd {
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
pub enum ModuleAction {
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
pub enum ChainCmd {
    /// Write the chain order (exactly 8 roles/indices, each module once)
    Set {
        /// 8 positions, each module exactly once (`fx1 fx2 amp cab delay mod reverb eq`)
        positions: Vec<String>,
    },
    /// Read the current chain order from the device
    Show,
}

#[derive(Subcommand, Debug)]
pub enum AssetCmd {
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
pub enum DebugCmd {
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
pub enum BlockArg {
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
    pub fn midicc(self) -> Block {
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
    pub fn key(self) -> &'static str {
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
pub enum RegionArg {
    Amp,
    Ir,
}

impl RegionArg {
    pub fn asset_region(self) -> u8 {
        match self {
            RegionArg::Amp => asset::REGION_AMP,
            RegionArg::Ir => asset::REGION_IR,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            RegionArg::Amp => "amp",
            RegionArg::Ir => "ir",
        }
    }
}

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum Power {
    On,
    Off,
}

impl Power {
    pub fn on(self) -> bool {
        matches!(self, Power::On)
    }
}

/// Connection options lifted off the global flags.
pub struct Conn {
    pub in_port: Option<usize>,
    pub out_port: Option<usize>,
    pub hint: String,
    pub channel: u8,
    pub timeout_ms: Option<u64>,
    pub retries: Option<u32>,
    pub expect: Vec<u8>,
    pub flags: u8,
    pub wait: bool,
    pub no_sysex: bool,
}

impl Conn {
    pub fn from_cli(cli: &Cli) -> Self {
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
