//! CLI 定义：clap 命令树与 `Conn` 选项集合。

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::{ArgValueCandidates, CompletionCandidate};

use nanocore::asset;
use nanocore::midi;
use nanocore::midicc::Block;
use nanocore::models_data;
use nanocore::protocol;

/// `chain set` 位置的补全候选：角色名、设备显示名（小写，`chain_role_index` 可接受的形式）以及 1-8 的索引。
fn chain_position_candidates() -> Vec<CompletionCandidate> {
    let mut out: Vec<CompletionCandidate> = protocol::CHAIN_ROLES
        .iter()
        .enumerate()
        .flat_map(|(i, (cli, disp))| {
            [
                CompletionCandidate::new(*cli).help(Some((*disp).to_string().into())),
                CompletionCandidate::new(disp.to_ascii_lowercase()),
                CompletionCandidate::new((i + 1).to_string())
                    .help(Some((*disp).to_string().into())),
            ]
        })
        .collect();
    out.sort_by(|a, b| a.get_value().cmp(b.get_value()));
    out.dedup_by(|a, b| a.get_value() == b.get_value());
    out
}

/// `module <m> model <N>` 的补全候选。
///
/// 编号的含义随模块而变（amp/cab 是 1–30 的型号表，其余模块是各自的效果类型
/// 编号），而 clap 的候选项是静态的——拿不到同一命令行的模块名。因此这里只给
/// 中性提示：候选必须对任何模块都成立，标注音箱名会对非 amp/cab 模块产生误导。
/// 31–38 不在候选内：amp/cab 的 CC 路径会拒绝它们，那几个槽位只对
/// `debug variant` 有意义。
fn model_id_candidates() -> Vec<CompletionCandidate> {
    (1u8..=30)
        .map(|i| {
            CompletionCandidate::new(i.to_string())
                .help(Some("model / effect type number — see `models` or `module <m> params`".into()))
        })
        .collect()
}

/// `debug variant <N>` 的补全候选：完整的音箱 / 踏板槽位表（1–38），
/// 该命令正是唯一能寻址 31–38 的路径，因此这里标注型号名是准确的。
fn variant_candidates() -> Vec<CompletionCandidate> {
    (1u8..=38)
        .map(|i| match models_data::lookup_amp(i - 1) {
            Some(m) => CompletionCandidate::new(i.to_string())
                .help(Some(format!("{} ({})", m.name, m.code).into())),
            None => CompletionCandidate::new(i.to_string()),
        })
        .collect()
}

// ─── CLI 定义 ─────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "nanocore-cli",
    version,
    about = "Control and inspect a Nanocore (TPE-S10) guitar effects processor.",
    long_about = None,
)]
pub struct Cli {
    /// Substring of the port name used for auto-detection [default: nanocore]
    #[arg(long, global = true, help_heading = "Global options", value_name = "NAME")]
    pub hint: Option<String>,
    /// MIDI input port number (see `ports`, 1-based)
    #[arg(long, global = true, help_heading = "Global options", value_name = "N")]
    pub in_port: Option<usize>,
    /// MIDI output port number (see `ports`, 1-based)
    #[arg(long, global = true, help_heading = "Global options", value_name = "N")]
    pub out_port: Option<usize>,
    /// MIDI channel for CC/PC commands (1–16, wire = N-1; a cabled 0 device is treated as omni)
    #[arg(long, global = true, help_heading = "Global options", value_name = "1-16", default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=16))]
    pub ch: u8,
    /// Initial reply timeout in milliseconds (doubles on each retry, hard cap 8x)
    ///
    /// Capped at 60000: a larger initial value is meaningless for a MIDI transaction
    /// and would overflow `Duration` in the retry multipliers (`retry_config`'s x4/x8).
    #[arg(long, global = true, help_heading = "Global options", value_name = "MS", value_parser = clap::value_parser!(u64).range(1..=60000))]
    pub timeout: Option<u64>,
    /// Maximum retries per transaction [default: 3]
    #[arg(long, global = true, help_heading = "Global options", value_name = "N")]
    pub retries: Option<u32>,

    #[command(subcommand)]
    pub command: Command,
}

