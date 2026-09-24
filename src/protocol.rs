//! Payload builders and parsers for the confirmed message types.
//!
//! All byte layouts here are derived from real MIDI captures.

use crate::field;
use crate::msg_type;
use crate::sub;

/// Encode a value as a little-endian float32 (raw, NOT wire-safe).
///
/// Use this only for decoding/inspection. To send a parameter value use
/// [`encode_param_value`], which produces bytes the device can actually
/// receive over MIDI.
pub fn encode_param_float(value: f32) -> [u8; 4] {
    value.to_le_bytes()
}

/// Decode a little-endian float32.
pub fn decode_param_float(bytes: &[u8]) -> Option<f32> {
    if bytes.len() < 4 {
        return None;
    }
    Some(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
}

/// Encode a parameter value for transmission — **this is the wire format.**
///
/// The value is a 32-bit IEEE754 float, but it travels inside a SysEx message
/// where every data byte must be `< 0x80`, so only 28 bits of the float reach
/// the device (the high bit of each of the 4 bytes is always forced to 0).
/// The official app deals with this by sending `float32(value)` with the high
/// bit of every byte cleared — and **so do we**. This is verified two ways:
///
/// * Every one of the 481 writes in `scream-knob-sweep.mmon` has all
///   four value bytes `< 0x80` — i.e. the app clears the high bits.
/// * A live round-trip (`setparam 0 0 0 0.2` -> snapshot) stored exactly
///   `4e 4c 4c 3e` (= 0.19951), i.e. the device keeps the cleared float.
///
/// Precision is therefore 28 bits. The four lost bits are bits 7/15/23/31 of
/// the float: bits 7 and 15 are low mantissa bits (negligible), bit 31 is the
/// sign (always 0 for positive values), but **bit 23 is the exponent's LSB** —
/// so a value whose float32 exponent is odd collapses to the next lower *even*
/// exponent (worst case ≈ 2× error, e.g. `0.3 -> 0.1495`). That is inherent to
/// the hardware, not a bug; the real app lives with the same quantization.
pub fn encode_param_value(value: f32) -> [u8; 4] {
    let b = value.to_le_bytes();
    [b[0] & 0x7F, b[1] & 0x7F, b[2] & 0x7F, b[3] & 0x7F]
}

// ---------------------------------------------------------------------------
// Builders
// ---------------------------------------------------------------------------

/// `0x6D` sub `0x02`: set a scalar field (preset / cab / variant).
pub fn set_field(field_id: u8, value: u8) -> (u8, Vec<u8>) {
    (msg_type::DATA, vec![0x00, sub::SET_FIELD, 0x00, 0x00, field_id, value])
}

/// Select a preset by 0-based index.
pub fn select_preset(index: u8) -> (u8, Vec<u8>) {
    set_field(field::PRESET, index)
}

/// Select a cabinet / IR by index.
pub fn select_cab(index: u8) -> (u8, Vec<u8>) {
    set_field(field::CAB, index)
}

/// Select an amp model / effect variant.
pub fn select_variant(variant: u8) -> (u8, Vec<u8>) {
    set_field(field::VARIANT, variant)
}

/// `0x6D` sub `0x03`: enable / disable an effect slot.
pub fn set_enabled(slot: u8, enabled: bool) -> (u8, Vec<u8>) {
    (msg_type::DATA, vec![0x00, sub::ENABLE, 0x00, 0x00, 0x03, slot, enabled as u8])
}

/// `0x6D` sub `0x07`: write a single parameter.
///
/// Wire layout, derived from every `0x07` message in the captures:
///
/// ```text
/// 00 07 00 [b3] 01 [b5] [b6] <float32 x 4>
/// ```
///
/// * `b6` — **parameter index** within the effect. Verified from
///   `noise-gate-tweak.mmon`: the noise gate exposes exactly two
///   params and `b6` takes exactly the values `0x00` (threshold) and
///   `0x01` (release).
/// * `b5` — **slot / effect instance**. `0x00` for every write in the fx1
///   capture, `0x01` for every write in the fx2 capture.
/// * `b3` — cycles through `0x00, 0x08, 0x10, 0x18, 0x20, 0x28, 0x30, 0x38`
///   (all combinations of bits 0x08/0x10/0x20) *within a single knob drag*, so
///   it is **not** part of the parameter address. It is an app-side fragment
///   marker: the official app re-sends the full float up to 8 times, one per
///   differing nibble. **Verified on live hardware that a single frame with
///   any `b3` (we used [`DEFAULT_B3`]) is fully accepted and updates the whole
///   parameter** — so for writing from this library `b3` can be left at 0.
///
/// The value is encoded with [`encode_param_value`] (a 7-bit-safe float32).
pub fn param_write(b3: u8, b5: u8, b6: u8, value: f32) -> (u8, Vec<u8>) {
    let mut p = vec![0x00, sub::PARAM_WRITE, 0x00, b3, 0x01, b5, b6];
    p.extend_from_slice(&encode_param_value(value));
    (msg_type::DATA, p)
}

/// `b3` value the captures use most often.
pub const DEFAULT_B3: u8 = 0x00;

// ---------------------------------------------------------------------------
// Effect chain order (`chainOrder`)
// ---------------------------------------------------------------------------

/// Number of effects in the chain.
pub const CHAIN_LEN: usize = 8;

/// Chain role table: `index -> (cli name, display name)`.
///
/// **This order is NOT the CC-block order.** The standard MIDI CC map lists the
/// blocks as `FX1 FX2 AMP CAB MOD DELAY REVERB EQ` (CC25 = Mod, CC26 = Delay),
/// whereas the chain uses `DEL` *before* `MOD`. Verified from
/// `enable-del-eq.mmon`: the user enabled DEL and EQ and the device received
/// `ENABLE slot=4` and `ENABLE slot=7`, so `DEL == 4`, `MOD == 5`, `EQ == 7`.
pub const CHAIN_ROLES: [(&str, &str); CHAIN_LEN] = [
    ("fx1", "FX1"),
    ("fx2", "FX2"),
    ("amp", "AMP"),
    ("cab", "CAB"),
    ("delay", "DEL"),
    ("mod", "MOD"),
    ("reverb", "REV"),
    ("eq", "EQ"),
];

/// Resolve a chain role by CLI/display name (case-insensitive).
///
/// Accepts both the short CLI names (`delay`, `reverb`) and the device display
/// names (`del`, `rev`), plus the obvious aliases.
pub fn chain_role_index(name: &str) -> Option<u8> {
    let n = name.trim().to_ascii_lowercase();
    let alias: &str = match n.as_str() {
        "del" | "dly" => "delay",
        "rev" | "r" => "reverb",
        "mod" | "modulation" => "mod",
        "b" | "box" => "reverb",
        other => other,
    };
    CHAIN_ROLES
        .iter()
        .position(|(cli, disp)| *cli == alias || disp.to_ascii_lowercase() == alias)
        .map(|i| i as u8)
}

/// Device display name for a chain role index.
pub fn chain_role_name(index: u8) -> Option<&'static str> {
    CHAIN_ROLES.get(index as usize).map(|(_, disp)| *disp)
}

