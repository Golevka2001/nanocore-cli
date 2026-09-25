//! `module` 命令：列出模块、开关、型号选择、参数列表与参数写入。

use nanocore::midicc::{self, Block};
use nanocore::models_data;
use nanocore::params_data;
use nanocore::protocol;

use crate::cli::{BlockArg, Conn, ModuleAction, ModuleCmd};
use crate::common::{Dev, open_device, send_standard, transact};

pub fn cmd_module(cmd: ModuleCmd, conn: &Conn) -> Result<(), String> {
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

fn module_action(block: BlockArg, action: Option<ModuleAction>, conn: &Conn) -> Result<(), String> {
    match action {
        None => print_module_help(block),
        Some(ModuleAction::Params { r#type }) => cmd_params(block, r#type),
        Some(ModuleAction::Param { key, value, r#type }) => {
            let Some(value) = value else {
                return Err(format!(
                    "reading a parameter value is not implemented yet (needs \
                     effect-data / snapshot read); pass a value to write:\n  \
                     nanocore-cli module {} param {key} <0..1>",
                    block.key()
                ));
            };
            let mut dev = open_device(conn)?;
            cmd_param(&mut dev, conn, block.key(), &key, value, r#type)
        }
        Some(ModuleAction::Model { id }) => {
            report_model_selection(block.midicc(), id)?;
            let mut dev = open_device(conn)?;
            send_standard(
                &mut dev,
                &midicc::block_type(conn.channel, block.midicc(), id),
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

fn cmd_module_list() -> Result<(), String> {
    println!("modules (b5 slot — the device string-table order):");
    for &(key, slot) in params_data::BLOCKS {
        let summary = match params_data::params(key, None) {
            Some(ps) => format!("type-independent: {} parameter(s)", ps.len()),
            None => "type-dependent: `module <m> params --type N` to list".into(),
        };
        println!("  {key:<7} b5={slot}  {summary}");
    }
    println!();
    println!("usage: nanocore-cli module <module> <action>");
    println!("       module <module> on | off");
    println!("       module <module> model <N>");
    println!("       module <module> params [--type N]");
    println!("       module <module> param <name|b6> <0..1> [--type N]");
    Ok(())
}

fn print_module_help(block: BlockArg) -> Result<(), String> {
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
        "  actions: on | off | model <N> | params [--type N] | param <name> [value] [--type N]"
    );
    Ok(())
}

fn cmd_params(block: BlockArg, r#type: Option<u8>) -> Result<(), String> {
    let key = block.key();
    let slot = params_data::block_slot(key).ok_or_else(|| format!("unknown module {key:?}"))?;
    let ps = params_data::params(key, r#type).ok_or_else(|| {
        format!(
            "module {key:?} is type-dependent: pass `--type N`, and note that some \
             types have no published parameter list (fx1 types 1-2, fx2 types 5-7)"
        )
    })?;
    println!(
        "{key} (b5={slot}){}:",
        match r#type {
            Some(t) => format!(", effect type {t}"),
            None => String::new(),
        }
    );
    for p in ps {
        println!("  b6={:<2} {:<24} CC{}", p.index, p.name, p.cc);
    }
    Ok(())
}

fn cmd_param(
    dev: &mut Dev,
    conn: &Conn,
    block: &str,
    key: &str,
    value: f32,
    r#type: Option<u8>,
) -> Result<(), String> {
    let slot = params_data::block_slot(block)
        .ok_or_else(|| format!("unknown module {block:?} (fx1 fx2 amp cab delay mod reverb eq)"))?;
    let info = match key.parse::<u8>() {
        Ok(i) => params_data::param_by_index(block, r#type, i),
        Err(_) => params_data::param_by_name(block, r#type, key),
    };
    let info = info.ok_or_else(|| {
        format!(
            "no parameter {key:?} for module {block:?}{}. \
             Run `nanocore-cli module {block} params` to list them; type-dependent \
             modules additionally need `--type N`. Some types have no published \
             parameter list (fx1 types 1-2, fx2 types 5-7).",
            match r#type {
                Some(t) => format!(" type {t}"),
                None => String::new(),
            }
        )
    })?;
    if !(0.0..=1.0).contains(&value) {
        return Err(format!(
            "value must be 0.0-1.0 (device-normalised), got {value}"
        ));
    }
    println!(
        "param {block} [b5={slot}] b6={} {} (CC{}) = {value}",
        info.index, info.name, info.cc
    );
    let (k, p) = protocol::param_write(protocol::DEFAULT_B3, slot, info.index, value);
    transact(dev, conn, k, p, &[])
}

/// 校验并描述通过标准 CC 进行的 Amp/Cab **型号**选择。
///
/// `CC43`（Amp）与 `CC44`（Cab）在文档中标记为 "Type CC"，但 OEM 指南未给出这两个模块的类型表，并声称型号选择未分配。
/// 硬件实测表明它们*确实是*覆盖 30 项表的型号选择器，且索引 `>= 30` 会被静默忽略。
/// 因此在此处拒绝，而不是让写入无声失效。
///
/// 对其余模块这是空操作：它们的 "type" 是真实的效果类型 ID，在 OEM MIDI CC 参考中拥有自己的表。
fn report_model_selection(block: Block, id: u8) -> Result<(), String> {
    let (label, count, name) = match block {
        Block::Amp => (
            "amp",
            midicc::AMP_MODEL_COUNT,
            models_data::lookup_amp(id).map(|m| format!("{} ({})", m.name, m.code)),
        ),
        Block::Cab => (
            "cab",
            midicc::CAB_MODEL_COUNT,
            models_data::lookup_cab(id).map(|m| format!("{} ({})", m.name, m.code)),
        ),
        _ => return Ok(()),
    };
    if id >= count {
        return Err(format!(
            "{label} model index must be 0-{} (device has {count} models); \
             {id} would be silently ignored — drive pedals 30-37 belong to FX2 and \
             need `debug variant`, not the {label} module",
            count - 1
        ));
    }
    match name {
        Some(n) => println!(
            "module {label} model {id} -> {n}   (standard CC{})",
            block.type_cc()
        ),
        None => println!(
            "module {label} model {id} (not in embedded table; device has 0-{})",
            count - 1
        ),
    }
    Ok(())
}
