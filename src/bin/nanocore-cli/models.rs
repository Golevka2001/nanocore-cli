//! `models` 命令：打印内置型号表，或查询实际设备。

use nanocore::client::Error;
use nanocore::models_data;
use nanocore::protocol;

use crate::cli::{Conn, RegionArg};
use crate::common::{Dev, open_device};

pub fn cmd_models(region: Option<RegionArg>, live: bool, conn: &Conn) -> Result<(), String> {
    if live {
        let mut dev = open_device(conn)?;
        return query_models_live(&mut dev);
    }
    match region {
        Some(RegionArg::Amp) => print_models(models_data::AMP),
        Some(RegionArg::Ir) => print_models(models_data::CAB),
        None => {
            println!("AMP models (0-37, device slot = index+1):");
            print_models(models_data::AMP);
            println!("\nCAB / IR models (0-29, device slot = index+1):");
            print_models(models_data::CAB);
        }
    }
    Ok(())
}

fn print_models(rows: &[models_data::ModelInfo]) {
    for m in rows {
        println!(
            "  {:>2}  {:<10} {:<34} [{}]",
            m.index, m.code, m.name, m.category
        );
    }
}

/// 向实际设备查询每个音箱 / 踏板效果型号名（槽位 0-37），并与内置表交叉核对。
/// 只读：发送 `0x36` 查询并解析 `0x36` 应答。
///
/// 实时名称与出厂表不一致的槽位标记为 `[CUSTOM]`（例如导入的 NAM 文件覆盖了它们）；
/// 设备当前正在使用的槽位额外标记 `[active]`。两者与真正的表项不一致分开计数。
fn query_models_live(dev: &mut Dev) -> Result<(), String> {
    println!("Querying device model list (amp/drive slots 0-37, read-only)...");
    let mut mismatches = 0usize;
    let mut custom = 0usize;
    for i in 0u8..=37 {
        let reply = dev.query_model(i).map_err(|e| match e {
            Error::Timeout { attempts, elapsed } => {
                format!("timeout on slot {i} after {attempts} attempt(s) in {elapsed:?}")
            }
            Error::Transport(t) => t.to_string(),
        })?;
        match protocol::parse_model_name(&reply.payload) {
            Some(mn) => {
                /// 仅比较名称：应答中的 `active` 字节标记的是设备当前正在使用的槽位。
                /// 它横跨出厂/自定义两类，不能单独决定"matches" 与否。
                let matches_factory = models_data::lookup_amp(i)
                    .map(|em| em.name == mn.name || em.code.eq_ignore_ascii_case(&mn.name))
                    .unwrap_or(false);
                let tag = if mn.active { " [active]" } else { "" };
                if matches_factory {
                    let em = models_data::lookup_amp(i).unwrap();
                    println!(
                        "  {i:>2}  {:<18} (matches embedded {}){tag}",
                        mn.name, em.code
                    );
                } else if mn.active
                    || mn.name != models_data::lookup_amp(i).map(|e| e.name).unwrap_or("")
                {
                    match models_data::lookup_amp(i) {
                        Some(em) => println!(
                            "  {i:>2}  {:<18} [CUSTOM]{tag} (embedded: {} / {})",
                            mn.name, em.name, em.code
                        ),
                        None => {
                            println!("  {i:>2}  {:<18} [CUSTOM]{tag} (no factory entry)", mn.name)
                        }
                    }
                    custom += 1;
                } else {
                    mismatches += 1;
                }
            }
            None => {
                println!("  {i:>2}  <no name in reply>");
                mismatches += 1;
            }
        }
    }
    if custom > 0 {
        println!("{custom} slot(s) carry a custom model.");
    }
    if mismatches == 0 {
        println!("OK: all 38 slots match the embedded models.json table.");
    } else {
        println!("WARN: {mismatches} slot(s) did not match the embedded table.");
    }
    Ok(())
}
