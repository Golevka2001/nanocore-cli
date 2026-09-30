//! 设备级命令：状态快照（`show`）、USB 输出音量、调音表。

use nanocore::midicc;
use nanocore::protocol;

use crate::cli::{Conn, Power};
use crate::common::{Failure, open_device, render_chain, send_standard, transact};

/// 以可读形式显示当前设备状态快照。
///
/// 快照解码于 2026-09-30 定案（40 组出厂基线验证，见
/// `docs/protocol/snapshot.md`）：载荷从 `[3]` 起是连续 `[mask][7 data]`
/// 打包流，数据流 = 前导 + 8 条链位记录 + 链尾。
pub fn cmd_show(conn: &Conn) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
    let s = dev.read_snapshot().map_err(|e| e.to_string())?;
    println!("Current device state:");
    match protocol::parse_snapshot_chain_order(&s.payload) {
        Some(order) => println!("  chain:    {}", render_chain(&order)),
        None => println!("  chain:    (not decodable from this snapshot)"),
    }
    match protocol::decode_snapshot_state(&s.payload) {
        Some(state) => {
            println!("  preset:   #{}", state.preset_index);
            println!("  volume:   {} (presetVolume)", state.preset_volume);
            println!("  chain positions:");
            for (i, r) in state.records.iter().enumerate() {
                let params = r
                    .params
                    .iter()
                    .map(|p| format!("{p:.3}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let state = if r.enabled != 0 { "on" } else { "off" };
                println!(
                    "    [{i}] {state} variant={} params=[{params}]",
                    r.variant
                );
            }
            println!("  note:     fx1(fx gate)/fx2(comp) `enabled` in the snapshot");
            println!("            reflects live DSP state (signal-dependent);");
            println!("            fx8 params are runtime values, not factory ones.");
        }
        None => {
            // 无法结构化解码（例如 BLE 抓包形态）时退回经验字段。
            if s.payload.len() > 6 {
                println!(
                    "  preset:   #{} (empirical — payload[6])",
                    s.payload[6]
                );
            }
            println!("  data:     snapshot {} bytes (structured decode unavailable)", s.payload.len());
        }
    }
    println!(
        "  data:     snapshot {} bytes, {} raw float candidate(s)",
        s.payload.len(),
        protocol::scan_snapshot_floats(&s.payload).len()
    );
    Ok(())
}

/// 设置 USB 输出音量（0–100，范围由 clap 保证）。
pub fn cmd_volume(percent: u8, conn: &Conn) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
    let (k, p) = protocol::set_usb_volume(percent);
    transact(&mut dev, conn, k, p, &[])
}

/// 开关调音表。读取当前状态需要快照解码，尚未实现。
pub fn cmd_tuner(state: Option<Power>, conn: &Conn) -> Result<(), Failure> {
    let Some(state) = state else {
        return Err(Failure::error(
            "tuner state read is not implemented yet (needs snapshot decode); \
             pass `tuner on` or `tuner off`",
        ));
    };
    let mut dev = open_device(conn)?;
    send_standard(
        &mut dev,
        &midicc::global::tuner(conn.channel, state.on()),
    )
}
