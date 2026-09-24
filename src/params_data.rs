//! Embedded Nanocore parameter tables.
//!
//! Mirror of `data/params.json` (schema `nanocore.params/v2`, vendored from
//! `nanocore-reverse/resources/params.json`); the sync is enforced by
//! `tests/data_sync.rs` — edit the JSON and this table together, `cargo test`
//! fails if they drift apart. That JSON is parsed out of the OEM MIDI CC
//! reference, which transcribes the OEM `NANOCORE-MIDI-Control-User-Guide.pdf`
//! - one human-reviewed source of truth, no second-hand transcription.
//!
//! ## Two ways to reach the same parameter
//!
//! * **Standard MIDI CC** - documented, 7-bit, one-way: [`ParamInfo::cc`].
//! * **SysEx float32** - App protocol `0x6D`/sub-`0x07`:
//!   `00 07 00 [b3] 01 [b5] [b6] <7-bit-safe float32>`, where `b5` =
//!   [`block_slot`] and `b6` = [`ParamInfo::index`].
//!
//! ## `b6` is per effect type, not per block
//!
//! [`ParamInfo::index`] is the index **within the block's current effect type**.
//! FX1 gate answers to `b6 = 0,1` while FX1 compressor answers to `b6 = 0..5`,
//! both at `b5 = 0` - so a flat block-wide list would be wrong. This is why the
//! tables are keyed `(block, effect type)`.
//!
//! ## Provenance
//!
//! * `b5` = block slot: the device string table IDs 14-21 order the blocks
//!   FX1,FX2,AMP,CAB,DEL,MOD,REV,EQ -> 0..7; captures agree (FX1 writes
//!   `b5=0x00`, FX2 writes `b5=0x01`, and the amp-enable frame is `ENABLE slot=2`).
//! * `b6` = parameter index: the compressor capture (whose filename names all six
//!   parameters in adjustment order) decodes to `b6 = 0..5` matching guide
//!   CC51-56 - two independent sources agreeing.
//! * `b3` is **not** addressing; see [`crate::protocol::DEFAULT_B3`].
//!
//! ## Known gaps (from the guide itself, not guesses)
//!
//! The OEM guide gives no parameter names for FX1 types 1/2 (Auto Gate,
//! Gate + Compressor) or FX2 types 5-7 (Boost 1-3), and no capture covers them
//! either - so [`params`] returns `None` for those instead of inventing names.
#![allow(dead_code)]

/// One parameter of a block / effect type.
pub struct ParamInfo {
    /// `b6` in the SysEx write - index within this block's effect type.
    pub index: u8,
    /// Display name (OEM guide).
    pub name: &'static str,
    /// Standard MIDI CC number controlling the same parameter.
    pub cc: u8,
}

/// `(block key, b5)`, ordered as the device string table lists them.
pub const BLOCKS: &[(&str, u8)] = &[
    ("fx1", 0),
    ("fx2", 1),
    ("amp", 2),
    ("cab", 3),
    ("delay", 4),
    ("mod", 5),
    ("reverb", 6),
    ("eq", 7),
];

