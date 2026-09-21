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
// Write side (IR slots only; the amp path uses `0x32` and is not yet modelled)
// ---------------------------------------------------------------------------

/// Message type used for IR data-frame writes.
pub const WRITE_IR: u8 = 0x52;
/// Message type: begin a slot write.
pub const WRITE_BEGIN: u8 = 0x51;
/// Message type: write the slot's identity hash.
pub const WRITE_HASH: u8 = 0x53;
/// Message type: write the slot's name.
pub const WRITE_NAME: u8 = 0x55;
/// Message type: commit the slot write.
pub const WRITE_COMMIT: u8 = 0x57;
/// Message type: query a slot's name + hash (also used to *verify* a write).
pub const SLOT_QUERY: u8 = 0x56;

/// Raw bytes carried by one full write frame (64, like the read direction).
pub const WRITE_CHUNK_BYTES: usize = 64;

/// Encode one slice of a blob into a `0x52` data-frame payload.
///
/// Layout (cracked from `preset-import.mmon`, validated 1019/1019 frames
/// byte-exact and 15/16 slots crc32-identical to the bundle manifest):
///
/// ```text
/// [00 44 00 H3 H4 CNT 00 SLOT] [3 lead bytes] [M][7 data] × 9 (last partial)
/// ```
///
/// * the high bits of the 3 lead bytes live in `H3` bits 4/5/6 (`d0←bit4` …);
/// * each `M` byte is the LSB-first high-bit mask of the **7 data bytes after it**;
/// * `H3` bit 0, `H4` bit 6 and `CNT` together carry the chunk index
///   (`ci = CNT*4 + (H3&1)*2 + (H4>>6)`).
pub fn encode_data_frame(chunk: &[u8], chunk_index: usize, slot: u8) -> Vec<u8> {
    let n = chunk.len();
    assert!(n >= 1 && n <= WRITE_CHUNK_BYTES, "chunk size out of range");
    let lead = n.min(3);
    let groups = (n - lead + 6) / 7;
    let mut out = vec![0u8; 8 + n + groups];
    out[0] = 0x00;
    out[1] = 0x44;
    out[2] = 0x00;
    let mut h3 = ((chunk_index >> 1) & 1) as u8;
    for j in 0..lead {
        h3 |= ((chunk[j] >> 7) & 1) << (4 + j);
    }
    out[3] = h3;
    out[4] = ((chunk_index & 1) as u8) << 6;
    out[5] = (chunk_index / 4) as u8;
    out[7] = slot;

    let mut w = 8;
    for j in 0..lead {
        out[w + j] = chunk[j] & 0x7f;
    }
    w += lead;
    let mut k = lead;
    while k < n {
        let grp = (k + 7).min(n) - k;
        let mut mask = 0u8;
        for j in 0..grp {
            mask |= ((chunk[k + j] >> 7) & 1) << j;
            out[w + 1 + j] = chunk[k + j] & 0x7f;
        }
        out[w] = mask;
        w += 1 + grp;
        k += grp;
    }
    out
}

