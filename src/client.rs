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

    /// Read a whole amp model or IR blob using a plan from [`crate::asset::read_plan`].
    ///
    /// Read-only. Sends each chunk request and concatenates the decoded replies
    /// in ascending `addr` order (the device answers out of order).
    ///
    /// `on_chunk(done, total)` is called after each reply so callers can show
    /// progress; pass a closure that does nothing if you don't care.
    ///
    /// The plan is taken as a parameter rather than derived here so that an
    /// unknown region or slot is rejected by the caller *before* any MIDI is
    /// sent, instead of surfacing as a fake transport error.
    pub fn read_asset<F>(
        &mut self,
        plan: &[crate::asset::ChunkRequest],
        total: u32,
        mut on_chunk: F,
    ) -> Result<Vec<u8>, Error<T::Error>>
    where
        F: FnMut(usize, usize),
    {
        let n = plan.len();
        let mut chunks = Vec::with_capacity(n);
        for (i, req) in plan.iter().enumerate() {
            let reply = self.transact_expect(req.region, req.payload(), &[req.region])?;
            if let Some(c) = crate::asset::parse_chunk(&reply.payload) {
                chunks.push(c);
            }
            on_chunk(i + 1, n);
        }
        let blob = crate::asset::assemble(&chunks);
        // The plan is built for the standard blob size; drop any slack.
        Ok(if blob.len() > total as usize {
            blob[..total as usize].to_vec()
        } else {
            blob
        })
    }

    /// Write an IR blob back to a slot and verify by reading it again.
    ///
    /// Replays the app's import flow for one slot:
    /// `0x56` (query name) → PING → `0x49` (session) → PING → `0x50` (sync) →
    /// `0x51` (begin) → `0x52`×N (data) → `0x53` (hash) → `0x55` (name) →
    /// `0x57` (commit) → full read-back.
    ///
    /// The identity hash is **derived from `blob`** (`identity_hash`): the
    /// device recomputes it on commit and silently rejects a mismatch.
    /// The `B3` byte of the `0x53` frame is derived from the blob (it carries
    /// the CRC's masked-out high bits — see `asset::write_hash`). The preamble
    /// steps are best-effort: the device keeps its write session across
    /// connections, and a session already open makes `0x49`/`0x50` go unanswered.
    ///
    /// Returns the read-back bytes so the caller can compare them with `blob`.
    pub fn write_ir_slot(
        &mut self,
        slot: u8,
        blob: &[u8],
        name_override: Option<&str>,
        mut on_progress: impl FnMut(&str),
    ) -> Result<Vec<u8>, String> {
        // Flash operations are slow: the device can take well over the default
        // 1 s budget to answer session/commit frames (observed: a 0x49 ACK
        // arriving after the whole 1 s budget expired). Use a generous policy
        // for the whole write flow.
        let saved = self.config;
        self.set_config(crate::client::RetryConfig {
            initial_timeout: Duration::from_millis(500),
            max_timeout: Duration::from_millis(2000),
            hard_limit: Duration::from_secs(10),
            max_retries: 2,
            poll_interval: Duration::from_millis(10),
        });
        let result =
            self.write_ir_slot_inner(slot, blob, name_override, &mut on_progress);
        self.set_config(saved);
        result
    }

    fn write_ir_slot_inner(
        &mut self,
        slot: u8,
        blob: &[u8],
        name_override: Option<&str>,
        on_progress: &mut dyn FnMut(&str),
    ) -> Result<Vec<u8>, String> {
        use crate::asset;

        // 1. Query the slot's name FIRST — while the device is still in its
        //    normal (read-friendly) mode. Once the write session is open the
        //    device only accepts the write flow. The identity hash is NOT
        //    reused: the device recomputes it from the blob on commit and
        //    silently rejects a mismatch, so it must be derived from the
        //    content being written (verified 16/16 in the capture).
        let mut name: Vec<u8>;
        match self.transact_expect(
            asset::SLOT_QUERY,
            asset::slot_query(slot),
            &[asset::SLOT_QUERY],
        ) {
            Ok(reply) => {
                let (_old_hash, n) = asset::parse_slot_query(&reply.payload)
                    .ok_or_else(|| {
                        "0x56 reply did not parse as a slot identity".to_string()
                    })
                    .map_err(|e| e.to_string())?;
                name = n;
            }
            Err(_) => {
                // Mid-session the device may not answer 0x56; without an
                // explicit override we cannot preserve the old name.
                name = match name_override {
                    Some(n) => n.as_bytes().to_vec(),
                    None => return Err("slot query failed and no --name given".into()),
                };
            }
        }
        if let Some(n) = name_override {
            name = n.as_bytes().to_vec();
        }
        on_progress(&format!("slot name {:?}", String::from_utf8_lossy(&name)));

        // 2. Session open, capture order: PING -> 0x49 -> PING -> 0x50.
        //    Both 0x49 and 0x50 replies echo our seq. However the device
        //    keeps its write session across connections: a 0x49 while the
        //    session is already open is ignored (no reply), and once open the
        //    device skips straight to the write flow. So every preamble step
        //    is best-effort; the data/commit frames are what matter.
        let (pk, pp) = crate::protocol::ping();
        on_progress("ping 1");
        self.transact_expect(pk, pp.clone(), &[crate::msg_type::PING])
            .map_err(|e| e.to_string())?;
        on_progress("session open (0x49)");
        let session_ok = self
            .transact_expect(
                asset::SESSION_OPEN,
                asset::session_open(),
                &[asset::SESSION_OPEN],
            )
            .is_ok();
        if session_ok {
            on_progress("session opened");
            // In the app's capture a PING after 0x49 is echoed, but once the
            // device is in write mode it may answer only with its own
            // heartbeat (flags=0x02, own seq) — best-effort, never fatal.
            let _ = self.transact_expect(pk, pp, &[crate::msg_type::PING]);
        } else {
            on_progress("session already open (0x49 ignored)");
        }
        on_progress("sync (0x50)");
        if self
            .transact_expect(0x50, vec![0x00, 0x00, 0x00], &[0x50])
            .is_ok()
        {
            on_progress("synced");
        } else {
            on_progress("sync unanswered (continuing)");
        }

        // 3. Begin. If the device is mid-write from an earlier interrupted
        //    attempt it ignores 0x51 and expects the data continuation — so a
        //    timeout here is tolerated and the data frames decide the outcome.
        let total = blob.len() as u32;
        on_progress("begin");
        if self
            .transact_expect(
                asset::WRITE_BEGIN,
                asset::write_begin(slot, total),
                &[asset::WRITE_BEGIN],
            )
            .is_ok()
        {
            on_progress("begin acked");
        } else {
            on_progress("begin unanswered (continuing)");
        }

        // 4. Data frames.
        let n_chunks = blob.len().div_ceil(asset::WRITE_CHUNK_BYTES);
        for (ci, chunk) in blob.chunks(asset::WRITE_CHUNK_BYTES).enumerate() {
            self.transact_expect(
                asset::WRITE_IR,
                asset::encode_data_frame(chunk, ci, slot),
                &[asset::WRITE_IR],
            ).map_err(|e| {
                on_progress(&format!("FAILED at chunk {}/{}", ci + 1, n_chunks));
                e.to_string()
            })?;
            on_progress(&format!("chunk {}/{}", ci + 1, n_chunks));
        }

        // 5. Hash (derived from the blob), 6. name, 7. commit.
        on_progress("hash");
        self.transact_expect(
            asset::WRITE_HASH,
            asset::write_hash(slot, total, blob),
            &[asset::WRITE_HASH],
        ).map_err(|e| e.to_string())?;
        on_progress("name");
        let name_bytes = asset::encode_name(&name);
        self.transact_expect(
            asset::WRITE_NAME,
            asset::write_name(slot, &name_bytes),
            &[asset::WRITE_NAME],
        ).map_err(|e| e.to_string())?;
        self.transact_expect(
            asset::WRITE_COMMIT,
            asset::write_commit(slot),
            &[asset::WRITE_COMMIT],
        )
        .map_err(|e| e.to_string())?;

        // The app re-enumerates the slot table between the commit and its
        // read-back; the flash write is not immediately visible. Re-query the
        // slot and give the device a moment before reading back.
        let _ = self.transact_expect(
            asset::SLOT_QUERY,
            asset::slot_query(slot),
            &[asset::SLOT_QUERY],
        );
        // The flash write settles asynchronously: an immediate read-back can
        // serve partially-stale data (observed: one stale byte range ~2.4 s
        // into the blob seconds after the commit). Give it time, and retry
        // the read-back until it matches or the budget runs out.
        std::thread::sleep(std::time::Duration::from_millis(1200));


        // 8. Read back and return. Retry the whole read a few times: right
        //    after the commit the device can serve partially-stale data.
        let (total, plan) = asset::read_plan(asset::REGION_IR, slot)
            .ok_or_else(|| "no read plan for that slot".to_string()).map_err(|e| e.to_string())?;
        let mut back = Vec::new();
        for attempt in 0..3 {
            let mut chunks = Vec::new();
            for req in &plan {
                let reply = self.transact_expect(req.region, req.payload(), &[req.region]).map_err(|e| e.to_string())?;
                if let Some(c) = asset::parse_chunk(&reply.payload) {
                    chunks.push(c);
                }
            }
            back = asset::assemble(&chunks);
            if back.len() > total as usize {
                back.truncate(total as usize);
            }
            if back == blob {
                if attempt > 0 {
                    on_progress(&format!("read-back stable on attempt {}", attempt + 1));
                }
                break;
            }
            on_progress(&format!("read-back stale (attempt {}), waiting…", attempt + 1));
            std::thread::sleep(std::time::Duration::from_millis(1200));
        }
        Ok(back)
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