/// Insert the fixed `0x00` filler this message layout requires.
fn chain_encode(order: &[u8], insert_at: usize) -> Vec<u8> {
    let mut v = Vec::with_capacity(CHAIN_LEN + 1);
    v.extend_from_slice(&order[..insert_at]);
    v.push(0x00);
    v.extend_from_slice(&order[insert_at..]);
    v
}

/// Remove the fixed `0x00` filler from a wire field.
///
/// Returns `None` unless the byte at `remove_at` really is `0x00` *and* what
/// remains is a permutation of `0..CHAIN_LEN`. That strictness is deliberate:
/// the encoded field contains two `0x00` bytes (the filler and a legitimate
/// `FX1` element), so a lenient decoder could silently produce a wrong chain.
fn chain_decode(encoded: &[u8], remove_at: usize) -> Option<Vec<u8>> {
    if encoded.len() != CHAIN_LEN + 1 || encoded.get(remove_at) != Some(&0x00) {
        return None;
    }
    let mut v = encoded.to_vec();
    v.remove(remove_at);
    let mut sorted = v.clone();
    sorted.sort_unstable();
    if sorted != (0..CHAIN_LEN as u8).collect::<Vec<_>>() {
        return None;
    }
    Some(v)
}

/// Filler index used by the `0x6D` chain **write** layout (see [`chain_write`]).
pub const CHAIN_WRITE_FILLER: usize = 5;
/// Filler index used by the `0x09` device **notification** layout.
pub const CHAIN_NOTIFY_FILLER: usize = 1;
/// Filler index used by the `0x43` preset-write layout.
pub const CHAIN_PRESET_FILLER: usize = 2;

/// `0x6D` sub `0x0A`, field `0x05`: write the effect chain order.
///
/// `order` is a permutation of `0..8` giving the chain from left to right, using
/// [`CHAIN_ROLES`] indices. Wire layout:
///
/// ```text
/// 00 0A 00 00 05 08 <9 bytes>
/// ```
///
/// The 9-byte field is the 8-element `chainOrder` with a `0x00` inserted at
/// index [`CHAIN_WRITE_FILLER`]. Returns `None` if `order` is not a permutation.
pub fn chain_write(order: &[u8]) -> Option<(u8, Vec<u8>)> {
    if order.len() != CHAIN_LEN {
        return None;
    }
    chain_decode(&chain_encode(order, CHAIN_WRITE_FILLER), CHAIN_WRITE_FILLER)?;
    let mut p = vec![0x00, sub::NAME, 0x00, 0x00, 0x05, CHAIN_LEN as u8];
    p.extend_from_slice(&chain_encode(order, CHAIN_WRITE_FILLER));
    Some((msg_type::DATA, p))
}

/// Parse the `0x6D` chain-write payload.
pub fn parse_chain_write(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() != 15 || payload[1] != sub::NAME || payload[4] != 0x05 {
        return None;
    }
    chain_decode(&payload[6..15], CHAIN_WRITE_FILLER)
}

