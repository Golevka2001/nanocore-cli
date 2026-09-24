//! `preset` commands: select / step / list the preset slots.

use nanocore::midicc;

use crate::cli::{Conn, PresetCmd};
use crate::common::{send_standard, Dev};

pub fn cmd_preset(
    dev: &mut Dev,
    conn: &Conn,
    cmd: PresetCmd,
) -> Result<(), String> {
    match cmd {
        PresetCmd::Set { index } => {
            send_standard(dev, &midicc::program_change(conn.channel, index))
        }
        PresetCmd::Next => send_standard(dev, &midicc::global::next_preset(conn.channel)),
        PresetCmd::Prev => send_standard(dev, &midicc::global::prev_preset(conn.channel)),
        PresetCmd::List => {
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
        PresetCmd::Show => Err(
            "the active preset is not decoded yet (full snapshot decode pending). \
             Try `preset list` for the slot names."
                .into(),
        ),
    }
}
