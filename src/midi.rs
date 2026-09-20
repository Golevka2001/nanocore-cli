//! Real MIDI transport backed by `midir`.
//!
//! Gated behind the `midi` feature so the core library stays dependency-free.
//! Backend: CoreMIDI (macOS), ALSA (Linux), WinMM (Windows).
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

/// Client name registered with the OS MIDI stack.
pub const CLIENT_NAME: &str = "nanocore";

/// Substring used to auto-detect the device among the system ports.
pub const DEFAULT_HINT: &str = "nanocore";

fn io_err<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::other(e.to_string())
}

/// A port descriptor: `(index, name)`.
pub type PortInfo = (usize, String);

/// Enumerate available MIDI input ports.
pub fn input_ports() -> io::Result<Vec<PortInfo>> {
    let mi = MidiInput::new(CLIENT_NAME).map_err(io_err)?;
    mi.ports()
        .iter()
        .enumerate()
        .map(|(i, p)| Ok((i, mi.port_name(p).map_err(io_err)?)))
        .collect()
}

/// Enumerate available MIDI output ports.
pub fn output_ports() -> io::Result<Vec<PortInfo>> {
    let mo = MidiOutput::new(CLIENT_NAME).map_err(io_err)?;
    mo.ports()
        .iter()
        .enumerate()
        .map(|(i, p)| Ok((i, mo.port_name(p).map_err(io_err)?)))
        .collect()
}

/// Index of the first port whose name contains `hint` (case-insensitive).
pub fn find_port(ports: &[PortInfo], hint: &str) -> Option<usize> {
    let hint = hint.to_lowercase();
    ports
        .iter()
        .find(|(_, name)| name.to_lowercase().contains(&hint))
        .map(|(i, _)| *i)
}

/// A live bidirectional MIDI connection.
///
/// The input connection must stay alive for as long as we want to receive,
/// so it is kept as a field.
pub struct MidiTransport {
    out: MidiOutputConnection,
    _in: MidiInputConnection<()>,
    rx: Arc<Mutex<VecDeque<u8>>>,
    sysex: bool,
}

impl MidiTransport {
    /// Open the given input/output port indices.
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

    /// Open the first input/output ports whose names contain `hint`.
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

    /// Wrap outgoing frames in `F0 ... F7`.
    ///
    /// The device only accepts SysEx-wrapped frames — a bare `7D 4E 43 ...`
    /// is silently dropped — so this is on by default. Turn it off only to
    /// replay raw bytes verbatim.
    pub fn set_sysex(&mut self, on: bool) {
        self.sysex = on;
    }

    /// Number of bytes currently buffered from the device.
    pub fn buffered(&self) -> usize {
        self.rx.lock().map(|q| q.len()).unwrap_or(0)
    }

    /// Close the output connection (the input connection is dropped).
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
