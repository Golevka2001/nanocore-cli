//! CLI 定义：clap 命令树与 `Conn` 选项集合。

use clap::{Args, Parser, Subcommand, ValueEnum};
use clap_complete::{ArgValueCandidates, CompletionCandidate};

use nanocore::asset;
use nanocore::midi;
use nanocore::midicc::Block;
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
/// 31–38（FX2 drive pedals）不在候选内：CC 路径只能寻址 1–30。
fn model_id_candidates() -> Vec<CompletionCandidate> {
    (1u8..=30)
        .map(|i| {
            CompletionCandidate::new(i.to_string())
                .help(Some("model / effect type number — see `models` or `module <m> params`".into()))
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
    /// MIDI input port number, 1-based (see `ports`)
    #[arg(long, global = true, help_heading = "Global options", value_name = "N")]
    pub in_port: Option<usize>,
    /// MIDI output port number, 1-based (see `ports`)
    #[arg(long, global = true, help_heading = "Global options", value_name = "N")]
    pub out_port: Option<usize>,
    ///  MIDI channel for CC/PC commands; a cabled 0 device is treated as omni
    #[arg(long, global = true, help_heading = "Global options", value_name = "1-16", default_value_t = 1, value_parser = clap::value_parser!(u8).range(1..=16))]
    pub ch: u8,
    /// Initial reply timeout in milliseconds (doubles per retry, hard cap 8x; max 60000)
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
    /// Accept only these reply types; empty means any
    #[arg(long, value_name = "HH,HH")]
    pub expect: Option<String>,
    /// Frame flag byte (hex) [default: 00]
    #[arg(long, value_name = "HH")]
    pub flags: Option<String>,
    ///  Send without waiting for a reply; SysEx only
    #[arg(long)]
    pub no_wait: bool,
    ///  Send without the F0...F7 wrapper; the device discards unwrapped frames
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
    ///  Chain order, active preset, volume, and every module's on/off state, effect type and
    ///  parameters. Modules are listed in slot order; the `chain` line gives their physical
    ///  order.
    Show {
        /// Also query the amp / cab models (about 40 extra round trips)
        #[arg(long)]
        full: bool,
    },

    /// Manage presets
    Preset {
        #[command(subcommand)]
        cmd: PresetCmd,
    },
    /// Control effect modules
    Module {
        #[command(subcommand)]
        cmd: ModuleCmd,
    },
    /// Set or show the effect chain order (8 slots, left to right)
    Chain {
        #[command(subcommand)]
        cmd: Option<ChainCmd>,
    },

    /// Toggle the tuner; its state cannot be read
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
        /// Query the device instead of the built-in table (amp/drive slots 1-38)
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
    /// Deprecated alias for `settings usb-volume`.
    #[command(hide = true)]
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },

    /// Low-level MIDI and protocol tools
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
    /// Rename a preset slot
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
        /// Override the IR slot reference (1–30, 0 = unknown; default: 0, limited App compatibility)
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
        /// Also write the IR/amp assets embedded in the bundle (always before the
        /// preset: an IR write clears the preset table)
        #[arg(long)]
        with_assets: bool,
        /// Only print a summary of the frames that would be sent; do not touch the device
        #[arg(long)]
        dry_run: bool,
    },
    /// Set the preset volume (0–100)
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// Replay the DSP-only factory restore flow: the names and contents of 40 presets
    RestoreFactory,
}

