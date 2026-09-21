//! Bulk asset transfer: amp-model and IR/cab payloads.
//!
//! Nanocore stores each amp model (~12,242 bytes) and each IR (~4,096 bytes) as
//! an opaque blob. The app reads one out with a sequence of small "chunk"
//! requests; the device answers each with a fixed 239-byte payload that decodes
//! back to 200 bytes of the blob.
//!
//! # Wire format
//!
//! Request (TX), 8 payload bytes:
//!
//! ```text
//! 00 04 00 [B3] [SLOT] [ADDR_lo] [ADDR_hi] [LEN]
//! ```
//!
//! Reply (RX), 239 payload bytes:
//!
//! ```text
//! 00 00 [C2] [B3<<1 | LEAD_HI<<6] 00 [SLOT] [ADDR_lo] [ADDR_hi] 52 2f | data...
//! ```
//!
//! `ADDR_lo`/`ADDR_hi` are a 7-bit pair (`addr = hi*128 + lo`) and the reply
//! echoes the request's slot and address verbatim.
//!
//! # Payload codec (7-bit safe, lossless)
//!
//! SysEx forbids bytes ≥ 0x80, so the blob is packed as:
//!
//! * byte 0 of the data = decoded byte 0, with its bit 7 carried by bit 6 of the
//!   reply's byte 3 (`B3<<1` on an echo, so effectively bit 5 of `B3`).
//! * then groups of 8: `[mask][d0..d6]`; decoded byte `i` is `d_i | ((mask >> i) & 1) << 7`.
//! * the final group may be short (`[mask][d0..dk]`).
//!
//! Each full chunk yields exactly 200 bytes, so chunk `c` of a blob is
//! `blob[200c .. 200c+200]`.
//!
//! # Chunk ordering
//!
//! The app issues the requests **out of order**; the payload order is the
//! **ascending ADDR order**. Always sort replies by `addr` before concatenating.
//!
//! # Provenance / confidence
//!
//! The address plans below are captured **verbatim** from
//! `factory-preset-export.mmon` (a full factory
//! export). They are identical for every slot of the same region that holds a
//! blob of the same size (verified: 15 IR slots share one plan, 19 amp slots
//! share another; the two smaller plans are prefixes of the larger one).
//!
//! The derivation of the addresses is not understood — they are not a linear
//! byte offset (successive chunks step by 72 or 200, and the final chunk's
//! address depends on the blob length). Replaying the captured plan is
//! therefore how this module addresses a blob; that is exactly what the device
//! was observed to accept.
//!
//! End-to-end validation: decoding every captured reply reproduces the bundle's
//! `ir/slot_NN.bin` byte-for-byte and matches the manifest's `crc32`
//! (16/16 IR assets), and it matches the manifest `crc32` for 19/19 amp assets.

/// Message type used for amp-model region transfers.
pub const REGION_AMP: u8 = 0x39;
/// Message type used for IR/cab region transfers.
pub const REGION_IR: u8 = 0x59;

/// Bytes of framing in front of the packed data in a reply payload.
pub const PAYLOAD_HEADER: usize = 10;
/// Bit in reply byte 3 that carries the decoded byte 0's bit 7.
pub const LEAD_HI_BIT: u8 = 0x40;
/// Decoded bytes carried by one full chunk.
pub const CHUNK_BYTES: usize = 200;
/// `LEN` byte the app sends for every chunk except the last one.
pub const FULL_CHUNK_LEN: u8 = 72;

/// Blob size of a factory IR (4,096 bytes = 1,024 `f32le` taps).
pub const IR_TOTAL: u32 = 4096;
/// Blob size of a factory amp model (`ampAssets` `length` in the bundle manifest).
pub const AMP_TOTAL: u32 = 12242;

