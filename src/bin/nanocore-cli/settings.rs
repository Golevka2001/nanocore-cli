//! `settings` 命令：显示 / 修改设备设置页项目（无线、回放、输入增益、蓝牙音量、
//! 输入通道）。USB 音量沿用顶层 `volume` 命令。
//!
//! 写入走 `0x66` SET_FIELD，读取走 `0x65` 设置快照——字段映射见
//! [`nanocore::setting`] 与 [`nanocore::protocol::decode_settings`]。

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
        SettingsCmd::Wireless { state } => {
            let value = parse_on_off(&state)?;
            set_and_verify(conn, setting::WIRELESS, value, |s| {
                format!("wireless: {} -> {}", s.wireless, value != 0)
            })
        }
        SettingsCmd::Playback { state } => {
            let value = parse_on_off(&state)?;
            set_and_verify(conn, setting::PLAYBACK, value, |s| {
                format!("playback: {} -> {}", s.playback, value != 0)
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
        SettingsCmd::BtVolume { percent } => {
            set_and_verify(conn, setting::BT_VOLUME, percent, |s| {
                format!("bt volume: {} -> {}", s.bt_volume, percent)
            })
        }
        SettingsCmd::InputChannel { channel } => {
            let value = parse_channel(&channel)?;
            set_and_verify(conn, setting::INPUT_CHANNEL, value, |s| {
                format!(
                    "input channel: {} -> {}",
                    channel_label(s.input_channel),
                    channel_label(value)
                )
            })
        }
    }
}

fn print_settings(s: &nanocore::protocol::DeviceSettings) {
    println!("wireless:        {}", on_off(s.wireless));
    println!("playback:        {}", on_off(s.playback));
    println!("input gain:      {} dB", s.input_gain_db);
    println!("usb volume:      {}", s.usb_volume);
    println!("bt volume:       {}", s.bt_volume);
    println!("input channel:   {}", channel_label(s.input_channel));
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
        setting::PLAYBACK => s.playback as u8,
        setting::USB_VOLUME => s.usb_volume,
        setting::BT_VOLUME => s.bt_volume,
        setting::INPUT_CHANNEL => s.input_channel,
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
    if s == "all" {
        return Ok(0);
    }
    match s.parse::<u8>() {
        Ok(v @ 1..=16) => Ok(v),
        _ => Err(Failure::error(format!("expected all|1-16, got {s:?}"))),
    }
}

fn on_off(v: bool) -> &'static str {
    if v {
        "on"
    } else {
        "off"
    }
}

fn channel_label(v: u8) -> String {
    if v == 0 {
        "all".into()
    } else {
        format!("ch{v}")
    }
}
