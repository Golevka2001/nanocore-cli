//! 设备级命令：状态快照（`show`）、USB 输出音量、调音表。

use nanocore::midicc;
use nanocore::protocol;

use crate::cli::{Conn, Power};
use crate::common::{Failure, open_device, send_standard, transact};

/// 以可读形式显示当前设备状态快照。
///
/// 快照解码于 2026-09-30 定案（40 组出厂基线验证，见
/// `docs/protocol/snapshot.md`）：载荷从 `[3]` 起是连续 `[mask][7 data]`
/// 打包流，数据流 = 前导 + 8 条链位记录 + 链尾。
///
/// 渲染由 [`crate::common::render_snapshot`] 承担，`debug snapshot --decode`
/// 复用同一份实现。
pub fn cmd_show(conn: &Conn) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
    let s = dev.read_snapshot().map_err(|e| e.to_string())?;
    println!("Current device state:");
    print!("{}", crate::common::render_snapshot(&s.payload, "  "));
    Ok(())
}

/// 设置 USB 输出音量（0–100，范围由 clap 保证）。
pub fn cmd_volume(percent: u8, conn: &Conn) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
    let (k, p) = protocol::set_usb_volume(percent);
    transact(&mut dev, conn, k, p, &[])
}

/// 开关调音表。
///
/// **读取状态不可行**（2026-10-01 真机验证，不是"还没做"）：
/// OEM 指南里 CC80 是只写的，App 侧 `global-tuner` 控制同样是 CC80；
/// `0x65` 设置查询（13 字节载荷，index 9 = USB 音量）在 `tuner on/off`
/// 前后逐字节不变，`0x63` 快照同样不变，且设备从不回显写入的 CC。
/// 协议里没有调音表状态查询这条消息。
pub fn cmd_tuner(state: Option<Power>, conn: &Conn) -> Result<(), Failure> {
    let Some(state) = state else {
        return Err(Failure::error(
            "the tuner state cannot be read — the protocol has no tuner query:\n  \
             CC80 is write-only (OEM guide), `tuner on/off` leaves both the 0x65\n  \
             settings reply and the 0x63 snapshot byte-identical, and the device\n  \
             never echoes written CCs. Pass `tuner on` or `tuner off`.",
        ));
    };
    let mut dev = open_device(conn)?;
    send_standard(
        &mut dev,
        &midicc::global::tuner(conn.channel, state.on()),
    )
}
