//! `chain` commands: write / show the effect-chain order, plus the `show`
//! device-state snapshot (which today decodes the chainOrder area).

use nanocore::client::Error;
use nanocore::protocol;

use crate::common::{describe, hex, render_chain, Dev};

pub fn print_chain_help() {
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
pub fn parse_chain_positions(tokens: &[String]) -> Result<Vec<u8>, String> {
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
pub fn chain_write_cmd(
    dev: &mut Dev,
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
pub fn cmd_chain_show(dev: &mut Dev) -> Result<(), String> {
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
pub fn cmd_show(dev: &mut Dev) -> Result<(), String> {
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
