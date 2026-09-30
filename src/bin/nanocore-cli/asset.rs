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
            if region != RegionArg::Ir {
                return Err(Failure::error(
                    "asset write currently supports ir slots only",
                ));
            }
            if slot >= asset::IR_SLOTS {
                return Err(Failure::error(format!(
                    "ir slot must be 0-{}, got {slot}",
                    asset::IR_SLOTS - 1
                )));
            }
            let blob = std::fs::read(&file).map_err(|e| format!("cannot read {file}: {e}"))?;
            if blob.len() as u32 != asset::IR_TOTAL {
                return Err(Failure::error(format!(
                    "IR blob must be exactly {} bytes (got {})",
                    asset::IR_TOTAL,
                    blob.len()
                )));
            }
            println!("asset write ir slot {slot} — {} bytes", blob.len());
            eprintln!(
                "warning: asset write is dangerous — a write session (even a clean, \
                 fully-committed one with passing read-back verification) has been \
                 observed to wipe the device's preset table: all 64 slots lose their \
                 names AND content, and only the official app's factory reset \
                 recovers them (observed 2026-09-30, fw 1.04). Root cause unknown; \
                 do not use until investigated."
            );
            let mut dev = open_device(conn)?;
            let back = dev.write_ir_slot(slot, &blob, name.as_deref(), |msg| {
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
    fn new(region: RegionArg, slot: u8) -> Result<Self, String> {
        let region_byte = region.asset_region();
        let slots = asset::slots_for(region_byte)
            .ok_or_else(|| format!("no read plan for region {}", region.name()))?;
        if slot >= slots {
            return Err(format!(
                "{} slot must be 0-{} (device has {slots} slots), got {slot}",
                region.name(),
                slots - 1
            ));
        }
        let (total, plan) = asset::read_plan(region_byte, slot)
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