/// Parse the 9-byte field out of a `0x09` device notification payload
/// (`00 08` + 9 bytes).
pub fn parse_chain_notify(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 11 || payload[1] != CHAIN_LEN as u8 {
        return None;
    }
    chain_decode(&payload[2..11], CHAIN_NOTIFY_FILLER)
}

/// Filler index used by the `0x6D` **ACK** the device returns for a chain write.
///
/// The ACK mirrors the request (`00 00` then the request payload from byte 1)
/// but re-encodes the 9-byte field with a **different** filler index than the
/// write used. Verified on four samples — two capture ACKs and two live ACKs
/// from real hardware — where removing index 4 yields exactly the order that
/// was requested. Note the field is inherently ambiguous in isolation (it
/// contains two `0x00` bytes), so this index matters.
pub const CHAIN_ACK_FILLER: usize = 4;

/// Parse the chain order out of the `0x6D` ACK payload.
///
/// The ACK is `00 00` followed by the request payload from its second byte, so
/// the sub-command id sits at `[2]`, the field selector at `[5]`, and the
/// 9-byte chain field at `[7..16)`. Live payload observed for a requested
/// `[4,7,0,1,2,3,5,6]`:
///
/// ```text
/// 00 00 0a 00 00 05 08 04 07 00 01 00 02 03 05 06
/// ```
pub fn parse_chain_ack(payload: &[u8]) -> Option<Vec<u8>> {
    if payload.len() < 16 || payload[2] != sub::NAME || payload[5] != 0x05 {
        return None;
    }
    chain_decode(&payload[7..16], CHAIN_ACK_FILLER)
}

/// `0x6D` sub `0x0A`: rename the current preset.
///
/// Note: the official app inserts a NUL byte inside longer names
/// (`"preset42"` -> `"prese\0t42"`); this writes the given bytes verbatim.
pub fn set_name(name: &[u8]) -> (u8, Vec<u8>) {
    let mut p = vec![0x00, sub::NAME, 0x00, 0x00, 0x08, name.len() as u8];
    p.extend_from_slice(name);
    (msg_type::DATA, p)
}

/// `0x6D` effect-type switch.
///
/// Wire layout (verified for fx1):
/// ```text
/// 00 [4*(N+1)] 00 30 [slot] 00 [effect_index] [N] <float32 * N>
/// ```
/// * `effect_index` is the 0-based option index within the slot
///   (fx1: 0 = noise gate, 1 = auto gate, 2 = NG+COM, 3 = compressor).
/// * `N` is the parameter count.
pub fn load_effect(slot: u8, effect_index: u8, model_flag: u8, params: &[f32]) -> (u8, Vec<u8>) {
    let n = params.len() as u8;
    let sub_byte = 4 * (params.len() as u8 + 1);
    let mut p = vec![0x00, sub_byte, 0x00, model_flag, slot, 0x00, effect_index, n];
    for &v in params {
        p.extend_from_slice(&encode_param_value(v));
    }
    (msg_type::DATA, p)
}

/// `0x66` sub `0x02`: set USB volume (0-100).
pub fn set_usb_volume(percent: u8) -> (u8, Vec<u8>) {
    (msg_type::USB_VOLUME, vec![0x00, 0x02, 0x00, 0x00, field::USB_VOLUME, percent])
}

/// `0x63`: request the current preset snapshot.
///
/// Payload is `00 00 00`. In captures the command is always followed by the
/// metadata tail (`da 00 0e ...`); see [`crate::tail`].
pub fn request_snapshot() -> (u8, Vec<u8>) {
    (msg_type::SNAPSHOT, vec![0x00, 0x00, 0x00])
}

/// `0x40`: query preset names for `count` slots starting at `start_slot`.
///
/// Wire layout: `00 02 00 00 [start_slot] [count]`. Captures use
/// `start_slot` in `0x00, 0x08, 0x10, ..` and `count = 0x08`.
pub fn query_preset_list(start_slot: u8, count: u8) -> (u8, Vec<u8>) {
    (msg_type::PRESET_LIST, vec![0x00, 0x02, 0x00, 0x00, start_slot, count])
}

/// `0x68`: connectivity ping.
pub fn ping() -> (u8, Vec<u8>) {
    (msg_type::PING, vec![0x00, 0x00, 0x00])
}

/// `0x36`: query the model name of a single amp / drive slot.
///
/// Observed TX query payload (all 38 slots in `resona-import.mmon`):
///
/// ```text
/// 00 01 00 00 [index]
/// ```
///
/// The device replies with a `0x36` frame carrying the model name; see
/// [`parse_model_name`]. `index` is the 0-based firmware slot (0-37 for amp +
/// FX2 drive pedals). The cab / IR slots (0-29) use a different message and
/// are NOT enumerated by this query — see the protocol notes.
pub fn model_query(index: u8) -> (u8, Vec<u8>) {
    (msg_type::MODEL_LIST, vec![0x00, 0x01, 0x00, 0x00, index])
}

// ---------------------------------------------------------------------------
// Parsers
// ---------------------------------------------------------------------------

/// One effect-parameter group from a `0x06` message.
#[derive(Debug, Clone, PartialEq)]
pub struct EffectParams {
    pub slot: u8,
    pub params: Vec<f32>,
}

