use std::collections::HashMap;

use darkbright_helper::core::config::Config;
use darkbright_helper::core::state::{MonitorId, MonitorState};
use darkbright_helper::platform::windows::ddc::DdcMonitor;
use darkbright_helper::platform::windows::osd::OsdWindow;
use darkbright_helper::platform::windows::overlay::OverlayManager;
use darkbright_helper::Result;

/// Haupt-Controller für die Helligkeitssteuerung.
///
/// Diese Struktur verwaltet die erkannten Monitore, deren Zustände,
/// die Dimming-Overlays sowie das On-Screen Display (OSD).
pub struct BrightnessController {
    /// Liste der erkannten DDC/CI-Monitore.
    pub monitors: Vec<DdcMonitor>,
    /// Aktueller Zustand (Helligkeit, Overlay) pro Monitor.
    pub states: HashMap<MonitorId, MonitorState>,
    /// Verwalter für die Dimming-Overlay-Fenster.
    pub overlay_manager: OverlayManager,
    /// Das On-Screen Display zur Anzeige von Änderungen.
    pub osd: OsdWindow,
    /// Die geladene Konfiguration.
    pub config: Config,
}

impl BrightnessController {
    /// Erstellt eine neue Instanz des `BrightnessController`.
    ///
    /// Initialisiert das OSD mit den Werten aus der Konfiguration.
    /// Die Monitore und Zustände werden in Phase 6 beim Anwendungsstart ermittelt.
    pub fn new(config: Config) -> Result<Self> {
        let osd = OsdWindow::new(config.osd.opacity, config.osd.timeout_ms)?;

        Ok(Self {
            monitors: Vec::new(),
            states: HashMap::new(),
            overlay_manager: OverlayManager::default(),
            osd,
            config,
        })
    }
}

fn main() {
    println!("Brightness Control Tool Starting...");
}
