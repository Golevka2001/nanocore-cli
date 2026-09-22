//! `nanocore-cli` — control and inspect a Nanocore (TPE-S10) guitar effects
//! processor over MIDI.
//!
//! The command surface is organised by what you want to do, not by which
//! transport a command happens to use underneath (standard MIDI CC/PC vs the
//! device's SysEx app protocol — that distinction is an implementation detail):
//!
//! ```text
//! nanocore-cli preset 2                 select preset 2
//! nanocore-cli fx amp off               mute the amp block
//! nanocore-cli amp 5                    pick amp model 5 (Fender 65 Darkface)
//! nanocore-cli chain delay eq fx1 ...   re-route the effect chain
//! nanocore-cli asset read ir 0 out.bin  pull the raw IR blob out of slot 0
//! ```
//!
//! The few commands under "Protocol probe" send raw / undocumented frames;
//! they exist for protocol work, not for making music.

use std::io::Write;

use clap::{Parser, Subcommand, ValueEnum};
use nanocore::client::{Error, Nanocore, RetryConfig};
use nanocore::midi::{self, MidiTransport};
use nanocore::midicc::{self, Block};
use nanocore::models_data;
use nanocore::protocol;
use nanocore::{Frame, Transport};

// ─── CLI definition ─────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "nanocore-cli",
    version,
    about = "Control and inspect a Nanocore (TPE-S10) guitar effects processor.",
    long_about = None,
    after_help = "Blocks: fx1 fx2 amp cab mod delay reverb eq \
                  (`delay` may be shortened to `del`, `reverb` to `rev`)"
)]
struct Cli {
    /// MIDI input port index (see `list`)
    #[arg(long, global = true, value_name = "N")]
    in_port: Option<usize>,
    /// MIDI output port index (see `list`)
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
    // raw frame work (`raw`, `replay`, `monitor`, `probe`).
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
    // ── sound engine ──────────────────────────────────────────────────────
    /// Select a preset by index (0–63, device shows index+1)
    Preset { index: u8 },
    /// Step to the previous preset
    Prev,
    /// Step to the next preset
    Next,
    /// Switch an effect block on/off (default: on)
    Fx {
        block: BlockArg,
        state: Option<Power>,
    },
    /// Set an effect block's effect type; for amp/cab this selects the model
    Type {
        block: BlockArg,
        /// Effect type id — amp/cab models are 0–29
        id: u8,
    },
    /// Select the amp model (0–29; 30–37 are FX2 drive pedals, not amps)
    Amp { index: u8 },
    /// Select the cab/IR model (0–29)
    Cab { index: u8 },
    /// Set the effect-chain order: 8 positions, left → right
    ///
    /// Each position is a role name (`fx1 fx2 amp cab delay mod reverb eq`,
    /// `del`/`rev` allowed) or its 0–7 index. With no arguments the role
    /// table is printed. This is a live edit of the current preset.
    Chain {
        /// Exactly 8 positions, each block exactly once
        positions: Vec<String>,
    },
    /// Set the USB output volume (0–100)
    Volume { percent: u8 },
    /// Switch the tuner on/off (default: on)
    Tuner { state: Option<Power> },

    // ── assets (amp models & IRs) ─────────────────────────────────────────
    // ── connection ────────────────────────────────────────────────────────
    /// List the available MIDI ports
    List,
    /// Ping the device
    Ping,
    /// Read the current-preset state snapshot
    Snapshot,
    /// Print every frame the device emits (ctrl-c to stop)
    Monitor,

