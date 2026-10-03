//! `chain` 命令：写入 / 显示效果链序。

use nanocore::client::Error;
use nanocore::protocol;

use crate::cli::{ChainCmd, Conn};
use crate::common::{Dev, Failure, describe, hex, open_device, render_chain};

pub fn print_chain_help() {
    println!("Effect chain order (`chainOrder`) — the order effects are routed in.");
    println!();
    println!("  Left-to-right order, using these role names (or their 1-8 numbers):");
    for (i, (cli, disp)) in protocol::CHAIN_ROLES.iter().enumerate() {
        println!("    {}  {cli:<8} ({disp})", i + 1);
    }
    println!();
    println!("  nanocore-cli chain set <8 roles|numbers>   write the chain order");
    println!("  nanocore-cli chain show                    read the current order");
    println!();
    println!("  examples:");
    println!("    nanocore-cli chain set delay eq fx1 fx2 amp cab mod reverb");
    println!("    nanocore-cli chain set 5 8 1 2 3 4 6 7          # same thing");
    println!();
    println!("  NOTE: this order is NOT the CC module order — in the chain, DEL comes");
    println!("  before MOD (CC25 is Mod, CC26 is Delay). This is a live edit to the");
    println!("  current preset; reload a factory preset afterwards to get back.");
}

/// `chain` 入口：写入或读取效果链序。
pub fn cmd_chain(cmd: ChainCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        ChainCmd::Set { positions } => cmd_chain_set(&positions, conn),
        ChainCmd::Show => cmd_chain_show(conn),
    }
}

/// 写入链序：先校验，后打开端口——坏链序永远不会到达设备。
fn cmd_chain_set(tokens: &[String], conn: &Conn) -> Result<(), Failure> {
    let order = parse_chain_positions(tokens)?;
    let mut dev = open_device(conn)?;

    let (kind, payload) = protocol::chain_write(&order)
        .ok_or_else(|| Failure::error("internal: chain_write rejected a valid order"))?;
    println!("chain (left -> right): {}", render_chain(&order));
    println!(
        "                    as: [{}]",
        order
            .iter()
            .map(|v| (v + 1).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "tx: type=0x{kind:02x} payload[{}] {}",
        payload.len(),
        hex(&payload)
    );
    chain_write(&mut dev, kind, payload)
}

/// 解析并校验 8 个链位置。
///
/// 接受角色名（`delay`、`del`、`mod` 等）或 1-8 的编号，可任意混用。
/// 拒绝任何不是 `1..8` 排列的输入，确保错误的链序永远不会到达设备。
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
            // 用户编号 1-8 → wire 角色下标 0-7。
            Ok(v) if v >= 1 && (v as usize) <= n => v - 1,
            Ok(v) => {
                return Err(format!(
                    "position {}: number {v} out of range (1-{})",
                    i + 1,
                    n
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
                .map(|v| (v + 1).to_string())
                .collect::<Vec<_>>()
                .join(", "),
            render_chain(&order)
        ));
    }
    Ok(order)
}

/// 写入一个已预校验的链序，并通过设备自身的回显进行确认。
fn chain_write(dev: &mut Dev, kind: u8, payload: Vec<u8>) -> Result<(), Failure> {
    let order = protocol::parse_chain_write(&payload)
        .ok_or_else(|| Failure::error("internal: device frame does not decode back to the chain"))?;

    let (tx, ack) = dev
        .transact_with_flags(0x00, kind, payload, &[])
        .map_err(|e| match e {
            Error::Timeout { attempts, elapsed } => {
                format!("no reply to the chain write after {attempts} attempt(s) in {elapsed:?}")
            }
            Error::Transport(t) => t.to_string(),
        })?;
    describe(&tx, "");
    describe(&ack, "");

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
/// `chainOrder` 尾部是少数已解码的快照区域之一，因此当前可用。
fn cmd_chain_show(conn: &Conn) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
    let s = dev.read_snapshot().map_err(|e| e.to_string())?;
    match protocol::parse_snapshot_chain_order(&s.payload) {
        Some(order) => {
            println!("chain (left -> right): {}", render_chain(&order));
            Ok(())
        }
        None => Err(Failure::error(
            "snapshot did not contain a decodable chainOrder",
        )),
    }
}