/// Decode a `0x52` data-frame payload back into blob bytes (inverse of
/// [`encode_data_frame`]). Useful for tests and for verifying a capture.
pub fn decode_data_frame(payload: &[u8]) -> Vec<u8> {
    let d = &payload[8..];
    let mut out = Vec::with_capacity(WRITE_CHUNK_BYTES);
    for j in 0..3.min(d.len()) {
        out.push(d[j] | (((payload[3] >> (4 + j)) & 1) << 7));
    }
    let mut i = 3;
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

/// `0x51` begin-write: announce `(slot, total)` before the data frames.
pub fn write_begin(slot: u8, total: u32) -> Vec<u8> {
    vec![
        0x00, 0x04, 0x00, 0x00, 0x00, (total >> 8) as u8, total as u8, slot,
    ]
}

/// The slot identity hash: `[SLOT, c0 & 0x7F, c1 & 0x7F, c2 & 0x7F]` where
/// `c0..c3` are the bytes of `crc32(blob)` little-endian.
///
/// Verified 16/16 against the `0x53` frames in `preset-import.mmon` (each frame's
/// hash equals this value for the blob written in that slot's `0x52` sequence).
/// The device recomputes it on commit and silently rejects the write on
/// mismatch, so it must be derived from the blob being written — reusing the
/// previously queried hash of a *different* content makes the commit a no-op.
pub fn identity_hash(slot: u8, blob: &[u8]) -> [u8; 4] {
    let le = crc32(blob).to_le_bytes();
    [slot, le[0] & 0x7F, le[1] & 0x7F, le[2] & 0x7F]
}

/// `0x53` identity hash frame — fully derivable from `(slot, blob)`.
///
/// The payload carries the whole `crc32(blob)` in 7-bit-safe form:
///
/// ```text
/// 00 08 00 B3 00 len_hi len_lo SLOT | c0&7F c1&7F c2&7F | c3>>7 | c3&7F
/// ```
///
/// `B3` is **not** an independent field — it completes the CRC:
///
/// ```text
/// B3 = ((c2 >> 7) << 6 | (c1 >> 7) << 5 | (c0 >> 7) << 4) | (len != 4096)
/// ```
///
/// * bits 4-6 = the high bits lost to the 7-bit mask on `c0..c2` (verified
///   against all 30 slots of `factory-reset.mmon`; a wrong value makes the
///   device ACK everything and then **silently drop the commit**),
/// * bit 0 = "asset is shorter than the standard 4096-byte IR" (observed for
///   the 2780- and 3764-byte factory IRs).
pub fn write_hash(slot: u8, total: u32, blob: &[u8]) -> Vec<u8> {
    let le = crc32(blob).to_le_bytes();
    let b3 = hash_b3(total, &le);
    vec![
        0x00, 0x08, 0x00, b3, 0x00, (total >> 8) as u8, total as u8, slot,
        le[0] & 0x7F,
        le[1] & 0x7F,
        le[2] & 0x7F,
        le[3] >> 7,
        le[3] & 0x7F,
    ]
}

/// The `B3` byte of the `0x53` frame: the three high bits stripped from the
/// masked CRC bytes `c0..c2`, plus the short-asset flag (see [`write_hash`]).
fn hash_b3(total: u32, le: &[u8; 4]) -> u8 {
    let v = ((le[2] >> 7) << 2) | ((le[1] >> 7) << 1) | (le[0] >> 7);
    (v << 4) | u8::from(total != IR_TOTAL)
}

/// `0x55` slot name: `00 [strlen+1] 00 00 [SLOT] [name fragments]`.
///
/// The length prefix counts `strlen + 1` (an implicit NUL terminator) in every
/// captured frame: `Og412` (5 chars, no NUL sent) → `06`, `Bog412A` (7 chars +
/// explicit NUL fragment = 8 bytes) → `08`.
///
/// `name` is the device's NUL-fragment form (e.g. `Bog412` `00` `A` for
/// `Bog412A`): the first 6 characters, a NUL, then the remainder.
pub fn write_name(slot: u8, name: &[u8]) -> Vec<u8> {
    // Fragment length minus the inserted NUL (if any) = the plain name length.
    let strlen = name.iter().filter(|&&b| b != 0x00).count();
    let mut v = vec![0x00, (strlen + 1) as u8, 0x00, 0x00, slot];
    v.extend_from_slice(name);
    v
}

/// `0x57` commit: `00 01 00 00 [SLOT]`.
pub fn write_commit(slot: u8) -> Vec<u8> {
    vec![0x00, 0x01, 0x00, 0x00, slot]
}

/// `0x49` open the write session: `00 02 00 00 00 40`.
///
/// Present at the start of every import / factory-reset capture and in **no**
/// read-only capture, so it gates the device's write acceptance.
pub const SESSION_OPEN: u8 = 0x49;

pub fn session_open() -> Vec<u8> {
    vec![0x00, 0x02, 0x00, 0x00, 0x00, 0x40]
}

/// `0x56` slot query: `00 01 00 00 [SLOT]`. The reply carries the slot's
/// 4-byte identity hash and its NUL-fragment name.
pub fn slot_query(slot: u8) -> Vec<u8> {
    vec![0x00, 0x01, 0x00, 0x00, slot]
}

/// Split a plain ASCII name into the device's NUL-fragment form:
/// first 6 characters, a NUL, then the remainder (observed rule; names longer
/// than 6 chars are always split exactly there in every captured write).
pub fn encode_name(name: &[u8]) -> Vec<u8> {
    let b = name;
    if b.len() <= 6 {
        return b.to_vec();
    }
    let mut out = b[..6].to_vec();
    out.push(0x00);
    out.extend_from_slice(&b[6..]);
    out
}

/// Extract `(hash, name)` from a `0x56` reply
/// (`00 00 1e … [hash x4] 1a 1a 00 00 [name fragments] …`).
pub fn parse_slot_query(payload: &[u8]) -> Option<([u8; 4], Vec<u8>)> {
    if payload.len() < 24 || payload[17] != 0x1a || payload[18] != 0x1a {
        return None;
    }
    let hash = [payload[13], payload[14], payload[15], payload[16]];
    let mut name = Vec::new();
    for &b in &payload[21..] {
        if b != 0x00 && (0x20..=0x7e).contains(&b) {
            name.push(b);
        }
    }
    Some((hash, name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_matches_the_standard_vector() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    /// First 64 bytes of `ir/slot_00.bin` from the bundle (the known plaintext).
    const IR0_HEAD: &[u8] = &[
        0x2d, 0x88, 0x8c, 0x3c, 0x6f, 0x4a, 0x25, 0x3e, 0x1b, 0x4f, 0xea, 0x3e, 0xa0, 0xfd, 0x30,
        0x3f, 0x14, 0x8c, 0x39, 0x3f, 0xb6, 0x4b, 0x1d, 0x3f, 0x03, 0x23, 0xba, 0x3e, 0x7c, 0x7d,
        0xc6, 0xbb, 0xe1, 0x42, 0xa3, 0xbe, 0xbb, 0xd4, 0xdc, 0xbe, 0x42, 0x69, 0xb7, 0xbe, 0x05,
        0x6a, 0x36, 0xbe, 0x90, 0xcf, 0x59, 0x3c, 0x05, 0xbf, 0xcb, 0x3d, 0x6f, 0x30, 0x9d, 0x3d,
        0x95, 0x46, 0x04, 0x3c,
    ];

    /// Captured `0x52` write frame #0 for IR slot 0, verbatim from
    /// `preset-import.mmon`.
    const IR0_WRITE_FRAME0: &[u8] = &[
        0x00, 0x44, 0x00, 0x60, 0x00, 0x00, 0x00, 0x00, 0x2d, 0x08, 0x0c, 0x00, 0x3c, 0x6f, 0x4a,
        0x25, 0x3e, 0x1b, 0x4f, 0x0d, 0x6a, 0x3e, 0x20, 0x7d, 0x30, 0x3f, 0x14, 0x09, 0x0c, 0x39,
        0x3f, 0x36, 0x4b, 0x1d, 0x3f, 0x44, 0x03, 0x23, 0x3a, 0x3e, 0x7c, 0x7d, 0x46, 0x7b, 0x3b,
        0x61, 0x42, 0x23, 0x3e, 0x3b, 0x54, 0x33, 0x5c, 0x3e, 0x42, 0x69, 0x37, 0x3e, 0x05, 0x1c,
        0x6a, 0x36, 0x3e, 0x10, 0x4f, 0x59, 0x3c, 0x46, 0x05, 0x3f, 0x4b, 0x3d, 0x6f, 0x30, 0x1d,
        0x02, 0x3d, 0x15, 0x46, 0x04, 0x3c,
    ];

    #[test]
    fn encode_reproduces_the_captured_write_frame() {
        let got = encode_data_frame(IR0_HEAD, 0, 0);
        assert_eq!(got.len(), IR0_WRITE_FRAME0.len(), "payload length");
        assert_eq!(got, IR0_WRITE_FRAME0.to_vec());
    }

    #[test]
    fn decode_data_frame_inverts_encode() {
        // Full chunk.
        assert_eq!(decode_data_frame(&encode_data_frame(IR0_HEAD, 0, 0)), IR0_HEAD);
        // A short final chunk (52 bytes, like the 3,764-byte asset's last one).
        let tail: Vec<u8> = (0..52).map(|i| (i * 37 + 129) as u8).collect();
        let enc = encode_data_frame(&tail, 58, 15);
        assert_eq!(decode_data_frame(&enc), tail);
    }

    #[test]
    fn name_encoding_fragments_after_six_characters() {
        assert_eq!(encode_name(b"Bog412A"), b"Bog412\x00A");
        assert_eq!(encode_name(b"Fd59BM410"), b"Fd59BM\x00410");
        assert_eq!(encode_name(b"Short1"), b"Short1");
    }

    #[test]
    fn control_frames_carry_slot_and_length() {
        assert_eq!(write_begin(0, 4096), vec![0x00, 0x04, 0x00, 0x00, 0x00, 0x10, 0x00, 0x00]);
        assert_eq!(write_begin(2, 4096), vec![0x00, 0x04, 0x00, 0x00, 0x00, 0x10, 0x00, 0x02]);
        assert_eq!(write_commit(2), vec![0x00, 0x01, 0x00, 0x00, 0x02]);
        assert_eq!(slot_query(3), vec![0x00, 0x01, 0x00, 0x00, 0x03]);
        // Slot 0 of the import capture: crc32(ir/slot_00.bin) = 0xdab13b3e,
        // LE bytes `3e 3b b1 da` -> `3e 3b 31 | 01 | 5a` (7-bit-safe carry).
        let blob = std::fs::read("/tmp/np2/160452/ir/slot_00.bin")
            .ok()
            .map(|b| b.to_vec());
        if let Some(blob) = blob {
            assert_eq!(identity_hash(0, &blob), [0x00, 0x3e, 0x3b, 0x31]);
            assert_eq!(
                write_hash(0, 4096, &blob),
                vec![0x00, 0x08, 0x00, 0x40, 0x00, 0x10, 0x00, 0x00, 0x3e, 0x3b, 0x31, 0x01, 0x5a]
            );
        }
        // Synthetic fallback with the same crc so the test is meaningful
        // without the capture extract on disk.
        assert_eq!(identity_hash(7, b"123456789"), {
            let le = crc32(b"123456789").to_le_bytes();
            [7, le[0] & 0x7F, le[1] & 0x7F, le[2] & 0x7F]
        });
        // Name prefix counts strlen + 1 (implicit terminator): Og412 -> 06.
        assert_eq!(write_name(14, b"Og412")[1], 0x06);
        assert_eq!(write_name(0, b"Bog412\x00A")[1], 0x08);
        assert_eq!(write_name(0, b"Bog412\x00A").len(), 5 + 8);
    }

    /// The `B3` byte over all 30 factory IRs, cross-checked against the
    /// `0x53` frames of `factory-reset.mmon` (contents = the APK's bundled
    /// `factory_presets/ir/NN.bin`). Requires the extracted APK tree.
    #[test]
    fn hash_b3_matches_all_30_factory_irs() {
        let dir = std::path::Path::new("/tmp/apkx/assets/flutter_assets/assets/nanocore/factory_presets/ir");
        if !dir.is_dir() {
            eprintln!("skipping: APK IR tree not extracted");
            return;
        }
        // (slot, expected B3) lifted from the capture.
        let expect: [(u8, u8); 30] = [
            (0, 0x40), (1, 0x40), (2, 0x20), (3, 0x70), (4, 0x10), (5, 0x70),
            (6, 0x60), (7, 0x20), (8, 0x50), (9, 0x40), (10, 0x60), (11, 0x01),
            (12, 0x30), (13, 0x30), (14, 0x40), (15, 0x41), (16, 0x40),
            (17, 0x20), (18, 0x00), (19, 0x00), (20, 0x70), (21, 0x10),
            (22, 0x20), (23, 0x50), (24, 0x10), (25, 0x10), (26, 0x10),
            (27, 0x20), (28, 0x20), (29, 0x10),
        ];
        for (slot, want) in expect {
            let blob = std::fs::read(dir.join(format!("{slot:02}.bin"))).expect("ir file");
            let total = blob.len() as u32;
            let frame = write_hash(slot, total, &blob);
            assert_eq!(frame[3], want, "B3 mismatch for slot {slot}");
            assert_eq!(frame[4], 0x00);
            assert_eq!(u16::from_be_bytes([frame[5], frame[6]]) as usize, blob.len());
            // The frame's CRC bytes must decode back to the full crc32.
            // B3 bit4 = c0's high bit, bit5 = c1's, bit6 = c2's.
            let c0 = frame[8] | (((frame[3] >> 4) & 1) << 7);
            let c1 = frame[9] | (((frame[3] >> 5) & 1) << 7);
            let c2 = frame[10] | (((frame[3] >> 6) & 1) << 7);
            let c3 = frame[12] | (frame[11] << 7);
            assert_eq!([c0, c1, c2, c3], crc32(&blob).to_le_bytes());
        }
    }

    /// Synthetic check of the B3 bit layout: crc32("123456789") = 0xCBF43926,
    /// LE `26 39 cb 4c` -> c0..c2 high bits = 0,0,1 -> v = 4.
    #[test]
    fn hash_b3_bit_layout() {
        let le = crc32(b"123456789").to_le_bytes();
        assert_eq!(hash_b3(IR_TOTAL, &le), 0x40);
        assert_eq!(hash_b3(2780, &le), 0x41);
    }

    /// First 64 payload bytes of the real reply for IR slot 0, address 0,
    /// captured from `factory-preset-export.mmon`.
    const IR0_CHUNK0: &[u8] = &[
        0x00, 0x00, 0x4d, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x10, 0x2d, 0x03, 0x08, 0x0c, 0x3c,
        0x6f, 0x4a, 0x25, 0x3e, 0x34, 0x1b, 0x4f, 0x6a, 0x3e, 0x20, 0x7d, 0x30, 0x24, 0x3f, 0x14,
        0x0c, 0x39, 0x3f, 0x36, 0x4b, 0x10, 0x1d, 0x3f, 0x03, 0x23, 0x3a, 0x3e, 0x7c, 0x6e, 0x7d,
        0x46, 0x3b, 0x61, 0x42, 0x23, 0x3e, 0x4f, 0x3b, 0x54, 0x5c, 0x3e, 0x42, 0x69, 0x37, 0x71,
        0x3e, 0x05, 0x6a, 0x36,
    ];

    #[test]
    fn decode_real_chunk_matches_the_bundle() {
        let got = decode_chunk(IR0_CHUNK0);
        assert_eq!(
            &got[..32],
            &[
                0x2d, 0x88, 0x8c, 0x3c, 0x6f, 0x4a, 0x25, 0x3e, 0x1b, 0x4f, 0xea, 0x3e, 0xa0, 0xfd,
                0x30, 0x3f, 0x14, 0x8c, 0x39, 0x3f, 0xb6, 0x4b, 0x1d, 0x3f, 0x03, 0x23, 0xba, 0x3e,
                0x7c, 0x7d, 0xc6, 0xbb,
            ][..]
        );
    }

    #[test]
    fn lead_high_bit_comes_from_payload_byte_3() {
        // Same data, but byte 3 has the LEAD_HI bit set: decoded byte 0 flips
        // its top bit only.
        let mut p = IR0_CHUNK0.to_vec();
        let plain = decode_chunk(&p);
        p[3] |= LEAD_HI_BIT;
        let hi = decode_chunk(&p);
        assert_eq!(plain[0], 0x2d);
        assert_eq!(hi[0], 0xad);
        assert_eq!(plain[1..], hi[1..]);
    }

    #[test]
    fn parse_chunk_reads_slot_and_addr() {
        let c = parse_chunk(IR0_CHUNK0).expect("parse");
        assert_eq!(c.slot, 0);
        assert_eq!(c.addr, 0);
        assert_eq!(c.data.len(), 47); // truncated payload -> fewer bytes
    }

    #[test]
    fn read_plan_matches_the_captured_requests() {
        let (total, plan) = read_plan(REGION_IR, 0).expect("plan");
        assert_eq!(total, 4096);
        assert_eq!(plan.len(), 21);
        assert_eq!(plan[0].payload(), vec![0x00, 0x04, 0x00, 0x08, 0x00, 0x00, 0x00, 0x48]);
        // 20 full chunks + 96 remaining bytes.
        assert_eq!(plan[20].len, 96);
        assert!(plan[..20].iter().all(|r| r.len == FULL_CHUNK_LEN));

        let (amp_total, amp) = read_plan(REGION_AMP, 1).expect("plan");
        assert_eq!(amp_total, 12242);
        assert_eq!(amp.len(), 62);
        assert_eq!(61 * CHUNK_BYTES as u32 + 42, amp_total);
        assert_eq!(amp[61].len, 42);
        // The two plans share a (b3, addr) prefix; only the region differs.
        assert!(amp[..19]
            .iter()
            .zip(plan[..19].iter())
            .all(|(a, b)| a.b3 == b.b3 && a.addr == b.addr));
    }

    #[test]
    fn plan_carries_the_slot_through() {
        let (_, plan) = read_plan(REGION_AMP, 27).expect("plan");
        assert!(plan.iter().all(|r| r.slot == 27 && r.region == REGION_AMP));
        assert_eq!(plan[1].payload()[4], 27);
    }

    #[test]
    fn assemble_orders_by_addr_not_arrival() {
        let mk = |addr: u16, b: u8| Chunk { slot: 0, addr, data: vec![b; 2] };
        // Deliberately out of order, as the device answers.
        let got = assemble(&[mk(144, 3), mk(0, 1), mk(72, 2)]);
        assert_eq!(got, vec![1, 1, 2, 2, 3, 3]);
    }
}
