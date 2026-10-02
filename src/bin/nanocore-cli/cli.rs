//! CLI 定义：clap 命令树与 `Conn` 选项集合。

use clap::{Parser, Subcommand, ValueEnum};
use clap_complete::{ArgValueCandidates, CompletionCandidate};

use nanocore::asset;
use nanocore::midi;
use nanocore::midicc::Block;
use nanocore::models_data;
use nanocore::protocol;

/// `chain set` 位置的补全候选：角色名、设备显示名（小写，`chain_role_index` 可接受的形式）以及 0-7 的索引。
fn chain_position_candidates() -> Vec<CompletionCandidate> {
    let mut out: Vec<CompletionCandidate> = protocol::CHAIN_ROLES
        .iter()
        .enumerate()
        .flat_map(|(i, (cli, disp))| {
            [
                CompletionCandidate::new(*cli).help(Some((*disp).to_string().into())),
                CompletionCandidate::new(disp.to_ascii_lowercase()),
                CompletionCandidate::new(i.to_string()).help(Some((*disp).to_string().into())),
            ]
        })
        .collect();
    out.sort_by(|a, b| a.get_value().cmp(b.get_value()));
    out.dedup_by(|a, b| a.get_value() == b.get_value());
    out
}

/// `module <m> model <N>` 的补全候选。
///
/// 索引的含义随模块而变（amp/cab 是 0–29 的型号表，其余模块是各自的效果类型
/// id），而 clap 的候选项是静态的——拿不到同一命令行的模块名。因此这里只给
/// 中性提示：候选必须对任何模块都成立，标注音箱名会对非 amp/cab 模块产生误导。
/// 30–37 不在候选内：amp/cab 的 CC 路径会拒绝它们，那几个槽位只对
/// `debug variant` 有意义。
fn model_id_candidates() -> Vec<CompletionCandidate> {
    (0u8..=29)
        .map(|i| {
            CompletionCandidate::new(i.to_string())
                .help(Some("model / effect type index — see `models` or `module <m> params`".into()))
        })
        .collect()
}

