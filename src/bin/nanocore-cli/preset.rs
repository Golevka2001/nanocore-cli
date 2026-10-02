//! `preset` 命令：选择 / 步进 / 列出预设槽位 / 恢复出厂预设。

use nanocore::midicc;

use crate::cli::{Conn, PresetCmd};
use crate::common::{Failure, open_device, send_standard, transact};

/// wire 槽位（0 起，`255` = unknown）→ 用户编号显示（1 起 / `unknown`）。
fn slot_disp(v: u8) -> String {
    if v == 255 {
        "unknown".into()
    } else {
        format!("#{}", v + 1)
    }
}

pub fn cmd_preset(cmd: PresetCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        PresetCmd::Show => {
            let mut dev = open_device(conn)?;
            let snap = dev.read_snapshot().map_err(|e| e.to_string())?;
            let state = nanocore::protocol::decode_snapshot_state(&snap.payload)
                .ok_or_else(|| {
                    Failure::error(
                        "the 0x63 snapshot did not decode (only the USB payload form \
                         is supported); `debug snapshot` prints the raw bytes",
                    )
                })?;
            let names = dev.read_preset_names().map_err(Failure::error)?;
            let label = match names.iter().find(|e| e.slot == state.preset_index) {
                Some(e) if e.named => format!(" {}", e.name),
                Some(_) => "  (slot is empty)".into(),
                None => String::new(),
            };
            println!("preset:   #{}{label}", state.preset_index + 1);
            println!("volume:   {} (presetVolume)", state.preset_volume);
            if let Some(chain) = state.chain.as_deref() {
                println!("chain:    {}", crate::common::render_chain(chain));
            }
            Ok(())
        }
        PresetCmd::Export { slot, out, amp_slot, ir_slot } => {
            // 用户编号 1–64 → wire 槽位 0–63。
            let wire = slot - 1;
            let mut dev = open_device(conn)?;
            let names = dev.read_preset_names()?;
            let entry = names.iter().find(|e| e.slot == wire);
            let name = match entry {
                Some(e) if e.named => e.name.clone(),
                _ => {
                    return Err(Failure::error(format!(
                        "slot #{slot} is empty or unnamed; nothing to export"
                    )))
                }
            };
            // 切到目标预设，让快照与"活动型号"都指向它。
            let (k, p) = nanocore::protocol::select_preset(wire);
            transact(&mut dev, conn, k, p, &[])?;
            dev.wait_for_preset_switch_event(wire, 5_000)
                .map_err(Failure::error)?;
            std::thread::sleep(std::time::Duration::from_millis(300));
            let snap = dev.read_snapshot().map_err(|e| e.to_string())?;
            let state = nanocore::protocol::decode_snapshot_state(&snap.payload)
                .ok_or_else(|| {
                    Failure::error("the 0x63 snapshot did not decode (only the USB payload form is supported)")
                })?;
            let Some(chain_vec) = state.chain else {
                return Err(Failure::error(
                    "snapshot has no chain order; cannot export without it",
                ));
            };
            let Ok(chain) = <[u8; 8]>::try_from(chain_vec.as_slice()) else {
                return Err(Failure::error(format!(
                    "snapshot chain order has {} entries, expected 8",
                    chain_vec.len()
                )));
            };
            // 角色记录 → manifest slots。effectId 是静态角色表
            // （40 组出厂 manifest 对账）：0-7 → [7,8,1,2,5,4,3,6]。
            const ROLE_EFFECT_IDS: [u8; 8] = [7, 8, 1, 2, 5, 4, 3, 6];
            let slots = state
                .records
                .iter()
                .enumerate()
                .map(|(i, r)| nanocore::preset_bundle::SlotSpec {
                    effect_id: ROLE_EFFECT_IDS[i],
                    variant: r.variant,
                    enabled: r.enabled != 0,
                    params: r.params.clone(),
                    tempo_sync: vec![],
                })
                .collect();
            // amp 引用：显式指定优先（1–38，0 = unknown），否则用设备报告的活动型号槽。
            let amp = match amp_slot {
                Some(0) => 255,
                Some(a) => a - 1,
                None => match dev.read_active_amp_slot() {
                    Ok(Some(a)) => a,
                    _ => {
                        return Err(Failure::error(
                            "could not determine the active amp slot (0x36 query \
                             ambiguous); pass --amp-slot explicitly",
                        ))
                    }
                },
            };
            let ir = match ir_slot {
                None | Some(0) => 255,
                Some(v) => v - 1,
            };
            let spec = nanocore::preset_bundle::PresetSpec {
                version: 7,
                name: name.clone(),
                chain_order: chain,
                slots,
                amp_slot: amp,
                ir_slot: ir,
                fx2_slot: 255,
                preset_volume: state.preset_volume,
                expression_mappings: vec![],
                extra_tlvs: vec![],
                runtime_effects: vec![],
            };
            let manifest = nanocore::preset_bundle::Manifest::new(
                "single",
                vec![nanocore::preset_bundle::ManifestPreset {
                    slot: wire,
                    name,
                    preset: spec,
                }],
            );
            nanocore::preset_bundle::write_bundle(
                std::path::Path::new(&out),
                &manifest,
                &Default::default(),
            )
            .map_err(Failure::error)?;
            println!(
                "OK: exported preset #{slot} ({}) to {out} — amp {}, ir {}",
                manifest.presets[0].name,
                slot_disp(amp),
                slot_disp(ir)
            );
            Ok(())
        }
        PresetCmd::Import { file, slot, with_assets, dry_run } => {
            let (manifest, assets) = nanocore::preset_bundle::read_bundle(std::path::Path::new(&file))
                .map_err(Failure::error)?;
            if manifest.presets.is_empty() {
                return Err(Failure::error("the bundle contains no presets"));
            }
            if manifest.presets.len() > 40 {
                return Err(Failure::error(format!(
                    "the bundle has {} presets; at most 40 can be imported in one session",
                    manifest.presets.len()
                )));
            }
            let mut pairs: Vec<(u8, nanocore::preset43::PresetWrite)> = Vec::new();
            for entry in &manifest.presets {
                // `--slot` 是用户编号 1–40；缺省沿用 manifest 里记录的 wire 槽位。
                let target = slot.map(|s| s - 1).unwrap_or(entry.slot);
                if target > 39 {
                    return Err(Failure::error(format!(
                        "import target slot {} out of range 1-40: the write path \
                         does not support slots 41-64 (content lands without its name \
                         and the previous slot gets cleared; observed 2026-10-01)",
                        target + 1
                    )));
                }
                let mut pw = entry.preset.to_preset_write();
                pw.name = entry.preset.name.clone();
                pairs.push((target, pw));
            }
            if dry_run {
                for (target, pw) in &pairs {
                    let (lo, sh) = nanocore::preset43::build_frames(pw)
                        .map_err(Failure::error)?;
                    println!(
                        "slot {:>2} {:?}: long {} B, short {} B, vol {}, amp {}, ir {}",
                        target + 1,
                        pw.name,
                        lo.len(),
                        sh.len(),
                        pw.preset_volume,
                        slot_disp(pw.amp_slot),
                        slot_disp(pw.ir_slot)
                    );
                }
                return Ok(());
            }
            eprintln!(
                "warning: importing overwrites the target preset slots. Do not \
                 disconnect the device until the command prints its final verdict."
            );
            let mut dev = open_device(conn)?;
            // 资产先行：IR 写会话（0x49）会清空预设表，必须先于预设写入；
            // amp 写（0x30 体系）实测不清表，但统一放前面。
            if with_assets {
                for a in &manifest.ir_slots {
                    let Some(blob) = assets.get(&a.asset) else {
                        return Err(Failure::error(format!(
                            "bundle references {} but does not embed it",
                            a.asset
                        )));
                    };
                    eprintln!("writing ir slot {} ({:?})…", a.slot + 1, a.name);
                    dev.write_ir_slot(a.slot, blob, Some(&a.name), |msg| {
                        eprint!("\r{msg}");
                    })?;
                    eprintln!();
                }
                for a in &manifest.amp_slots {
                    let Some(blob) = assets.get(&a.asset) else {
                        eprintln!(
                            "note: bundle references {} without embedded data; \
                             skipping (device slot must already hold the model)",
                            a.asset
                        );
                        continue;
                    };
                    let ead = nanocore::amp_write::prepare_amp_input(blob)
                        .map_err(Failure::error)?;
                    eprintln!("writing amp slot {} ({:?})…", a.slot + 1, a.name);
                    dev.write_amp_slot(a.slot, &ead, Some(&a.name), |msg| {
                        eprint!("\r{msg}");
                    })?;
                    eprintln!();
                }
            }
            let targets: Vec<(u8, String)> =
                pairs.iter().map(|(s, pw)| (*s, pw.name.clone())).collect();
            dev.import_presets(&pairs, |msg| {
                eprint!("\r{msg}");
            })?;
            eprintln!();
            // flash 落盘异步生效（整体滞后可达数十秒），重试验证名表直到稳定。
            let mut ok = 0usize;
            let mut names = Vec::new();
            for attempt in 0..6 {
                std::thread::sleep(std::time::Duration::from_secs(3));
                names = dev.read_preset_names()?;
                ok = targets
                    .iter()
                    .filter(|(target, want)| {
                        names
                            .iter()
                            .find(|e| e.slot == *target)
                            .map(|e| e.named && &e.name == want)
                            .unwrap_or(false)
                    })
                    .count();

                if ok == targets.len() {
                    break;
                }
                eprintln!(
                    "verify: {ok}/{} settled (attempt {})…",
                    targets.len(),
                    attempt + 1
                );
            }
            for (target, want) in &targets {
                let got = names
                    .iter()
                    .find(|e| e.slot == *target)
                    .map(|e| (e.named, e.name.clone()));
                match got {
                    Some((true, name)) if &name == want => {}
                    Some((true, name)) => {
                        eprintln!("slot #{}: name {name:?} != {want:?}", target + 1)
                    }
                    other => eprintln!("slot #{}: not written ({other:?})", target + 1),
                }
            }
            println!("OK: {ok}/{} preset(s) verified by name table.", targets.len());
            if ok == targets.len() {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "{}/{} presets verified",
                    ok,
                    targets.len()
                )))
            }
        }
        PresetCmd::Volume { percent } => {
            let mut dev = open_device(conn)?;
            let read_volume =
                |dev: &mut crate::common::Dev| -> Result<u8, Failure> {
                let snap = dev.read_snapshot().map_err(|e| e.to_string())?;
                let state = nanocore::protocol::decode_snapshot_state(&snap.payload).ok_or_else(
                    || {
                        Failure::error(
                            "the 0x63 snapshot did not decode (only the USB payload form \
                             is supported)",
                        )
                    },
                )?;
                Ok(state.preset_volume)
            };
            let before = read_volume(&mut dev)?;
            let (k, p) = nanocore::protocol::set_preset_volume(percent);
            transact(&mut dev, conn, k, p, &[])?;

            // 设备的快照不是即时刷新的，稍候再回读。
            std::thread::sleep(std::time::Duration::from_secs(1));
            let after = read_volume(&mut dev)?;
            println!("volume:   {before} -> {after} (presetVolume)");
            if after == percent {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "volume write did not stick: set {percent}, snapshot reads {after}"
                )))
            }
        }
        PresetCmd::List => {
            let mut dev = open_device(conn)?;
            let entries = dev.read_preset_names()?;
            for e in &entries {
                println!(
                    "  #{:<2} {:<12} {}",
                    e.slot + 1,
                    e.name,
                    if e.named { "" } else { "(empty)" }
                );
            }
            Ok(())
        }
        PresetCmd::Rename { slot, name } => {
            let field = nanocore::protocol::encode_preset_name_field(&name)
                .map_err(Failure::error)?;
            let mut dev = open_device(conn)?;
            // 与官方抓包一致：field 0x09 选槽 → 等设备自发切槽事件 → 写名字区。
            // （官方在选槽与改名之间有 ~6 个 PING 的等待；等事件比等时间更可靠。）
            let (k, p) = nanocore::protocol::select_preset(slot);
            transact(&mut dev, conn, k, p, &[])?;
            match dev.wait_for_preset_switch_event(slot, 5_000) {
                Ok(f) => println!("switch event: {}", crate::common::hex(&f.payload)),
                Err(e) => {
                    let traffic = dev.drain_frames(3_000);
                    return Err(Failure::error(format!(
                        "{e}; device traffic during 3s drain: {traffic}"
                    )));
                }
            }
            let (k, p) = nanocore::protocol::set_name_field(&field);
            transact(&mut dev, conn, k, p, &[])?;

            // flash 落盘异步生效，稍候验证。
            std::thread::sleep(std::time::Duration::from_secs(2));
            let entries = dev.read_preset_names()?;
            let got = entries
                .iter()
                .find(|e| e.slot == slot)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            let want: String = name.chars().filter(|&c| c != '\0').collect();
            println!("slot #{slot}: {got:?} (expected {want:?})");
            if got == want {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "rename did not stick: got {got:?}, expected {want:?}"
                )))
            }
        }
        PresetCmd::Set { index } => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::program_change(conn.channel, index))
        }
        PresetCmd::Next => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::global::next_preset(conn.channel))
        }
        PresetCmd::Prev => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &midicc::global::prev_preset(conn.channel))
        }
        PresetCmd::RestoreFactory => {
            // 语义与官方 App 的"仅 DSP 恢复"一致：40 组出厂预设整体重写。
            // 会话开场（0x49）会清空预设表，随后流程将其完整重写。
            // seq 7 位安全修复（b632aaa）后已在 fw 1.04 真机验证 40/40 恢复。
            eprintln!(
                "warning: this overwrites slots 1-40 with factory names and\n  \
                 contents. Do not disconnect the device until the command\n  \
                 prints its final verdict."
            );
            let mut dev = open_device(conn)?;
            dev.restore_factory_presets(|msg| {
                eprint!("\r{msg}");
            })
            .map_err(Failure::error)?;
            eprintln!();
            println!("OK: official DSP restore sequence replayed.");

            // flash 落盘异步生效（观察到提交后 ~2.4 s 的过期读），稍候再验证名表。
            std::thread::sleep(std::time::Duration::from_secs(3));
            let entries = dev.read_preset_names()?;
            let named = entries.iter().filter(|e| e.named).count();
            println!("preset table now has {named}/40 factory-named slots.");
            if named == 40 {
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "expected 40 named slots, got {named}"
                )))
            }
        }
    }
}
