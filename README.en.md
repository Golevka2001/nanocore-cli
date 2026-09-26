# nanocore-cli

[简体中文](README.md) | English

---

Unofficial Rust CLI for the [LIVTRA NanoCore](https://livtramusic.com/products/nanocore) pocket effects pedal (TPE-S10).

Drives a real device over USB-MIDI. Controls preset switching, on/off / model / parameters of the 8 effect modules, effect-chain order, USB volume, the tuner, and read/write of amp-model and IR assets.

## CLI

```sh
nanocore-cli ports                        # enumerate MIDI ports

nanocore-cli preset list                  # list the 64 preset slots
nanocore-cli preset set 5                 # select preset 5
nanocore-cli preset next / prev           # step to the next / previous preset

nanocore-cli module amp off               # turn the amp module off
nanocore-cli module amp model 3           # select an amp model
nanocore-cli module cab model 8           # select a cab/IR model
nanocore-cli module fx1 on                # enable / disable a module
nanocore-cli module delay params          # list a module's parameters (offline)
nanocore-cli module delay param mix 0.3   # set a parameter
nanocore-cli module delay model 2         # select an effect type

nanocore-cli chain                        # usage for `chain set` / `chain show`
nanocore-cli chain set 4 7 0 1 2 3 5 6    # write the chain order (echo-verified)

nanocore-cli models / models --live       # built-in model tables / live query

nanocore-cli show                         # readable state snapshot

nanocore-cli volume 90                    # USB output volume

nanocore-cli tuner on / off               # tuner (state read pending)

nanocore-cli asset read ir 0 --out out.bin   # pull the slot-0 IR blob

# protocol / debugging (undocumented framing):
nanocore-cli debug monitor                # dump every inbound frame
nanocore-cli debug raw 6d 00 02 00 00 09 01
nanocore-cli debug replay 7d 4e 43 70 ...
nanocore-cli debug probe 1                # preset-switch flow + 3s dump
```

The command surface mirrors the device model: a **preset** holds 8 effect **modules** (`fx1 fx2 amp cab mod delay reverb eq`), and parameters belong to their module:

```text
preset  →  preset set/next/prev/list/show
module  →  module <m> on|off|model|params|param
chain   →  chain set/show
```

Where a module's parameter list depends on its current effect type (`delay`/`reverb`/`fx1`/`fx2`/`mod`/`eq`), pass `--type N` (`nanocore-cli module delay params --type 1`). Once the snapshot decode is finished, `--type` will become optional (inferred from the device), and `module <m> param <name>` without a value will read the current value (currently it is a write; omit-VALUE reads are pending).

Two caveats inherited from the hardware:

- `module amp model` / `module cab model` accept **0–29**. The 8 FX2 drive pedals live in amp slots 30–37 and are only reachable via `debug variant`.
- `show` is best-effort for now — active preset, per-module state, volume and tuner are marked `(TBD)` until the `0x63` snapshot decode is completed.

Global flags: `--in-port N` `--out-port N` `--hint NAME` `--ch 0-15` `--timeout MS` `--retries N` (plus hidden protocol-debugging flags, see the `debug` command docs).

Exit codes: `0` success; `2` bad arguments or a communication error; `3` a verification mismatch (`asset verify` / the read-back check in `asset write`).

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
```

```sh
cargo test
```

## Disclaimer & license

Unofficial project, not affiliated with LIVTRA. Research-oriented; use at your own risk.

GPL-3.0-only. See [LICENSE](LICENSE).