/// 协议探测旋钮：逐个 flatten 到 `debug` 下**会发帧的**叶子命令
/// （`ping` / `snapshot` / `enable` / `setparam` / `variant` / `raw` /
/// `replay` / `probe`）。
///
/// 这些是裸协议层的应急手段——非 `debug` 命令一个都用不到，所以**不**是
/// 全局参数：让 `preset list --no-sysex` 之类通过只会掩盖"此处不该有这个
/// 开关"。
///
/// 为什么逐个挂而不挂在 `Command::Debug` 上：clap 的 `flatten` **不向孙命令
/// 传播**。挂在 `debug` 层的话 `debug raw --no-sysex` 会被拒——而 `raw` /
/// `replay` 恰恰是最需要它的两个（逐字节重放正是 `--no-sysex` 的用途）。
/// `monitor`（纯监听）与 `cc`（裸 3 字节包，自己切 SysEx 模式）不发 SysEx
/// 帧，故不挂。
#[derive(Args, Debug)]
pub struct ProtocolKnobs {
    /// Treat only these reply types as valid (comma-separated hex); empty = accept any.
    /// A malformed value aborts instead of silently degrading to "accept anything"
    #[arg(long, value_name = "HH,HH")]
    pub expect: Option<String>,
    /// Frame flag byte (hex) [default: 00]
    #[arg(long, value_name = "HH")]
    pub flags: Option<String>,
    /// Send without waiting for a reply. SysEx transactions only — CC/PC commands
    /// (preset, module on/off/model, tuner, debug cc) never wait in the first place
    #[arg(long)]
    pub no_wait: bool,
    /// Send without the F0...F7 SysEx wrapper. The device only accepts wrapped frames;
    ///  bare `7D 4E 43 ...` is silently discarded — only needed for byte-exact replays
    #[arg(long)]
    pub no_sysex: bool,
}

/// 顶层命令。
///
/// 变体声明顺序 = `--help` 里的显示顺序，按「拿到程序 → 第一次用 → 逐步
/// 深入」排列，而不是按功能分类：
///
/// 1. `ports` —— 不知道端口号，后面全跑不了，所以排第一
/// 2. `show` —— 确认连上之后第一件事：看设备现在什么状态
/// 3. `preset` / `module` —— 两个核心操作对象
/// 4. `chain` —— 改链序，需要先理解模块概念
/// 5. `tuner` —— 演奏向
/// 6. `settings` —— 设备级设置页
/// 7. `models` / `asset` —— 低频
/// 8. `debug` —— 协议研究，最低频
#[derive(Subcommand, Debug)]
pub enum Command {
    /// List the available MIDI ports
    Ports,

    /// Show the current device state snapshot in readable form
    ///
    /// Structurally decoded (validated against 40 factory baselines): chain order, active
    /// preset, volume, plus the 8 modules' on/off state, effect type and parameters.
    /// Modules print in record order, which is the `b5` slot = role; the physical
    /// order along the chain is the `chain` line above.
    Show {
        /// Skip the preset-name lookup (one extra 0x40 round trip). The `preset`
        /// line then shows the slot number only. Use it when polling from a script
        /// or when the name is irrelevant to the check.
        #[arg(long)]
        no_name: bool,
        /// Skip the settings-page block (one extra 0x65 round trip). Use it when
        /// polling from a script or when only the preset state matters.
        #[arg(long)]
        no_settings: bool,
    },

    /// Manage presets
    Preset {
        #[command(subcommand)]
        cmd: PresetCmd,
    },
    /// Control effect modules (fx1 fx2 amp cab mod delay reverb eq)
    Module {
        #[command(subcommand)]
        cmd: ModuleCmd,
    },
    /// Set or show the effect chain order (8 slots, left to right)
    Chain {
        #[command(subcommand)]
        cmd: Option<ChainCmd>,
    },

    /// Toggle the tuner (its state cannot be read; invoked bare it explains and errors)
    ///
    /// Verified on hardware: CC80 is write-only, and both `0x65` and `0x63` are
    /// byte-identical before and after `tuner on/off` — no path can read the state back.
    Tuner { state: Option<Power> },

    /// Show or modify the device settings-page items
    Settings {
        #[command(subcommand)]
        cmd: SettingsCmd,
    },

    /// List the built-in model tables, or query them from the device
    Models {
        /// List both tables when omitted
        table: Option<ModelTableArg>,
        /// Query the real device instead of the built-in table (read-only; covers amp/drive slots 1-38)
        #[arg(long)]
        live: bool,
    },
    /// Read, verify or write the raw amp/IR blobs stored in device slots
    Asset {
        #[command(subcommand)]
        cmd: AssetCmd,
    },

