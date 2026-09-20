//! Standard MIDI CC / Program Change control surface for Nanocore (fw 1.04+).
//!
//! These are the **official, documented** control messages — see
//! the OEM MIDI CC reference (sourced from the OEM
//! `NANOCORE-MIDI-Control-User-Guide.pdf`). They are ordinary 3-byte MIDI
//! messages and travel *without* the custom `F0 7D 4E 43 …` SysEx wrapper.
//!
//! This module is transport-agnostic: every builder returns the raw bytes to
//! send. The `midir` transport sends them verbatim when `set_sysex(false)` is
//! in effect (see `crate::midi::MidiTransport` and `nanocore-cli`).
//!
//! ## Correction to the OEM guide: CC43 / CC44 *do* select models
//!
//! The guide's main CC map lists `CC43` / `CC44` as the Amp / Cab "Type CC",
//! but then prints type tables for FX1/FX2/Mod/Delay/Reverb/EQ only, and states
//! *"Model/profile and IR-slot selection are not assigned here."*
//!
//! Hardware testing (fw 1.04) shows that statement is **wrong for the Amp and
//! Cab blocks**: `CC43 = n` selects amp slot `n` and `CC44 = n` selects cab
//! slot `n`, both over the 30-entry tables in [`crate::models_data`]. Values
//! `>= 30` are ignored. Use [`amp_model`] / [`cab_model`]. This means ordinary
//! third-party MIDI controllers can switch amp/cab models — no SysEx needed.

/// MIDI status byte for Control Change on `channel` (0–15).
#[inline]
pub fn cc_status(channel: u8) -> u8 {
    0xB0u8 | (channel & 0x0F)
}

/// MIDI status byte for Program Change on `channel` (0–15).
#[inline]
pub fn pc_status(channel: u8) -> u8 {
    0xC0u8 | (channel & 0x0F)
}

/// Build a 3-byte Control Change message: `[0xB0|ch, cc, value]`.
///
/// `cc` and `value` are masked to 7 bits — bytes `>= 0x80` are illegal in a
/// MIDI data stream and would be dropped by the device.
pub fn control_change(channel: u8, cc: u8, value: u8) -> Vec<u8> {
    vec![cc_status(channel), cc & 0x7F, value & 0x7F]
}

/// Build a 2-byte Program Change message: `[0xC0|ch, program]`.
///
/// Program number is 0-based and maps 1:1 to the device preset index
/// (PC `2` recalls the preset the device displays as `03`).
pub fn program_change(channel: u8, program: u8) -> Vec<u8> {
    vec![pc_status(channel), program & 0x7F]
}

/// An effect block addressable via the standard CC map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Block {
    Fx1,
    Fx2,
    Amp,
    Cab,
    Mod,
    Delay,
    Reverb,
    Eq,
}

impl Block {
    /// CC that toggles this block on/off (value `0`=OFF, `127`=ON).
    pub fn onoff_cc(self) -> u8 {
        match self {
            Block::Fx1 => 20,
            Block::Fx2 => 21,
            Block::Amp => 23,
            Block::Cab => 24,
            Block::Mod => 25,
            Block::Delay => 26,
            Block::Reverb => 27,
            Block::Eq => 28,
        }
    }

    /// CC that selects this block's effect/model type (send the exact Type ID).
    pub fn type_cc(self) -> u8 {
        match self {
            Block::Fx1 => 40,
            Block::Fx2 => 41,
            Block::Amp => 43,
            Block::Cab => 44,
            Block::Mod => 45,
            Block::Delay => 46,
            Block::Reverb => 47,
            Block::Eq => 48,
        }
    }

    /// Parse a block from its CLI / CC-map name (case-insensitive).
    pub fn from_name(name: &str) -> Option<Block> {
        match name.to_ascii_lowercase().as_str() {
            "fx1" => Some(Block::Fx1),
            "fx2" => Some(Block::Fx2),
            "amp" => Some(Block::Amp),
            "cab" => Some(Block::Cab),
            "mod" | "modulation" => Some(Block::Mod),
            "delay" => Some(Block::Delay),
            "rev" | "reverb" => Some(Block::Reverb),
            "eq" => Some(Block::Eq),
            _ => None,
        }
    }
}

/// Send a block on/off toggle (standard CC).
pub fn block_enable(channel: u8, block: Block, on: bool) -> Vec<u8> {
    control_change(channel, block.onoff_cc(), if on { 127 } else { 0 })
}

/// Send a block type/effect/model selection (standard CC).
pub fn block_type(channel: u8, block: Block, type_id: u8) -> Vec<u8> {
    control_change(channel, block.type_cc(), type_id)
}

/// Number of selectable models in the Amp block (valid indices `0..=29`).
pub const AMP_MODEL_COUNT: u8 = 30;

/// Number of selectable models in the Cab / IR block (valid indices `0..=29`).
pub const CAB_MODEL_COUNT: u8 = 30;