/// `(block key, effect type id, parameters)`. Only types with a known parameter
/// list appear; absent types are the documented gaps.
pub const TYPED: &[(&str, u8, &[ParamInfo])] = &[
("fx1", 0, &[
    ParamInfo { index: 0, name: "Gate Threshold", cc: 50 },
    ParamInfo { index: 1, name: "Release", cc: 50 },
]),
("fx1", 3, &[
    ParamInfo { index: 0, name: "Compressor Threshold", cc: 51 },
    ParamInfo { index: 1, name: "Ratio", cc: 52 },
    ParamInfo { index: 2, name: "Knee", cc: 53 },
    ParamInfo { index: 3, name: "Attack", cc: 54 },
    ParamInfo { index: 4, name: "Release", cc: 55 },
    ParamInfo { index: 5, name: "Makeup", cc: 56 },
]),
("fx2", 0, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 1, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 2, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 3, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 4, &[
    ParamInfo { index: 0, name: "Drive Gain / Boost", cc: 57 },
    ParamInfo { index: 1, name: "Tone", cc: 58 },
    ParamInfo { index: 2, name: "Level", cc: 59 },
]),
("fx2", 8, &[
    ParamInfo { index: 0, name: "Pitch", cc: 68 },
    ParamInfo { index: 1, name: "Fine", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
    ParamInfo { index: 3, name: "Level", cc: 71 },
]),
("fx2", 9, &[
    ParamInfo { index: 0, name: "Freq", cc: 68 },
    ParamInfo { index: 1, name: "Env", cc: 69 },
    ParamInfo { index: 2, name: "Q", cc: 70 },
    ParamInfo { index: 3, name: "Mix", cc: 71 },
    ParamInfo { index: 4, name: "Level", cc: 72 },
]),
("fx2", 10, &[
    ParamInfo { index: 0, name: "Freq", cc: 68 },
    ParamInfo { index: 1, name: "Sweep", cc: 69 },
    ParamInfo { index: 2, name: "Rate", cc: 70 },
    ParamInfo { index: 3, name: "Shape", cc: 71 },
    ParamInfo { index: 4, name: "Mix", cc: 72 },
    ParamInfo { index: 5, name: "Level", cc: 73 },
]),
("mod", 0, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Level", cc: 70 },
]),
("mod", 1, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Feedback", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
    ParamInfo { index: 3, name: "Level", cc: 71 },
]),
("mod", 2, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Feedback", cc: 69 },
    ParamInfo { index: 2, name: "Mix", cc: 70 },
    ParamInfo { index: 3, name: "Level", cc: 71 },
]),
("mod", 3, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Wave", cc: 70 },
    ParamInfo { index: 3, name: "Smooth", cc: 71 },
    ParamInfo { index: 4, name: "Mix", cc: 72 },
    ParamInfo { index: 5, name: "Level", cc: 73 },
]),
("mod", 4, &[
    ParamInfo { index: 0, name: "Rate", cc: 68 },
    ParamInfo { index: 1, name: "Depth", cc: 69 },
    ParamInfo { index: 2, name: "Wave", cc: 70 },
    ParamInfo { index: 3, name: "Mix", cc: 71 },
    ParamInfo { index: 4, name: "Level", cc: 72 },
]),
("delay", 0, &[
    ParamInfo { index: 0, name: "Delay", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Age", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
    ParamInfo { index: 4, name: "Level", cc: 78 },
]),
("delay", 1, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mix", cc: 76 },
    ParamInfo { index: 3, name: "Level", cc: 77 },
]),
("delay", 2, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Filter", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
    ParamInfo { index: 4, name: "Level", cc: 78 },
]),
("delay", 3, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Interval", cc: 76 },
    ParamInfo { index: 3, name: "Mix", cc: 77 },
    ParamInfo { index: 4, name: "Level", cc: 78 },
]),
("delay", 4, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mix", cc: 76 },
    ParamInfo { index: 3, name: "Level", cc: 77 },
]),
("delay", 5, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Bit Depth", cc: 76 },
    ParamInfo { index: 3, name: "LoFi Mix", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
    ParamInfo { index: 5, name: "Level", cc: 79 },
]),
("delay", 6, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Smear", cc: 76 },
    ParamInfo { index: 3, name: "Filter", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
    ParamInfo { index: 5, name: "Level", cc: 79 },
]),
("delay", 7, &[
    ParamInfo { index: 0, name: "Time", cc: 74 },
    ParamInfo { index: 1, name: "Feedback", cc: 75 },
    ParamInfo { index: 2, name: "Mod Depth", cc: 76 },
    ParamInfo { index: 3, name: "Mod Rate", cc: 77 },
    ParamInfo { index: 4, name: "Mix", cc: 78 },
    ParamInfo { index: 5, name: "Level", cc: 79 },
]),
("reverb", 0, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 1, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Mod", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 2, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "LF Damp", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 3, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "HF Damp", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 4, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Pitch", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 5, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "Tone", cc: 86 },
    ParamInfo { index: 2, name: "Mod Depth", cc: 87 },
    ParamInfo { index: 3, name: "Mod Rate", cc: 88 },
    ParamInfo { index: 4, name: "Mix", cc: 89 },
    ParamInfo { index: 5, name: "Level", cc: 90 },
]),
("reverb", 6, &[
    ParamInfo { index: 0, name: "Decay", cc: 85 },
    ParamInfo { index: 1, name: "PreDelay", cc: 86 },
    ParamInfo { index: 2, name: "Tone", cc: 87 },
    ParamInfo { index: 3, name: "Crossover", cc: 88 },
    ParamInfo { index: 4, name: "Mod", cc: 89 },
    ParamInfo { index: 5, name: "Mix", cc: 90 },
]),
];

