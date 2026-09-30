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
        PresetCmd::RestoreFactory { resume } => {
            eprintln!(
                "warning: EXPERIMENTAL / KNOWN BROKEN — do not use on a real device\n  \
                 until root-caused. Observed on fw 1.04 (2026-09-30):\n  \
                 - the write session (0x49) wipes the whole preset table on open;\n  \
                 - a session accepts only ~22-24 preset commits, then refuses;\n  \
                 - preset writes WITHOUT a session ack normally but corrupt the\n  \
                   device (model table, IR slots) — recovery only via the official\n  \
                   app's factory reset.\n  \
                 The sequence itself is verified byte-exact against the official\n  \
                 captures; the missing piece is whatever lets the official app do\n  \
                 40 commits in one session (asset-phase priming is suspected)."
            );
            let mut dev = open_device(conn)?;

            // 断点续传：名表里已有非占位名的槽位视为已恢复，跳过。
            let mut skip: Vec<usize> = Vec::new();
            if resume {
                let entries = dev.read_preset_names()?;
                skip.extend(entries.iter().filter(|e| e.named).map(|e| e.slot as usize));
                if skip.is_empty() {
                    println!("resume: no named slots found — restoring everything.");
                } else {
                    println!(
                        "resume: skipping {} already-restored slot(s): {:?}",
                        skip.len(),
                        skip
                    );
                }
            }

            dev.restore_factory_presets(&skip, |msg| {
                eprint!("\r{msg}");
            })
            .map_err(Failure::error)?;
            eprintln!();
            println!("OK: all requested factory preset groups replayed.");

            // flash 落盘异步生效（观察到提交后 ~2.4 s 的过期读），稍候再验证名表。
            std::thread::sleep(std::time::Duration::from_secs(3));
            // 收尾验证：名表里有多少个非占位名。
            let entries = dev.read_preset_names()?;
            let named = entries.iter().filter(|e| e.named).count();
            println!("preset table now has {named}/40 factory-named slots.");
            if named == 40 {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "expected 40 named slots, got {named} — reboot the device and \
                     re-run with --resume to continue"
                )))
            }
        }
    }
}