/// Parse an `0x06` effect-data payload:
/// `00 01 [slot] [count] <float32 * count>`.
pub fn parse_effect_data(payload: &[u8]) -> Option<EffectParams> {
    if payload.len() < 4 || payload[0] != 0x00 || payload[1] != 0x01 {
        return None;
    }
    let slot = payload[2];
    let count = payload[3] as usize;
    let body = &payload[4..];
    if body.len() < count * 4 {
        return None;
    }
    let params = (0..count)
        .map(|i| decode_param_float(&body[i * 4..]).unwrap())
        .collect();
    Some(EffectParams { slot, params })
}

/// A named preset-list entry from a `0x40` response.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetEntry {
    pub slot: u8,
    pub named: bool,
    pub name: String,
}

/// Number of characters in a stored preset name. Names are space-padded to
/// this width, and are stored with `0x00` bytes inserted between fragments.
pub const PRESET_NAME_CHARS: usize = 8;

/// Parse a `0x40` preset-list response payload.
///
/// Layout — **validated end-to-end**, not inferred:
///
/// ```text
/// [0..7]  header: 00 00 52 00 00 BASE COUNT      (BASE = first slot, COUNT = 8)
/// then COUNT records:
///   [slot: 1 byte, MUST equal BASE + k]
///   [1+ bytes < 0x20   -- the first one is 0x01 when the slot holds a name]
///   [name: exactly 8 printable characters]
/// ```
///
/// The name field is stored as **NUL-separated fragments**: the `0x00` bytes
/// are skipped and do **not** count towards the 8 characters. That is why a
/// record's byte length varies (11 or 12) even though every name is 8 chars
/// wide. After the 8 characters at most one further byte `< 0x20` may precede
/// the next record's slot index.
///
/// Validation evidence:
/// * `preset-export.mmon` — all 8 chunks (64 slots) decode with every slot index
///   matching `BASE + k`, and every name coming out exactly 8 chars wide.
/// * The 40 factory names decoded this way are **byte-identical** to
///   the factory preset table → `products.nanocore.presets`
///   (independent source). Slots 41..63 carry the device placeholder `preset`.
///
/// Every record is cross-checked, so any misalignment returns `None` instead of
/// silently producing wrong names.
///
/// `named` is `true` when the lead padding bytes after the slot index are **not
/// all zero**. Validated on the capture: all 40 factory slots plus the user
/// preset in slot 40 are flagged, while the 24 placeholder slots (41..63, whose
/// stored name is literally `preset`) have all-zero lead bytes. The exact
/// byte-level meaning of those 1-2 bytes is not fully explained, so `named`
/// should be treated as "the device flags this slot as holding a name", not as
/// a documented flag value.
pub fn parse_preset_list(payload: &[u8]) -> Option<Vec<PresetEntry>> {
    // Header is exactly 7 bytes and starts with the `0x52` body marker.
    if payload.len() < 8 || payload[2] != 0x52 {
        return None;
    }
    let base = payload[5];
    let count = payload[6] as usize;
    if count == 0 || count > 64 {
        return None;
    }

    let mut out = Vec::with_capacity(count);
    let mut i = 7usize;
    for k in 0..count {
        let slot = base.wrapping_add(k as u8);
        // 1. slot index, cross-checked against the expected position.
        if i >= payload.len() || payload[i] != slot {
            return None;
        }
        i += 1;

        // 2. one or more lead bytes < 0x20; `named` = not all zero.
        if i >= payload.len() || payload[i] >= 0x20 {
            return None;
        }
        let mut named = false;
        while i < payload.len() && payload[i] < 0x20 {
            named |= payload[i] != 0x00;
            i += 1;
        }

        // 3. exactly PRESET_NAME_CHARS printable chars, NULs skipped.
        let mut name = String::with_capacity(PRESET_NAME_CHARS);
        while i < payload.len() && name.len() < PRESET_NAME_CHARS {
            let b = payload[i];
            if b == 0x00 {
                i += 1;
            } else if (0x20..=0x7E).contains(&b) {
                name.push(b as char);
                i += 1;
            } else {
                return None;
            }
        }
        if name.len() != PRESET_NAME_CHARS {
            return None;
        }
        out.push(PresetEntry { slot, named, name: name.trim_end().to_string() });

        // 4. at most one trailing byte < 0x20, but never the next slot index.
        //    (Named slots can hold a value < 0x20, so compare against the
        //    expected next index rather than against a threshold.)
        let next_slot = base.wrapping_add(k as u8).wrapping_add(1);
        if i < payload.len() && payload[i] < 0x20 && payload[i] != next_slot {
            i += 1;
        }
    }
    Some(out)
}

/// Decode all plausible 7-bit-safe float32 values in a raw snapshot payload.
///
/// Returns `(offset, value)` pairs for values in `[0.0, 1.0]`.
pub fn scan_snapshot_floats(payload: &[u8]) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    if payload.len() < 4 {
        return out;
    }
    for i in 0..=payload.len() - 4 {
        let b = [payload[i], payload[i + 1], payload[i + 2], payload[i + 3]];
        let v = f32::from_le_bytes(b);
        if v.is_finite() && (0.0..=1.0).contains(&v) && v.to_le_bytes() == b {
            out.push((i, v));
        }
    }
    out
}