    // ── protocol probe (raw / undocumented) ───────────────────────────────
    /// Raw Control Change (decimal; 0x.. prefix for hex)
    Cc {
        /// CC number 0–127
        cc: String,
        /// CC value 0–127
        value: u8,
    },
    /// Raw slot enable (SysEx; prefer `fx` for block on/off)
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
    /// Amp model select via SysEx (same table as `amp`)
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
        Command::List => return cmd_list(),
        Command::Chain { positions } if positions.is_empty() => return print_chain_help(),
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
        Standard(Vec<u8>),
    }
    let preflight: Option<Preflight> = match &command {
        Command::Chain { positions } => {
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
        Command::Amp { index } => {
            report_model_selection(Block::Amp, *index)?;
            Some(Preflight::Standard(midicc::amp_model(conn.channel, *index)))
        }
        Command::Cab { index } => {
            report_model_selection(Block::Cab, *index)?;
            Some(Preflight::Standard(midicc::cab_model(conn.channel, *index)))
        }
        _ => None,
    };

    let mut dev = open_device(conn)?;

    match command {
        Command::List => unreachable!(),
        Command::Chain { .. } => {
            let (kind, payload) = match preflight.expect("chain preflight") {
                Preflight::Chain(k, p) => (k, p),
                _ => unreachable!(),
            };
            chain_write_cmd(&mut dev, kind, payload)
        }
        Command::Preset { index } => {
            send_standard(&mut dev, &midicc::program_change(conn.channel, index))
        }
        Command::Prev => send_standard(&mut dev, &midicc::global::prev_preset(conn.channel)),
        Command::Next => send_standard(&mut dev, &midicc::global::next_preset(conn.channel)),
        Command::Tuner { state } => send_standard(
            &mut dev,
            &midicc::global::tuner(conn.channel, state.map(Power::on).unwrap_or(true)),
        ),
        Command::Fx { block, state } => send_standard(
            &mut dev,
            &midicc::block_enable(
                conn.channel,
                block.midicc(),
                state.map(Power::on).unwrap_or(true),
            ),
        ),
        Command::Type { block, id } => {
            report_model_selection(block.midicc(), id)?;
            send_standard(&mut dev, &midicc::block_type(conn.channel, block.midicc(), id))
        }
        Command::Amp { .. } | Command::Cab { .. } => {
            match preflight.expect("model preflight") {
                Preflight::Standard(bytes) => {
                    dev.transport().set_sysex(false);
                    dev.transport().send(&bytes).map_err(|e| e.to_string())
                }
                _ => unreachable!(),
            }
        }
        Command::Volume { percent } => {
            let (k, p) = protocol::set_usb_volume(percent);
            transact(&mut dev, conn, k, p, &[])
        }
        Command::Ping => {
            let (k, p) = protocol::ping();
            transact(&mut dev, conn, k, p, &[nanocore::msg_type::PING])
        }
        Command::Snapshot => {
            let (k, p) = protocol::request_snapshot();
            transact(&mut dev, conn, k, p, &[nanocore::msg_type::SNAPSHOT])
        }
        Command::Monitor => {
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
        Command::Cc { cc, value } => {
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
            send_standard(&mut dev, &midicc::control_change(conn.channel, cc_num, value))
        }
        Command::Enable { slot, state } => {
            let (k, p) = protocol::set_enabled(slot, state.map(Power::on).unwrap_or(true));
            transact(&mut dev, conn, k, p, &[])
        }
        Command::Setparam { b3, b5, b6, value } => {
            let p = |s: &str| u8::from_str_radix(s.trim_start_matches("0x"), 16)
                .map_err(|_| format!("{s:?} must be hex"));
            let (b3, b5, b6) = (p(&b3)?, p(&b5)?, p(&b6)?);
            let (k, payload) = protocol::param_write(b3, b5, b6, value);
            transact(&mut dev, conn, k, payload, &[])
        }
        Command::Variant { index } => {
            match models_data::lookup_amp(index) {
                Some(m) => println!("variant {index} -> {} ({})", m.name, m.code),
                None => println!("variant {index} (unknown slot; device has 0-37)"),
            }
            let (k, p) = protocol::select_variant(index);
            transact(&mut dev, conn, k, p, &[])
        }
        Command::Raw { bytes } => {
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
            transact(&mut dev, conn, bytes[0], bytes[1..].to_vec(), &conn.expect.clone())
        }
        Command::Replay { bytes } => {
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
        Command::Probe { index } => probe(&mut dev, conn, index),
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

// ── chain ───────────────────────────────────────────────────────────────────

fn print_chain_help() {
    println!("Effect chain order (`chainOrder`) — the order effects are routed in.");
    println!();
    println!("  Left-to-right order, using these role names (or their 0-7 indices):");
    for (i, (cli, disp)) in protocol::CHAIN_ROLES.iter().enumerate() {
        println!("    {i}  {cli:<8} ({disp})");
    }
    println!();
    println!("  nanocore-cli chain <8 roles|indices>   write the chain order");
    println!();
    println!("  examples:");
    println!("    nanocore-cli chain delay eq fx1 fx2 amp cab mod reverb");
    println!("    nanocore-cli chain 4 7 0 1 2 3 5 6        # same thing");
    println!();
    println!("  NOTE: this order is NOT the CC block order — in the chain, DEL comes");
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
            "order must contain each block exactly once; got [{}] = {}",
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
        .map(|p| p)
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

    let steps: Vec<(&str, (u8, Vec<u8>), Vec<u8>)> = vec![
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
    if frame.kind == 0x06 {
        if let Some(ep) = protocol::parse_effect_data(&frame.payload) {
            println!("  effect data: slot {} params {:?}", ep.slot, ep.params);
        }
    }
    if frame.kind == 0x40 {
        if let Some(entries) = protocol::parse_preset_list(&frame.payload) {
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

/// Validate + describe an Amp/Cab **model** selection made over standard CC.
///
/// `CC43` (Amp) and `CC44` (Cab) are documented as "Type CC" but the OEM guide
/// prints no type table for either block and claims model selection is
/// unassigned. Hardware testing shows they *are* model selectors over the
/// 30-entry tables, and that indices `>= 30` are silently ignored — so reject
/// them here instead of letting the write disappear without effect.
///
/// For every other block this is a no-op: their "type" is a real effect-type ID
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
             {id} would be silently ignored — drive pedals 30-37 belong to FX2, not the {label} block",
            count - 1
        ));
    }
    match name {
        Some(n) => println!("{label} {id} -> {n}   (standard CC{})", block.type_cc()),
        None => println!("{label} {id} (not in embedded table; device has 0-{})", count - 1),
    }
    Ok(())
}

/// Render a chain order as `DEL FX1 FX2 ...` using the device display names.
fn render_chain(order: &[u8]) -> String {
    order
        .iter()
        .map(|i| protocol::chain_role_name(*i).unwrap_or("??").to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
