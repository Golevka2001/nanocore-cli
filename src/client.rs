//! High-level client.
//!
//! Besides fire-and-forget [`Nanocore::send`], this provides
//! [`Nanocore::transact`] which correlates a request with the device's reply
//! and retries on timeout.
//!
//! The retry policy mirrors what the official app was observed to do
//! (`liblivtra.so`, `FUN_00483094`):
//!
//! * initial response timeout 120 ms
//! * doubles on every retry, capped at 600 ms
//! * total budget (hard limit) 1 s
//! * retries **only** on timeout — never on a malformed or error response

use std::collections::VecDeque;
use std::fmt;
use std::time::{Duration, Instant};

use crate::frame::{Direction, Frame};
use crate::protocol;
use crate::transport::Transport;

/// Initial per-attempt response timeout used by the official app.
pub const INITIAL_TIMEOUT: Duration = Duration::from_millis(120);
/// Per-attempt timeout ceiling used by the official app.
pub const MAX_TIMEOUT: Duration = Duration::from_millis(600);
/// Total budget for a transaction including all retries.
pub const HARD_LIMIT: Duration = Duration::from_millis(1000);
/// How often the transport is polled while waiting for a reply.
pub const POLL_INTERVAL: Duration = Duration::from_millis(5);

/// Retry/timeout policy for [`Nanocore::transact`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryConfig {
    pub initial_timeout: Duration,
    pub max_timeout: Duration,
    pub hard_limit: Duration,
    /// Maximum number of *retries* (so `max_retries + 1` attempts total).
    pub max_retries: u32,
    pub poll_interval: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            initial_timeout: INITIAL_TIMEOUT,
            max_timeout: MAX_TIMEOUT,
            hard_limit: HARD_LIMIT,
            max_retries: 3,
            poll_interval: POLL_INTERVAL,
        }
    }
}

/// Errors produced by [`Nanocore::transact`].
#[derive(Debug)]
pub enum Error<E> {
    /// The underlying transport failed.
    Transport(E),
    /// The device never answered within the timeout budget.
    Timeout { attempts: u32, elapsed: Duration },
}

impl<E: fmt::Display> fmt::Display for Error<E> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Transport(e) => write!(f, "transport error: {e}"),
            Error::Timeout { attempts, elapsed } => write!(
                f,
                "timed out waiting for response after {attempts} attempt(s) in {elapsed:?}"
            ),
        }
    }
}

impl<E: fmt::Debug + fmt::Display> std::error::Error for Error<E> {}

impl<E> From<E> for Error<E> {
    fn from(e: E) -> Self {
        Error::Transport(e)
    }
}

/// High-level Nanocore controller. Wraps a [`Transport`] and manages the
/// little-endian sequence counter.
pub struct Nanocore<T: Transport> {
    transport: T,
    seq: u16,
    config: RetryConfig,
    /// Frames received but not yet consumed by a transaction — unsolicited
    /// events, or a reply that arrived before its caller looked for it.
    pending: VecDeque<Frame>,
}

impl<T: Transport> Nanocore<T> {
    pub fn new(transport: T) -> Self {
        Self::with_config(transport, RetryConfig::default())
    }

    pub fn with_config(transport: T, config: RetryConfig) -> Self {
        Self { transport, seq: 0, config, pending: VecDeque::new() }
    }

    pub fn into_inner(self) -> T {
        self.transport
    }

    pub fn transport(&mut self) -> &mut T {
        &mut self.transport
    }

    /// Override the retry/timeout policy.
    pub fn set_config(&mut self, config: RetryConfig) {
        self.config = config;
    }

    fn next_seq(&mut self) -> u16 {
        let s = self.seq;
        self.seq = (self.seq + 1) & 0x7FFF;
        s
    }

    /// Send a message by type + payload with `flags = 0x00` (normal).
    ///
    /// Fire and forget: no reply correlation, no retry.
    pub fn send(&mut self, kind: u8, payload: Vec<u8>) -> Result<(), T::Error> {
        let seq = self.next_seq();
        let frame = Frame::new(Direction::Tx, 0x00, seq, kind, payload);
        self.transport.send(&frame.encode())
    }

    /// Receive and decode whatever frames are currently available.
    pub fn receive(&mut self) -> Result<Vec<Frame>, T::Error> {
        let bytes = self.transport.receive()?;
        Ok(Frame::split_stream(&bytes))
    }

    /// Pull available bytes from the transport into the pending queue and
    /// return the frames that just arrived.
    fn pump(&mut self) -> Result<Vec<Frame>, T::Error> {
        let frames = self.receive()?;
        self.pending.extend(frames.iter().cloned());
        Ok(frames)
    }

