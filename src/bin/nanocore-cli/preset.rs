//! `preset` 命令：选择 / 步进 / 列出预设槽位 / 恢复出厂预设。

use std::collections::BTreeMap;

use nanocore::frame::Frame;
use nanocore::preset_bundle::FX2_SLOT_NONE;

use crate::cli::{Conn, PresetCmd};
use crate::common::{Dev, Failure, open_device, progress, send_standard, transact, transact_reply};

/// wire 槽位（0 起，`255` = unknown）→ 用户编号显示（1 起 / `unknown`）。
fn slot_disp(v: u8) -> String {
    if v == 255 {
        "unknown".into()
    } else {
        format!("#{}", v + 1)
    }
}

/// 等待设备报告的预设槽位稳定在期望值。
///
/// 背景（2026-10-11 真机发现）：`preset set` 原先只发一条**裸 MIDI program change**
/// 就返回，不等任何应答。设备是异步提交的，于是紧接着跑 `preset show` 会稳定地读到
/// **上一个**预设（一位延迟），要等 2 s 才对。`RestoreFactory` 分支早就知道这个行为
///（"flash 落盘异步生效，观察到提交后 ~2.4 s 的过期读"）并用固定 sleep 兜住，
/// 但 set / next / prev 一直没有。
///
/// 现在改用 `select_preset_checked`（`0x66 SET_FIELD` + 等应答）并轮询快照复核，
/// 让返回值可信。`expect = None`（next/prev）只要求读数稳定下来。
pub fn settle(dev: &mut Dev, expect: Option<u8>) -> Result<(), Failure> {
    wait_preset_settled(dev, expect)
}

