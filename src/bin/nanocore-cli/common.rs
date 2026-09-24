//! Shared plumbing: port listing, device open, hex dumps, frame description,
//! transactions and chain rendering.

use nanocore::client::{Error, Nanocore, RetryConfig};
use nanocore::midi::{self, MidiTransport};
use nanocore::protocol;
use nanocore::{Frame, Transport};

use crate::cli::Conn;

/// An opened device connection over the real MIDI transport.
pub type Dev = Nanocore<MidiTransport>;

#[allow(clippy::needless_pass_by_value)]
pub fn cmd_list() {
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

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")
}

pub fn describe(frame: &Frame) {
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

pub fn open_device(conn: &Conn) -> Result<Dev, String> {
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
pub fn send_standard(dev: &mut Dev, bytes: &[u8]) -> Result<(), String> {
    dev.transport().set_sysex(false);
    dev.transport().send(bytes).map_err(|e| e.to_string())
}

pub fn parse_hex(parts: &[String]) -> Result<Vec<u8>, String> {
    parts
        .iter()
        .map(|h| {
            u8::from_str_radix(h.trim().trim_start_matches("0x"), 16)
                .map_err(|_| format!("bad hex byte {h:?}"))
        })
        .collect()
}

/// Generic SysEx transact: send, wait for a (type-filtered) reply, describe it.
pub fn transact(
    dev: &mut Dev,
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
pub fn render_chain(order: &[u8]) -> String {
    order
        .iter()
        .map(|i| protocol::chain_role_name(*i).unwrap_or("??").to_string())
        .collect::<Vec<_>>()
        .join(" ")
}