/// `debug variant <INDEX>` 的补全候选：完整的音箱 / 踏板槽位表（0–37），
/// 该命令正是唯一能寻址 30–37 的路径，因此这里标注型号名是准确的。
fn variant_candidates() -> Vec<CompletionCandidate> {
    (0u8..=37)
        .map(|i| match models_data::lookup_amp(i) {
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
    after_help = "Modules: fx1 fx2 amp cab mod delay reverb eq \
                  (`delay` may be shortened to `del`, `reverb` to `rev`)"
)]
pub struct Cli {
    /// MIDI 输入端口索引（见 `ports`）
    #[arg(long, global = true, value_name = "N")]
    pub in_port: Option<usize>,
    /// MIDI 输出端口索引（见 `ports`）
    #[arg(long, global = true, value_name = "N")]
    pub out_port: Option<usize>,
    /// 用于自动探测的端口名子串 [default: nanocore]
    #[arg(long, global = true, value_name = "NAME")]
    pub hint: Option<String>,
    /// CC/PC 命令使用的 MIDI 通道（0–15；设备将 0 视为 omni）
    #[arg(long, global = true, value_name = "0-15", default_value_t = 0, value_parser = clap::value_parser!(u8).range(0..=15))]
    pub ch: u8,
    /// 应答超时初值，单位毫秒（每次重试翻倍，硬上限为 8 倍）
    ///
    /// 上限 60000：更大的初值对 MIDI 事务没有意义，且会在重试倍增
    /// （`retry_config` 的 ×4/×8）中溢出 `Duration`。
    #[arg(long, global = true, value_name = "MS", value_parser = clap::value_parser!(u64).range(1..=60000))]
    pub timeout: Option<u64>,
    /// 单次事务的最大重试次数 [default: 3]
    #[arg(long, global = true, value_name = "N")]
    pub retries: Option<u32>,

    // 协议探测旋钮——主帮助中隐藏；仅对 `debug raw`、`debug replay`、`debug monitor`、`debug probe` 有意义。
    /// 仅将这些应答类型视为有效应答（逗号分隔的十六进制）
    #[arg(long, global = true, hide = true, value_name = "HH,HH")]
    pub expect: Option<String>,
    /// 帧标志字节（十六进制）[default: 00]
    #[arg(long, global = true, hide = true, value_name = "HH")]
    pub flags: Option<String>,
    /// 即发即忘：不等待应答（仅影响 SysEx 事务；CC/PC 命令本就不等待）
    #[arg(long, global = true, hide = true)]
    pub no_wait: bool,
    /// 发送时不带 F0…F7 SysEx 包装
    #[arg(long, global = true, hide = true)]
    pub no_sysex: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    // ── 预设 ────────────────────────────────────────────────────────────
    /// 管理预设
    Preset {
        #[command(subcommand)]
        cmd: PresetCmd,
    },

    // ── 音频引擎 ──────────────────────────────────────────────────────
    /// 控制效果模块（fx1 fx2 amp cab mod delay reverb eq）
    Module {
        #[command(subcommand)]
        cmd: ModuleCmd,
    },
    /// 设置或显示效果链序（8 个槽位，从左到右）
    Chain {
        #[command(subcommand)]
        cmd: Option<ChainCmd>,
    },
    /// 以可读形式显示当前设备状态快照
    ///
    /// 结构化解码（40 组出厂基线验证）：链序、活动预设、音量，以及 8 个模块的
    /// 开关 / 效果类型 / 参数。模块按记录序打印，记录序 = `b5` 槽位 = 角色；
    /// 链上的物理顺序见 `chain` 那一行。
    Show,
    /// 设置 USB 输出音量（0–100）
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// 控制调音表
    Tuner { state: Option<Power> },

    // ── 设备信息 ───────────────────────────────────────────────────────
    /// 列出可用的 MIDI 端口
    Ports,
    /// 列出内置型号表，或从设备查询
    Models {
        /// 省略时列出两张表
        table: Option<ModelTableArg>,
        /// 查询真实设备而非内置表（只读；覆盖 amp/drive 槽位 0-37）
        #[arg(long)]
        live: bool,
    },

    // ── 资源（音箱型号与 IR）─────────────────────────────────────────
    /// 读取、校验或写入存储在设备槽位中的原始音箱/IR 数据块
    Asset {
        #[command(subcommand)]
        cmd: AssetCmd,
    },

    // ── 设置页（无线 / 回放 / 输入增益 / 蓝牙音量 / 输入通道）───────
    /// 显示或修改设备设置页项目
    Settings {
        #[command(subcommand)]
        cmd: SettingsCmd,
    },

    // ── 协议探测（原始/未公开）───────────────────────────────
    /// 底层 MIDI/协议工具（原始帧、SysEx、调试）
    Debug {
        #[command(subcommand)]
        cmd: DebugCmd,
    },
}

#[derive(Subcommand, Debug)]
pub enum PresetCmd {
    /// 按索引选择预设（0–63，设备显示索引+1）
    Set {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=63))]
        index: u8,
    },
    /// 切换到下一个预设
    Next,
    /// 切换到上一个预设
    Prev,
    /// 列出全部 64 个预设槽位的名称（只读）
    List,
    /// 重命名预设槽位（1-8 个可打印 ASCII 字符，写入设备的名字区）
    Rename {
        /// 预设槽位（0–63）
        #[arg(value_parser = clap::value_parser!(u8).range(0..=63))]
        slot: u8,
        /// 新名字（1-8 个可打印 ASCII 字符）
        name: String,
    },
    /// 显示当前活动预设（槽位号 + 名称、音量、链序）
    Show,
    /// 导出预设槽位为 .npreset 包（manifest + 引用的资产）
    Export {
        /// 预设槽位（1–64）
        #[arg(value_parser = clap::value_parser!(u8).range(1..=64))]
        slot: u8,
        /// 输出文件路径（.npreset）
        #[arg(long, value_name = "FILE")]
        out: String,
        /// 覆盖 amp 型号槽引用（1–38，0 = unknown；缺省用设备报告的活动型号）
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=38))]
        amp_slot: Option<u8>,
        /// 覆盖 IR 槽引用（1–30，0 = unknown；缺省 0 = 未知，App 兼容性受限）
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(0..=30))]
        ir_slot: Option<u8>,
    },
    /// 从 .npreset 包导入预设（覆盖目标槽位）
    Import {
        /// .npreset 文件路径
        file: String,
        /// 重定向目标槽位（1–40；缺省用包内记录的槽位）
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=40))]
        slot: Option<u8>,
        /// 同时写包内内嵌的 IR / amp 资产（先资产后预设；IR 写会清预设表，
        /// 因此资产总是先于预设写入）
        #[arg(long)]
        with_assets: bool,
        /// 只打印将发送的帧概要，不写设备
        #[arg(long)]
        dry_run: bool,
    },
    /// 设置预设音量（0–100，App 中的"预设音量"滑条）
    Volume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// 重放官方"仅 DSP 恢复出厂"流程（40 组预设的名字与内容）
    RestoreFactory,
}