/// Select an Amp-block model by 0-based firmware slot (`0..=29`).
///
/// This rides on the standard **CC43** — the CC the OEM guide lists as the Amp
/// block's "Type CC". The guide then claims *"Model/profile and IR-slot
/// selection are not assigned here"* and prints **no** type table for Amp or
/// Cab. **That claim is wrong for the Amp block.**
///
/// Verified on hardware (`Nanocore`, fw 1.04), read-only snapshot diffing plus
/// visual confirmation of the device display:
///
/// | sent | device display |
/// |------|----------------|
/// | `CC43 = 20` | `RoJC120` (Roland JC-120) |
/// | `CC43 = 29` | slot 29 (a user-imported NAM model) |
/// | `CC43 = 30` | **unchanged** — out of range, rejected |
/// | `CC43 = 37` | **unchanged** — out of range, rejected |
///
/// So `CC43 = n` selects amp slot `n` from the same 30-entry table as the
/// SysEx `SET_FIELD(VARIANT, n)` path (see `models_data::AMP`). Indices `>= 30`
/// are silently ignored by the device, so callers should validate first.
/// Note the drive-pedal entries at `models_data::AMP[30..38]` are **not**
/// reachable here — they belong to the FX2 block.
pub fn amp_model(channel: u8, index: u8) -> Vec<u8> {
    control_change(channel, Block::Amp.type_cc(), index)
}

/// Select a Cab / IR-block model by 0-based firmware slot (`0..=29`).
///
/// Rides on the standard **CC44**, same story as [`amp_model`]: the OEM guide
/// gives no Cab type table and claims model selection is unassigned, but on
/// hardware `CC44 = n` selects cab slot `n`.
///
/// | sent | device display |
/// |------|----------------|
/// | `CC44 = 14` | `Og412` (Orange Vintage30 4x12) |
/// | `CC44 = 29` | `Mar412` (Marshall 4x12) |
/// | `CC44 = 30` | **unchanged** — out of range, rejected |
///
/// Indices `>= 30` are silently ignored, so callers should validate first.
pub fn cab_model(channel: u8, index: u8) -> Vec<u8> {
    control_change(channel, Block::Cab.type_cc(), index)
}

/// Global controls (see `midi_cc_reference.md`).
pub mod global {
    use super::control_change;

    /// Tuner on/off (CC80).
    pub fn tuner(channel: u8, on: bool) -> Vec<u8> {
        control_change(channel, 80, if on { 127 } else { 0 })
    }
    /// Step to the previous preset (CC81, value 127).
    pub fn prev_preset(channel: u8) -> Vec<u8> {
        control_change(channel, 81, 127)
    }
    /// Step to the next preset (CC82, value 127).
    pub fn next_preset(channel: u8) -> Vec<u8> {
        control_change(channel, 82, 127)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_layout_and_masking() {
        // status = 0xB0 | channel, data bytes masked to 7 bits
        assert_eq!(control_change(0, 60, 100), vec![0xB0, 60, 100]);
        assert_eq!(control_change(15, 0x80, 0xFF), vec![0xBF, 0x00, 0x7F]);
    }

    #[test]
    fn pc_layout() {
        assert_eq!(program_change(0, 2), vec![0xC0, 2]);
        // PC 2 recalls the preset displayed as "03" (0-based)
        assert_eq!(program_change(0, 2)[1], 2);
    }

    #[test]
    fn block_cc_maps() {
        assert_eq!(Block::Fx1.onoff_cc(), 20);
        assert_eq!(Block::Fx1.type_cc(), 40);
        assert_eq!(Block::Amp.onoff_cc(), 23);
        assert_eq!(Block::Reverb.type_cc(), 47);
        assert_eq!(Block::from_name("REV"), Some(Block::Reverb));
        assert_eq!(block_enable(0, Block::Fx2, true), vec![0xB0, 21, 127]);
        assert_eq!(block_type(0, Block::Mod, 4), vec![0xB0, 45, 4]);
    }

    #[test]
    fn global_controls() {
        assert_eq!(global::tuner(0, true), vec![0xB0, 80, 127]);
        assert_eq!(global::prev_preset(0), vec![0xB0, 81, 127]);
        assert_eq!(global::next_preset(0), vec![0xB0, 82, 127]);
    }

    /// CC43 / CC44 model selection — the values below are the exact ones
    /// verified on hardware (device display read back by eye).
    #[test]
    fn amp_and_cab_model_use_cc43_cc44() {
        assert_eq!(amp_model(0, 20), vec![0xB0, 43, 20]); // -> RoJC120
        assert_eq!(cab_model(0, 14), vec![0xB0, 44, 14]); // -> Og412
        assert_eq!(amp_model(0, AMP_MODEL_COUNT - 1), vec![0xB0, 43, 29]);
        assert_eq!(cab_model(0, CAB_MODEL_COUNT - 1), vec![0xB0, 44, 29]);
        // Channel is honoured, data bytes stay 7-bit.
        assert_eq!(amp_model(3, 5), vec![0xB3, 43, 5]);
        assert_eq!(cab_model(0, 200), vec![0xB0, 44, 200 & 0x7F]);
    }

    #[test]
    fn amp_cab_model_cc_matches_block_type_cc() {
        // `amp_model`/`cab_model` are thin, named wrappers over `block_type`,
        // and must agree with the documented CC map.
        assert_eq!(amp_model(0, 7), block_type(0, Block::Amp, 7));
        assert_eq!(cab_model(0, 7), block_type(0, Block::Cab, 7));
        assert_eq!(AMP_MODEL_COUNT, CAB_MODEL_COUNT);
    }
}
