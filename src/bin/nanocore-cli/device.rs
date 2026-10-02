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
    let decoded = protocol::decode_snapshot_state(&s.payload);
    println!("Current device state:");
    // 链序优先取数据流解码（对所有长度都成立），旧的尾部锚点只作退回。
    let chain = decoded
        .as_ref()
        .and_then(|st| st.chain.clone())
        .or_else(|| protocol::parse_snapshot_chain_order(&s.payload));
    match chain {
        Some(order) => println!("  chain:         {}", render_chain(&order)),
        None => println!("  chain:         (not decodable from this snapshot)"),
    }
    match decoded {
        Some(state) => {
            println!("  preset:        #{}", state.preset_index + 1);
            println!("  preset volume: {}", state.preset_volume);
            // 记录按角色序（= b5 槽位）排列，**不是**链上的物理顺序；
            // 链序见上面那行（它给出角色 -> 链位的映射）。
            println!("  modules (record order = b5 slot = role):");
            for (i, r) in state.records.iter().enumerate() {
                let role = protocol::CHAIN_ROLES[i].0;
                let params = r
                    .params
                    .iter()
                    .map(|p| format!("{p:.3}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                let on = if r.enabled != 0 { "on " } else { "off" };
                println!(
                    "    {role:<6} {on} type={} params=[{params}]",
                    r.variant + 1
                );
            }
            println!("  note:          fx1(fx gate)/fx2(comp) `enabled` in the snapshot");
            println!("                reflects live DSP state (signal-dependent);");
            println!("                fx8 params are runtime values, not factory ones.");
        }
        None => {
            // 无法结构化解码（例如 BLE 抓包形态）时退回经验字段。
            if s.payload.len() > 6 {
                println!(
                    "  preset:   #{} (empirical — payload[6])",
                    s.payload[6] + 1
                );
            }
        }
    }
    println!(
        "  data:          snapshot {} bytes, {} raw float candidate(s)",
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