/// Parse `chainOrder` out of a `0x43` preset-write payload.
///
/// Two layouts have been observed:
///
/// * **234-byte preset frame** (40 of them in `factory-reset.mmon`, one per
///   factory preset): the chain field sits at `[33..42)`, preceded by the
///   field id `0x03` at `[31]` and the count `0x08` at `[32]`. The 9-byte field
///   is the 8-element order with a `0x00` inserted at index
///   [`CHAIN_PRESET_FILLER`]. Verified against `.npreset`'s
///   `manifest.json` -> `presets[].preset.chainOrder` for **40/40** presets,
///   including a non-identity order (`[0,1,2,3,5,4,6,7]`).
/// * **Legacy 356-byte "first message"**: field id at `[30]`, count at `[31]`,
///   order from `[32]` (no filler). Kept for the older analysis path.
pub fn parse_chain_order(payload: &[u8]) -> Option<Vec<u8>> {
    // 234-byte preset frame.
    if payload.len() >= 42 && payload[31] == 0x03 && payload[32] == CHAIN_LEN as u8 {
        return chain_decode(&payload[33..42], CHAIN_PRESET_FILLER);
    }
    // Legacy 356-byte layout (no filler byte).
    if payload.len() < 40 || payload[30] != 0x03 {
        return None;
    }
    let count = payload[31] as usize;
    if count == 0 || count > 16 || payload.len() < 32 + count {
        return None;
    }
    Some(payload[32..32 + count].to_vec())
}

/// A model-list entry parsed from a `0x36` response payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelName {
    /// 0-based firmware slot (device UI shows `index + 1`).
    pub index: u8,
    /// The device's model string, reconstructed from NUL-separated fragments.
    pub name: String,
    /// `true` when byte 7 of the response is non-zero.
    ///
    /// **This is not a "user model" marker.** An earlier revision of this
    /// module assumed the opposite based on the `resona-import` capture,
    /// where slot 29 (a user-imported NAM) was the only flagged slot. The
    /// `preset-export` capture refutes that: it queries a single slot, 9, which
    /// holds the *factory* model `Mar8001`, and byte 7 is still `0x01`.
    ///
    /// The surviving hypothesis — consistent with both captures — is that byte
    /// 7 marks the slot whose model is **currently loaded / in use**, i.e. in
    /// the import capture the active amp was the user NAM in slot 29, and in
    /// the export capture the active amp was `Mar8001` in slot 9.
    ///
    /// To identify user/custom models, compare [`ModelName::name`] against the
    /// factory table in [`crate::models_data`] instead — that is
    /// flag-independent.
    pub active: bool,
}