/// `(block key, b5, parameters)` for blocks whose parameter list does not depend
/// on the effect type.
pub const FIXED: &[(&str, u8, &[ParamInfo])] = &[
("amp", 2, &[
    ParamInfo { index: 0, name: "Amp Gain", cc: 60 },
    ParamInfo { index: 1, name: "Bass", cc: 61 },
    ParamInfo { index: 2, name: "Mid", cc: 62 },
    ParamInfo { index: 3, name: "Treble", cc: 63 },
    ParamInfo { index: 4, name: "Amp Level", cc: 64 },
]),
("cab", 3, &[
    ParamInfo { index: 0, name: "Cab Low Cut", cc: 65 },
    ParamInfo { index: 1, name: "Cab High Cut", cc: 66 },
    ParamInfo { index: 2, name: "Cab Level", cc: 67 },
]),
];

/// `(EQ type id, band parameters)`.
pub const EQ_BANDS: &[(u8, &[ParamInfo])] = &[
(0, &[
    ParamInfo { index: 0, name: "Low", cc: 91 },
    ParamInfo { index: 1, name: "Mid", cc: 92 },
    ParamInfo { index: 2, name: "High", cc: 93 },
]),
(1, &[
    ParamInfo { index: 0, name: "120 Hz", cc: 91 },
    ParamInfo { index: 1, name: "250 Hz", cc: 92 },
    ParamInfo { index: 2, name: "500 Hz", cc: 93 },
    ParamInfo { index: 3, name: "1 kHz", cc: 94 },
    ParamInfo { index: 4, name: "2 kHz", cc: 95 },
    ParamInfo { index: 5, name: "4 kHz", cc: 96 },
]),
(2, &[
    ParamInfo { index: 0, name: "60 Hz", cc: 91 },
    ParamInfo { index: 1, name: "120 Hz", cc: 92 },
    ParamInfo { index: 2, name: "250 Hz", cc: 93 },
    ParamInfo { index: 3, name: "500 Hz", cc: 94 },
    ParamInfo { index: 4, name: "1 kHz", cc: 95 },
    ParamInfo { index: 5, name: "2 kHz", cc: 96 },
    ParamInfo { index: 6, name: "4 kHz", cc: 97 },
    ParamInfo { index: 7, name: "8 kHz", cc: 33 },
]),
];

/// `b5` for a block key (`fx1`, `fx2`, `amp`, `cab`, `delay`, `mod`, `reverb`, `eq`).
pub fn block_slot(key: &str) -> Option<u8> {
    BLOCKS.iter().find(|(k, _)| *k == key).map(|(_, s)| *s)
}

/// EQ band parameters for an EQ type.
pub fn eq_bands(type_id: u8) -> Option<&'static [ParamInfo]> {
    EQ_BANDS.iter().find(|(t, _)| *t == type_id).map(|(_, ps)| *ps)
}

/// All parameters of a block for its current effect type.
///
/// Returns `None` when the block's parameters depend on the effect type and that
/// type is unknown or has no published parameter list (see module docs).
pub fn params(block: &str, type_id: Option<u8>) -> Option<&'static [ParamInfo]> {
    if block == "eq" {
        return type_id.and_then(eq_bands);
    }
    if TYPED.iter().any(|(k, _, _)| *k == block) {
        let t = type_id?;
        return TYPED
            .iter()
            .find(|(k, t2, _)| *k == block && *t2 == t)
            .map(|(_, _, ps)| *ps);
    }
    FIXED.iter().find(|(k, _, _)| *k == block).map(|(_, _, ps)| *ps)
}

/// Look up a parameter by its `b6` index.
pub fn param_by_index(block: &str, type_id: Option<u8>, index: u8) -> Option<&'static ParamInfo> {
    params(block, type_id)?.iter().find(|p| p.index == index)
}