    /// Set the USB output volume (0–100)
    ///
    /// Deprecated alias for `settings usb-volume` — the same `0x66` field 0x08.
    /// Kept working so existing scripts don't break; hidden from `--help`.
    #[command(hide = true)]
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },

    /// Low-level MIDI/protocol tools (raw frames, SysEx, debugging)
    Debug {
        #[command(subcommand)]
        cmd: DebugCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum PresetCmd {
    /// Select a preset by number (1–64, same numbering as the device screen / App)
    Set {
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64))]
        index: u8,
    },
    /// Switch to the next preset
    Next,
    /// Switch to the previous preset
    Prev,
    /// List the names of all 64 preset slots (read-only)
    List,
    /// Rename a preset slot (1-8 printable ASCII characters, written to the device's name area)
    Rename {
        /// Preset slot (1–64)
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64))]
        slot: u8,
        /// New name (1-8 printable ASCII characters)
        name: String,
    },
    /// Show the active preset (slot number + name, volume, chain order)
    Show,
    /// Export a preset slot as a .npreset bundle (manifest + referenced assets)
    Export {
        /// Preset slot (1–64)
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64))]
        slot: u8,
        /// Output file path (.npreset)
        #[arg(long, value_name = "FILE")]
        out: String,
        /// Override the amp model slot reference (1–38, 0 = unknown; defaults to the device-reported active model)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=38))]
        amp_slot: Option<u8>,
        /// Override the IR slot reference (1–30, 0 = unknown; default 0 = unknown, limited App compatibility)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=30))]
        ir_slot: Option<u8>,
    },
    /// Import a preset from a .npreset bundle (overwrites the target slot)
    Import {
        /// .npreset file path
        file: String,
        /// Redirect to a target slot (1–40; defaults to the slot recorded in the bundle)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=40))]
        slot: Option<u8>,
        /// Also write the IR/amp assets embedded in the bundle (assets before preset; an IR write
        /// clears the preset table, so assets always go first)
        #[arg(long)]
        with_assets: bool,
        /// Only print a summary of the frames that would be sent; do not touch the device
        #[arg(long)]
        dry_run: bool,
    },
    /// Set the preset volume (0–100, the App's "preset volume" slider)
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Replay the official "DSP-only factory restore" flow (names and contents of 40 presets)
    RestoreFactory,
}

