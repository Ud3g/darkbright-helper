//! Manual DDC/CI hardware probe.
//!
//! Ignored by default: it needs a real display that answers DDC/CI, which a
//! headless CI runner does not have. Run it deliberately, on the machine whose
//! monitors are in question:
//!
//! ```text
//! cargo test --test ddc_test -- --ignored --nocapture
//! ```
//!
//! `--nocapture` is the point — the printed per-monitor breakdown (identity,
//! physical handle count, current/maximum brightness and the percentage it
//! scales to) is what makes this useful for diagnosing "brightness does nothing
//! on this monitor" reports. A reported maximum other than 100 is the single
//! most useful thing this probe can tell you.
//! The assertions only pin the two outcomes that would make the probe itself
//! meaningless: no monitors enumerated at all, or not one of them answering a
//! brightness read.

#[cfg(target_os = "windows")]
#[test]
#[ignore = "requires a real DDC/CI-capable display; run with --ignored --nocapture"]
fn ddc_hardware_probe() -> darkbright_helper::Result<()> {
    use darkbright_helper::core::brightness::percent_from_vcp;
    use darkbright_helper::platform::windows::ddc::{
        enumerate_monitors, get_monitor_id, get_physical_monitors, get_vcp_feature,
    };

    // 1. Enumerate Monitors
    let monitors = enumerate_monitors()?;
    println!("Found {} logical monitors.", monitors.len());
    assert!(
        !monitors.is_empty(),
        "no logical monitors enumerated — run this on a machine with a display attached"
    );

    // Tracks whether any monitor answered a brightness read, so the probe can
    // distinguish "DDC works here" from "every monitor refused".
    let mut brightness_reads = 0_usize;

    for (i, &hmonitor) in monitors.iter().enumerate() {
        println!("\n--- Logical Monitor #{i} ---");

        // 2. Identify Monitor (EDID)
        match get_monitor_id(hmonitor) {
            Ok(id) => {
                println!(
                    "  ID: {} {} (SN: {:?})",
                    id.manufacturer, id.model_name, id.serial_number
                );
            }
            Err(e) => {
                println!("  Failed to identify monitor: {e}");
            }
        }

        // 3. Get Physical Monitors
        match get_physical_monitors(hmonitor) {
            Ok(physical_monitors) => {
                println!("  Physical monitors: {}", physical_monitors.len());

                for (j, pm) in physical_monitors.iter().enumerate() {
                    // 4. Read Brightness (VCP 0x10)
                    match get_vcp_feature(pm, &format!("physical #{j}"), 0x10) {
                        Ok((current, max)) => {
                            // Both numbers, because they are the ones that
                            // diverge: a maximum other than 100 means the raw
                            // value is not the percentage, and this line is
                            // where that shows up.
                            let percent = percent_from_vcp(current, Some(max));
                            println!(
                                "    [Physical #{j}] Brightness: {current} of {max} → {percent}%"
                            );
                            brightness_reads += 1;
                        }
                        Err(e) => {
                            println!("    [Physical #{j}] Failed to read brightness: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                println!("  Failed to get physical monitors: {e}");
            }
        }
    }

    assert!(
        brightness_reads > 0,
        "no monitor answered a VCP 0x10 brightness read — DDC/CI is unavailable on this machine \
         (check the monitor's DDC/CI setting, the cable, and any KVM in between)"
    );

    Ok(())
}
