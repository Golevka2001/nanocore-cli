//! `asset` commands: read / verify / write the raw amp/IR blobs stored in
//! device slots.

use nanocore::asset;

use crate::cli::{AssetCmd, Conn, RegionArg};
use crate::common::{open_device, Dev};

pub fn cmd_asset(cmd: AssetCmd, conn: &Conn) -> Result<(), String> {
    match cmd {
        AssetCmd::List => {
            println!("asset regions (device blob storage):");
            println!(
                "  amp   region 0x{:02x} — slots 0-37 (30 amp sims + 8 FX2 drive pedals), {} bytes each",
                asset::REGION_AMP,
                asset::AMP_TOTAL
            );
            println!(
                "  ir    region 0x{:02x} — slots 0-29, {} bytes each (write supported)",
                asset::REGION_IR,
                asset::IR_TOTAL
            );
            println!("  operations: read | verify | write (ir only)");
            Ok(())
        }
        AssetCmd::Read { region, slot, out } => {
            let mut dev = open_device(conn)?;
            asset_read(&mut dev, region, slot, out, None)
        }
        AssetCmd::Verify { region, slot, reference, out } => {
            let want = std::fs::read(&reference)
                .map_err(|e| format!("cannot read {reference}: {e}"))?;
            let mut dev = open_device(conn)?;
            asset_read(&mut dev, region, slot, out, Some(&want))
        }
        AssetCmd::Write { region, slot, file, name } => {
            if region != RegionArg::Ir {
                return Err("asset write currently supports ir slots only".into());
            }
            let blob = std::fs::read(&file)
                .map_err(|e| format!("cannot read {file}: {e}"))?;
            if blob.len() as u32 != asset::IR_TOTAL {
                return Err(format!(
                    "IR blob must be exactly {} bytes (got {})",
                    asset::IR_TOTAL,
                    blob.len()
                ));
            }
            println!("asset write ir slot {slot} — {} bytes", blob.len());
            eprintln!(
                "warning: asset write is experimental — do not interrupt the session; \
                 an interrupted write has been observed to wipe the preset-name table \n\
                 (recoverable via a factory reset in the official app)"
            );
            let mut dev = open_device(conn)?;
            let back = dev.write_ir_slot(slot, &blob, name.as_deref(), |msg| {
                eprint!("\r{msg}");
            })?;
            eprintln!();
            match back == blob {
                true => {
                    let crc = asset::crc32(&back);
                    println!("OK: read-back is byte-identical ({} bytes, crc32 {crc:08x}).", back.len());
                    Ok(())
                }
                false => {
                    let at = back
                        .iter()
                        .zip(blob.iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(0);
                    println!(
                        "MISMATCH after write: first difference at byte {at} \
                         (device 0x{:02x}, file 0x{:02x})",
                        back[at], blob[at]
                    );
                    std::process::exit(3);
                }
            }
        }
    }
}

/// Shared body of `asset read` / `asset verify`.
fn asset_read(
    dev: &mut Dev,
    region: RegionArg,
    slot: u8,
    out: Option<String>,
    reference: Option<&[u8]>,
) -> Result<(), String> {
    let region_byte = region.asset_region();
    let Some((total, plan)) = asset::read_plan(region_byte, slot) else {
        return Err(format!("no read plan for region {}", region.name()));
    };
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
            println!(
                "MISMATCH: device returned {} bytes, reference has {}",
                blob.len(),
                want.len()
            );
            std::process::exit(3);
        }
        match blob.iter().zip(want.iter()).position(|(a, b)| a != b) {
            None => println!(
                "OK: byte-identical to the reference ({} bytes, crc32 {crc:08x}).",
                blob.len()
            ),
            Some(at) => {
                println!(
                    "MISMATCH: first difference at byte {at}: device 0x{:02x}, reference 0x{:02x}",
                    blob[at], want[at]
                );
                std::process::exit(3);
            }
        }
    }
    Ok(())
}
