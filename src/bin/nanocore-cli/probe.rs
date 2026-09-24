//! `debug probe`: the preset-switch discovery flow from the protocol notes.

use nanocore::protocol;
use nanocore::{msg_type, Frame, Transport};

use crate::cli::Conn;
use crate::common::{describe, hex, Dev};

/// Run the preset-switch flow documented in the protocol notes:
/// select preset -> read snapshot (0x63) -> query (0x41), then dump every
/// frame the device emits for a few seconds.
pub fn probe(
    dev: &mut Dev,
    conn: &Conn,
    index: Option<u8>,
) -> Result<(), String> {
    let idx = index.unwrap_or(0);

    /// One discovery step: label, (msg type, payload), expected reply types.
    type Step<'a> = (&'a str, (u8, Vec<u8>), Vec<u8>);
    let steps: Vec<Step> = vec![
        ("1 select preset", protocol::select_preset(idx), vec![]),
        ("2 read snapshot", protocol::request_snapshot(), vec![msg_type::SNAPSHOT]),
        ("3 query", (msg_type::QUERY, vec![0x00, 0x04, 0x00, 0x08]), vec![]),
        ("4 preset list", protocol::query_preset_list(0, 8), vec![msg_type::PRESET_LIST]),
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