fn wait_preset_settled(dev: &mut Dev, expect: Option<u8>) -> Result<(), Failure> {
    use std::time::{Duration, Instant};

    let deadline = Instant::now() + Duration::from_secs(8);
    let mut last: Option<u8> = None;
    let mut stable_since = Instant::now();

    loop {
        // 刚发出切槽指令后设备往往还在忙，0x63 会偶发超时。真机实测：
        // `preset next` / `preset prev` 发的是 CC81/CC82（无应答），紧接着的第一次
        // 快照请求就可能连超时 4 次。因此单次读失败不该直接判失败——预算内继续重试，
        // 只有最后一次仍读不到才报错。
        let idx = match dev.read_snapshot() {
            Ok(snap) => nanocore::protocol::decode_snapshot_state(&snap.payload)
                .map(|s| s.preset_index),
            Err(_) => None,
        };
        let Some(idx) = idx else {
            if Instant::now() >= deadline {
                return Err(Failure::error(
                    "the device stopped answering the snapshot request".to_string(),
                ));
            }
            std::thread::sleep(Duration::from_millis(150));
            continue;
        };

        if let Some(want) = expect {
            if idx == want {
                return Ok(());
            }
        } else if last == Some(idx) && stable_since.elapsed() >= Duration::from_millis(2000) {
            return Ok(());
        } else if last != Some(idx) {
            stable_since = Instant::now();
        }
        last = Some(idx);

        if Instant::now() >= deadline {
            return Err(Failure::mismatch(match expect {
                Some(want) => format!(
                    "the device still reports preset #{} after 8 s (expected #{want})",
                    last.unwrap_or(0) as u32 + 1
                ),
                None => "the preset index never settled after 8 s".into(),
            }));
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

/// 应答的 `payload[1]` 是状态字节，非 0 即被拒。
///
/// 改名三帧里第三帧（提交）被拒时设备回 `01 7e 00 00 00` —— 之前这里没有检查，
/// 命令便一路当成功往下走，最后只报一句"名表没变"，把真正的拒收原因掩盖了。
fn check_status(reply: Option<Frame>, what: &str) -> Result<(), Failure> {
    let Some(frame) = reply else {
        return Err(Failure::error(format!(
            "the device did not answer the `{what}` step of the rename"
        )));
    };
    match frame.payload.get(1) {
        Some(&0x00) => Ok(()),
        Some(&status) => Err(Failure::mismatch(format!(
            "the device rejected the `{what}` step: status {status:02x}"
        ))),
        None => Ok(()),
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
            println!("preset:        #{}{label}", state.preset_index + 1);
            println!("preset volume: {}", state.preset_volume);
            if let Some(chain) = state.chain.as_deref() {
                println!("chain:         {}", crate::common::render_chain(chain));
            }
            Ok(())
        }
        PresetCmd::Export {
            slot,
            out,
            amp_slot,
            ir_slot,
        } => {
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
            //
            // ⚠️ 设备对 `select_preset` 只回 status 00 的应答，**从不发自发切槽事件**
            // （真机实测 fw 1.04，5 s drain 结果为 "nothing arrived"）。原先把等事件
            // 当硬依赖，导致 `preset export` 在任何槽上都必然超时失败。
            // 改为：事件只是加速手段，拿不到就退化为轮询 0x63 快照确认 preset_index，
            // 那才是权威判据。
            let (k, p) = nanocore::protocol::select_preset(wire);
            transact(&mut dev, conn, k, p, &[])?;
            if let Ok(f) = dev.wait_for_preset_switch_event(wire, 1_500) {
                let _ = f;
            } else {
                let _ = dev.drain_frames(300);
            }
            wait_preset_settled(&mut dev, Some(wire))?;
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
            // ：0-7 → [7,8,1,2,5,4,3,6]。
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
            // 与 amp 对称：不指定就查设备当前引用的 IR，而不是直接记成"未知"。
            // 之前 IR 一律填 `FX2_SLOT_NONE`(255)，导出文件里的 `irSlot` 就永远是
            // 255 —— 等于把"CAB"这一项悄悄丢掉了，而 amp 却能自动填上。
            let ir = match ir_slot {
                Some(0) => FX2_SLOT_NONE,
                Some(v) => v - 1,
                None => match dev.read_active_ir_slot() {
                    Ok(Some(s)) => s.index,
                    Ok(None) => FX2_SLOT_NONE,
                    Err(e) => {
                        return Err(Failure::error(format!(
                            "could not determine the active IR slot ({e}); \
                             pass --ir-slot explicitly"
                        )))
                    }
                },
            };
            let spec = nanocore::preset_bundle::PresetSpec {
                version: 7,
                name: name.clone(),
                chain_order: chain,
                slots,
                amp_slot: amp,
                ir_slot: ir,
                fx2_slot: FX2_SLOT_NONE,
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
            let manifest = manifest;
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
            println!(
                "note: expressionMappings / extraTlvs / runtimeEffects / fx2Slot / \
                 tempoSync are written empty — the snapshot does not carry them"
            );
            Ok(())
        }
        PresetCmd::Import { file, slot, with_assets, dry_run, force } => {
            let (manifest, assets) = nanocore::preset_bundle::read_bundle(std::path::Path::new(&file))
                .map_err(Failure::error)?;
            if manifest.presets.is_empty() {
                return Err(Failure::error("the bundle contains no presets"));
            }
            let dropped: BTreeMap<&str, usize> = manifest
                .presets
                .iter()
                .flat_map(|e| e.preset.passthrough_counts())
                .fold(BTreeMap::new(), |mut m, (k, n)| {
                    *m.entry(k).or_default() += n;
                    m
                });
            if !dropped.is_empty() {
                let list: Vec<String> =
                    dropped.iter().map(|(k, n)| format!("{k}×{n}")).collect();
                eprintln!(
                    "warning: the bundle carries {}; 0x43 cannot express them, importing \
                     will drop them",
                    list.join(", ")
                );
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
            // **导入是破坏性的**：写会话用 `0x49` 开启，设备会先清空整张预设表，
            // 再靠随后的每组 `0x43` 逐槽写回来。官方 App 从不导入少于 40 组——
            // `导入预设包.mmon` 里是 41 组写、40 个保存帧，出厂 40 个预设全部重写。
            //
            // 只导入 1 个预设就会把其余 39 个的名字抹成空，**且不报错**（真机实测）。
            // 与其在这里悄悄做一份完整导出（那要走一遍全部槽的 0x41 往返，很容易再出
            // 岔子），不如直接拒绝：破坏性操作要由用户显式确认。
            //
            // 守卫放在 `--dry-run` 之后：dry-run 只打印将要发送的帧，不碰设备，
            // 没有理由要求它先去读设备名表（没有设备时也能预览导入内容）。
            {
                let mut dev = open_device(conn)?;
                let existing = dev.read_preset_names().map_err(Failure::error)?;
                let named = existing.iter().filter(|e| e.named).count();
                if named > manifest.presets.len() && !force {
                    return Err(Failure::error(format!(
                        "refusing to import: the device holds {named} named presets but the \
                         bundle has {}. The write session (0x49) clears the whole table and \
                         rewrites only the slots in the bundle, so the other {} would end up \
                         empty with no error. Export them first, or pass --force to import \
                         anyway.",
                        manifest.presets.len(),
                        named.saturating_sub(manifest.presets.len())
                    )));
                }
            }
            eprintln!(
                "warning: importing overwrites the target preset slots. Do not \
                 disconnect the device until the command prints its final verdict."
            );
            let mut dev = open_device(conn)?;
            // 资产先行：IR 写会话（0x49）会清空预设表，必须先于预设写入；
            // amp 写（0x30 体系）不清表，但统一放前面。
            if with_assets {
                for a in &manifest.ir_slots {
                    let Some(blob) = assets.get(&a.asset) else {
                        return Err(Failure::error(format!(
                            "bundle references {} but does not embed it",
                            a.asset
                        )));
                    };
                    eprintln!("writing ir slot {} ({:?})…", a.slot + 1, a.name);
                    dev.write_ir_slot(a.slot, blob, Some(&a.name), progress)?;
                    eprintln!();
                }
                for a in &manifest.amp_slots {
                    if a.encoding == "ead-payload" {
                        eprintln!(
                            "note: amp slot {} is a device-internal DDBP blob; skipping \
                             (its .ead source is not recoverable)",
                            a.slot + 1
                        );
                        continue;
                    }
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
                    dev.write_amp_slot(a.slot, &ead, Some(&a.name), progress)?;
                    eprintln!();
                }
            }
            let targets: Vec<(u8, String)> =
                pairs.iter().map(|(s, pw)| (*s, pw.name.clone())).collect();
            dev.import_presets(&pairs, progress)?;
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

            // 帧里没有槽位字段 → 作用于**当前活动预设**。而且只发 `sub 0x02` 是不
            // 落盘的：真机实测设成 50，切走再切回仍是原值。要跟一帧 `0x46` 保存，
            // 与改名同一模式（`name_commit`）。
            let active = dev
                .read_snapshot()
                .ok()
                .and_then(|sn| nanocore::protocol::decode_snapshot_state(&sn.payload))
                .map(|st| st.preset_index);
            let wire = active.ok_or_else(|| {
                Failure::error("could not read the active preset index".to_string())
            })?;

            let before = read_volume(&mut dev)?;
            let (k, p) = nanocore::protocol::set_preset_volume(percent);
            transact(&mut dev, conn, k, p, &[])?;
            let (k, p) = nanocore::protocol::name_commit(wire);
            check_status(transact_reply(&mut dev, conn, k, p, &[])?, "volume save")?;

            // 设备的快照不是即时刷新的，稍候再回读。
            std::thread::sleep(std::time::Duration::from_secs(1));
            let after = read_volume(&mut dev)?;
            if after != percent {
                return Err(Failure::mismatch(format!(
                    "volume write did not stick: set {percent}, snapshot reads {after}"
                )));
            }

            // 快照只反映运行时值，**不能证明落盘**。切走再切回才算数。
            let hop = if wire == 0 { 1 } else { 0 };
            let (k, p) = nanocore::protocol::select_preset(hop);
            transact(&mut dev, conn, k, p, &[])?;
            wait_preset_settled(&mut dev, Some(hop))?;
            let (k, p) = nanocore::protocol::select_preset(wire);
            transact(&mut dev, conn, k, p, &[])?;
            wait_preset_settled(&mut dev, Some(wire))?;

            let persisted = read_volume(&mut dev)?;
            if persisted == percent {
                println!("preset volume: {before} -> {persisted} (saved)");
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "volume did not persist: set {percent} on slot #{}, after switching \
                     away and back it reads {persisted}",
                    wire + 1
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
        PresetCmd::Set { index } => {
            let mut dev = open_device(conn)?;
            // 不用裸 program change：它不等应答，设备异步提交，导致紧随其后的
            // `preset show` 读到上一个槽。
            dev.select_preset_checked(index - 1)
                .map_err(|e| Failure::error(e.to_string()))?;
            wait_preset_settled(&mut dev, Some(index - 1))
        }
        PresetCmd::Next => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &nanocore::midicc::global::next_preset(conn.channel))?;
            wait_preset_settled(&mut dev, None)
        }
        PresetCmd::Prev => {
            let mut dev = open_device(conn)?;
            send_standard(&mut dev, &nanocore::midicc::global::prev_preset(conn.channel))?;
            wait_preset_settled(&mut dev, None)
        }
        PresetCmd::Rename { slot, name } => {
            let field = nanocore::protocol::encode_preset_name_field(&name)
                .map_err(Failure::error)?;
            let wire = slot - 1;
            let mut dev = open_device(conn)?;

            // **改名只作用于当前活动的那个预设**，设备不接受"改第 N 槽"的寻址——帧里
            // 根本没有槽位字段。之前我一直在改一个非活动的槽：设备照样接受、照样回显
            // 正确的名字区，但名表纹丝不动，看上去像"改名不生效"，其实名字写到别处去了
            // （`preset show` 能看到活动预设已经变成新名字）。
            let active = dev
                .read_snapshot()
                .ok()
                .and_then(|s| nanocore::protocol::decode_snapshot_state(&s.payload))
                .map(|st| st.preset_index)
                .unwrap_or(wire);
            if active != wire {
                return Err(Failure::mismatch(format!(
                    "slot #{slot} is not the active preset (the device is on #{});                      run `preset set {slot}` first",
                    active + 1
                )));
            }

            // 改名一帧 + 保存一帧。抓包 `预设重命名P14 Psych改Psycn-re.mmon`：
            //   96.3601  6d 00 0a …<名字区>
            //   98.8239  46 00 01 00 00 0d      ← 手动点的保存
            //   99.2257  46 00 00 01 00 00 0d    status 00
            // 那 2.5 秒是**用户手动点保存**的间隔，不是协议要求；收到改名的确认应答后
            // 立刻发保存即可。我曾因为它"晚到"就断定与改名无关，于是砍掉保存帧——
            // 结果设备只回显、不落盘。
            let (k, p) = nanocore::protocol::set_name_field(&field);
            check_status(transact_reply(&mut dev, conn, k, p, &[])?, "name field")?;

            std::thread::sleep(std::time::Duration::from_millis(2500));
            let (k, p) = nanocore::protocol::name_commit(wire);
            check_status(transact_reply(&mut dev, conn, k, p, &[])?, "name save")?;

            // flash 落盘异步生效，稍候再读名表复核。
            std::thread::sleep(std::time::Duration::from_secs(3));
            let entries = dev.read_preset_names()?;
            let got = entries
                .iter()
                .find(|e| e.slot == wire)
                .map(|e| e.name.clone())
                .unwrap_or_default();
            if got == name {
                println!("slot #{slot}: renamed to {name:?}");
                Ok(())
            } else {
                Err(Failure::mismatch(format!(
                    "the device accepted the frames but the name table still says {got:?}"
                )))
            }
        }
        PresetCmd::RestoreFactory => {
            // 语义与官方 App 的"仅 DSP 恢复"一致：40 组出厂预设整体重写。
            // 会话开场（0x49）会清空预设表，随后流程将其完整重写。
            // seq 做 7 位安全修复后，40 组恢复全部通过。
            eprintln!(
                "warning: this overwrites slots 1-40 with factory names and\n  \
                 contents. Do not disconnect the device until the command\n  \
                 prints its final verdict."
            );
            let mut dev = open_device(conn)?;
            dev.restore_factory_presets(progress)
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
