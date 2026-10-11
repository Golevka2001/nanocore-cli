//! `module` 命令：列出模块、开关、型号选择、参数列表与参数写入。

use nanocore::midicc::{self, Block};
use nanocore::models_data;
use nanocore::params_data;
use nanocore::protocol;
use nanocore::protocol::SnapshotRecord;
use nanocore::msg_type;

use crate::cli::{BlockArg, Conn, ModuleAction, ModuleCmd};
use crate::common::{Dev, Failure, open_device, send_standard, transact_reply};

pub fn cmd_module(cmd: ModuleCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        ModuleCmd::List => cmd_module_list(),
        ModuleCmd::Fx1 { action } => module_action(BlockArg::Fx1, action, conn),
        ModuleCmd::Fx2 { action } => module_action(BlockArg::Fx2, action, conn),
        ModuleCmd::Amp { action } => module_action(BlockArg::Amp, action, conn),
        ModuleCmd::Cab { action } => module_action(BlockArg::Cab, action, conn),
        ModuleCmd::Mod { action } => module_action(BlockArg::Mod, action, conn),
        ModuleCmd::Delay { action } => module_action(BlockArg::Delay, action, conn),
        ModuleCmd::Reverb { action } => module_action(BlockArg::Reverb, action, conn),
        ModuleCmd::Eq { action } => module_action(BlockArg::Eq, action, conn),
    }
}

