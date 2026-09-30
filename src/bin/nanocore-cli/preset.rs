//! `preset` 命令：选择 / 步进 / 列出预设槽位 / 恢复出厂预设。

use nanocore::midicc;

use crate::cli::{Conn, PresetCmd};
use crate::common::{Failure, open_device, send_standard};

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