#[derive(Subcommand, Debug)]
pub enum ModuleCmd {
    /// List the modules and their parameter tables (no device needed)
    List,
    /// FX1 slot (noise gate, compressor, ...)
    Fx1 {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// FX2 slot (second FX / pedal-type effects)
    Fx2 {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Amp slot (amp-simulation models)
    Amp {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Cab / IR slot (cabinet / impulse-response models)
    Cab {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Mod slot (chorus, phaser, flanger, ...)
    Mod {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Delay slot (`del`)
    #[command(alias = "del")]
    Delay {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Reverb slot (`rev`)
    #[command(alias = "rev")]
    Reverb {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// EQ slot
    Eq {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ModuleAction {
    /// Turn the module on
    On,
    /// Turn the module off
    Off,
    /// Choose the model / effect type
    ///
    /// `amp` and `cab` take a model number 1–30 (CC43 / CC44); the other modules take
    /// their own effect type number (CC40-CC48). Amp-table numbers 31–38 are FX2 drive
    /// pedals, not reachable here — use `debug variant`.
    Model {
        #[arg(add = ArgValueCandidates::new(model_id_candidates), value_parser = clap::value_parser!(u8).range(1..=255))]
        id: u8,
    },
    /// List the module's parameter table
    ///
    /// For type-dependent modules without `--type`, the current effect type is read from
    /// the device; with no device, the listing falls back to `--type` alone (or to the
    /// fixed type-independent table).
    Params {
        /// Effect type number required by type-dependent modules (1-based, wire = N-1;
        /// e.g. `delay --type 2` = Digital)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=255))]
        r#type: Option<u8>,
    },
    /// Set or read a named module parameter (0.0–1.0, normalised by the device)
    ///
    /// Omitting VALUE reads it: the value comes from the current preset snapshot, where
    /// `b6` indexes the device array directly (hardware-calibrated to match bit for
    /// bit). For type-dependent modules without `--type`, the current type is read from
    /// the snapshot. Writes check the device's reply: when `b6` is outside the module's
    /// storage the device rejects the frame and **leaves the old value unchanged**, so
    /// this errors out rather than pretending the write succeeded.
    Param {
        /// Parameter name (e.g. `threshold`) or its b6 index (raw wire byte, 0-based)
        key: String,
        /// Parameter value 0.0–1.0; omit to read the current value from the snapshot
        value: Option<f32>,
        /// Effect type number required by type-dependent modules (1-based; read from the snapshot if omitted)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=255))]
        r#type: Option<u8>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ChainCmd {
    /// Write the chain order (exactly 8 roles/numbers, each module once)
    Set {
        /// 8 positions, each module exactly once (role names, or numbers 1-8: 1=fx1 … 8=eq)
        #[arg(add = ArgValueCandidates::new(chain_position_candidates))]
        positions: Vec<String>,
    },
    /// Read the current chain order from the device
    Show,
}

/// 变体声明顺序 = `--help` 显示顺序：先总览，再按「音量 → 开关 → 增益 →
/// 通道」——音量是最常调的，所以排在最前。
#[derive(Subcommand, Debug)]
pub enum SettingsCmd {
    /// Show all settings items (0x65 snapshot)
    Show,
    /// Set the USB output volume (0–100)
    UsbVolume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Set the Bluetooth volume (0–100)
    BtVolume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Turn wireless on or off
    Wireless {
        /// `on` or `off`
        #[arg(value_name = "on|off")]
        state: String,
    },
    /// Turn playback on or off
    Playback {
        /// `on` or `off`
        #[arg(value_name = "on|off")]
        state: String,
    },
    /// Set the input gain (dB, may be negative; the positive path is untested on hardware)
    InputGain {
        /// Gain value, e.g. `-3` or `0`
        db: i8,
    },
    /// Set the input channel (all = every channel, or 1-16)
    InputChannel {
        /// `all` or 1-16
        #[arg(value_name = "all|1-16")]
        channel: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum AssetCmd {
    /// List the asset regions and their slot ranges (no device needed)
    List,
    /// Fetch a blob, print its crc32, optionally save it to a file
    Read {
        region: RegionArg,
        slot: u8,
        /// Write the blob to this file
        #[arg(long, value_name = "FILE")]
        out: Option<String>,
    },
    /// Fetch a blob and compare it byte for byte with a reference file
    ///
    /// For example, use `ir/slot_00.bin` from inside a .npreset bundle as the reference.
    Verify {
        region: RegionArg,
        slot: u8,
        /// Reference file to compare against
        reference: String,
        /// Write the blob to this file
        #[arg(long, value_name = "FILE")]
        out: Option<String>,
    },
    /// Write a blob to a slot and verify by read-back (IR only)
    ///
    /// Warning (experimental): an interrupted write session may leave the device's
    /// preset-name table cleared (every slot shows the placeholder name "preset"); root
    /// cause under investigation. Keep the session alive until this command prints its
    /// final OK/MISMATCH line.
    Write {
        region: RegionArg,
        slot: u8,
        /// File holding the blob (must be the standard IR size, 4096 bytes)
        file: String,
        /// Slot name written alongside the blob (default: keep the current name)
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
#[command(
    arg_required_else_help = true
)]
pub enum DebugCmd {
    /// Ping the device
    Ping {
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Read the current preset's state snapshot
    ///
    /// By default only the raw tx/rx frames are printed. `--decode` appends a structured
    /// decoding — the same rendering as `show` (chain order, active preset, volume, and
    /// the 8 modules' on/off state, effect type and parameters).
    Snapshot {
        /// Append a readable structured decoding (0x63 masked stream -> state table)
        #[arg(long)]
        decode: bool,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Print every frame the device emits (ctrl-c to stop)
    ///
    /// Passive: it never transmits, so an idle device produces nothing and the
    /// command prints a heartbeat instead. Frames the device originates on its
    /// own are what shows up here.
    Monitor {
        /// Also expand 0x63 snapshots into a field table (each one prints a
        /// hexdump of the masked stream alongside)
        #[arg(long)]
        decode: bool,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Raw Control Change (decimal; prefix 0x.. for hex)
    Cc {
        /// CC number 0–127
        cc: String,
        /// CC value 0–127
        #[arg(value_parser = clap::value_parser!(u8).range(0..=127))]
        value: u8,
    },
    /// Raw slot enable (SysEx; for module on/off prefer `module <m> on/off`)
    Enable {
        /// Slot number 1–8 (1=fx1 2=fx2 3=amp 4=cab 5=delay 6=mod 7=reverb 8=eq, i.e. b5+1)
        #[arg(value_parser = clap::value_parser!(u8).range(1..=8))]
        slot: u8,
        /// Must be given explicitly: commands with side effects carry no default state
        state: Power,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Raw SysEx parameter write (b5/b6 in hex; the restore mask is derived from the value)
    Setparam {
        b5: String,
        b6: String,
        /// Parameter value 0.0–1.0
        value: f32,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Select an amp model via SysEx (the same table as `module amp model`)
    ///
    /// Unlike the CC path, this command reaches numbers 31–38 (FX2 drive pedals).
    Variant {
        /// Amp/pedal slot 1–38 (30 amp sims + 8 FX2 drive pedals)
        #[arg(add = ArgValueCandidates::new(variant_candidates), value_parser = clap::value_parser!(u8).range(1..=38))]
        index: u8,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Send a raw frame: the first hex byte is the message type, the rest is the payload
    Raw {
        /// Hex bytes, e.g. `raw 6d 00 02 00 00 09 01`
        bytes: Vec<String>,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Replay a frame byte for byte (header included) and print replies within 2 s
    Replay {
        bytes: Vec<String>,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Preset-switch probe sequence: select -> snapshot -> query -> 3 s trailing listen
    Probe {
        /// Preset number (1–64; omit for #1)
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64))]
        index: Option<u8>,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
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

    /// `params_data` 使用的键名。
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

/// `models` 的表选择。
///
/// 与 [`RegionArg`]（`asset` 用的设备存储区域）刻意分开：这里选的是型号表，
/// 表名是 `cab`——`ir` 是存储区域的名字，不是表名。
#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum ModelTableArg {
    Amp,
    Cab,
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

/// 从全局标志提取出的连接选项。
///
/// 编号约定：`in_port` / `out_port` 是 1 起的用户编号（打开端口时换算成
/// 0 起的系统索引）；`channel` 已是线缆字节（0–15）。
#[derive(Clone)]
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

/// 解析协议旋钮中的单字节十六进制值（`--flags`，及 `--expect` 的每一段）。
fn parse_hex_byte(arg: &str, value: &str) -> Result<u8, String> {
    u8::from_str_radix(value.trim().trim_start_matches("0x"), 16)
        .map_err(|_| format!("--{arg}: invalid hex byte {value:?}"))
}

impl Conn {
    pub fn from_cli(cli: &Cli) -> Result<Self, String> {
        Ok(Self {
            // 端口号保持用户空间的 1 起编号，打开时再减 1（见 `open_device`）。
            in_port: cli.in_port,
            out_port: cli.out_port,
            hint: cli
                .hint
                .clone()
                .unwrap_or_else(|| midi::DEFAULT_HINT.into()),
            // 通道在这里就换算成线缆字节：`--ch 1-16` → wire `0-15`。
            channel: cli.ch - 1,
            timeout_ms: cli.timeout,
            retries: cli.retries,
            // 协议旋钮的默认值；`debug` 下的显式取值由 `apply_knobs` 覆盖。
            expect: Vec::new(),
            flags: 0x00,
            wait: true,
            no_sysex: false,
        })
    }

    /// 用 `debug` 子命令携带的 [`ProtocolKnobs`] 覆盖协议层默认值。
    ///
    /// 旋钮不静默回落：传了但解析失败就报错退出。否则错误的过滤条件会悄悄
    /// 退化为「接受任何应答」，对协议调试是最危险的失败模式。
    pub fn apply_knobs(&mut self, knobs: &ProtocolKnobs) -> Result<(), String> {
        if let Some(v) = knobs.expect.as_deref() {
            self.expect = v
                .split(',')
                .map(|p| parse_hex_byte("expect", p))
                .collect::<Result<Vec<u8>, _>>()?;
        }
        if let Some(v) = knobs.flags.as_deref() {
            self.flags = parse_hex_byte("flags", v)?;
        }
        self.wait = !knobs.no_wait;
        self.no_sysex = knobs.no_sysex;
        Ok(())
    }
}