#[derive(Subcommand, Debug)]
pub enum ModuleCmd {
    /// List the modules and their parameter tables (no device needed)
    List,
    /// Noise gate or compressor
    Fx1 {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Second FX, or a pedal-type effect
    Fx2 {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Amp-simulation model
    Amp {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Cabinet / impulse-response model
    Cab {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Chorus, phaser, flanger, ...
    Mod {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Delay — the device calls it `del`
    #[command(alias = "del")]
    Delay {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Reverb — the device calls it `rev`
    #[command(alias = "rev")]
    Reverb {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// Three-band EQ
    Eq {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
}

/// `module <key>` 各子命令的 `about`，供 `module list` 复用。
///
/// 从命令树里取，而不是另抄一份：`module <m> --help` 与 `module list` 讲的是
/// 同一件事，两处手写文案早晚会漂移。
pub fn module_descriptions() -> Vec<(String, String)> {
    use clap::CommandFactory;

    let cli = Cli::command();
    let Some(module) = cli.get_subcommands().find(|s| s.get_name() == "module") else {
        return Vec::new();
    };
    module
        .get_subcommands()
        .map(|s| {
            (
                s.get_name().to_string(),
                s.get_about().map(|a| a.to_string()).unwrap_or_default(),
            )
        })
        .collect()
}

#[derive(Subcommand, Debug)]
pub enum ModuleAction {
    /// Turn the module on
    On,
    /// Turn the module off
    Off,
    /// Choose the model / effect type
    ///
    /// `amp` and `cab` take 1–30; the other modules take their own effect type
    /// number. 31–38 (FX2 drive pedals) are out of CC range — read them with
    /// `asset read amp`.
    Model {
        #[arg(add = ArgValueCandidates::new(model_id_candidates), value_parser = clap::value_parser!(u8).range(1..=255))]
        id: u8,
    },
    /// List the module's parameter table
    ///
    /// For type-dependent modules `--type` defaults to the device's current effect
    /// type; without a device only `--type` (or the fixed table) can be used.
    Params {
        /// Effect type number required by type-dependent modules (1-based, wire = N-1;
        /// e.g. `delay --type 2` = Digital)
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=255))]
        r#type: Option<u8>,
    },
    /// Set or read a named module parameter (0.0–1.0, normalised by the device)
    ///
    /// Omitting VALUE reads it from the current preset snapshot. A rejected write
    /// (index outside the module's storage) leaves the old value unchanged and is
    /// reported as an error rather than a success.
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
        /// Role names or numbers 1-8; see `module list`
        #[arg(add = ArgValueCandidates::new(chain_position_candidates))]
        positions: Vec<String>,
    },
    /// Read the current chain order from the device
    Show,
}

/// 变体声明顺序 = 官方手册「全局设置界面」的菜单顺序，`settings show` 的输出
/// 与之一致 —— 两处顺序不同会让「输出的第 N 行」对不上「--help 的第 N 项」。
#[derive(Subcommand, Debug)]
pub enum SettingsCmd {
    /// Show all settings items
    Show,
    /// Turn wireless on or off
    Wireless {
        /// `on` or `off`
        #[arg(value_name = "on|off")]
        state: String,
    },
    /// Turn loopback on or off (mix external audio into the recorded output)
    Loopback {
        /// `on` or `off`
        #[arg(value_name = "on|off")]
        state: String,
    },
    /// Set the input gain (-20 to +20 dB)
    InputGain {
        /// Gain value, e.g. `-3` or `0`
        #[arg(allow_negative_numbers = true)]
        db: i8,
    },
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
    /// Set the MIDI channel (omni = all channels, or 1-16)
    MidiChannel {
        /// `omni` or 1-16
        #[arg(value_name = "omni|1-16")]
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
    ///  Experimental: an interrupted write may clear the device's preset-name table, after
    ///  which every slot shows the placeholder name "preset". Keep the session alive until
    ///  this command prints its final OK/MISMATCH line.
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
    ///  Prints the tx/rx frames, the unpacked stream, and a field table giving each
    ///  field's offset in that stream.
    Snapshot {
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Print every frame the device emits (ctrl-c to stop)
    ///
    ///  Never transmits. Prints the frames the device originates on its own; an idle
    ///  device produces nothing but a heartbeat.
    Monitor {
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
    #[command(after_help = format!("Chain position: {}", nanocore::params_data::slot_legend()))]
    Enable {
        /// Slot number 1–8
        #[arg(value_parser = clap::value_parser!(u8).range(1..=8))]
        slot: u8,
        state: Power,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Read the preset-name list (0x40); same frame as `preset list`
    Presets {
        /// First slot, 1-based
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64), default_value_t = 1)]
        start: u8,
        /// How many slots to request
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64), default_value_t = 8)]
        count: u8,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Read the settings-page snapshot (0x65); same frame as `settings show`
    Settings {
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Write one settings field (0x66)
    Setfield {
        /// Field id: 01 wireless, 02 playback, 04 input gain, 08 USB volume,
        /// 10 BT volume, 20 input channel (hex)
        field: String,
        /// Value (decimal; for 04 it is 7-bit two's-complement dB)
        value: u8,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Read a slot's effect-data name (0x36); same frame as `models`
    Model {
        /// Slot index, 0-based
        #[arg(value_parser = clap::value_parser!(u8))]
        index: u8,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Write a scalar field (0x6D sub 0x02); same frame as `preset set`
    Setfield6d {
        /// Field id (hex)
        field: String,
        /// Value (decimal)
        value: u8,
        #[command(flatten)]
        knobs: ProtocolKnobs,
    },
    /// Rename a preset (0x6D sub 0x0A); `preset rename` also waits and verifies
    Rename {
        /// Slot number, 1-based
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64))]
        slot: u8,
        /// New name
        name: String,
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

/// 解析单个十六进制字节：接受 `0x` 前缀与两侧空白。
///
/// 同一条规则同时服务协议旋钮（`--expect` / `--flags`）与 `debug raw` /
/// `debug replay` 的位置参数（[`crate::common::parse_hex`]），放在一起以免
/// 两处对空白和 `0x` 前缀的处理悄悄分叉。
pub fn parse_hex_byte(value: &str) -> Result<u8, std::num::ParseIntError> {
    u8::from_str_radix(value.trim().trim_start_matches("0x"), 16)
}

/// 协议旋钮专用：在 [`parse_hex_byte`] 之上带上参数名，让报错指得出是哪个开关。
fn parse_hex_arg(arg: &str, value: &str) -> Result<u8, String> {
    parse_hex_byte(value).map_err(|_| format!("--{arg}: invalid hex byte {value:?}"))
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
                .map(|p| parse_hex_arg("expect", p))
                .collect::<Result<Vec<u8>, _>>()?;
        }
        if let Some(v) = knobs.flags.as_deref() {
            self.flags = parse_hex_arg("flags", v)?;
        }
        self.wait = !knobs.no_wait;
        self.no_sysex = knobs.no_sysex;
        Ok(())
    }
}
