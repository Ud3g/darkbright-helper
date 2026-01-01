use std::collections::HashMap;

use darkbright_helper::core::brightness::calculate_adjustment;
use darkbright_helper::core::config::Config;
use darkbright_helper::core::state::{BrightnessMessage, MonitorId, MonitorState};
use darkbright_helper::platform::windows::ddc::{get_monitor_id, DdcMonitor};
use darkbright_helper::platform::windows::get_monitor_under_cursor;
use darkbright_helper::platform::windows::osd::OsdWindow;
use darkbright_helper::platform::windows::overlay::OverlayManager;
use darkbright_helper::{BrightnessError, Result};

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
    /// Ermittelt den Zielmonitor (Mausposition), berechnet die neuen Werte,
    /// zeigt das OSD sofort an und führt dann das DDC-Update durch.
    fn handle_adjust(&mut self, monitor_id: Option<MonitorId>, delta: i8) -> Result<()> {
        // 1. Ziel-Monitor und Handle ermitteln (Schritt #34)
        // Wir benötigen das HMONITOR Handle für die OSD- und Overlay-Positionierung.
        let hmonitor = get_monitor_under_cursor()?;

        // Wenn keine ID übergeben wurde, identifizieren wir den Monitor unter dem Cursor.
        let target_id = match monitor_id {
            Some(id) => id,
            None => get_monitor_id(hmonitor)?,
        };

        // 2. Zustand und Monitor-Objekt finden
        let state = self.states.get_mut(&target_id).ok_or_else(|| {
            BrightnessError::MonitorNotFound(target_id.to_string())
        })?;

        let monitor = self.monitors.iter_mut().find(|m| m.id() == &target_id).ok_or_else(|| {
            BrightnessError::MonitorNotFound(target_id.to_string())
        })?;

        // 3. Neue Helligkeit berechnen (Schritt #37)
        let adjustment = calculate_adjustment(
            state.effective_brightness(),
            state.overlay_opacity,
            delta
        );

        // 4. Optimistisches Update (Schritt #38)
        state.set_pending(adjustment.hardware_brightness);
        let old_overlay = state.overlay_opacity;
        state.overlay_opacity = adjustment.overlay_opacity;

        // Overlay aktualisieren (Software-Ebene ist sofort wirksam)
        if state.overlay_opacity != old_overlay {
            self.overlay_manager.update(&target_id, hmonitor, state.overlay_opacity)?;
        }

        // OSD anzeigen oder aktualisieren
        if self.osd.is_visible() {
            self.osd.update(state)?;
        } else {
            self.osd.show(hmonitor, state)?;
        }

        // 5. Hardware-Update via DDC (Blocking im Controller-Thread)
        log::debug!("Setting DDC brightness for {}: {}%", target_id, adjustment.hardware_brightness);
        
        match monitor.set_brightness(adjustment.hardware_brightness as u32) {
            Ok(_) => {
                state.confirm_brightness();
                // OSD zur Bestätigung aktualisieren (entfernt ggf. Error-Färbung)
                self.osd.update(state)?;
            }
            Err(e) => {
                log::error!("DDC-Fehler für {}: {}", target_id, e);
                // 6. Fehler-Rollback (Schritt #39)
                state.revert_pending();
                state.overlay_opacity = old_overlay;
                
                // Overlay auf alten Wert zurücksetzen
                let _ = self.overlay_manager.update(&target_id, hmonitor, old_overlay);
                
                // OSD auf Fehlerzustand setzen
                self.osd.update_error(state)?;
                return Err(e);
            }
        }

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
