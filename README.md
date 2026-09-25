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
| Output gain (`0x65`) | ❌ not decoded |

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

## Shell completions

Dynamic completion (data-driven candidates: model names for `module … model`,
chain roles for `chain set`, …) — add to your shell config, regenerates itself
on every shell start:

```bash
fish: COMPLETE=fish nanocore-cli | source      # ~/.config/fish/completions/nanocore-cli.fish
bash: source <(COMPLETE=bash nanocore-cli)     # ~/.bashrc
zsh:  source <(COMPLETE=zsh nanocore-cli)      # ~/.zshrc
```

Prefer a static script (subcommands + flags only, no re-sourcing needed)?

```bash
nanocore-cli completions fish > ~/.config/fish/completions/nanocore-cli.fish
# bash/zsh/powershell: see `nanocore-cli completions --help`
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
write commands is unknown — helpers in [`tail`] let you detect/strip it.

## Resolved details

* **Parameter address bytes** in the `0x07` param-write message are now
  decoded: `b6` = parameter index within the effect, `b5` = slot/instance
  (`0x00` fx1, `0x01` fx2), and `b3` is an **app-side fragment marker**
  (cycles `0x00..0x38` in steps of 8) that is *not* needed for writes — a
  single frame with `b3 = 0x00` updates the whole parameter on real hardware.
* **Float encoding is confirmed.** Parameter values are IEEE754 float32, 4
  little-endian bytes, sent with the high bit of every byte cleared
  (`encode_param_value`). This is mandatory: the transport is SysEx, so any
  byte `>= 0x80` is a status byte and the message is dropped. With 4 bytes ×
  7 data bits the device receives only 28 bits; bit 23 (the exponent LSB) is
  among the four forced-to-zero bits, so odd-exponent values collapse to the
  next lower even exponent (`0.3 → 0.1495`). That quantization is inherent to
  the hardware and the official app does the same — verified across all 481
  writes in a real knob-sweep capture (every value byte `< 0x80`).

## Open questions

1. **Metadata tail** requirement on writes (the `da 00 0e ...` block the app
   appends; the device answers full 13-byte ACKs without us mirroring it, but
   its exact role is unconfirmed).
2. **Effect option tables** for the other 7 slots (only fx1 is mapped).
3. **Output gain** (`0x65`) value location.
4. **b3 semantics** — what the 8-way `0x08/0x10/0x20` selector means on the
   app side (purely cosmetic for our writes).

## Tests

```bash
cargo test                      # protocol + retry tests, no hardware needed
cargo build --features midi     # builds nanocore-cli
```

Tests use real captured byte sequences from the official app.

## CLI

```bash
nanocore-cli ports                        # enumerate MIDI ports
nanocore-cli preset set 5                 # select preset index 5
nanocore-cli preset next / prev / list    # step / list the 64 preset slots
nanocore-cli module amp off               # mute the amp
nanocore-cli module amp model 3           # select amp model (CC43)
nanocore-cli module cab model 8           # select cab/IR model (CC44)
nanocore-cli module fx1 on                # enable a module
nanocore-cli module delay params          # list a module's parameters (offline)
nanocore-cli module delay param mix 0.3   # set a parameter (SysEx)
nanocore-cli module delay model 2         # select an effect type (CC)
nanocore-cli chain                        # usage for `chain set` / `chain show`
nanocore-cli chain set 4 7 0 1 2 3 5 6    # write the chain order (echo-verified)
nanocore-cli models / models --live       # built-in model tables / live checksum
nanocore-cli show                         # readable state snapshot (best-effort)
nanocore-cli volume 90                    # USB output volume
nanocore-cli tuner on / off               # tuner (state read pending)
nanocore-cli asset read ir 0 out.bin      # pull a raw IR blob out of slot 0
# protocol / debugging (undocumented framing):
nanocore-cli debug monitor                # dump every inbound frame
nanocore-cli debug raw 6d 00 02 00 00 09 01
nanocore-cli debug replay 7d 4e 43 70 ...
nanocore-cli debug probe 1                # preset-switch flow + 3s dump
```

The command surface mirrors the device model: a **preset** holds 8 effect
**modules** (`fx1 fx2 amp cab mod delay reverb eq`), and parameters belong to
their module:

```text
preset  →  preset set/next/prev/list/show
module  →  module <m> on|off|model|params|param
chain   →  chain set/show
```

Where a module's parameter list depends on its current effect type
(`delay`/`reverb`/`fx1`/`fx2`/`mod`/`eq`), pass `--type N`
(`nanocore-cli module delay params --type 1`). Once the snapshot decode is
finished, `--type` will become optional and inferred from the device, and
`module <m> param <name>` without a value will read the current value
(currently it is a write; omit-VALUE reads are pending).

Two caveats inherited from the hardware:
* `module amp model` / `module cab model` accept **0–29**. The 8 FX2 drive
  pedals live in amp slots 30–37 and are only reachable via `debug variant`.
* `show` is best-effort for now — active preset, per-module state, volume and
  tuner are marked `(TBD)` until the `0x63` snapshot decode is completed.

Flags: `--in N` `--out N` `--hint NAME` `--timeout MS` `--retries N`
`--expect HH` `--flags HH` `--no-wait` (protocol work, hidden from the help).

## Disclaimer & license

Unofficial project, not affiliated with LIVTRA. Research-oriented; use at
your own risk.

GPL-3.0-only. See [LICENSE](LICENSE).
