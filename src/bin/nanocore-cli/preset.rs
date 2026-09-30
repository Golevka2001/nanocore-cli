//! `preset` 命令：选择 / 步进 / 列出预设槽位 / 恢复出厂预设。

use nanocore::midicc;

use crate::cli::{Conn, PresetCmd};
use crate::common::{Failure, open_device, send_standard, transact};

pub fn cmd_preset(cmd: PresetCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        // 读取活动预设需要快照解码——在打开端口之前就拒绝。
        PresetCmd::Show => Err(Failure::error(
            "the active preset is not decoded yet (full snapshot decode pending). \
             Try `preset list` for the slot names.",
        )),
        PresetCmd::List => {
            let mut dev = open_device(conn)?;
            let entries = dev.read_preset_names()?;
            for e in &entries {
                println!(
                    "  #{:<2} {:<12} {}",
                    e.slot,
                    e.name,
                    if e.named { "" } else { "(empty)" }
                );
            }
            Ok(())
        }
        PresetCmd::Rename { slot, name } => {
            let field = nanocore::protocol::encode_preset_name_field(&name)
                .map_err(Failure::error)?;
            let mut dev = open_device(conn)?;
            // 与官方抓包一致：field 0x09 选槽 → 等设备自发切槽事件 → 写名字区。
            // （官方在选槽与改名之间有 ~6 个 PING 的等待；等事件比等时间更可靠。）
            let (k, p) = nanocore::protocol::select_preset(slot);
            transact(&mut dev, conn, k, p, &[])?;
            match dev.wait_for_preset_switch_event(slot, 5_000) {
                Ok(f) => println!("switch event: {}", crate::common::hex(&f.payload)),
                Err(e) => {
                    let traffic = dev.drain_frames(3_000);
                    return Err(Failure::error(format!(
                        "{e}; device traffic during 3s drain: {traffic}"
                    )));
                }
            }
            let (k, p) = nanocore::protocol::set_name_field(&field);
            transact(&mut dev, conn, k, p, &[])?;

            // flash 落盘异步生效，稍候验证。
            std::thread::sleep(std::time::Duration::from_secs(2));
            let entries = dev.read_preset_names()?;
            let got = entries
                .iter()
                .find(|e| e.slot == slot)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            let want: String = name.chars().filter(|&c| c != '\0').collect();
            println!("slot #{slot}: {got:?} (expected {want:?})");
            if got == want {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "rename did not stick: got {got:?}, expected {want:?}"
                )))
            }
        }
        PresetCmd::Set { index } => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::program_change(conn.channel, index))
        }
        PresetCmd::Next => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::global::next_preset(conn.channel))
        }
        PresetCmd::Prev => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::global::prev_preset(conn.channel))
        }
        PresetCmd::RestoreFactory => {
            // 语义与官方 App 的"仅 DSP 恢复"一致：40 组出厂预设整体重写。
            // 会话开场（0x49）会清空预设表，随后流程将其完整重写。
            eprintln!(
                "warning: experimental — replays the official DSP-only factory restore\n  \
                 (preamble enumeration + 40 preset groups). Slots 0-39 will be\n  \
                 overwritten with factory names and contents. Do not disconnect the\n  \
                 device until the command prints its final verdict."
            );
            let mut dev = open_device(conn)?;
            dev.restore_factory_presets(|msg| {
                eprint!("\r{msg}");
            })
            .map_err(Failure::error)?;
            eprintln!();
            println!("OK: official DSP restore sequence replayed.");

            // flash 落盘异步生效（观察到提交后 ~2.4 s 的过期读），稍候再验证名表。
            std::thread::sleep(std::time::Duration::from_secs(3));
            let entries = dev.read_preset_names()?;
            let named = entries.iter().filter(|e| e.named).count();
            println!("preset table now has {named}/40 factory-named slots.");
            if named == 40 {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "expected 40 named slots, got {named}"
                )))
            }
        }
    }
}
