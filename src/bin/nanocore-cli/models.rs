//! `models` 命令：打印内置型号表，或查询实际设备。

use nanocore::client::Error;
use nanocore::models_data;
use nanocore::protocol;

use crate::cli::{Conn, ModelTableArg};
use crate::common::{Dev, Failure, open_device};

/// 表标题：打印的编号即 `module amp model` / `module cab model` 的编号（1 起）。
const AMP_HEADER: &str = "AMP models (1-38):";
const CAB_HEADER: &str = "CAB / IR models (1-30):";

pub fn cmd_models(table: Option<ModelTableArg>, live: bool, conn: &Conn) -> Result<(), Failure> {
    if live {
        // 实时查询覆盖设备的 amp/drive 槽位（1-38），表过滤在此不适用。
        if table.is_some() {
            return Err(Failure::error(
                "`models --live` queries the device's amp/drive slots (1-38) and takes \
                 no table argument; drop it (or drop --live to read the embedded tables)",
            ));
        }
        let mut dev = open_device(conn)?;
        return query_models_live(&mut dev);
    }
    match table {
        Some(ModelTableArg::Amp) => print_table(AMP_HEADER, models_data::AMP),
        Some(ModelTableArg::Cab) => print_table(CAB_HEADER, models_data::CAB),
        None => {
            print_table(AMP_HEADER, models_data::AMP);
            println!();
            print_table(CAB_HEADER, models_data::CAB);
        }
    }
    Ok(())
}

fn print_table(header: &str, rows: &[models_data::ModelInfo]) {
    println!("{header}");
    for m in rows {
        println!(
            "  {:>2}  {:<10} {:<34} [{}]",
            m.index + 1,
            m.code,
            m.name,
            m.category
        );
    }
}

/// 向实际设备查询每个音箱 / 踏板效果型号名（槽位 1-38），并与内置表交叉核对。
/// 只读：发送 `0x36` 查询并解析 `0x36` 应答。
///
/// 实时名称与出厂表不一致的槽位标记为 `[CUSTOM]`（例如导入的 NAM 文件覆盖了它们）；
/// 设备当前正在使用的槽位额外标记 `[active]`。两者与真正的表项不一致分开计数。
fn query_models_live(dev: &mut Dev) -> Result<(), Failure> {
    println!("Querying device model list (amp/drive slots 1-38, read-only)...");
    let mut mismatches = 0usize;
    let mut custom = 0usize;
    for i in 0u8..=37 {
        let reply = dev.query_model(i).map_err(|e| match e {
            Error::Timeout { attempts, elapsed } => {
                format!("timeout on slot {} after {attempts} attempt(s) in {elapsed:?}", i + 1)
            }
            Error::Transport(t) => t.to_string(),
        })?;
        match protocol::parse_model_name(&reply.payload) {
            Some(mn) => {
                // 仅比较名称：应答中的 `active` 字节标记的是设备当前正在使用的槽位。
                // 它横跨出厂/自定义两类，不能单独决定 "matches" 与否。
                let matches_factory = models_data::lookup_amp(i)
                    .map(|em| em.name == mn.name || em.code.eq_ignore_ascii_case(&mn.name))
                    .unwrap_or(false);
                let tag = if mn.active { " [active]" } else { "" };
                if matches_factory {
                    let em = models_data::lookup_amp(i).unwrap();
                    println!(
                        "  {:>2}  {:<18} (matches embedded {}){tag}",
                        i + 1,
                        mn.name,
                        em.code
                    );
                } else if mn.active
                    || mn.name != models_data::lookup_amp(i).map(|e| e.name).unwrap_or("")
                {
                    match models_data::lookup_amp(i) {
                        Some(em) => println!(
                            "  {:>2}  {:<18} [CUSTOM]{tag} (embedded: {} / {})",
                            i + 1,
                            mn.name,
                            em.name,
                            em.code
                        ),
                        None => {
                            println!("  {:>2}  {:<18} [CUSTOM]{tag} (no factory entry)", i + 1, mn.name)
                        }
                    }
                    custom += 1;
                } else {
                    mismatches += 1;
                }
            }
            None => {
                println!("  {:>2}  <no name in reply>", i + 1);
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