/// Look up a parameter by name (case-, space- and punctuation-insensitive).
pub fn param_by_name(block: &str, type_id: Option<u8>, name: &str) -> Option<&'static ParamInfo> {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    let want = norm(name);
    let all = params(block, type_id)?;
    if let Some(p) = all.iter().find(|p| norm(p.name) == want) {
        return Some(p);
    }
    // Fall back to a suffix match, but only when it is unambiguous, so a short
    // name like "gain" resolves to "Amp Gain" while "level" stays an error if
    // several parameters end with it.
    let mut hits = all.iter().filter(|p| norm(p.name).ends_with(&want));
    let first = hits.next()?;
    if hits.next().is_some() {
        return None;
    }
    Some(first)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_slots_match_capture_evidence() {
        assert_eq!(block_slot("fx1"), Some(0));
        assert_eq!(block_slot("fx2"), Some(1));
        assert_eq!(block_slot("amp"), Some(2)); // `ENABLE slot=2` = AMP
        assert_eq!(block_slot("cab"), Some(3));
        assert_eq!(block_slot("eq"), Some(7));
        assert_eq!(block_slot("nope"), None);
    }

    /// b6 is per effect type: FX1 gate answers 0,1 and FX1 compressor 0..5, both
    /// at b5=0. The compressor order is pinned by the capture and by CC51-56.
    #[test]
    fn compressor_param_order_matches_capture() {
        let comp = params("fx1", Some(3)).expect("fx1 compressor");
        assert_eq!(
            comp.iter().map(|p| p.name).collect::<Vec<_>>(),
            vec!["Compressor Threshold", "Ratio", "Knee", "Attack", "Release", "Makeup"]
        );
        assert_eq!(comp.iter().map(|p| p.index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4, 5]);
        let makeup = param_by_name("fx1", Some(3), "makeup").expect("makeup");
        assert_eq!(makeup.index, 5); // b6 = 5 in the capture
        assert_eq!(makeup.cc, 56);

        // Gate is a different type in the same block, and only has two params.
        let gate = params("fx1", Some(0)).expect("fx1 gate");
        assert_eq!(gate.iter().map(|p| p.name).collect::<Vec<_>>(), vec!["Gate Threshold", "Release"]);
    }

    #[test]
    fn lookup_by_name_is_forgiving() {
        assert_eq!(param_by_name("amp", None, "gain").unwrap().cc, 60);
        assert_eq!(param_by_name("amp", None, "Amp Gain").unwrap().index, 0);
        assert_eq!(param_by_name("cab", None, "low cut").unwrap().cc, 65);
        assert_eq!(param_by_name("amp", None, "Amp-Level").unwrap().cc, 64);
        assert!(param_by_name("amp", None, "nope").is_none());
        // Exact match wins even when it is also a suffix of a longer name:
        // delay type 5 has both "Mix" and "Lo-Fi Mix".
        assert_eq!(param_by_name("delay", Some(5), "mix").unwrap().cc, 78);
        assert_eq!(param_by_name("delay", Some(5), "lofi mix").unwrap().cc, 77);
        // Suffix fallback helps when there is no exact match.
        assert_eq!(param_by_name("amp", None, "gain").unwrap().name, "Amp Gain");
        // ...but an ambiguous suffix is refused rather than guessed: the 8-band EQ
        // has 1 kHz / 2 kHz / 4 kHz.
        assert!(param_by_name("eq", Some(2), "khz").is_none());
        assert_eq!(param_by_name("eq", Some(2), "1khz").unwrap().cc, 95);
    }

    #[test]
    fn typed_tables_resolve_by_type() {
        assert_eq!(params("delay", Some(1)).unwrap().len(), 4); // Digital
        assert_eq!(params("delay", Some(0)).unwrap().len(), 5); // BBD
        assert_eq!(params("delay", Some(1)).unwrap()[0].name, "Time");
        assert_eq!(params("delay", Some(0)).unwrap()[3].name, "Mix"); // index == b6
        // type-dependent block: unknown type or no type -> unresolvable
        assert!(params("delay", Some(99)).is_none());
        assert!(params("delay", None).is_none());
    }

    /// The guide publishes no parameter names for these types; we must return
    /// None rather than invent them.
    #[test]
    fn documented_gaps_stay_unknown() {
        // fx1 type 1: no published parameter list
        assert!(params("fx1", Some(1)).is_none());
        // fx1 type 2: no published parameter list
        assert!(params("fx1", Some(2)).is_none());
        // fx2 boost types 5-7 are gaps too
        for t in 5u8..=7 { assert!(params("fx2", Some(t)).is_none()); }
    }

    #[test]
    fn eq_uses_band_tables_per_type() {
        assert_eq!(params("eq", Some(0)).unwrap().len(), 3);
        assert_eq!(params("eq", Some(1)).unwrap().len(), 6);
        assert_eq!(params("eq", Some(2)).unwrap().len(), 8);
        assert_eq!(eq_bands(0).unwrap()[0].name, "Low");
        assert!(eq_bands(9).is_none());
    }

    #[test]
    fn fixed_blocks_ignore_type() {
        assert_eq!(params("amp", None).unwrap().len(), 5);
        assert_eq!(params("amp", Some(7)).unwrap().len(), 5); // type ignored
        assert_eq!(params("cab", Some(3)).unwrap().len(), 3);
    }
}
