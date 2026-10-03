//! `settings` 命令：显示 / 修改设备设置页项目（USB / 蓝牙音量、无线、回放、
//! 输入增益、输入通道）。
//!
//! 写入走 `0x66` SET_FIELD，读取走 `0x65` 设置快照——字段映射见
//! [`nanocore::setting`] 与 [`nanocore::protocol::decode_settings`]。
//!
//! 顶层 `volume` 是本命令 `usb-volume` 的历史别名（同为 `0x66` 字段 0x08），
//! 已从 `--help` 隐藏，见 [`crate::cli::Command::Volume`]。

use nanocore::setting;

use crate::cli::{Conn, SettingsCmd};
use crate::common::{Failure, open_device};

pub fn cmd_settings(cmd: SettingsCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        SettingsCmd::Show => {
            let mut dev = open_device(conn)?;
            let s = dev.read_settings()?;
            print_settings(&s);
            Ok(())
        }
        SettingsCmd::UsbVolume { percent } => set_and_verify(
            conn,
            setting::USB_VOLUME,
            percent,
            |s| format!("usb volume: {} -> {}", s.usb_volume, percent),
        ),
        SettingsCmd::BtVolume { percent } => {
            set_and_verify(conn, setting::BT_VOLUME, percent, |s| {
                format!("bt volume: {} -> {}", s.bt_volume, percent)
            })
        }
        SettingsCmd::Wireless { state } => {
            let value = parse_on_off(&state)?;
            set_and_verify(conn, setting::WIRELESS, value, |s| {
                format!("wireless: {} -> {}", s.wireless, value != 0)
            })
        }
        SettingsCmd::Loopback { state } => {
            let value = parse_on_off(&state)?;
            set_and_verify(conn, setting::PLAYBACK, value, |s| {
                format!("loopback: {} -> {}", s.loopback, value != 0)
            })
        }
        SettingsCmd::InputGain { db } => {
            let mut dev = open_device(conn)?;
            let before = dev.read_settings()?;
            let (k, p) = nanocore::protocol::set_input_gain(db);
            crate::common::transact(&mut dev, conn, k, p, &[])?;
            // 快照是周期广播 + 主动查询，稍候回读确认。
            std::thread::sleep(std::time::Duration::from_millis(400));
            let after = dev.read_settings()?;
            println!("input gain: {} dB -> {} dB", before.input_gain_db, after.input_gain_db);
            if after.input_gain_db == db {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "input gain did not stick: set {db} dB, snapshot reads {} dB",
                    after.input_gain_db
                )))
            }
        }
        SettingsCmd::MidiChannel { channel } => {
            let value = parse_channel(&channel)?;
            set_and_verify(conn, setting::INPUT_CHANNEL, value, |s| {
                format!(
                    "midi channel: {} -> {}",
                    nanocore::protocol::midi_channel_label(s.midi_channel),
                    nanocore::protocol::midi_channel_label(value)
                )
            })
        }
    }
}

/// 打印顺序 = 官方手册「全局设置界面」的菜单顺序。
fn print_settings(s: &nanocore::protocol::DeviceSettings) {
    // 末行也要换行：否则紧跟其后的 shell 提示符会粘在值后面。
    println!("{}", render_settings(s, "").trim_end_matches('\n'));
}

/// 把 `0x65` 设置快照渲染为多行可读文本。
///
/// `indent` 是每行的前导空白——`settings show` 用空串（顶格），`show` 里的
/// 那一段用两格缩进以与 `chain` / `preset` 等行对齐。
pub fn render_settings(s: &nanocore::protocol::DeviceSettings, indent: &str) -> String {
    format!(
        "{indent}wireless:        {}\n\
         {indent}loopback:        {}\n\
         {indent}input gain:      {} dB\n\
         {indent}usb volume:      {}%\n\
         {indent}bt volume:       {}%\n\
         {indent}midi channel:    {}",
        on_off(s.wireless),
        on_off(s.loopback),
        s.input_gain_db,
        s.usb_volume,
        s.bt_volume,
        nanocore::protocol::midi_channel_label(s.midi_channel),
    )
}

fn set_and_verify(
    conn: &Conn,
    field: u8,
    value: u8,
    describe: impl Fn(&nanocore::protocol::DeviceSettings) -> String,
) -> Result<(), Failure> {
    let mut dev = open_device(conn)?;
    let before = dev.read_settings()?;
    let (k, p) = nanocore::protocol::set_setting(field, value);
    crate::common::transact(&mut dev, conn, k, p, &[])?;
    std::thread::sleep(std::time::Duration::from_millis(400));
    let after = dev.read_settings()?;
    println!("{}", describe(&before));
    let got = setting_value(field, &after);
    if got == value {
        Ok(())
    } else {
        Err(Failure::mismatch(format!(
            "setting did not stick: set {value}, snapshot reads {got}"
        )))
    }
}

fn setting_value(field: u8, s: &nanocore::protocol::DeviceSettings) -> u8 {
    match field {
        setting::WIRELESS => s.wireless as u8,
        setting::PLAYBACK => s.loopback as u8,
        setting::USB_VOLUME => s.usb_volume,
        setting::BT_VOLUME => s.bt_volume,
        setting::INPUT_CHANNEL => s.midi_channel,
        _ => 0,
    }
}

fn parse_on_off(s: &str) -> Result<u8, Failure> {
    match s {
        "on" => Ok(1),
        "off" => Ok(0),
        other => Err(Failure::error(format!("expected on|off, got {other:?}"))),
    }
}

fn parse_channel(s: &str) -> Result<u8, Failure> {
    if s == "omni" {
        return Ok(0);
    }
    match s.parse::<u8>() {
        Ok(v @ 1..=16) => Ok(v),
        _ => Err(Failure::error(format!("expected omni|1-16, got {s:?}"))),
    }
}

fn on_off(v: bool) -> &'static str {
    if v {
        "on"
    } else {
        "off"
    }
}
