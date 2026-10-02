//! `debug` 命令：底层 MIDI/协议工具（原始帧、SysEx、监听、重放）。

use std::io::Write;

use nanocore::midicc;
use nanocore::models_data;
use nanocore::protocol;
use nanocore::{Frame, Transport, msg_type};

use crate::cli::{Conn, DebugCmd};
use crate::common::{
    Failure, describe, hex, open_device, parse_hex, render_snapshot, send_standard, transact,
    transact_reply,
};
use crate::probe::probe;

/// 各分支都先完成自己的输入解析，再打开端口——手误不该触碰 MIDI。
pub fn cmd_debug(cmd: DebugCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        DebugCmd::Ping => {
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::ping();
            transact(&mut dev, conn, k, p, &[msg_type::PING])
        }
        DebugCmd::Snapshot { decode } => {
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::request_snapshot();
            let reply = transact_reply(&mut dev, conn, k, p, &[msg_type::SNAPSHOT])?;
            if decode {
                let Some(reply) = reply else {
                    return Err(Failure::error(
                        "no reply to decode — drop --no-wait, the snapshot needs its answer",
                    ));
                };
                print!("{}", render_snapshot(&reply.payload, ""));
            }
            Ok(())
        }
        DebugCmd::Monitor => {
            let mut dev = open_device(conn)?;
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
            // CC 编号默认使用**十进制**，便于直接从 OEM MIDI CC 参考中照抄；
            // `0x..` 前缀选择十六进制。
            let cc_num = match cc.strip_prefix("0x") {
                Some(hex) => {
                    u8::from_str_radix(hex, 16).map_err(|_| format!("bad hex CC number {cc:?}"))?
                }
                None => cc.parse().map_err(|_| {
                    format!("CC number must be decimal 0-127 ({cc:?}); use 0x.. for hex")
                })?,
            };
            if cc_num > 127 {
                return Err(Failure::error(format!(
                    "CC number must be 0-127, got {cc_num}"
                )));
            }
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::control_change(conn.channel, cc_num, value))
        }
        DebugCmd::Enable { slot, state } => {
            let mut dev = open_device(conn)?;
            // 模块编号 1-8 → wire b5 槽位 0-7。
            let (k, p) = protocol::set_enabled(slot - 1, state.on());
            transact(&mut dev, conn, k, p, &[])
        }
        DebugCmd::Setparam { b5, b6, value } => {
            let p = |s: &str| {
                u8::from_str_radix(s.trim_start_matches("0x"), 16)
                    .map_err(|_| format!("{s:?} must be hex"))
            };
            let (b5, b6) = (p(&b5)?, p(&b6)?);
            let mut dev = open_device(conn)?;
            let (k, payload) = protocol::param_write(b5, b6, value);
            transact(&mut dev, conn, k, payload, &[])
        }
        DebugCmd::Variant { index } => {
            // 槽位编号 1-38 → wire 0-37。
            let wire = index - 1;
            match models_data::lookup_amp(wire) {
                Some(m) => println!("variant {index} -> {} ({})", m.name, m.code),
                None => println!("variant {index} (unknown slot; device has 1-38)"),
            }
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::select_variant(wire);
            transact(&mut dev, conn, k, p, &[])
        }
        DebugCmd::Raw { bytes } => {
            let bytes = parse_hex(&bytes)?;
            if bytes.len() < 2 {
                return Err(Failure::error("raw needs at least a message type"));
            }
            let mut dev = open_device(conn)?;
            transact(
                &mut dev,
                conn,
                bytes[0],
                bytes[1..].to_vec(),
                &conn.expect.clone(),
            )
        }
        DebugCmd::Replay { bytes } => {
            let bytes = parse_hex(&bytes)?;
            let mut dev = open_device(conn)?;
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
        DebugCmd::Probe { index } => {
            let mut dev = open_device(conn)?;
            // 预设编号 1-64 → wire 0-63；缺省仍是 wire 0（#1）。
            probe(&mut dev, conn, index.map(|i| i - 1))
        }
    }
}
