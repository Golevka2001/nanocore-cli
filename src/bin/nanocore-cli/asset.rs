//! `asset` 命令：把音箱 / IR 数据块写入设备槽位。
//!
//! 只保留写入。读取与校验已按 ToneCommand 的实际行为对齐后删除——官方 App
//! 既不回读资产也不做内容校验，只有 `importEadBytes` / `importSlot` 这类写入口，
//! 以及 `getSlots` / `getIrSlots` 这种只读**槽位目录**的查询。

use nanocore::asset;

use crate::cli::{AssetCmd, Conn, RegionArg};
use crate::common::{Failure, open_device, progress};

pub fn cmd_asset(cmd: AssetCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        AssetCmd::List => {
            println!("asset regions (device blob storage):");
            println!(
                "  amp   region 0x{:02x} — slots 1-{} (30 amp sims + 8 FX2 drive pedals), {} bytes each",
                asset::REGION_AMP,
                asset::AMP_SLOTS,
                asset::AMP_TOTAL
            );
            println!(
                "  ir    region 0x{:02x} — slots 1-{}, {} bytes each (write supported)",
                asset::REGION_IR,
                asset::IR_SLOTS,
                asset::IR_TOTAL
            );
            println!("  operations: write (ir: 4-byte-aligned blob up to 4096; amp: full .ead/.eadl)");
            Ok(())
        }
        AssetCmd::Write {
            region,
            slot,
            file,
            name,
        } => {
            // 槽位先于文件读取校验，避免编号错误被文件错误盖住。
            let slots = match region {
                RegionArg::Amp => asset::AMP_SLOTS,
                RegionArg::Ir => asset::IR_SLOTS,
            };
            if slot == 0 || slot > slots {
                return Err(Failure::error(format!(
                    "{} slot must be 1-{slots}, got {slot}",
                    region.name()
                )));
            }
            let blob = std::fs::read(&file).map_err(|e| format!("cannot read {file}: {e}"))?;
            warn_write_session();
            match region {
                RegionArg::Amp => cmd_write_amp(slot, blob, name, conn),
                RegionArg::Ir => cmd_write_ir(slot, blob, name, conn),
            }
        }
    }
}

/// 写会话破坏性警告（amp / IR 共用）：写会话有清空预设名表等副作用。
fn warn_write_session() {
    eprintln!(
        "warning: asset write is dangerous — write sessions have been observed to \
         wipe the device's preset table (all slots lose their names and content; \
         observed 2026-09-30, fw 1.04). After the write completes, run \
         `preset restore-factory` to restore factory presets. \
         preset restore-factory recovers slots 1-40 (factory presets). \
         Do not disconnect the device until this command prints its final verdict."
    );
}

/// `asset write amp`：写入完整 `.ead`（12302 B）或 `.eadl`（解包内嵌 `.ead`）。
fn cmd_write_amp(
    slot: u8,
    blob: Vec<u8>,
    name: Option<String>,
    conn: &Conn,
) -> Result<(), Failure> {
    use nanocore::amp_write as aw;

    // 用户编号 1 起 → wire 槽位 0 起。槽位范围已由 `cmd_asset` 校验。
    let wire = slot - 1;
    let ead = aw::prepare_amp_input(&blob).map_err(Failure::error)?;
    println!("asset write amp slot {slot} — full .ead, {} bytes", ead.len());
    let mut dev = open_device(conn)?;
    // 成功判据在 write_amp_slot 内部：0x33 提交状态 00 + 0x35 名字被接受 +
    // 成败只看设备自己的两个状态字节：0x33 提交 + 0x35 型号名（内部已校验），
    // 外加随后的 0x36 型号表刷新确认新名字。官方 App 同样不回读资产。
    dev.write_amp_slot(wire, &ead, name.as_deref(), progress)?;
    eprintln!();
    println!("OK: device accepted the model (0x33/0x35 status 00, model table updated).");
    Ok(())
}

/// `asset write ir`：写入 4096 B 裸 f32le IR blob。
fn cmd_write_ir(
    slot: u8,
    blob: Vec<u8>,
    name: Option<String>,
    conn: &Conn,
) -> Result<(), Failure> {
    // 用户编号 1 起 → wire 槽位 0 起。槽位范围已由 `cmd_asset` 校验。
    let wire = slot - 1;
    // 设备本身就会存**变长** IR（实测槽 12 = 2780 B、槽 16 = 3764 B，其余 28 槽 = 4096 B），
    // 写路径也一直按 `blob.len()` 走：`write_begin` 带真实长度、`hash_b3` 的 bit0 就是
    // "长度非标准 4096"。所以这里不该硬性要求 4096 —— 那样会让槽 12/16 事实上只读，
    // 导出后无法写回。
    let len = blob.len();
    if len == 0 || len > asset::IR_TOTAL as usize || len % 4 != 0 {
        return Err(Failure::error(format!(
            "IR blob must be 4-byte aligned and between 4 and {} bytes (got {len})",
            asset::IR_TOTAL
        )));
    }
    println!("asset write ir slot {slot} — {} bytes", blob.len());
    let mut dev = open_device(conn)?;
    dev.write_ir_slot(wire, &blob, name.as_deref(), progress)?;
    eprintln!();
    // 成功判据就是设备自己的 0x57 提交状态字节。官方 App 不回读比对，
    // 而 0x39/0x59 的读应答没有块级校验和、实测会间歇性损坏内容，
    // 拿它当"验证写入"只会给出虚假的 OK 或虚假的失败。
    println!("OK: the device acknowledged the commit (0x57 status 00).");
    Ok(())
}
