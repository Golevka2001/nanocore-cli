//! 基于 `midir` 的真实 MIDI 传输层。
//!
//! 通过 `midi` feature 门控，使核心库保持无依赖。
//! 后端：CoreMIDI（macOS）、ALSA（Linux）、WinMM（Windows）。
//!
//! ```no_run
//! use nanocore::client::Nanocore;
//! use nanocore::midi::MidiTransport;
//!
//! let transport = MidiTransport::open_by_name("nanocore")?;
//! let mut dev = Nanocore::new(transport);
//! dev.select_preset(0)?;
//! # Ok::<_, std::io::Error>(())
//! ```

use std::collections::VecDeque;
use std::io;
use std::sync::{Arc, Mutex};

use midir::{MidiInput, MidiInputConnection, MidiOutput, MidiOutputConnection};

use crate::transport::Transport;

/// 向 OS MIDI 栈注册的客户端名称。
pub const CLIENT_NAME: &str = "nanocore";

/// 在系统端口中自动检测设备所用的子串。
pub const DEFAULT_HINT: &str = "nanocore";

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

/// 端口描述符：`(index, name)`。
pub type PortInfo = (usize, String);

/// 枚举可用的 MIDI 输入端口。
pub fn input_ports() -> io::Result<Vec<PortInfo>> {
    let mi = MidiInput::new(CLIENT_NAME).map_err(io_err)?;
    mi.ports()
        .iter()
        .enumerate()
        .map(|(i, p)| Ok((i, mi.port_name(p).map_err(io_err)?)))
        .collect()
}

/// 枚举可用的 MIDI 输出端口。
pub fn output_ports() -> io::Result<Vec<PortInfo>> {
    let mo = MidiOutput::new(CLIENT_NAME).map_err(io_err)?;
    mo.ports()
        .iter()
        .enumerate()
        .map(|(i, p)| Ok((i, mo.port_name(p).map_err(io_err)?)))
        .collect()
}

/// 名称包含 `hint`（不区分大小写）的第一个端口的索引。
pub fn find_port(ports: &[PortInfo], hint: &str) -> Option<usize> {
    let hint = hint.to_lowercase();
    ports
        .iter()
        .find(|(_, name)| name.to_lowercase().contains(&hint))
        .map(|(i, _)| *i)
}

/// 一条活动的双向 MIDI 连接。
///
/// 只要还想接收数据，输入连接就必须保持存活，因此将其保留为字段。
pub struct MidiTransport {
    out: MidiOutputConnection,
    _in: MidiInputConnection<()>,
    rx: Arc<Mutex<VecDeque<u8>>>,
    sysex: bool,
}

impl MidiTransport {
    /// 打开给定的输入/输出端口索引。
    pub fn open(in_index: usize, out_index: usize) -> io::Result<Self> {
        let rx: Arc<Mutex<VecDeque<u8>>> = Arc::new(Mutex::new(VecDeque::new()));

        let mi = MidiInput::new(CLIENT_NAME).map_err(io_err)?;
        let in_ports = mi.ports();
        let in_port = in_ports.get(in_index).ok_or_else(|| {
            io::Error::other(format!(
                "no MIDI input port at index {in_index} ({} available)",
                in_ports.len()
            ))
        })?;
        let sink = rx.clone();
        let conn_in = mi
            .connect(
                in_port,
                CLIENT_NAME,
                move |_timestamp, bytes, _data| {
                    if let Ok(mut q) = sink.lock() {
                        q.extend(bytes.iter().copied());
                    }
                },
                (),
            )
            .map_err(io_err)?;

        let mo = MidiOutput::new(CLIENT_NAME).map_err(io_err)?;
        let out_ports = mo.ports();
        let out_port = out_ports.get(out_index).ok_or_else(|| {
            io::Error::other(format!(
                "no MIDI output port at index {out_index} ({} available)",
                out_ports.len()
            ))
        })?;
        let conn_out = mo.connect(out_port, CLIENT_NAME).map_err(io_err)?;

        Ok(Self { out: conn_out, _in: conn_in, rx, sysex: true })
    }

    /// 打开名称包含 `hint` 的第一个输入/输出端口。
    pub fn open_by_name(hint: &str) -> io::Result<Self> {
        let ins = input_ports()?;
        let outs = output_ports()?;
        let in_index = find_port(&ins, hint).ok_or_else(|| {
            io::Error::other(format!(
                "no MIDI input port matching {hint:?}; available: {:?}",
                ins.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>()
            ))
        })?;
        let out_index = find_port(&outs, hint).ok_or_else(|| {
            io::Error::other(format!(
                "no MIDI output port matching {hint:?}; available: {:?}",
                outs.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>()
            ))
        })?;
        eprintln!(
            "using input #{} {:?}, output #{} {:?}",
            in_index, ins[in_index].1, out_index, outs[out_index].1
        );
        Self::open(in_index, out_index)
    }

    /// 用 `F0 ... F7` 包裹发出的帧。
    ///
    /// 设备只接受 SysEx 包裹的帧——裸的 `7D 4E 43 ...` 会被静默丢弃——
    /// 因此默认开启。仅在需要逐字节重放原始数据时关闭。
    pub fn set_sysex(&mut self, on: bool) {
        self.sysex = on;
    }

    /// 当前从设备缓冲的字节数。
    pub fn buffered(&self) -> usize {
        self.rx.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// 关闭输出连接（输入连接随 self 一起 drop）。
    pub fn close(self) {
        let _ = self.out.close();
    }
}

impl Transport for MidiTransport {
    type Error = io::Error;

    fn send(&mut self, data: &[u8]) -> Result<(), io::Error> {
        if self.sysex {
            let mut buf = Vec::with_capacity(data.len() + 2);
            buf.push(crate::frame::SYSEX_START);
            buf.extend_from_slice(data);
            buf.push(crate::frame::SYSEX_END);
            self.out.send(&buf).map_err(io_err)
        } else {
            self.out.send(data).map_err(io_err)
        }
    }

    fn receive(&mut self) -> Result<Vec<u8>, io::Error> {
        let mut q = self.rx.lock().map_err(io_err)?;
        Ok(q.drain(..).collect())
    }
}
