//! `debug` 命令：底层 MIDI/协议工具（原始帧、SysEx、监听、重放）。

use std::io::Write;

use nanocore::midicc;
use nanocore::models_data;
use nanocore::protocol;
use nanocore::{Frame, Transport, msg_type};

use crate::cli::{Conn, DebugCmd, ProtocolKnobs, parse_hex_byte};
use crate::common::{
    Failure, describe, hex, open_device, parse_hex, send_standard, transact, transact_reply,
};
use crate::probe::probe;

/// 把协议旋钮落到连接选项的副本上。
///
/// 旋钮挂在每个叶子变体上（`raw` / `replay` 恰恰只有叶子级才拿得到），
/// 所以这里克隆一份 `Conn` 再应用，而不是就地改调用方持有的那个——
/// `Conn` 的其余字段（端口、通道、超时）与旋钮无关，克隆成本可忽略。
fn conn_with(conn: &Conn, knobs: &ProtocolKnobs) -> Result<Conn, Failure> {
    let mut c = conn.clone();
    c.apply_knobs(knobs).map_err(Failure::error)?;
    Ok(c)
}

/// 各分支都先完成自己的输入解析，再打开端口——手误不该触碰 MIDI。
pub fn cmd_debug(cmd: DebugCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        DebugCmd::Ping { knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::ping();
            transact(&mut dev, conn, k, p, &[msg_type::PING])
        }
        DebugCmd::Snapshot { knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            // 端口回显后的分节线由 open_device 统一打（见 common.rs）。
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::request_snapshot();
            // describe 收到快照帧就渲染完整展开版（unpacked 流 + 字段表），
            // 所以这里不再另行输出。
            let reply = transact_reply(&mut dev, conn, k, p, &[msg_type::SNAPSHOT])?;
            if reply.is_none() {
                return Err(Failure::error(
                    "no reply — drop --no-wait, the snapshot needs its answer",
                ));
            }
            Ok(())
        }
        DebugCmd::Monitor { knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let mut dev = open_device(conn)?;
            // 它只被动打印设备**主动发出**的帧（Event / 别人的请求的回包），
            // 自己不发任何请求——所以设备空闲时这里本来就该是安静的。
            println!("monitoring... (ctrl-c to stop)");
            let mut frames = 0usize;
            loop {
                let raw = dev.transport().receive().map_err(|e| e.to_string())?;
                for f in Frame::split_stream(&raw) {
                    // 一次 receive 可能带回几十帧（如 0x36 型号表），一次 dump
                    // 一行单帧字节；那行 `raw[..]` 装的正是同一批字节，重复一遍
                    // 只会让边界更难找，故不打印。
                    describe(&f, "", Some(frames + 1));
                    frames += 1;
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
                // 逐帧观察的工具：重定向到文件时 stdout 是块缓冲，不刷就得等
                // 攒满 4KB 才看到东西，实时性就没了。
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
        DebugCmd::Enable { slot, state, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let mut dev = open_device(conn)?;
            // 模块编号 1-8 → wire b5 槽位 0-7。
            let (k, p) = protocol::set_enabled(slot - 1, state.on());
            transact(&mut dev, conn, k, p, &[])
        }
        DebugCmd::Presets { start, count, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let mut dev = open_device(conn)?;
            // 槽位号线上 0 起始。
            let (k, p) = protocol::query_preset_list(start - 1, count);
            transact_reply(&mut dev, conn, k, p, &[msg_type::PRESET_LIST]).map(|_| ())
        }
        DebugCmd::Settings { knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::query_settings();
            transact_reply(&mut dev, conn, k, p, &[msg_type::SETTINGS]).map(|_| ())
        }
        DebugCmd::Setfield { field, value, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let f = parse_hex_byte(&field).map_err(|e| Failure::error(e.to_string()))?;
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::set_setting(f, value);
            transact(&mut dev, conn, k, p, &[])
        }
        DebugCmd::Model { index, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::model_query(index);
            transact_reply(&mut dev, conn, k, p, &[msg_type::MODEL_LIST]).map(|_| ())
        }
        DebugCmd::Setfield6d { field, value, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let f = parse_hex_byte(&field).map_err(|e| Failure::error(e.to_string()))?;
            let mut dev = open_device(conn)?;
            let (k, p) = protocol::set_field(f, value);
            transact(&mut dev, conn, k, p, &[])
        }
        DebugCmd::Rename { slot, name, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let encoded = protocol::encode_preset_name_field(&name).map_err(Failure::error)?;
            let wire = slot - 1;
            let mut dev = open_device(conn)?;
            // 与官方抓包一致：先选槽，等设备自发切槽事件，再写名字区。
            let (k, p) = protocol::select_preset(wire);
            let (tx, rx) = dev
                .transact_with_flags(conn.flags, k, p, &[])
                .map_err(|e| Failure::error(e.to_string()))?;
            describe(&tx, "", None);
            describe(&rx, "", None);
            // 切槽事件是设备的**附带**通知，不是写名字的前置条件：槽位已是
            // 当前预设时它不会发。因此等不到就继续——`preset rename` 需要它
            // 只因为后面还要读回校验，这里不需要。
            if let Ok(f) = dev.wait_for_preset_switch_event(wire, 2_000) {
                describe(&f, "", None);
            }
            let (k, p) = protocol::set_name_field(&encoded);
            transact(&mut dev, conn, k, p, &[])
        }
        DebugCmd::Setparam { b5, b6, value, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let p = |s: &str| parse_hex_byte(s).map_err(|_| format!("{s:?} must be hex"));
            let (b5, b6) = (p(&b5)?, p(&b6)?);
            let mut dev = open_device(conn)?;
            let (k, payload) = protocol::param_write(b5, b6, value);
            transact(&mut dev, conn, k, payload, &[])
        }
        DebugCmd::Variant { index, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
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
        DebugCmd::Raw { bytes, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
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
        DebugCmd::Replay { bytes, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
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
                        describe(&f, "", None);
                        got += 1;
                    }
                }
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            println!("{got} frame(s) in 2s");
            Ok(())
        }
        DebugCmd::Probe { index, knobs } => {
            let conn = &conn_with(conn, &knobs)?;
            let mut dev = open_device(conn)?;
            // 预设编号 1-64 → wire 0-63；缺省仍是 wire 0（#1）。
            probe(&mut dev, conn, index.map(|i| i - 1))
        }
    }
}
