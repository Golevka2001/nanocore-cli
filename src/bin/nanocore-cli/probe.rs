//! `debug probe`：协议笔记中的预设切换探测流程。

use nanocore::protocol;
use nanocore::{Frame, Transport, msg_type};

use crate::cli::Conn;
use crate::common::{Dev, describe, hex};

/// 按协议笔记中的记载执行预设切换流程：
/// 选择预设 -> 读取快照（0x63）-> 查询（0x41），然后转储设备在数秒内发出的每一帧。
pub fn probe(dev: &mut Dev, conn: &Conn, index: Option<u8>) -> Result<(), String> {
    let idx = index.unwrap_or(0);

    /// 一个探测步骤：标签、（消息类型, 载荷）、期望的应答类型。
    type Step<'a> = (&'a str, (u8, Vec<u8>), Vec<u8>);
    let steps: Vec<Step> = vec![
        ("1 select preset", protocol::select_preset(idx), vec![]),
        (
            "2 read snapshot",
            protocol::request_snapshot(),
            vec![msg_type::SNAPSHOT],
        ),
        (
            "3 query",
            (msg_type::QUERY, vec![0x00, 0x04, 0x00, 0x08]),
            vec![],
        ),
        (
            "4 preset list",
            protocol::query_preset_list(0, 8),
            vec![msg_type::PRESET_LIST],
        ),
    ];

    for (label, (kind, payload), expect) in steps {
        println!("--- step {label} ---");
        println!(
            "tx: type=0x{kind:02x} payload[{}] {}",
            payload.len(),
            hex(&payload)
        );
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