fn module_action(
    block: BlockArg,
    action: Option<ModuleAction>,
    conn: &Conn,
) -> Result<(), Failure> {
    match action {
        None => print_module_help(block),
        Some(ModuleAction::Params { r#type }) => cmd_params(block, r#type, conn),
        Some(ModuleAction::Param { key, value, r#type }) => {
            cmd_param_action(block, key, value, r#type, conn)
        }
        Some(ModuleAction::Model { id }) => {
            // 用户编号 1 起 → wire 效果类型 / 型号 id（0 起）。
            let wire_id = id - 1;
            report_model_selection(block.midicc(), id, wire_id)?;
            let mut dev = open_device(conn)?;
            send_standard(
                &mut dev,
                &midicc::block_type(conn.channel, block.midicc(), wire_id),
            )
        }
        Some(ModuleAction::On) => {
            let mut dev = open_device(conn)?;
            send_standard(
                &mut dev,
                &midicc::block_enable(conn.channel, block.midicc(), true),
            )
        }
        Some(ModuleAction::Off) => {
            let mut dev = open_device(conn)?;
            send_standard(
                &mut dev,
                &midicc::block_enable(conn.channel, block.midicc(), false),
            )
        }
    }
}

/// 列出 8 个模块与各自的参数表。不碰设备。
///
/// 模块说明取自 clap 的 `about`，与 `module <m> --help` 同源。
fn cmd_module_list() -> Result<(), Failure> {
    let descs = crate::cli::module_descriptions();
    let rows: Vec<(&str, String, String)> = params_data::BLOCKS
        .iter()
        .map(|&(key, _)| {
            let desc = descs
                .iter()
                .find(|(k, _)| k == key)
                .map(|(_, v)| v.as_str())
                .unwrap_or_default()
                .to_string();
            // 查得到多少参数就说多少，查不到就说「跟着效果类型走」——不编一个
            // 看似确定的数字出来。
            let params = match params_data::params(key, None) {
                Some(ps) => format!("{} parameters, the same for every model", ps.len()),
                None => "parameters follow the effect type".to_string(),
            };
            (key, desc, params)
        })
        .collect();
    let width = rows.iter().map(|(_, d, _)| d.len()).max().unwrap_or(0);

    println!("modules:");
    println!();
    for (key, desc, params) in &rows {
        println!("  {key:<7} {desc:<width$}  {params}", width = width);
    }
    println!();
    println!(
        "Every module chooses its effect type with `module <module> model <N>`:\n\
         amp and cab take 1-{count}, the others their own type number.\n\
         `params` and `param` take the same number as `--type N`; without it,\n\
         they read the one the device is already set to.",
        count = midicc::AMP_MODEL_COUNT
    );
    Ok(())
}

fn print_module_help(block: BlockArg) -> Result<(), Failure> {
    let key = block.key();
    let slot = params_data::block_slot(key)
        .map(|s| s.to_string())
        .unwrap_or_else(|| "?".into());
    println!(
        "{key} (b5={slot}, CC on/off {}, CC type {})",
        block.midicc().onoff_cc(),
        block.midicc().type_cc()
    );
    println!(
        "  actions: on | off | model <N> | params [--type N] | param <name|b6> [value] [--type N]"
    );
    println!("  (param without a value reads the current device value)");
    Ok(())
}

/// 该块的参数表是否依赖效果类型（等价于 `params(key, None)` 为空）。
fn type_dependent(key: &str) -> bool {
    params_data::params(key, None).is_none()
}

/// 读快照并取出该模块的记录。
///
/// 快照记录是**角色序**：下标 == [`protocol::CHAIN_ROLES`] 下标 == `b5` 槽位
/// 。
fn module_record(dev: &mut Dev, block: BlockArg) -> Result<SnapshotRecord, Failure> {
    let key = block.key();
    let slot = params_data::block_slot(key)
        .ok_or_else(|| Failure::error(format!("unknown module {key:?}")))?;
    let snap = dev.read_snapshot().map_err(|e| e.to_string())?;
    let state = protocol::decode_snapshot_state(&snap.payload).ok_or_else(|| {
        Failure::error(
            "the 0x63 snapshot did not decode (only the USB payload form is supported)",
        )
    })?;
    state
        .records
        .get(slot as usize)
        .cloned()
        .ok_or_else(|| Failure::error(format!("snapshot has no record for b5={slot}")))
}

/// `module <m> params [--type N]`。
///
/// 未给 `--type` 时从设备读当前效果类型（类型依赖的块因此不再必须传 `--type`）；
/// 只要拿得到快照，就顺带打印每个 `b6` 的当前值。
fn cmd_params(block: BlockArg, r#type: Option<u8>, conn: &Conn) -> Result<(), Failure> {
    // `--type` 是 1 起的用户编号；表查询与快照比较都用 wire 编号。
    let r#type = r#type.map(|t| t - 1);
    let key = block.key();
    let slot = params_data::block_slot(key)
        .ok_or_else(|| Failure::error(format!("unknown module {key:?}")))?;
    let rec = open_device(conn)
        .ok()
        .and_then(|mut dev| module_record(&mut dev, block).ok());
    let ty = r#type.or_else(|| rec.as_ref().map(|r| r.variant));
    let head_ty = match ty {
        Some(t) => {
            // 设备报的类型（省略 `--type` 时）走 `type_number` —— reverb 的编号是
            // 手册产品顺序，不等于 `variant + 1`。显式 `--type` 是用户给的 1 起始
            // 编号，原样显示。两条路都在这里汇合，印出来的数必须是同一个。
            let shown = match r#type {
                Some(_) => t + 1,
                None => protocol::type_number(slot, t).unwrap_or(t + 1),
            };
            format!(
                ", effect type {shown}{}",
                if r#type.is_none() { " (from device)" } else { "" }
            )
        }
        None => String::new(),
    };
    let Some(ps) = params_data::params(key, ty) else {
        // 没有已发布参数表的效果类型（wire：fx1 1-2、fx2 5-7；用户编号 2-3 / 6-8）。
        let t = ty.unwrap_or(rec.as_ref().map_or(0, |r| r.variant));
        return match (&rec, ty == rec.as_ref().map(|r| r.variant)) {
            (Some(rec), true) => {
                println!(
                    "{key} (b5={slot}), effect type {} (no published parameter list):",
                    t + 1
                );
                for (i, v) in rec.params.iter().enumerate() {
                    println!("  b6={i:<2} {:<24} = {v}", "(not in the OEM guide)");
                }
                Ok(())
            }
            (Some(rec), false) => Err(Failure::error(format!(
                "no published parameter list for {key:?} effect type {} (fx1 types 2-3 \
                 and fx2 types 6-8 are undocumented); the device is currently on type {} — \
                 drop `--type` to list its {} stored value(s)",
                t + 1,
                rec.variant + 1,
                rec.params.len()
            ))),
            (None, _) => Err(Failure::error(format!(
                "module {key:?} is type-dependent: pass `--type N` (the snapshot read \
                 also failed, so the current type could not be inferred; numbers are \
                 1-based), and note that some types have no published parameter list \
                 (fx1 types 2-3, fx2 types 6-8)"
            ))),
        };
    };
    println!("{key} (b5={slot}){head_ty}:");
    // 值可信的两种情形：类型就是设备当前的类型，或该块本来与类型无关。
    let show_values =
        rec.is_some() && (!type_dependent(key) || ty == rec.as_ref().map(|r| r.variant));
    match (&rec, show_values) {
        (Some(rec), true) => {
            for p in ps {
                match rec.params.get(p.index as usize) {
                    Some(v) => println!("  b6={:<2} {:<24} CC{:<3} = {v}", p.index, p.name, p.cc),
                    None => println!(
                        "  b6={:<2} {:<24} CC{:<3} = (not stored by the device)",
                        p.index, p.name, p.cc
                    ),
                }
            }
            // 设备存了、OEM 指南没列的 b6（EQ 比指南多一个尾参数）。
            for i in 0..rec.params.len() as u8 {
                if ps.iter().all(|p| p.index != i) {
                    println!(
                        "  b6={i:<2} {:<24}       = {}",
                        "(not in the OEM guide)",
                        rec.params[i as usize]
                    );
                }
            }
        }
        _ => {
            for p in ps {
                println!("  b6={:<2} {:<24} CC{}", p.index, p.name, p.cc);
            }
            if let Some(rec) = &rec {
                // 设备当前类型用 `type_number` —— reverb 的编号不是 `variant + 1`
                // （手册产品顺序，variant 7 = 第 5 个类型）。这里报的是设备侧
                // 的编号，必须与 `show` 印的一致。
                println!(
                    "  (values omitted: --type does not match the device's current type {})",
                    protocol::type_number(slot, rec.variant).unwrap_or(0)
                );
            }
        }
    }
    Ok(())
}

/// `module <m> param <name|b6> [<0..1>]`：不给值则读，给值则写。
///
/// 类型依赖的块未给 `--type` 时，从快照读当前效果类型。
fn cmd_param_action(
    block: BlockArg,
    key: String,
    value: Option<f32>,
    r#type: Option<u8>,
    conn: &Conn,
) -> Result<(), Failure> {
    // `--type` 是 1 起的用户编号；参数表解析用 wire 编号。
    let r#type = r#type.map(|t| t - 1);
    let need_snapshot = value.is_none() || (r#type.is_none() && type_dependent(block.key()));
    if !need_snapshot {
        // 纯校验路径（写入 + 类型无关）：先解析，再打开端口。
        let v = value.expect("need_snapshot is false only when a value was given");
        check_value(v)?;
        let info = resolve_param(block, &key, r#type)?;
        let mut dev = open_device(conn)?;
        return cmd_param(&mut dev, conn, block, info, v, None);
    }
    let mut dev = open_device(conn)?;
    let rec = module_record(&mut dev, block)?;
    let ty = r#type.or(Some(rec.variant));
    match value {
        None => read_param(block, &key, ty, &rec),
        Some(v) => {
            check_value(v)?;
            let info = resolve_param(block, &key, ty)?;
            cmd_param(&mut dev, conn, block, info, v, Some(&rec))
        }
    }
}

/// 读一个参数：`b6` 直接索引快照里的设备数组。
fn read_param(
    block: BlockArg,
    key: &str,
    type_id: Option<u8>,
    rec: &SnapshotRecord,
) -> Result<(), Failure> {
    let (idx, info) = match key.parse::<u8>() {
        Ok(i) => (
            i,
            params_data::param_by_index(block.key(), type_id, i),
        ),
        Err(_) => {
            let p = params_data::param_by_name(block.key(), type_id, key)
                .ok_or_else(|| no_param_message(block, key, type_id))?;
            (p.index, Some(p))
        }
    };
    let Some(v) = rec.params.get(idx as usize) else {
        let guide = params_data::params(block.key(), type_id);
        let extra = match guide.and_then(|ps| ps.iter().find(|p| p.index == idx)) {
            // OEM 指南列了它，但设备没存 —— 单说"读不到"会让人以为是 bug。
            Some(p) => format!(
                "; the OEM guide lists it as CC{}, but the device only persists b6 0..{}",
                p.cc,
                rec.params.len().saturating_sub(1)
            ),
            None => String::new(),
        };
        return Err(Failure::error(format!(
            "the device stores {} value(s) for {} — b6={idx} is not in device storage{extra}",
            rec.params.len(),
            block.key()
        )));
    };
    match info {
        Some(p) => println!(
            "{} param {} (b6={idx}, CC{}) = {v}",
            block.key(),
            p.name,
            p.cc
        ),
        None => println!(
            "{} param b6={idx} = {v} (no published name for effect type {})",
            block.key(),
            type_id.unwrap_or(rec.variant) + 1
        ),
    }
    Ok(())
}

fn check_value(v: f32) -> Result<(), Failure> {
    if (0.0..=1.0).contains(&v) {
        Ok(())
    } else {
        Err(Failure::error(format!(
            "value must be 0.0-1.0 (device-normalised), got {v}"
        )))
    }
}

/// 设备拒绝了这次写入（`b6` 超出该模块的存储范围）时给出可执行的替代方案。
///
/// 设备回 [`protocol::PARAM_WRITE_REJECTED`] 并**保持原值不变** ——
/// 静默通过会让用户以为参数已经改了。
fn rejected_message(
    block: BlockArg,
    info: &params_data::ParamInfo,
    rec: Option<&SnapshotRecord>,
) -> String {
    let guidance = format!(
        "use the standard MIDI CC{} for it instead",
        info.cc
    );
    match rec {
        Some(rec) => format!(
            "the device rejected b6={} for {}: it only stores {} value(s) (b6 0..{}), \
             so the value was left unchanged — {}.",
            info.index,
            block.key(),
            rec.params.len(),
            rec.params.len().saturating_sub(1),
            guidance
        ),
        None => format!(
            "the device rejected b6={} for {} (not in device storage; the value was \
             left unchanged) — run `nanocore-cli module {} params` to see the stored \
             range, or {}.",
            info.index,
            block.key(),
            block.key(),
            guidance
        ),
    }
}

fn no_param_message(block: BlockArg, key: &str, type_id: Option<u8>) -> String {
    format!(
        "no parameter {key:?} for module {module:?}{ty}. \
         Run `nanocore-cli module {module} params` to list them. Some effect types \
         have no published parameter list (fx1 types 2-3, fx2 types 6-8).",
        module = block.key(),
        ty = match type_id {
            Some(t) => format!(" type {}", t + 1),
            None => String::new(),
        }
    )
}

/// 解析参数名或 b6 索引（纯查询，不触碰设备）。
fn resolve_param(
    block: BlockArg,
    key: &str,
    type_id: Option<u8>,
) -> Result<&'static params_data::ParamInfo, String> {
    let info = match key.parse::<u8>() {
        Ok(i) => params_data::param_by_index(block.key(), type_id, i),
        Err(_) => params_data::param_by_name(block.key(), type_id, key),
    };
    info.ok_or_else(|| no_param_message(block, key, type_id))
}

/// 写入一个已解析并已校验的参数。
///
/// 写入必须检查应答：`b6` 超出该模块的设备存储范围时，设备回一条
/// [`protocol::PARAM_WRITE_REJECTED`] 并**保持原值不变**。
/// `rec` 只用来把拒绝信息说得更具体（没有快照时也能报错，只是少几个数字）。
fn cmd_param(
    dev: &mut Dev,
    conn: &Conn,
    block: BlockArg,
    info: &'static params_data::ParamInfo,
    value: f32,
    rec: Option<&SnapshotRecord>,
) -> Result<(), Failure> {
    let slot = params_data::block_slot(block.key()).ok_or_else(|| {
        Failure::error(format!(
            "unknown module {:?} (fx1 fx2 amp cab delay mod reverb eq)",
            block.key()
        ))
    })?;
    println!(
        "param {} [b5={slot}] b6={} {} (CC{}) = {value}",
        block.key(),
        info.index,
        info.name,
        info.cc
    );
    let (k, p) = protocol::param_write(slot, info.index, value);
    let reply = transact_reply(dev, conn, k, p, &[])?;
    if let Some(reply) = reply
        && reply.kind == msg_type::DATA
        && reply.payload.as_slice() == protocol::PARAM_WRITE_REJECTED
    {
        return Err(Failure::error(rejected_message(block, info, rec)));
    }
    Ok(())
}

/// 校验并描述通过标准 CC 进行的 Amp/Cab **型号**选择。
///
/// `CC43`（Amp）与 `CC44`（Cab）在文档中标记为 "Type CC"，但 OEM 指南未给出这两个模块的类型表，并声称型号选择未分配。
/// 它们*确实是*覆盖 30 项表的型号选择器，且 wire id `>= 30` 会被静默忽略。
/// 因此在此处拒绝，而不是让写入无声失效。
///
/// 对其余模块这是空操作：它们的 "type" 是真实的效果类型编号，在 OEM MIDI CC 参考中拥有自己的表。
///
/// `id` 是用户编号（1 起），`wire` 是发送到 CC 的 0 起 id。
fn report_model_selection(block: Block, id: u8, wire: u8) -> Result<(), Failure> {
    let (label, count, name) = match block {
        Block::Amp => (
            "amp",
            midicc::AMP_MODEL_COUNT,
            models_data::lookup_amp(wire).map(|m| format!("{} ({})", m.name, m.code)),
        ),
        Block::Cab => (
            "cab",
            midicc::CAB_MODEL_COUNT,
            models_data::lookup_cab(wire).map(|m| format!("{} ({})", m.name, m.code)),
        ),
        _ => return Ok(()),
    };
    if wire >= count {
        return Err(Failure::error(format!(
            "{label} model number must be 1-{count} (device has {count} models); \
             {id} would be silently ignored — drive pedals 31-38 are outside CC \
             range and belong to FX2, not the {label} module"
        )));
    }
    match name {
        Some(n) => println!(
            "module {label} model {id} -> {n}   (standard CC{})",
            block.type_cc()
        ),
        None => println!(
            "module {label} model {id} (not in embedded table; device has 1-{count})"
        ),
    }
    Ok(())
}
