# nanocore-cli

[简体中文](README.md) | English

<p align="center">
  <img src="README.assets/banner.jpeg" alt="nanocore-cli banner" />
</p>

---

Unofficial Rust CLI for the [LIVTRA NanoCore](https://livtramusic.com/products/nanocore) pocket effects pedal (TPE-S10).

Drives a real device over USB-MIDI. Controls preset switching, on/off / model / parameters of the 8 effect modules, effect-chain order, and IR asset read/write — covering most of the official app's operations.

## CLI

> [!NOTE]
> All numbers in commands are **1-based** (preset 1-64, chain/module 1-8, slots 1-30/38, channel 1-16 — same as the device and app); only wire-byte parameters like `b5`/`b6` and CC stay raw.

```sh
nanocore-cli ports                        # enumerate MIDI ports

nanocore-cli preset list                  # list the 64 preset slots
nanocore-cli preset set 5                 # select preset 5
nanocore-cli preset next / prev           # step to the next / previous preset
nanocore-cli preset volume 80             # preset volume
nanocore-cli preset rename 5 MyRig        # rename a preset slot
nanocore-cli preset export 5 --out rig.npreset        # export a preset as .npreset
nanocore-cli preset import rig.npreset --with-assets  # import from .npreset (with IR/amp assets)
nanocore-cli preset restore-factory       # replay the official DSP-only factory restore (overwrites slots 1-40)

nanocore-cli module amp off               # turn the amp module off
nanocore-cli module amp model 3           # select an amp model
nanocore-cli module cab model 8           # select a cab/IR model
nanocore-cli module fx1 on                # enable / disable a module
nanocore-cli module delay params          # list a module's parameters (offline; reads the device when type-dependent)
nanocore-cli module delay param mix 0.3   # set a parameter (mask derived automatically)
nanocore-cli module delay param mix       # read the current value (from the snapshot)
nanocore-cli module delay model 2         # select an effect type

nanocore-cli chain                        # usage for `chain set` / `chain show`
nanocore-cli chain set 5 8 1 2 3 4 6 7    # write the chain order (echo-verified)

nanocore-cli tuner on / off               # tuner on/off

nanocore-cli models / models --live       # built-in model tables / live query

nanocore-cli show                         # state snapshot: chain, preset, volume, modules

nanocore-cli settings show                # settings overview (volume/wireless/playback/gain/channel)
nanocore-cli settings usb-volume 90        # USB output volume
nanocore-cli settings bt-volume 80        # bluetooth volume
nanocore-cli settings wireless off        # enable / disable wireless
nanocore-cli settings input-gain -- -3    # input gain (dB, may be negative)
nanocore-cli settings input-channel 2     # input channel

nanocore-cli asset read ir 1 --out out.bin   # pull the slot-1 IR blob
nanocore-cli asset write amp 29 tone.ead     # write an .ead/.eadl model to an amp slot

# protocol / debugging (undocumented framing):
nanocore-cli debug monitor                # dump every inbound frame
nanocore-cli debug raw 6d 00 02 00 00 09 01
nanocore-cli debug replay 7d 4e 43 70 ...
nanocore-cli debug probe 1                # preset-switch flow + 3s dump
```

The command surface mirrors the device model: a **preset** holds 8 effect **modules** (`fx1 fx2 amp cab mod delay reverb eq`), and parameters belong to their module:

```text
preset  →  preset set/next/prev/list/show/volume/rename/export/import/restore-factory
module  →  module <m> on|off|model|params|param
chain   →  chain set/show
```

Where a module's parameter list depends on its current effect type (`delay`/`reverb`/`fx1`/`fx2`/`mod`/`eq`), you can pass `--type N` (`nanocore-cli module delay params --type 1`); omit it and the current effect type is read from the device snapshot. `module <m> param <name>` without a value reads the current value: it comes from the active preset snapshot, where `b6` indexes the device array directly.

Global flags: `--in-port N` `--out-port N` `--hint NAME` `--ch 1-16` `--timeout MS` `--retries N` (plus hidden protocol-debugging flags, see the `debug` command docs).

Exit codes: `0` success; `2` bad arguments or a communication error; `3` a verification mismatch.

## Shell completions

fish:

```sh
 # ~/.config/fish/completions/nanocore-cli.fish
COMPLETE=fish nanocore-cli | source
```

bash:

```sh
# ~/.bashrc
source <(COMPLETE=bash nanocore-cli)
```

zsh:

```sh
# ~/.zshrc
source <(COMPLETE=zsh nanocore-cli)
```

## Build & test

```sh
cargo build --features midi
# Without `--features midi`, cargo builds the library only and produces no executable
```

```sh
cargo test
```

## Disclaimer & license

Unofficial project, not affiliated with LIVTRA. Research-oriented; use at your own risk.

GPL-3.0-only. See [LICENSE](LICENSE).
