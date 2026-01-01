use std::collections::HashMap;

use darkbright_helper::core::config::Config;
use darkbright_helper::core::state::{BrightnessMessage, MonitorId, MonitorState};
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

    /// Verarbeitet eine Nachricht zur Helligkeitssteuerung.
    ///
    /// Gibt `Ok(true)` zurück, wenn die Anwendung weiterlaufen soll,
    /// oder `Ok(false)`, wenn ein Herunterfahren angefordert wurde.
    ///
    /// # Errors
    ///
    /// Gibt einen Fehler zurück, wenn die Verarbeitung der Nachricht fehlschlägt.
    pub fn handle_message(&mut self, message: BrightnessMessage) -> Result<bool> {
        match message {
            BrightnessMessage::Adjust { monitor_id, delta } => {
                self.handle_adjust(monitor_id, delta)?;
            }
            BrightnessMessage::SetAbsolute { monitor_id, value } => {
                self.handle_set_absolute(monitor_id, value)?;
            }
            BrightnessMessage::Refresh => {
                self.handle_refresh()?;
            }
            BrightnessMessage::Shutdown => {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Wendet eine relative Helligkeitsänderung an.
    ///
    /// Die eigentliche Logik wird in Schritt #37 implementiert.
    fn handle_adjust(&mut self, _monitor_id: Option<MonitorId>, _delta: i8) -> Result<()> {
        // TODO: Implementierung folgt in Schritt #37
        Ok(())
    }

    /// Setzt einen absoluten Helligkeitswert für einen Monitor.
    fn handle_set_absolute(&mut self, _monitor_id: Option<MonitorId>, _value: u8) -> Result<()> {
        // Platzhalter für zukünftige Erweiterungen (z.B. feste Helligkeit via CLI-Befehl)
        Ok(())
    }

    /// Aktualisiert die Liste der Monitore und liest deren Zustände neu ein.
    fn handle_refresh(&mut self) -> Result<()> {
        // TODO: Implementierung folgt in Schritt #40
        Ok(())
    }
}

fn main() {
    println!("Brightness Control Tool Starting...");
}
