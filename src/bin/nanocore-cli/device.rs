//! 设备级命令：状态快照（`show`）、USB 输出音量、调音表。

use nanocore::midicc;
use nanocore::protocol;

use crate::cli::{Conn, Power};
use crate::common::{Failure, open_device, send_standard, transact};

/// 以可读形式显示当前设备状态快照。
///
/// 载荷从 `[3]` 起是连续 `[mask][7 data]`
/// 打包流，数据流 = 前导 + 8 条链位记录 + 链尾。
///
/// 另有两处补充查询各 1 次往返，缺了不影响其余字段：预设名（`0x40`，
/// 只查当前槽位所在的那一批）与设置页（`0x65`）。
///
/// `--full` 加第三处：amp / cab 型号。它们**不在** `0x63` 里（那两个块的
/// `variant` 恒为 0），要各查一张表 —— amp 走 `0x36`（38 项）、cab 走 `0x56`
/// （30 项 IR，且必须先走工厂恢复的前置序列才应答），合计约 40 次往返。
/// 查不到就整段不印型号，不让 `show` 因此失败。
///
/// 渲染由 [`crate::common::render_snapshot`] 与
/// [`crate::settings::render_settings`] 承担，`debug snapshot` 与
/// `settings show` 分别复用同一份实现。
pub fn cmd_show(full: bool, conn: &Conn) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
    let s = dev.read_snapshot().map_err(|e| e.to_string())?;
    // 名字是补充信息：查不到就让 `preset` 行只显示编号，不因此让整个命令失败。
    let name = protocol::decode_snapshot_state(&s.payload).and_then(|st| {
        dev.read_preset_name(st.preset_index)
            .ok()
            .flatten()
            .filter(|e| e.named)
            .map(|e| e.name)
    });
    // amp 型号（`0x36` 的 active 槽）与 cab 型号（`0x56` 的 active 槽）。两者都
    // 查不到就传 `None` —— 上一版把恒 0 的 `variant` 印成 `type=1` 就是「查不到
    // 就编一个」的结果。
    //
    // 印 `ModelInfo::name`（`Bogner XTC` / `VOX AC30 2x12`）而非 `code`：
    // 设备侧存的是 8 字符定宽码（`BogXTC1` / `AC3021`），不完整也不可读。
    // 槽号在表外（用户导入的 NAM 之类）时印设备返回的码。
    let (amp, cab) = if full {
        (
            dev.read_active_amp_slot()
                .ok()
                .flatten()
                .map(|idx| match nanocore::models_data::lookup_amp(idx) {
                    Some(m) => m.name.to_string(),
                    None => format!("slot {idx}"),
                }),
            dev.read_active_ir_slot()
                .ok()
                .flatten()
                .map(|s| match nanocore::models_data::lookup_cab(s.index) {
                    Some(m) => m.name.to_string(),
                    None => s.name_str(),
                }),
        )
    } else {
        (None, None)
    };
    // 分节标题而非一句总起——`show` 现在有三段（快照 / 名称来源 / 设置页），
    // 顶格说明一句反而多余。各节自带标题，读者扫一眼就知道边界在哪。
    print!(
        "{}",
        crate::common::render_snapshot(
            &s.payload,
            "",
            name.as_deref(),
            (amp.as_deref(), cab.as_deref()),
            crate::common::Bytes::None
        )
    );
    // 同理：设置页读不到就整段略过，不让 `show` 因此失败。
    if let Ok(st) = dev.read_settings() {
        // 两节字段名都是「名字 = 值」，空行分不开边界，用横线。
        crate::common::divider();
        println!("settings:");
        println!("{}", crate::settings::render_settings(&st, "  "));
    }
    if !full {
        eprintln!("hint: `show --full` adds the amp / cab model (about 40 round trips)");
    }
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
/// **读取状态不可行**（不是"还没做"）：
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