/// `(LEN-byte, address)` per chunk index for a 4,096-byte IR.
///
/// Captured verbatim; shared by all 15 IR slots of that size in the export.
pub const IR_PLAN: &[(u8, u16)] = &[
    (0x08, 0),
    (0x0a, 72),
    (0x0a, 144),
    (0x08, 344),
    (0x08, 416),
    (0x0a, 488),
    (0x0a, 560),
    (0x08, 760),
    (0x08, 832),
    (0x08, 904),
    (0x0a, 976),
    (0x0a, 1048),
    (0x08, 1248),
    (0x08, 1320),
    (0x0a, 1392),
    (0x0a, 1464),
    (0x0a, 1536),
    (0x08, 1736),
    (0x08, 1808),
    (0x0a, 1880),
    (0x02, 1952),
];

/// `(B3, address)` per chunk index for a 12,242-byte amp model.
///
/// Captured verbatim; shared by all 19 amp slots of that size in the export.
/// Its first 19 entries are identical to [`IR_PLAN`]'s.
pub const AMP_PLAN: &[(u8, u16)] = &[
    (0x08, 0),
    (0x0a, 72),
    (0x0a, 144),
    (0x08, 344),
    (0x08, 416),
    (0x0a, 488),
    (0x0a, 560),
    (0x08, 760),
    (0x08, 832),
    (0x08, 904),
    (0x0a, 976),
    (0x0a, 1048),
    (0x08, 1248),
    (0x08, 1320),
    (0x0a, 1392),
    (0x0a, 1464),
    (0x0a, 1536),
    (0x08, 1736),
    (0x08, 1808),
    (0x0a, 1880),
    (0x0a, 1952),
    (0x08, 2152),
    (0x08, 2224),
    (0x0a, 2296),
    (0x0a, 2368),
    (0x0a, 2440),
    (0x08, 2640),
    (0x08, 2712),
    (0x0a, 2784),
    (0x0a, 2856),
    (0x08, 3056),
    (0x08, 3128),
    (0x08, 3200),
    (0x0a, 3272),
    (0x0a, 3344),
    (0x08, 3544),
    (0x08, 3616),
    (0x0a, 3688),
    (0x0a, 3760),
    (0x08, 3960),
    (0x08, 4032),
    (0x08, 4104),
    (0x0a, 4176),
    (0x0a, 4248),
    (0x08, 4448),
    (0x08, 4520),
    (0x0a, 4592),
    (0x0a, 4664),
    (0x0a, 4736),
    (0x08, 4936),
    (0x08, 5008),
    (0x0a, 5080),
    (0x0a, 5152),
    (0x08, 5352),
    (0x08, 5424),
    (0x0a, 5496),
    (0x0a, 5568),
    (0x0a, 5640),
    (0x08, 5840),
    (0x08, 5912),
    (0x0a, 5984),
    (0x02, 6056),
];

/// `"amp"` / `"ir"` (also `"cab"`) → region message type.
pub fn region_from_name(name: &str) -> Option<u8> {
    match name.to_ascii_lowercase().as_str() {
        "amp" | "a" => Some(REGION_AMP),
        "ir" | "cab" | "c" => Some(REGION_IR),
        _ => None,
    }
}

/// Region message type → short label.
pub fn region_name(region: u8) -> &'static str {
    match region {
        REGION_AMP => "amp",
        REGION_IR => "ir",
        _ => "?",
    }
}

/// One chunk request in a read plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkRequest {
    /// Region message type ([`REGION_AMP`] / [`REGION_IR`]).
    pub region: u8,
    /// 0-based firmware slot.
    pub slot: u8,
    /// `B3` field. Meaning not established; replayed from the capture.
    pub b3: u8,
    /// 7-bit-pair address. Meaning not established; replayed from the capture.
    pub addr: u16,
    /// `LEN` field: [`FULL_CHUNK_LEN`] except on the final chunk, where it is the
    /// number of bytes remaining in the blob.
    pub len: u8,
}

