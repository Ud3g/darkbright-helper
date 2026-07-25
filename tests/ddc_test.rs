#[cfg(target_os = "windows")]
#[test]
fn test_ddc_communication() -> darkbright_helper::Result<()> {
    use darkbright_helper::platform::windows::ddc::{
        enumerate_monitors, get_monitor_id, get_physical_monitors, get_vcp_feature,
    };

    // 1. Enumerate Monitors
    let monitors = enumerate_monitors()?;
    println!("Found {} logical monitors.", monitors.len());

    if monitors.is_empty() {
        println!("No monitors found. Skipping DDC tests.");
        return Ok(());
    }

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
                            println!("    [Physical #{j}] Brightness: {current} (Max: {max})");
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

    Ok(())
}
