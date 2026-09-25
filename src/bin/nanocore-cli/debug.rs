//! `debug` 命令：底层 MIDI/协议工具（原始帧、SysEx、监听、重放）。

use std::io::Write;

use nanocore::midicc;
use nanocore::models_data;
use nanocore::protocol;
use nanocore::{Frame, Transport, msg_type};

use crate::cli::{Conn, DebugCmd, Power};
use crate::common::{Dev, describe, hex, parse_hex, send_standard, transact};
use crate::probe::probe;

pub fn cmd_debug(dev: &mut Dev, conn: &Conn, cmd: DebugCmd) -> Result<(), String> {
    match cmd {
        DebugCmd::Ping => {
            let (k, p) = protocol::ping();
            transact(dev, conn, k, p, &[msg_type::PING])
        }
        DebugCmd::Snapshot => {
            let (k, p) = protocol::request_snapshot();
            transact(dev, conn, k, p, &[msg_type::SNAPSHOT])
        }
        DebugCmd::Monitor => {
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
                return Err(format!("CC number must be 0-127, got {cc_num}"));
            }
            if value > 127 {
                return Err(format!("CC value must be 0-127, got {value}"));
            }
            send_standard(dev, &midicc::control_change(conn.channel, cc_num, value))
        }
        DebugCmd::Enable { slot, state } => {
            let (k, p) = protocol::set_enabled(slot, state.map(Power::on).unwrap_or(true));
            transact(dev, conn, k, p, &[])
        }
        DebugCmd::Setparam { b3, b5, b6, value } => {
            let p = |s: &str| {
                u8::from_str_radix(s.trim_start_matches("0x"), 16)
                    .map_err(|_| format!("{s:?} must be hex"))
            };
            let (b3, b5, b6) = (p(&b3)?, p(&b5)?, p(&b6)?);
            let (k, payload) = protocol::param_write(b3, b5, b6, value);
            transact(dev, conn, k, payload, &[])
        }
        DebugCmd::Variant { index } => {
            match models_data::lookup_amp(index) {
                Some(m) => println!("variant {index} -> {} ({})", m.name, m.code),
                None => println!("variant {index} (unknown slot; device has 0-37)"),
            }
            let (k, p) = protocol::select_variant(index);
            transact(dev, conn, k, p, &[])
        }
        DebugCmd::Raw { bytes } => {
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
            transact(
                dev,
                conn,
                bytes[0],
                bytes[1..].to_vec(),
                &conn.expect.clone(),
            )
        }
        DebugCmd::Replay { bytes } => {
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
        DebugCmd::Probe { index } => probe(dev, conn, index),
    }
}