    /// Remove and return the oldest pending frame echoing `seq`, optionally
    /// restricted to one of `kinds`.
    ///
    /// The device echoes the host's sequence number on replies (verified in
    /// captures: TX `seq=0x0344` -> RX `seq=0x0344`). Its *unsolicited* status
    /// frames carry the device's own counter instead, so they never match —
    /// which is exactly what we want.
    fn take_matching(&mut self, seq: u16, kinds: &[u8]) -> Option<Frame> {
        let pos = self.pending.iter().position(|f| {
            f.seq == seq && (kinds.is_empty() || kinds.contains(&f.kind))
        })?;
        self.pending.remove(pos)
    }

    /// Send a command and wait for the device's reply.
    ///
    /// A reply is any frame echoing the request's sequence number. Pass a
    /// non-empty `expect` to additionally require one of the given message
    /// types. Times out and retries according to [`RetryConfig`].
    pub fn transact_expect(
        &mut self,
        kind: u8,
        payload: Vec<u8>,
        expect: &[u8],
    ) -> Result<Frame, Error<T::Error>> {
        self.transact_with_flags(0x00, kind, payload, expect)
    }

    /// Same as [`Nanocore::transact_expect`] but with an explicit flags byte.
    ///
    /// Observed flag values: `0x00` normal, `0x02` live, `0x22` preset write.
    pub fn transact_with_flags(
        &mut self,
        flags: u8,
        kind: u8,
        payload: Vec<u8>,
        expect: &[u8],
    ) -> Result<Frame, Error<T::Error>> {
        let seq = self.next_seq();
        let wire = Frame::new(Direction::Tx, flags, seq, kind, payload).encode();

        let started = Instant::now();
        let mut timeout = self.config.initial_timeout;
        let mut attempts = 0u32;

        for attempt in 0..=self.config.max_retries {
            self.transport.send(&wire).map_err(Error::Transport)?;
            attempts = attempt + 1;
            let attempt_start = Instant::now();

            loop {
                if let Some(f) = self.take_matching(seq, expect) {
                    return Ok(f);
                }
                if started.elapsed() >= self.config.hard_limit {
                    break;
                }
                if attempt_start.elapsed() >= timeout {
                    break;
                }
                let arrived = self.pump().map_err(Error::Transport)?;
                if arrived.is_empty() {
                    std::thread::sleep(self.config.poll_interval);
                }
            }

            if attempt == self.config.max_retries || started.elapsed() >= self.config.hard_limit {
                break;
            }
            timeout = (timeout * 2).min(self.config.max_timeout);
        }

        Err(Error::Timeout { attempts, elapsed: started.elapsed() })
    }

    /// [`Nanocore::transact_expect`] accepting any reply type.
    pub fn transact(&mut self, kind: u8, payload: Vec<u8>) -> Result<Frame, Error<T::Error>> {
        self.transact_expect(kind, payload, &[])
    }

    /// Remove and return all frames that were never consumed — unsolicited
    /// events, or replies that arrived without a matching request. Useful for
    /// diagnosing a timeout.
    pub fn drain_pending(&mut self) -> Vec<Frame> {
        self.pending.drain(..).collect()
    }

    /// Send raw bytes on the transport **without** the SysEx frame wrapper.
    ///
    /// Used for the standard MIDI control surface (Control Change / Program
    /// Change), which the device accepts as bare 3-byte packets. Make sure the
    /// transport is in non-SysEx mode first — e.g.
    /// `self.transport().set_sysex(false)` for [`crate::midi::MidiTransport`].
    pub fn send_raw(&mut self, bytes: &[u8]) -> Result<(), T::Error> {
        self.transport.send(bytes)
    }

    // -- checked convenience commands ----------------------------------------

