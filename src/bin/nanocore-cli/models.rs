//! `models` command: print the embedded model tables or query the live device.

use nanocore::client::Error;
use nanocore::models_data;
use nanocore::protocol;

use crate::cli::{Conn, RegionArg};
use crate::common::{open_device, Dev};

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

/// Query the real device for every amp / drive model name (slots 0-37) and
/// cross-check against the embedded table. Read-only: sends `0x36` queries
/// and parses the `0x36` replies.
///
/// Slots whose live name differs from the factory table are flagged
/// `[CUSTOM]` (e.g. an imported NAM file overwrote them); the slot the device
/// is currently playing is additionally flagged `[active]`. Both are counted
/// separately from real table mismatches.
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
                // Name comparison only: the reply's `active` byte flags the
                // slot the device is currently using, which crosses factory /
                // custom lines and must not decide "matches" by itself.
                let matches_factory = models_data::lookup_amp(i)
                    .map(|em| em.name == mn.name || em.code.eq_ignore_ascii_case(&mn.name))
                    .unwrap_or(false);
                let tag = if mn.active { " [active]" } else { "" };
                if matches_factory {
                    let em = models_data::lookup_amp(i).unwrap();
                    println!("  {i:>2}  {:<18} (matches embedded {}){tag}", mn.name, em.code);
                } else if mn.active || mn.name != models_data::lookup_amp(i).map(|e| e.name).unwrap_or("") {
                    match models_data::lookup_amp(i) {
                        Some(em) => println!(
                            "  {i:>2}  {:<18} [CUSTOM]{tag} (embedded: {} / {})",
                            mn.name, em.name, em.code
                        ),
                        None => println!(
                            "  {i:>2}  {:<18} [CUSTOM]{tag} (no factory entry)",
                            mn.name
                        ),
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
