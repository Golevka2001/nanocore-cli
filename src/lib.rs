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
