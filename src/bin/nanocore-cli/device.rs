//! 设备级命令：状态快照（`show`）、USB 输出音量、调音表。

use nanocore::midicc;
use nanocore::protocol;

use crate::cli::{Conn, Power};
use crate::common::{Failure, open_device, render_chain, send_standard, transact};

/// 以可读形式显示当前设备状态快照。
///
/// 尽力而为：仅打印已解码的部分（chainOrder、载荷形状）；
/// 其余标注 (TBD)，直至 `0x63` 快照解码完成。
pub fn cmd_show(conn: &Conn) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
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