#[derive(Subcommand, Debug)]
pub enum ModuleCmd {
    /// 列出模块及其参数表（无需设备）
    List,
    /// FX1 槽位（噪声门、压缩器等）
    Fx1 {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// FX2 槽位（第二 FX / 踏板效果类）
    Fx2 {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// 音箱槽位（音箱仿真型号）
    Amp {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// 箱体 / IR 槽位（箱体 / 脉冲响应型号）
    Cab {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// 调制槽位（合唱、移相、镶边等）
    Mod {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// 延迟槽位（`del`）
    #[command(alias = "del")]
    Delay {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// 混响槽位（`rev`）
    #[command(alias = "rev")]
    Reverb {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
    /// EQ 槽位
    Eq {
        #[command(subcommand)]
        action: Option<ModuleAction>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ModuleAction {
    /// 开启模块
    On,
    /// 关闭模块
    Off,
    /// 选择型号 / 效果类型
    ///
    /// `amp` 与 `cab` 接受型号索引 0–29（CC43 / CC44）；其余模块接受各自的
    /// 效果类型 id（CC40-CC48）。音箱表索引 30–37 是 FX2 踏板效果，此处
    /// 无法访问——请使用 `debug variant`。
    Model {
        #[arg(add = ArgValueCandidates::new(model_id_candidates))]
        id: u8,
    },
    /// 列出模块的参数（无需设备）
    /// 列出模块的参数表
    ///
    /// 类型依赖的模块未给 `--type` 时会尝试从设备读当前效果类型；设备不在时
    /// 退回只按 `--type`（或类型无关的固定表）列出。
    Params {
        /// 参数依赖效果类型的模块所需的效果类型编号
        /// （1 起，wire = N-1；如 `delay --type 2` = Digital）
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=255))]
        r#type: Option<u8>,
    },
    /// 设置或读取指定名称的模块参数（0.0–1.0，设备归一化）
    ///
    /// 省略 VALUE 即读取：值来自当前预设快照，`b6` 直接索引快照里的设备数组
    /// （真机校准二者逐位对应）。类型依赖的模块未给 `--type` 时从快照读当前
    /// 效果类型。写入会检查设备应答：`b6` 超出该模块的存储时设备回拒绝帧并
    /// **保持原值不变**，此处报错而不是假装写入成功。
    Param {
        /// 参数名（如 `threshold`）或其 b6 索引（裸线缆字节，0 起）
        key: String,
        /// 参数值 0.0–1.0；省略则从快照读取当前值
        value: Option<f32>,
        /// 参数依赖效果类型的模块所需的效果类型编号（1 起；省略时从快照读）
        #[arg(long, value_name = "N", value_parser = clap::value_parser!(u8).range(1..=255))]
        r#type: Option<u8>,
    },
}

#[derive(Subcommand, Debug)]
pub enum ChainCmd {
    /// 写入链序（恰好 8 个角色/索引，每个模块一次）
    Set {
        /// 8 个位置，每个模块恰好一次（`fx1 fx2 amp cab delay mod reverb eq`）
        #[arg(add = ArgValueCandidates::new(chain_position_candidates))]
        positions: Vec<String>,
    },
    /// 从设备读取当前链序
    Show,
}

#[derive(Subcommand, Debug)]
pub enum SettingsCmd {
    /// 显示全部设置项（0x65 快照）
    Show,
    /// 启用或关闭无线
    Wireless {
        /// `on` 或 `off`
        #[arg(value_name = "on|off")]
        state: String,
    },
    /// 启用或关闭回放
    Playback {
        /// `on` 或 `off`
        #[arg(value_name = "on|off")]
        state: String,
    },
    /// 设置输入增益（dB，可为负；正数路径未真机实测）
    InputGain {
        /// 增益值，如 `-3` 或 `0`
        db: i8,
    },
    /// 设置蓝牙音量（0–100）
    BtVolume {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=100))]
        percent: u8,
    },
    /// 设置输入通道（all = 全通道，或 1-16）
    InputChannel {
        #[arg(value_name = "all|1-16")]
        channel: String,
    },
}

#[derive(Subcommand, Debug)]
pub enum AssetCmd {
    /// 列出资源区域及其槽位范围（无需设备）
    List,
    /// 取回数据块，打印其 crc32，可选保存到文件
    Read {
        region: RegionArg,
        slot: u8,
        /// 将数据块写入此文件
        #[arg(long, value_name = "FILE")]
        out: Option<String>,
    },
    /// 取回数据块并与参考文件逐字节比较
    ///
    /// 例如可使用 `.npreset` 包内的 `ir/slot_00.bin` 作为参考。
    Verify {
        region: RegionArg,
        slot: u8,
        /// 用于比较的参考文件
        reference: String,
        /// 将数据块写入此文件
        #[arg(long, value_name = "FILE")]
        out: Option<String>,
    },
    /// 将数据块写入槽位并通过回读校验（仅 IR）
    ///
    /// 警告（实验性）：写入会话中断可能导致设备的预设名表被清空（所有槽位显示占位名 "preset"）；
    /// 根因调查中。请保持会话存活，直到本命令打印最终的 OK/MISMATCH 行。
    Write {
        region: RegionArg,
        slot: u8,
        /// 存放数据块的文件（必须是标准 IR 大小，4096 字节）
        file: String,
        /// 与数据块一起写入的槽位名（默认：保留当前名称）
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
    },
}

#[derive(Subcommand, Debug)]
#[command(
    arg_required_else_help = true,
    after_help = "Hidden global knobs — accepted by every command, meaningful mainly here
(the main help omits them on purpose):

  --expect HH,HH  accept only these reply types
  --flags HH      frame flag byte [default: 00]
  --no-wait       send without waiting for a reply; SysEx transactions only —
                  CC/PC commands (preset, module on/off/model, tuner, debug cc)
                  never wait in the first place
  --no-sysex      send without the F0...F7 SysEx wrapper"
)]
pub enum DebugCmd {
    /// 对设备执行 ping
    Ping,
    /// 读取当前预设的状态快照
    Snapshot,
    /// 打印设备发出的每一帧（ctrl-c 停止）
    Monitor,
    /// 原始 Control Change（十进制；十六进制加 0x.. 前缀）
    Cc {
        /// CC 编号 0–127
        cc: String,
        /// CC 值 0–127
        #[arg(value_parser = clap::value_parser!(u8).range(0..=127))]
        value: u8,
    },
    /// 原始槽位开关（SysEx；模块开关建议使用 `module <m> on/off`）
    Enable {
        /// b5 槽位 0–7（fx1 fx2 amp cab mod delay reverb eq）
        #[arg(value_parser = clap::value_parser!(u8).range(0..=7))]
        slot: u8,
        /// 必须显式给出：有副作用的命令不设默认状态
        state: Power,
    },
    /// 原始 SysEx 参数写入（b5/b6 为十六进制；还原 mask 由值自动推导）
    Setparam {
        b5: String,
        b6: String,
        /// 参数值 0.0–1.0
        value: f32,
    },
    /// 通过 SysEx 选择音箱型号（与 `module amp model` 同一张表）
    ///
    /// 与 CC 路径不同，此命令可访问槽位 30–37（FX2 踏板效果）。
    Variant {
        /// 音箱/踏板槽位 0–37（30 amp sims + 8 FX2 drive pedals）
        #[arg(add = ArgValueCandidates::new(variant_candidates), value_parser = clap::value_parser!(u8).range(0..=37))]
        index: u8,
    },
    /// 发送原始帧：第一个十六进制字节为消息类型，其余为载荷
    Raw {
        /// 十六进制字节，如 `raw 6d 00 02 00 00 09 01`
        bytes: Vec<String>,
    },
    /// 按字节原样重放帧（含头部）并在 2 秒内打印应答
    Replay { bytes: Vec<String> },
    /// 预设切换探测流程：选择 → 快照 → 查询 → 3 秒尾随监听
    Probe {
        #[arg(value_parser = clap::value_parser!(u8).range(0..=63))]
        index: Option<u8>,
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

/// 解析隐藏协议旋钮中的单字节十六进制值（`--flags`，及 `--expect` 的每一段）。
fn parse_hex_byte(arg: &str, value: &str) -> Result<u8, String> {
    u8::from_str_radix(value.trim().trim_start_matches("0x"), 16)
        .map_err(|_| format!("--{arg}: invalid hex byte {value:?}"))
}

impl Conn {
    pub fn from_cli(cli: &Cli) -> Result<Self, String> {
        // 隐藏旋钮不静默回落：传了但解析失败就报错退出。否则错误的过滤条件
        // 会悄悄退化为"接受任何应答"，对协议调试是最危险的失败模式。
        let expect = match cli.expect.as_deref() {
            None => Vec::new(),
            Some(v) => v
                .split(',')
                .map(|p| parse_hex_byte("expect", p))
                .collect::<Result<Vec<u8>, _>>()?,
        };
        let flags = match cli.flags.as_deref() {
            None => 0x00,
            Some(v) => parse_hex_byte("flags", v)?,
        };
        Ok(Self {
            in_port: cli.in_port,
            out_port: cli.out_port,
            hint: cli
                .hint
                .clone()
                .unwrap_or_else(|| midi::DEFAULT_HINT.into()),
            channel: cli.ch,
            timeout_ms: cli.timeout,
            retries: cli.retries,
            expect,
            flags,
            wait: !cli.no_wait,
            no_sysex: cli.no_sysex,
        })
    }
}
