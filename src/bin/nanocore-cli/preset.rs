//! `preset` 命令：选择 / 步进 / 列出预设槽位。

use nanocore::midicc;

use crate::cli::{Conn, PresetCmd};
use crate::common::{open_device, send_standard};

pub fn cmd_preset(cmd: PresetCmd, conn: &Conn) -> Result<(), String> {
    match cmd {
        // 读取活动预设需要快照解码——在打开端口之前就拒绝。
        PresetCmd::Show => Err(
            "the active preset is not decoded yet (full snapshot decode pending). \
             Try `preset list` for the slot names."
                .into(),
        ),
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
    }
}