    /// Ping and return the device's reply (connectivity check).
    pub fn ping_checked(&mut self) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::ping();
        self.transact_expect(k, p, &[crate::msg_type::PING])
    }

    /// Select a preset and wait for the acknowledgement.
    pub fn select_preset_checked(&mut self, index: u8) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::select_preset(index);
        self.transact(k, p)
    }

    /// Request the snapshot and return the device's reply frame.
    pub fn read_snapshot(&mut self) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::request_snapshot();
        self.transact_expect(k, p, &[crate::msg_type::SNAPSHOT])
    }

    /// Query the model name of a single amp / drive slot and return the reply.
    ///
    /// Read-only — sends a `0x36` query and waits for the `0x36` reply. The
    /// reply payload is parsed by [`crate::protocol::parse_model_name`]. The
    /// cab / IR slots are enumerated by a different message and are NOT covered
    /// here.
    pub fn query_model(&mut self, index: u8) -> Result<Frame, Error<T::Error>> {
        let (k, p) = protocol::model_query(index);
        self.transact_expect(k, p, &[crate::msg_type::MODEL_LIST])
    }

    /// Read the names of all preset slots (0–63).
    ///
    /// The device answers a `0x40` preset-list query with 8 fixed-size records
    /// per reply, so 8 queries cover all 64 slots. The parser is strict: any
    /// misalignment returns `None` and the whole read fails rather than
    /// producing shifted names.
    pub fn read_preset_names(&mut self) -> Result<Vec<protocol::PresetEntry>, String> {
        let mut out = Vec::with_capacity(64);
        for base in (0..64).step_by(8) {
            let (k, p) = protocol::query_preset_list(base, 8);
            let reply = self
                .transact_expect(k, p, &[crate::msg_type::PRESET_LIST])
                .map_err(|e| e.to_string())?;
            let entries = protocol::parse_preset_list(&reply.payload)
                .ok_or_else(|| format!("preset-list reply for slots {base}.. did not decode"))?;
            out.extend(entries);
        }
        Ok(out)
    }

    // -- fire-and-forget convenience commands ---------------------------------

    pub fn ping(&mut self) -> Result<(), T::Error> {
        let (k, p) = protocol::ping();
        self.send(k, p)
    }

    pub fn select_preset(&mut self, index: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::select_preset(index);
        self.send(k, p)
    }

    pub fn select_cab(&mut self, index: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::select_cab(index);
        self.send(k, p)
    }

    pub fn select_variant(&mut self, variant: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::select_variant(variant);
        self.send(k, p)
    }

    pub fn set_enabled(&mut self, slot: u8, enabled: bool) -> Result<(), T::Error> {
        let (k, p) = protocol::set_enabled(slot, enabled);
        self.send(k, p)
    }

    pub fn set_usb_volume(&mut self, percent: u8) -> Result<(), T::Error> {
        let (k, p) = protocol::set_usb_volume(percent);
        self.send(k, p)
    }

    pub fn set_name(&mut self, name: &[u8]) -> Result<(), T::Error> {
        let (k, p) = protocol::set_name(name);
        self.send(k, p)
    }

    pub fn load_effect(
        &mut self,
        slot: u8,
        effect_index: u8,
        model_flag: u8,
        params: &[f32],
    ) -> Result<(), T::Error> {
        let (k, p) = protocol::load_effect(slot, effect_index, model_flag, params);
        self.send(k, p)
    }

    pub fn request_snapshot(&mut self) -> Result<(), T::Error> {
        let (k, p) = protocol::request_snapshot();
        self.send(k, p)
    }

    /// Write the effect chain order (`0x6D` sub `0x0A`, field `0x05`).
    ///
    /// `order` is the left-to-right chain as a permutation of `0..8`, using
    /// [`protocol::CHAIN_ROLES`] indices (`0=FX1 1=FX2 2=AMP 3=CAB 4=DEL
    /// 5=MOD 6=REV 7=EQ`). Panics if `order` is not a permutation — validate
    /// user input before calling, or use [`Nanocore::set_chain_order_checked`].
    pub fn set_chain_order(&mut self, order: &[u8]) -> Result<(), T::Error> {
        let (k, p) = protocol::chain_write(order)
            .expect("chain order must contain each of 0..8 exactly once");
        self.send(k, p)
    }

    /// Write the effect chain order (`0x6D` sub `0x0A`, field `0x05`) and verify.
    ///
    /// `order` is the left-to-right chain as a permutation of `0..8`, using
    /// [`protocol::CHAIN_ROLES`] indices (`0=FX1 1=FX2 2=AMP 3=CAB 4=DEL
    /// 5=MOD 6=REV 7=EQ`).
    ///
    /// Returns:
    /// * `Ok(None)` — `order` is not a permutation; nothing was sent.
    /// * `Ok(Some(order))` — the device acknowledged it. The ACK mirrors the
    ///   9-byte chain field back (re-encoded with a different filler index), so
    ///   this is the order the device reports it took.
    ///
    /// The unsolicited `0x09` notification seen in app captures is additionally
    /// harvested for a short window when present, but it carries the device's own
    /// sequence number and **does not arrive in every session**, so it is never
    /// required — the ACK is the reliable read-back.
    pub fn write_chain_order(&mut self, order: &[u8]) -> Result<Option<Vec<u8>>, Error<T::Error>> {
        let (kind, payload) = match protocol::chain_write(order) {
            Some(x) => x,
            None => return Ok(None),
        };
        let ack = self.transact(kind, payload)?;
        let echoed = protocol::parse_chain_ack(&ack.payload);

        // Optional: harvest a `0x09` notification (never a matched reply).
        let deadline = Instant::now() + std::time::Duration::from_millis(300);
        loop {
            let mut frames = self.drain_pending();
            if let Ok(more) = self.receive() {
                frames.extend(more);
            }
            for f in frames {
                if f.seq != ack.seq && f.kind == crate::msg_type::NOTIFY {
                    if protocol::parse_chain_notify(&f.payload).is_some() {
                        return Ok(echoed);
                    }
                }
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(self.config.poll_interval);
        }
        Ok(echoed)
    }

    /// [`Nanocore::set_chain_order`] with validation: returns `Ok(false)` instead
    /// of panicking when `order` is not a valid permutation.
    pub fn set_chain_order_checked(&mut self, order: &[u8]) -> Result<bool, T::Error> {
        match protocol::chain_write(order) {
            Some((k, p)) => self.send(k, p).map(|()| true),
            None => Ok(false),
        }
    }
}