/// Parse a `0x36` model-list response payload.
///
/// Layout (verified against all 38 entries in `resona-import.mmon` and
/// the single-slot query in `preset-export.mmon`):
///
/// ```text
/// 00 00 1e 10 00 [index] 01 [flag] 52 2f 00 [len] 00 [hash x4] 1a 1a 00 00 [name...]
/// ```
///
/// * `index` is at byte 5.
/// * `flag` is at byte 7 — see [`ModelName::active`]; it is *not* a user-model
///   flag.
/// * The model name sits immediately after the `1a 1a 00 00` marker and is
///   stored as a run of **NUL-separated printable fragments**. Concatenating
///   the fragments reconstructs the device's model string:
///   * `BogXTC` + `1`        -> `BogXTC1`
///   * `NDSP A` + `rchetyp` + `e:` -> `NDSP Archetype:`
///
/// Non-printable bytes and the separating NULs are dropped.
pub fn parse_model_name(payload: &[u8]) -> Option<ModelName> {
    if payload.len() < 6 {
        return None;
    }
    // Locate the `1a 1a 00 00` name marker.
    let mut marker = None;
    for i in 0..payload.len().saturating_sub(3) {
        if payload[i] == 0x1a && payload[i + 1] == 0x1a && payload[i + 2] == 0x00 && payload[i + 3] == 0x00 {
            marker = Some(i);
            break;
        }
    }
    let m = marker?;
    let index = payload[5];
    let active = payload.get(7).copied().unwrap_or(0) != 0;
    let mut name = String::new();
    for &b in &payload[m + 4..] {
        if b == 0x00 {
            continue;
        }
        if (0x20..=0x7E).contains(&b) {
            name.push(b as char);
        }
    }
    if name.is_empty() {
        return None;
    }
    Some(ModelName { index, name, active })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Raw `0x36` RX payload for amp slot 0 (BogXTC1) and slot 29 (NDSP
    // Archetype: after a RESONA NAM import), lifted verbatim from
    // `resona-import.mmon`.
    const SLOT0: &[u8] = &[
        0x00, 0x00, 0x1e, 0x10, 0x00, 0x00, 0x01, 0x00, 0x52, 0x2f, 0x00, 0x18, 0x00, 0x6a,
        0x33, 0x02, 0x57, 0x1a, 0x1a, 0x00, 0x00, 0x42, 0x6f, 0x67, 0x58, 0x54, 0x43, 0x00,
        0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    const SLOT29: &[u8] = &[
        0x00, 0x00, 0x1e, 0x10, 0x00, 0x1d, 0x01, 0x01, 0x52, 0x2f, 0x00, 0x0e, 0x00, 0x6b,
        0x7c, 0x21, 0x54, 0x1a, 0x1a, 0x00, 0x00, 0x4e, 0x44, 0x53, 0x50, 0x20, 0x41, 0x00,
        0x72, 0x63, 0x68, 0x65, 0x74, 0x79, 0x70, 0x00, 0x65, 0x3a, 0x00,
    ];
    // The single-slot query from `preset-export.mmon`: slot 9, which holds the
    // *factory* model `Mar8001`, yet byte 7 is 0x01. This is the counter-example
    // that proves byte 7 is not a user-model flag.
    const SLOT9_FACTORY_ACTIVE: &[u8] = &[
        0x00, 0x00, 0x1e, 0x10, 0x00, 0x09, 0x01, 0x01, 0x52, 0x2f, 0x00, 0x06, 0x00, 0x46,
        0x36, 0x31, 0x44, 0x1a, 0x1a, 0x00, 0x00, 0x4d, 0x61, 0x72, 0x38, 0x30, 0x30, 0x31,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
    ];

    #[test]
    fn model_query_is_5_bytes_with_index() {
        let (k, p) = model_query(7);
        assert_eq!(k, msg_type::MODEL_LIST);
        assert_eq!(p, vec![0x00, 0x01, 0x00, 0x00, 7]);
    }

    #[test]
    fn parse_model_name_reconstructs_fragments() {
        let a = parse_model_name(SLOT0).expect("slot0 parse");
        assert_eq!(a.index, 0);
        assert_eq!(a.name, "BogXTC1");
        assert!(!a.active);

        let b = parse_model_name(SLOT29).expect("slot29 parse");
        assert_eq!(b.index, 29);
        assert_eq!(b.name, "NDSP Archetype:");
        assert!(b.active);
    }

    /// Regression guard for the corrected byte-7 semantics: a *factory* model
    /// reports the flag too, so callers must not treat it as "user model".
    #[test]
    fn byte7_is_not_a_user_model_flag() {
        let m = parse_model_name(SLOT9_FACTORY_ACTIVE).expect("slot9 parse");
        assert_eq!(m.index, 9);
        assert_eq!(m.name, "Mar8001");
        assert!(m.active, "factory model Mar8001 also reports byte7 != 0");
    }

    #[test]
    fn parse_model_name_needs_marker() {
        // No `1a 1a 00 00` anchor -> None.
        assert!(parse_model_name(&[0x00, 0x00, 0x1e, 0x10, 0x00, 0x05]).is_none());
        // Truncated before the marker.
        assert!(parse_model_name(&[]).is_none());
    }

    // Real `0x40` PRESET_LIST replies from `preset-export.mmon`. All bytes below
    // are lifted verbatim; the decoded names were cross-checked against
    // the factory preset table -> products.nanocore.presets.
    const PRESET_LIST_BASE32: &[u8] = &[
        0x00, 0x00, 0x52, 0x00, 0x00, 0x20, 0x08, 0x20, 0x01, 0x50, 0x77, 0x00, 0x43, 0x68,
        0x6f, 0x72, 0x64, 0x20, 0x21, 0x00, 0x01, 0x52, 0x61, 0x77, 0x4c, 0x65, 0x61, 0x00,
        0x64, 0x20, 0x22, 0x01, 0x53, 0x77, 0x65, 0x00, 0x65, 0x70, 0x20, 0x20, 0x20, 0x23,
        0x01, 0x00, 0x4d, 0x75, 0x74, 0x65, 0x4f, 0x44, 0x20, 0x00, 0x20, 0x24, 0x01, 0x42,
        0x61, 0x73, 0x73, 0x00, 0x46, 0x69, 0x6e, 0x67, 0x25, 0x01, 0x42, 0x00, 0x61, 0x73,
        0x73, 0x53, 0x6c, 0x61, 0x70, 0x00, 0x26, 0x01, 0x42, 0x61, 0x73, 0x73, 0x50, 0x00,
        0x69, 0x63, 0x6b, 0x27, 0x01, 0x42, 0x61, 0x00, 0x73, 0x73, 0x46, 0x75, 0x7a, 0x7a,
    ];
    const PRESET_LIST_BASE40: &[u8] = &[
        0x00, 0x00, 0x52, 0x00, 0x00, 0x28, 0x08, 0x28, 0x01, 0x70, 0x69, 0x00, 0x74, 0x62,
        0x20, 0x20, 0x20, 0x20, 0x29, 0x00, 0x00, 0x70, 0x72, 0x65, 0x73, 0x65, 0x74, 0x00,
        0x20, 0x20, 0x2a, 0x00, 0x70, 0x72, 0x65, 0x00, 0x73, 0x65, 0x74, 0x20, 0x20, 0x2b,
        0x00, 0x00, 0x70, 0x72, 0x65, 0x73, 0x65, 0x74, 0x20, 0x00, 0x20, 0x2c, 0x00, 0x70,
        0x72, 0x65, 0x73, 0x00, 0x65, 0x74, 0x20, 0x20, 0x2d, 0x00, 0x70, 0x00, 0x72, 0x65,
        0x73, 0x65, 0x74, 0x20, 0x20, 0x00, 0x2e, 0x00, 0x70, 0x72, 0x65, 0x73, 0x65, 0x00,
        0x74, 0x20, 0x20, 0x2f, 0x00, 0x70, 0x72, 0x00, 0x65, 0x73, 0x65, 0x74, 0x20, 0x20,
    ];

    #[test]
    fn preset_list_decodes_factory_names() {
        // Slots 32..39 — includes slot bytes 0x20..0x27, which are printable,
        // so a naive "scan for the slot byte" would break here.
        let got = parse_preset_list(PRESET_LIST_BASE32).expect("base32 parse");
        let names: Vec<&str> = got.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(
            names,
            vec!["PwChord", "RawLead", "Sweep", "MuteOD", "BassFing", "BassSlap", "BassPick", "BassFuzz"]
        );
        assert!(got.iter().enumerate().all(|(k, e)| e.slot == 32 + k as u8));
        assert!(got.iter().all(|e| e.named));
    }

    #[test]
    fn preset_list_marks_placeholders_unnamed() {
        // Slot 40 is a user preset ("pitb"); 41..47 are device placeholders
        // whose stored name is literally `preset`.
        let got = parse_preset_list(PRESET_LIST_BASE40).expect("base40 parse");
        assert_eq!(got.len(), 8);
        assert_eq!(got[0].slot, 40);
        assert_eq!(got[0].name, "pitb");
        assert!(got[0].named);
        for e in &got[1..] {
            assert_eq!(e.name, "preset");
            assert!(!e.named, "slot {} should be an unnamed placeholder", e.slot);
        }
    }

    /// The decoder validates every record, so a corrupted slot index must be
    /// rejected rather than silently yielding shifted names.
    #[test]
    fn preset_list_rejects_misaligned_input() {
        let mut bad = PRESET_LIST_BASE32.to_vec();
        bad[7] = 0x21; // first slot byte no longer equals BASE
        assert!(parse_preset_list(&bad).is_none());
        assert!(parse_preset_list(&[]).is_none());
        // Not an 0x40 body marker.
        assert!(parse_preset_list(&[0x00, 0x00, 0x10, 0x00, 0x00, 0x00, 0x08]).is_none());
    }

    // -- chain order ------------------------------------------------------
    //
    // The three frames below are verbatim from
    // `block-reorder.mmon`, where the user moved
    // DEL to chain position 1 and EQ to position 2. With the role table
    // (DEL == 4, EQ == 7) the chain must come out as [4,0,1,2,3,5,6,7] and
    // then [4,7,0,1,2,3,5,6].

    /// `0x6D` write payload, read from the capture.
    const CHAIN_WRITE_1: &[u8] = &[
        0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x04, 0x00, 0x01, 0x02, 0x03, 0x00, 0x05, 0x06, 0x07,
    ];
    const CHAIN_WRITE_2: &[u8] = &[
        0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x04, 0x07, 0x00, 0x01, 0x02, 0x00, 0x03, 0x05, 0x06,
    ];
    /// Matching `0x09` device notification payloads (`00 08` + 9 bytes).
    const CHAIN_NOTIFY_1: &[u8] = &[
        0x00, 0x08, 0x04, 0x00, 0x00, 0x01, 0x02, 0x03, 0x05, 0x06, 0x07,
    ];
    const CHAIN_NOTIFY_2: &[u8] = &[
        0x00, 0x08, 0x04, 0x00, 0x07, 0x00, 0x01, 0x02, 0x03, 0x05, 0x06,
    ];

    #[test]
    fn chain_roles_match_the_enable_evidence() {
        // The user enabled DEL and EQ; the device got ENABLE slot=4 / slot=7.
        assert_eq!(chain_role_index("delay"), Some(4));
        assert_eq!(chain_role_index("del"), Some(4));
        assert_eq!(chain_role_index("mod"), Some(5));
        assert_eq!(chain_role_index("reverb"), Some(6));
        assert_eq!(chain_role_index("rev"), Some(6));
        assert_eq!(chain_role_index("eq"), Some(7));
        assert_eq!(chain_role_index("nope"), None);
        assert_eq!(chain_role_name(4), Some("DEL"));
    }

    #[test]
    fn chain_write_matches_the_capture_and_round_trips() {
        let order = [4u8, 0, 1, 2, 3, 5, 6, 7];
        let (k, p) = chain_write(&order).expect("valid permutation");
        assert_eq!(k, msg_type::DATA);
        assert_eq!(p, CHAIN_WRITE_1, "byte-exact against the capture");
        assert_eq!(parse_chain_write(&p).unwrap(), order.to_vec());

        let order2 = [4u8, 7, 0, 1, 2, 3, 5, 6];
        let (_, p2) = chain_write(&order2).unwrap();
        assert_eq!(p2, CHAIN_WRITE_2);
        assert_eq!(parse_chain_write(&p2).unwrap(), order2.to_vec());
    }

    #[test]
    fn chain_notify_decodes_to_the_same_chain_as_the_write() {
        // Two *independent* encodings of the same chain must agree.
        for (write, notify) in [(CHAIN_WRITE_1, CHAIN_NOTIFY_1), (CHAIN_WRITE_2, CHAIN_NOTIFY_2)] {
            let a = parse_chain_write(write).expect("write parse");
            let b = parse_chain_notify(notify).expect("notify parse");
            assert_eq!(a, b);
        }
        assert_eq!(parse_chain_notify(CHAIN_NOTIFY_2).unwrap(), vec![4, 7, 0, 1, 2, 3, 5, 6]);
    }

    #[test]
    fn chain_order_from_a_234_byte_preset_frame() {
        // `factory-reset.mmon`: the `ClnArp` preset frame's chain field is the
        // identity order [0,1,2,3,4,5,6,7] with the 0x00 filler at index 2.
        let mut frame = vec![0u8; 234];
        frame[31] = 0x03;
        frame[32] = 0x08;
        frame[33..42].copy_from_slice(&[0x00, 0x01, 0x00, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07]);
        assert_eq!(parse_chain_order(&frame).unwrap(), vec![0, 1, 2, 3, 4, 5, 6, 7]);

        // The discriminating (non-identity) sample from the same capture:
        // `ColdCln` -> [0,1,2,3,5,4,6,7].
        let mut f2 = vec![0u8; 234];
        f2[31] = 0x03;
        f2[32] = 0x08;
        f2[33..42].copy_from_slice(&[0x00, 0x01, 0x00, 0x02, 0x03, 0x05, 0x04, 0x06, 0x07]);
        assert_eq!(parse_chain_order(&f2).unwrap(), vec![0, 1, 2, 3, 5, 4, 6, 7]);
    }

    #[test]
    fn chain_rejects_non_permutations_and_bad_filler() {
        assert!(chain_write(&[0, 1, 2, 3, 4, 5, 6]).is_none(), "too short");
        assert!(chain_write(&[0, 1, 2, 3, 4, 5, 6, 7, 0]).is_none(), "too long");
        assert!(chain_write(&[0, 0, 2, 3, 4, 5, 6, 7]).is_none(), "duplicate");
        assert!(chain_write(&[0, 1, 2, 3, 4, 5, 6, 9]).is_none(), "out of range");
        // Filler must really be at index 5 for the write layout.
        let mut bad = CHAIN_WRITE_1.to_vec();
        bad[6] = 0x00;
        bad[11] = 0x04;
        assert!(parse_chain_write(&bad).is_none());
    }

    // -- chain ACK --------------------------------------------------------
    //
    // Verbatim from live hardware (`nanocore-cli chain …` on fw 1.04). The ACK
    // re-encodes the same order with the filler at index 4 instead of 5.

    /// ACK for a requested identity order `[0,1,2,3,4,5,6,7]`.
    const CHAIN_ACK_IDENTITY: &[u8] = &[
        0x00, 0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x00, 0x01, 0x02, 0x03, 0x00, 0x04, 0x05, 0x06,
        0x07,
    ];
    /// ACK for a requested `[4,7,0,1,2,3,5,6]`.
    const CHAIN_ACK_DEL_EQ: &[u8] = &[
        0x00, 0x00, 0x0A, 0x00, 0x00, 0x05, 0x08, 0x04, 0x07, 0x00, 0x01, 0x00, 0x02, 0x03, 0x05,
        0x06,
    ];

    #[test]
    fn chain_ack_round_trips_the_requested_order() {
        assert_eq!(
            parse_chain_ack(CHAIN_ACK_IDENTITY).unwrap(),
            vec![0, 1, 2, 3, 4, 5, 6, 7]
        );
        assert_eq!(
            parse_chain_ack(CHAIN_ACK_DEL_EQ).unwrap(),
            vec![4, 7, 0, 1, 2, 3, 5, 6]
        );
        // And it agrees with what `chain_write` produces for the same order —
        // the write and the ACK use different filler indices but the same chain.
        let (_, req) = chain_write(&[4, 7, 0, 1, 2, 3, 5, 6]).unwrap();
        assert_ne!(&req[6..15], &CHAIN_ACK_DEL_EQ[7..16], "encodings differ");
        assert_eq!(parse_chain_write(&req).unwrap(), parse_chain_ack(CHAIN_ACK_DEL_EQ).unwrap());
    }

    // ── wire-safe parameter encoding ────────────────────────────────────────

    #[test]
    fn param_write_matches_the_captured_layout() {
        let (kind, p) = param_write(0x08, 0x00, 0x01, 0.25);
        assert_eq!(kind, msg_type::DATA);
        assert_eq!(&p[..7], &[0x00, 0x07, 0x00, 0x08, 0x01, 0x00, 0x01]);
        assert_eq!(p.len(), 11);
        // 0.25f32 = 0x3E800000 -> LE `00 00 80 3E`; masking the high bits
        // clears the exponent LSB (bit 23) -> `00 00 00 3E`.
        assert_eq!(&p[7..], &[0x00, 0x00, 0x00, 0x3E]);
    }
}
