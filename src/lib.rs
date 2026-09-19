//! Nanocore (TPE-S10) protocol library.
//!
//! Frame layout (verified from MIDI captures):
//!
//! ```text
//! 7D 4E 43 | dir | flags | 02 | seq_lo | seq_hi | type | payload...
//! ```
//!
//! * `dir`   0x70 = TX (app -> device), 0x71 = RX, 0x72 = unsolicited event
//! * `flags` observed values: 0x00 (normal), 0x02 (live/streaming),
//!   0x22 (0x43 preset write), 0x40 (device RX)
//! * byte 5 is constant `0x02`
//! * `seq` is a little-endian u16 counter echoed by the device
//! * `type` is the message type (e.g. 0x6D data, 0x63 snapshot, 0x68 ping)

pub mod frame;

pub use frame::{Frame, Direction, MAGIC};

pub mod protocol;
pub mod tail;

/// Message types (byte 8 of a frame).
pub mod msg_type {
    pub const PING: u8 = 0x68;
    pub const DATA: u8 = 0x6D;
    pub const SNAPSHOT: u8 = 0x63;
    pub const PRESET_LIST: u8 = 0x40;
    pub const QUERY: u8 = 0x41;
    pub const EFFECT_DATA: u8 = 0x06;
    /// Model-list enumeration: query one amp/drive slot name (`0x36`).
    pub const MODEL_LIST: u8 = 0x36;
    pub const SETTINGS: u8 = 0x65;
    pub const USB_VOLUME: u8 = 0x66;
    pub const PRESET_WRITE: u8 = 0x43;
    /// Device-originated notification (unsolicited; carries the new chain order
    /// after a chain write, among other things).
    pub const NOTIFY: u8 = 0x09;
}

/// `DATA` (0x6D) sub-commands.
pub mod sub {
    /// Generic "set scalar field" (preset index / cab / variant / ...).
    pub const SET_FIELD: u8 = 0x02;
    /// Enable / disable an effect slot.
    pub const ENABLE: u8 = 0x03;
    /// Write a single parameter value (float32).
    pub const PARAM_WRITE: u8 = 0x07;
    /// Preset naming.
    pub const NAME: u8 = 0x0A;
    // Effect-type switch uses `4 * (param_count + 1)`.
}

/// Field ids used by [`sub::SET_FIELD`] and the settings messages.
pub mod field {
    /// Effect variant / amp model.
    pub const VARIANT: u8 = 0x06;
    /// Cabinet / IR.
    pub const CAB: u8 = 0x07;
    /// USB volume (0-100), sent as a 0x66 settings message.
    pub const USB_VOLUME: u8 = 0x08;
    /// Preset index (0-based).
    pub const PRESET: u8 = 0x09;
}