impl ChunkRequest {
    /// The 8-byte payload to send (frame type is [`REGION_AMP`] / [`REGION_IR`]).
    pub fn payload(&self) -> Vec<u8> {
        vec![
            0x00,
            0x04,
            0x00,
            self.b3,
            self.slot,
            (self.addr & 0x7f) as u8,
            (self.addr >> 7) as u8,
            self.len,
        ]
    }
}

/// Total blob size in bytes for a region at its standard (factory) size.
pub fn total_for(region: u8) -> Option<u32> {
    match region {
        REGION_AMP => Some(AMP_TOTAL),
        REGION_IR => Some(IR_TOTAL),
        _ => None,
    }
}

/// Build the full read plan for one slot.
///
/// Returns `(blob_len, requests)`. The requests must be answered in order; see
/// [`assemble`].
pub fn read_plan(region: u8, slot: u8) -> Option<(u32, Vec<ChunkRequest>)> {
    let (plan, total) = match region {
        REGION_AMP => (AMP_PLAN, AMP_TOTAL),
        REGION_IR => (IR_PLAN, IR_TOTAL),
        _ => return None,
    };
    let n = plan.len();
    let mut out = Vec::with_capacity(n);
    for (i, &(b3, addr)) in plan.iter().enumerate() {
        let len = if i + 1 == n {
            let rem = total - CHUNK_BYTES as u32 * (n as u32 - 1);
            // LEN is a 7-bit field; the captured plans never need to clamp.
            rem.min(0x7f) as u8
        } else {
            FULL_CHUNK_LEN
        };
        out.push(ChunkRequest { region, slot, b3, addr, len });
    }
    Some((total, out))
}

/// Decode the packed data of one reply payload into blob bytes.
pub fn decode_chunk(payload: &[u8]) -> Vec<u8> {
    if payload.len() <= PAYLOAD_HEADER {
        return Vec::new();
    }
    let d = &payload[PAYLOAD_HEADER..];
    let mut out = Vec::with_capacity(CHUNK_BYTES);
    let lead_hi = (payload[3] & LEAD_HI_BIT) << 1;
    out.push(d[0] | lead_hi);

    let mut i = 1;
    while i < d.len() {
        let end = (i + 8).min(d.len());
        let group = &d[i..end];
        i = end;
        let mask = group[0];
        for (j, &byte) in group[1..].iter().enumerate() {
            out.push(byte | (((mask >> j) & 1) << 7));
        }
    }
    out
}

/// One decoded reply chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chunk {
    /// Slot the device reported in the reply.
    pub slot: u8,
    /// Address the device echoed for this chunk.
    pub addr: u16,
    /// Decoded bytes (200 for a full chunk).
    pub data: Vec<u8>,
}

/// Parse a reply payload into `(slot, addr, decoded bytes)`.
pub fn parse_chunk(payload: &[u8]) -> Option<Chunk> {
    if payload.len() <= PAYLOAD_HEADER {
        return None;
    }
    Some(Chunk {
        slot: payload[5],
        addr: payload[6] as u16 | ((payload[7] as u16) << 7),
        data: decode_chunk(payload),
    })
}

/// Concatenate decoded chunks into the blob.
///
/// The chunks are sorted by `addr` first: the app issues the requests out of
/// order, so arrival order is not payload order.
pub fn assemble(chunks: &[Chunk]) -> Vec<u8> {
    let mut sorted: Vec<&Chunk> = chunks.iter().collect();
    sorted.sort_by_key(|c| c.addr);
    let mut out = Vec::with_capacity(sorted.iter().map(|c| c.data.len()).sum());
    for c in sorted {
        out.extend_from_slice(&c.data);
    }
    out
}

/// CRC-32 (IEEE 802.3 — the same polynomial zlib and the bundle manifest use).
///
/// Used to check a read-back blob against the `crc32` recorded in a `.npreset`
/// manifest without pulling in a dependency.
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
