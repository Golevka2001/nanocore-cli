# nanocore-cli

Unofficial Rust CLI to control the LIVTRA NanoCore (TPE-S10), a pocket
effects pedal.

The crate `nanocore` provides a control **library**; the `nanocore-cli`
binary (feature `midi`) drives a real device over USB-MIDI. All byte
layouts are derived from real MIDI captures of the official app.

## Status

| Feature | Status |
|---------|--------|
| Frame encode/decode (`7D 4E 43`) | ✅ verified |
| Select preset `0x02/field 0x09` | ✅ verified |
| Select cab `0x02/field 0x07` | ✅ verified |
| Select variant/amp `0x02/field 0x06` | ✅ verified |
| Enable/disable slot `0x03` | ✅ verified |
| Param write `0x07` (float32) | ✅ value layout verified; address bytes TBD |
| Rename `0x0A` | ✅ verified |
| Effect-type switch `4*(N+1)` | ✅ verified for fx1 |
| USB volume `0x66/field 0x08` | ✅ verified |
| Snapshot `0x63` read | ⚠️ partial (float scan + chainOrder) |
| Effect data `0x06` read | ✅ parse verified |
| Preset list `0x40` read | ⚠️ parser present, TX format TBD |

## Real-device status (USB-MIDI, macOS) — working

The device enumerates as a MIDI in/out port named `Nanocore`. Verified against
real hardware:

| Command | Result |
|---------|--------|
| `0x68` ping | RX 9 B, sequence echoed |
| `0x6D/0x02` select preset | RX `6d 00 00 02 00 00 09 <idx>` — device switches |
| `0x63` snapshot | RX 169 B, chain order at the tail |
| `0x40` preset list | RX 98 B, names decoded (`ClnArp`, `Ambient`, …) |

Preset indices are **0-based**: `select_preset(2)` shows `03` on the device.

### Three traps, found the hard way

1. **Every frame is SysEx-wrapped on the wire**: `F0 7D 4E 43 ... F7`. The
   captured `.mmon` files show *bare* frames because the monitor strips the
   wrapper — sending bare frames gets you silently ignored. `MidiTransport`
   adds the wrapper by default.
2. **The device broadcasts unsolicited `0x65`/`0x68` frames** roughly every
   250 ms using its *own* sequence counter (`0x0Fxx`, `0x13xx`). They look
   exactly like replies and are not one. Reply matching is an exact
   sequence-number echo check.
3. Replies echo the request's sequence number, so unsolicited traffic never
   matches — but a *late* reply is buffered in `pending` rather than dropped.

## Usage

```rust
use nanocore::{Nanocore, Transport};

struct MyMidi { /* midir connection */ }
impl Transport for MyMidi {
    type Error = std::io::Error;
    fn send(&mut self, data: &[u8]) -> Result<(), Self::Error> { /* ... */ Ok(()) }
    fn receive(&mut self) -> Result<Vec<u8>, Self::Error> { Ok(vec![]) }
}

let mut dev = Nanocore::new(MyMidi { /* ... */ });
dev.select_preset(0)?;
dev.set_enabled(2, true)?;
dev.set_usb_volume(90)?;
```

## Frame format

```text
7D 4E 43 | dir | flags | 02 | seq_lo | seq_hi | type | payload
```

* `dir`: `0x70` TX, `0x71` RX, `0x72` unsolicited event
* `flags`: observed `0x00` (normal), `0x02` (live), `0x22` (0x43), `0x40` (RX)
* byte 5 is constant `0x02`; `seq` is a little-endian counter echoed by the device

## Metadata tail

The official app appends a device-generated "compact state" block to most
messages:

```text
DA 00 0E <u16 ids...> 5A "statusByte" 5E "clockTimestamp" ... 5D "From Nanocore"
```

These strings are produced by the **device firmware** (they are absent from
both `libapp.so` and `liblivtra.so`). Whether the device requires the tail on
write commands is unknown — helpers in `nanocore::tail` let you detect/strip it.

## Tests

```bash
cargo test                      # protocol + retry tests, no hardware needed
cargo build --features midi     # builds nanocore-cli
```

Tests use real captured byte sequences from the official app.
