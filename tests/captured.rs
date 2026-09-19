//! Tests built from real MIDI captures in ``.

use nanocore::frame::{Direction, Frame};
use nanocore::protocol::*;
use nanocore::{field, sub};

#[test]
fn decode_captured_preset_select() {
    // From `preset-switch-1-2-3-4.mmon` (TX, select preset index 1).
    let bytes = [
        0x7D, 0x4E, 0x43, 0x70, 0x00, 0x02, 0x45, 0x03, 0x6D, // header + type
        0x00, 0x02, 0x00, 0x00, 0x09, 0x01, // payload
    ];
    let (f, _) = Frame::decode(&bytes).unwrap();
    assert_eq!(f.direction, Direction::Tx);
    assert_eq!(f.flags, 0x00);
    assert_eq!(f.seq, 0x0345);
    assert_eq!(f.kind, 0x6D);
    assert_eq!(f.payload, vec![0x00, 0x02, 0x00, 0x00, 0x09, 0x01]);
}

#[test]
fn frame_roundtrip() {
    let f = Frame::new(Direction::Tx, 0x00, 0x0345, 0x6D, vec![1, 2, 3]);
    let enc = f.encode();
    let (dec, _) = Frame::decode(&enc).unwrap();
    assert_eq!(f, dec);
    assert_eq!(enc[0..3], nanocore::MAGIC);
}

#[test]
fn preset_select_payload() {
    let (kind, payload) = select_preset(1);
    assert_eq!(kind, 0x6D);
    assert_eq!(payload, vec![0x00, sub::SET_FIELD, 0x00, 0x00, field::PRESET, 0x01]);
}

#[test]
fn cab_select_payload() {
    let (_, payload) = select_cab(3);
    assert_eq!(payload, vec![0x00, 0x02, 0x00, 0x00, field::CAB, 0x03]);
}

#[test]
fn enable_payload_matches_capture() {
    // `enable-all-slots`: slot 0..4 enabled.
    let (_, payload) = set_enabled(0, true);
    assert_eq!(payload, vec![0x00, 0x03, 0x00, 0x00, 0x03, 0x00, 0x01]);
}

#[test]
fn usb_volume_matches_capture() {
    // `usb-volume-stepdown` -> 0x64, 0x5F, 0x5A, 0x55
    for (pct, byte) in [(100u8, 0x64u8), (95, 0x5F), (90, 0x5A), (85, 0x55)] {
        let (kind, payload) = set_usb_volume(pct);
        assert_eq!(kind, 0x66);
        assert_eq!(payload, vec![0x00, 0x02, 0x00, 0x00, field::USB_VOLUME, byte]);
    }
}

#[test]
fn param_float_encoding() {
    assert_eq!(encode_param_float(0.5), [0x00, 0x00, 0x00, 0x3F]);
    assert_eq!(encode_param_float(0.25), [0x00, 0x00, 0x80, 0x3E]);
    assert_eq!(decode_param_float(&encode_param_float(0.617)), Some(0.617f32));
    // Wire format = float32 with every byte's high bit cleared (SysEx-safe).
    assert_eq!(encode_param_value(0.617), [0x36, 0x73, 0x1D, 0x3F]);
    assert_eq!(encode_param_value(0.2), [0x4D, 0x4C, 0x4C, 0x3E]);
}

/// The app's writes in `scream-knob-sweep.mmon` are exactly
/// `float32(value)` with each byte's high bit cleared. Because the wire bytes
/// are already < 0x80, re-encoding them is a no-op — so decoding then encoding
/// any captured value must return the same bytes. This pins down the wire
/// format independently of any assumptions about the knob's true float.
#[test]
fn scream_capture_wire_format_is_msb_cleared_float() {
    // (b5, b6, 4 value bytes) drawn from the live scream-tone knob sweep.
    let wires: &[[u8; 4]] = &[
        [0x61, 0x3A, 0x4C, 0x3E], // 0.199441
        [0x6C, 0x51, 0x4D, 0x3E], // 0.200506
        [0x00, 0x00, 0x4E, 0x3E], // 0.201172
        [0x0A, 0x17, 0x4F, 0x3E], // 0.202236
        [0x7B, 0x54, 0x64, 0x3E], // 0.222979
        [0x00, 0x00, 0x00, 0x3F], // 0.5
    ];
    for w in wires {
        // Every value byte the app sends is a legal MIDI data byte.
        assert!(w.iter().all(|b| *b < 0x80), "app sent byte >= 0x80: {w:x?}");
        // Re-encoding the decoded float reproduces the captured bytes exactly.
        let v = decode_param_float(w).unwrap();
        assert!(v.is_finite() && (0.0..=1.0).contains(&v));
        assert_eq!(&encode_param_value(v), w);
    }
}

#[test]
fn scream_knob_is_linear_0_2_to_0_5() {
    // The capture is a knob sweep 0->100; decoded values trace ~[0.2, 0.5].
    let lo = decode_param_float(&[0x61, 0x3A, 0x4C, 0x3E]).unwrap(); // near knob 0
    let hi = decode_param_float(&[0x00, 0x00, 0x00, 0x3F]).unwrap(); // knob 100
    assert!((lo - 0.2).abs() < 0.01, "low end {lo}");
    assert!((hi - 0.5).abs() < 1e-6, "high end {hi}");
}

