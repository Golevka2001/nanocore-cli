//! `asset` 命令：读取 / 校验 / 写入存储在设备槽位中的原始音箱/IR 数据块。

use nanocore::asset::{self, ChunkRequest};

use crate::cli::{AssetCmd, Conn, RegionArg};
use crate::common::{Dev, Failure, open_device};

pub fn cmd_asset(cmd: AssetCmd, conn: &Conn) -> Result<(), Failure> {
    match cmd {
        AssetCmd::List => {
            println!("asset regions (device blob storage):");
            println!(
                "  amp   region 0x{:02x} — slots 0-{} (30 amp sims + 8 FX2 drive pedals), {} bytes each",
                asset::REGION_AMP,
                asset::AMP_SLOTS - 1,
                asset::AMP_TOTAL
            );
            println!(
                "  ir    region 0x{:02x} — slots 0-{}, {} bytes each (write supported)",
                asset::REGION_IR,
                asset::IR_SLOTS - 1,
                asset::IR_TOTAL
            );
            println!("  operations: read | verify | write (ir only)");
            Ok(())
        }
        AssetCmd::Read { region, slot, out } => {
            let target = ReadTarget::new(region, slot)?;
            let mut dev = open_device(conn)?;
            asset_read(&mut dev, target, out, None)
        }
        AssetCmd::Verify {
            region,
            slot,
            reference,
            out,
        } => {
            let want =
                std::fs::read(&reference).map_err(|e| format!("cannot read {reference}: {e}"))?;
            let target = ReadTarget::new(region, slot)?;
            let mut dev = open_device(conn)?;
            asset_read(&mut dev, target, out, Some(&want))
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
    use nanocore::asset;

    // 用户编号 1 起 → wire 槽位 0 起。
    if slot == 0 || slot > asset::AMP_SLOTS {
        return Err(Failure::error(format!(
            "amp slot must be 1-{}, got {slot}",
            asset::AMP_SLOTS
        )));
    }
    let wire = slot - 1;
    let ead = aw::prepare_amp_input(&blob).map_err(Failure::error)?;
    println!("asset write amp slot {slot} — full .ead, {} bytes", ead.len());
    let mut dev = open_device(conn)?;
    // 成功判据在 write_amp_slot 内部：0x33 提交状态 00 + 0x35 名字被接受 +
    // 型号表（0x36）回读为新名字。回读的 DDBP 内部格式仅作信息展示。
    let back = dev.write_amp_slot(wire, &ead, name.as_deref(), |msg| {
        eprint!("\r{msg}");
    })?;
    eprintln!();
    println!(
        "OK: model committed; device reports it as a custom model (read-back \
         internal blob {} bytes, crc32 {:08x}).",
        back.len(),
        asset::crc32(&back)
    );
    Ok(())
}

/// `asset write ir`：写入 4096 B 裸 f32le IR blob。
fn cmd_write_ir(
    slot: u8,
    blob: Vec<u8>,
    name: Option<String>,
    conn: &Conn,
) -> Result<(), Failure> {
    // 用户编号 1 起 → wire 槽位 0 起。
    if slot == 0 || slot > asset::IR_SLOTS {
        return Err(Failure::error(format!(
            "ir slot must be 1-{}, got {slot}",
            asset::IR_SLOTS
        )));
    }
    let wire = slot - 1;
    if blob.len() as u32 != asset::IR_TOTAL {
        return Err(Failure::error(format!(
            "IR blob must be exactly {} bytes (got {})",
            asset::IR_TOTAL,
            blob.len()
        )));
    }
    println!("asset write ir slot {slot} — {} bytes", blob.len());
    let mut dev = open_device(conn)?;
    let back = dev.write_ir_slot(wire, &blob, name.as_deref(), |msg| {
        eprint!("\r{msg}");
    })?;
    eprintln!();
    match back == blob {
        true => {
            let crc = asset::crc32(&back);
            println!(
                "OK: read-back is byte-identical ({} bytes, crc32 {crc:08x}).",
                back.len()
            );
            Ok(())
        }
        false => {
            let at = back
                .iter()
                .zip(blob.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            Err(Failure::mismatch(format!(
                "MISMATCH after write: first difference at byte {at} \
                 (device 0x{:02x}, file 0x{:02x})",
                back[at], blob[at]
            )))
        }
    }
}

/// 一次读取的已校验目标：区域、槽位、分块计划与预期长度。
///
/// 构造即校验——非法区域或槽位在打开端口之前就被拒绝。
struct ReadTarget {
    region: RegionArg,
    slot: u8,
    plan: Vec<ChunkRequest>,
    total: u32,
}

impl ReadTarget {
    /// `slot` 是用户编号（1 起）；分块计划按 wire 槽位（0 起）构造。
    fn new(region: RegionArg, slot: u8) -> Result<Self, String> {
        let region_byte = region.asset_region();
        let slots = asset::slots_for(region_byte)
            .ok_or_else(|| format!("no read plan for region {}", region.name()))?;
        if slot == 0 || slot > slots {
            return Err(format!(
                "{} slot must be 1-{} (device has {slots} slots), got {slot}",
                region.name(),
                slots
            ));
        }
        let (total, plan) = asset::read_plan(region_byte, slot - 1)
            .ok_or_else(|| format!("internal: no plan for {} slot {slot}", region.name()))?;
        Ok(Self {
            region,
            slot,
            plan,
            total,
        })
    }
}

/// `asset read` / `asset verify` 的共享实现体。
fn asset_read(
    dev: &mut Dev,
    target: ReadTarget,
    out: Option<String>,
    reference: Option<&[u8]>,
) -> Result<(), Failure> {
    let ReadTarget {
        region,
        slot,
        plan,
        total,
    } = target;
    println!(
        "asset {} slot {} (read-only, {} chunk request(s), expecting {} bytes)...",
        region.name(),
        slot,
        plan.len(),
        total
    );
    let blob = dev
        .read_asset(&plan, total, |done, n| {
            eprint!("\r  chunk {done}/{n}");
        })
        .map_err(|e| e.to_string())?;
    eprintln!();
    println!("read {} bytes", blob.len());

    let crc = asset::crc32(&blob);
    println!("crc32 = {crc:08x}");

    if let Some(path) = &out {
        match std::fs::write(path, &blob) {
            Ok(()) => println!("written to {path}"),
            Err(e) => eprintln!("warning: cannot write {path}: {e}"),
        }
    }
    if let Some(want) = reference {
        if blob.len() != want.len() {
            return Err(Failure::mismatch(format!(
                "MISMATCH: device returned {} bytes, reference has {}",
                blob.len(),
                want.len()
            )));
        }
        match blob.iter().zip(want.iter()).position(|(a, b)| a != b) {
            None => println!(
                "OK: byte-identical to the reference ({} bytes, crc32 {crc:08x}).",
                blob.len()
            ),
            Some(at) => {
                return Err(Failure::mismatch(format!(
                    "MISMATCH: first difference at byte {at}: device 0x{:02x}, reference 0x{:02x}",
                    blob[at], want[at]
                )));
            }
        }
    }
    Ok(())
}
