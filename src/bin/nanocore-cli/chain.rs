//! `chain` 命令：写入 / 显示效果链序，外加 `show` 设备状态快照
//! （目前仅解码 chainOrder 区域）。

use nanocore::client::Error;
use nanocore::protocol;

use crate::common::{Dev, describe, hex, render_chain};

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

/// 解析并校验 8 个链位置。
///
/// 接受角色名（`delay`、`del`、`mod` 等）或 0-7 索引，可任意混用。
/// 拒绝任何不是 `0..8` 排列的输入，确保错误的链序永远不会到达设备。
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
            Ok(v) => {
                return Err(format!(
                    "position {}: index {v} out of range (0-{})",
                    i + 1,
                    n - 1
                ));
            }
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
            order
                .iter()
                .map(|v| v.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            render_chain(&order)
        ));
    }
    Ok(order)
}

/// 写入一个已预校验的链序，并通过设备自身的回显进行确认。
pub fn chain_write_cmd(dev: &mut Dev, kind: u8, payload: Vec<u8>) -> Result<(), String> {
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

/// 从设备快照中读取当前链序。
///
/// `chainOrder` 尾部是少数已解码的快照区域之一，因此当前可用；
/// `show` 的其余部分仍标注 (TBD)。
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

/// 以可读形式显示当前设备状态快照。
///
/// 尽力而为：仅打印已解码的部分（chainOrder、载荷形状）；
/// 其余标注 (TBD)，直至 `0x63` 快照解码完成。
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