#[test]
fn load_effect_layout() {
    // fx1 compressor: effect index 3, 6 params.
    let params = [0.5f32; 6];
    let (kind, payload) = load_effect(2, 3, 0x30, &params);
    assert_eq!(kind, 0x6D);
    assert_eq!(payload[1], 4 * (6 + 1)); // 0x1C
    assert_eq!(payload[3], 0x30);
    assert_eq!(payload[4], 2); // slot
    assert_eq!(payload[6], 3); // effect index
    assert_eq!(payload[7], 6); // count
    assert_eq!(payload.len(), 8 + 6 * 4);
}

#[test]
fn parse_effect_data_msg() {
    let mut p = vec![0x00, 0x01, 0x02, 0x02]; // 00 01 slot=2 count=2
    p.extend_from_slice(&encode_param_float(0.25));
    p.extend_from_slice(&encode_param_float(0.75));
    let ep = parse_effect_data(&p).unwrap();
    assert_eq!(ep.slot, 2);
    assert_eq!(ep.params.len(), 2);
    assert!((ep.params[0] - 0.25).abs() < 1e-6);
    assert!((ep.params[1] - 0.75).abs() < 1e-6);
}

#[test]
fn parse_chain_order_clnarp() {
    let mut payload = vec![0u8; 40];
    payload[30] = 0x03;
    payload[31] = 0x08;
    payload[32..40].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(parse_chain_order(&payload), Some(vec![0, 1, 2, 3, 4, 5, 6, 7]));
}

#[test]
fn metadata_tail_detection() {
    let mut payload = vec![0x00, 0x02, 0x00, 0x00, 0x09, 0x01, 0xDA, 0x00, 0x0E];
    payload.extend_from_slice(&nanocore::tail::STATUS_MARKER);
    payload.extend_from_slice(b"From Nanocore");
    let core = nanocore::tail::core_payload(&payload);
    assert_eq!(core, &[0x00, 0x02, 0x00, 0x00, 0x09, 0x01]);
    assert!(nanocore::tail::extract_tail(&payload).is_some());
}

#[test]
fn parse_preset_list_from_live_device() {
    // Real 0x40 reply captured from hardware via nanocore-cli (slots 0..7).
    let payload: Vec<u8> = [
        0x00, 0x00, 0x52, 0x00, 0x00, 0x00, 0x08, 0x00, 0x01, 0x43, 0x6c, 0x00, 0x6e, 0x41, 0x72,
        0x70, 0x20, 0x20, 0x01, 0x00, 0x01, 0x41, 0x6d, 0x62, 0x69, 0x65, 0x6e, 0x00, 0x74, 0x20,
        0x02, 0x01, 0x52, 0x68, 0x79, 0x00, 0x4f, 0x44, 0x20, 0x20, 0x20, 0x03, 0x01, 0x00, 0x45,
        0x70, 0x69, 0x63, 0x53, 0x6f, 0x6c, 0x00, 0x6f, 0x04, 0x01, 0x43, 0x6f, 0x6c, 0x64, 0x00,
        0x43, 0x6c, 0x6e, 0x20, 0x05, 0x01, 0x50, 0x00, 0x69, 0x65, 0x72, 0x63, 0x69, 0x6e, 0x67,
        0x00, 0x06, 0x01, 0x54, 0x69, 0x67, 0x68, 0x74, 0x00, 0x52, 0x68, 0x79, 0x07, 0x01, 0x48,
        0x65, 0x00, 0x61, 0x76, 0x79, 0x4d, 0x75, 0x74,
    ]
    .to_vec();

    let entries = parse_preset_list(&payload).unwrap();
    let names: Vec<&str> = entries.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "ClnArp", "Ambient", "RhyOD", "EpicSolo", "ColdCln", "Piercing", "TightRhy",
            "HeavyMut",
        ]
    );
    assert_eq!(entries.len(), 8);
    assert_eq!(entries[0].slot, 0);
    assert!(entries[0].named);
}

#[test]
fn param_write_layout_and_seven_bit_safety() {
    // From `noise-gate-tweak.mmon`: slot 0, param 1 (release).
    let (kind, payload) = param_write(0x00, 0x00, 0x01, 0.25);
    assert_eq!(kind, 0x6D);
    assert_eq!(&payload[..7], &[0x00, 0x07, 0x00, 0x00, 0x01, 0x00, 0x01]);
    // 0.25 would be `00 00 80 3E`; clearing the high bit of each byte turns
    // that into 0.125. See the lossiness note below.
    assert_eq!(&payload[7..], &[0x00, 0x00, 0x00, 0x3E]);

    // Every byte that goes on the wire must be a legal MIDI data byte.
    for v in [0.1f32, 0.2, 0.3, 0.4, 0.617, 0.777, 0.9] {
        let (_, p) = param_write(0x00, 0x00, 0x00, v);
        let f = Frame::new(Direction::Tx, 0x00, 0, 0x6D, p);
        assert!(f.is_seven_bit_safe(), "value {v} produced a byte >= 0x80");
    }

    // MSB clearing is lossy for values whose mantissa bytes exceed 0x7F:
    // 0.3 -> 9A 99 99 3E -> 1A 19 19 3E -> 0.14951. This matches the hardware:
    // bit 23 of the float (the exponent LSB) is one of the four bits the SysEx
    // wire forces to 0, so an odd-exponent value collapses to the next lower
    // even exponent. The official app accepts the same quantization.
    assert_eq!(encode_param_value(0.3), [0x1A, 0x19, 0x19, 0x3E]);
    let decoded = f32::from_le_bytes(encode_param_value(0.3));
    assert!((decoded - 0.14951).abs() < 1e-4, "got {decoded}");
}
