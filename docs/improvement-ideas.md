# Improvement Ideas

This document tracks potential future enhancements for the Brightness Control Tool.

---

## 🚀 TOP PRIORITY (Low Effort / High Value)
**Quick wins that significantly improve user experience:**

- **"Always start with the System" option**
  Add a configuration option to automatically launch the application when Windows starts. This eliminates the need for manual startup and ensures brightness control is always available without user intervention.

- **Configuration import/export**
  Add functionality to export and import configuration settings, allowing users to backup their preferences or share them across multiple machines. This would extend the existing JSON configuration system in `src/core/config.rs` with import/export capabilities.

## 🔥 HIGH PRIORITY (Low-Medium Effort / High Value)
**Core functionality enhancements:**

- **Basic Settings GUI with per-monitor settings**
  Create a graphical interface for managing application settings and individual monitor configurations. This would leverage the existing JSON configuration system in `src/core/config.rs` while providing an intuitive way for users to adjust hotkeys, OSD settings, and monitor-specific parameters.

- **Dark/light theme switching**
  Implement a theme system that allows users to switch between dark and light modes for the application interface. This would involve creating a theme configuration structure and updating the GUI components to respect the selected theme preference.

## ⭐ MEDIUM-HIGH PRIORITY (Medium Effort / High Value)
**Advanced features that differentiate the product:**

- **OSD Animations (fade in/out, animated progress)**
  Implement smooth fade-in and fade-out transitions for the on-screen display, along with animated progress bar changes when brightness is adjusted. This would enhance the visual feedback by making brightness changes feel more polished and responsive, building upon the existing GDI-based OSD system.

- **Real-time hotkey capture in settings**
  Add a hotkey capture interface in the settings GUI that allows users to press keys directly to set custom hotkey combinations. This would eliminate the need for manual text input and provide immediate validation of key combinations, integrating with the existing hotkey parsing system.

- **Sleep/wake brightness restoration**
  *Partially implemented:* the power event listener in `src/platform/windows/power.rs` already triggers a cache refresh on system resume, resyncing state with whatever the monitors report. Remaining idea: save the pre-sleep brightness values and actively *restore* them on wake for monitors that reset themselves during sleep, instead of adopting the reset values.

- **Basic internationalization foundation**
  Implement the technical infrastructure needed to support multiple languages, including string externalization and locale-aware formatting. This would involve creating a translation system that can be extended later with actual language translations.

- **Ambient light sensor integration**
  Integrate with Windows ambient light sensor APIs to automatically adjust screen brightness based on surrounding light conditions. This would provide a more comfortable viewing experience by dynamically adapting to changing lighting environments throughout the day.

- **Contrast control alongside brightness**
  Extend the DDC/CI communication to include contrast adjustments (VCP 0x12) in addition to brightness control. This would give users more comprehensive display control, allowing them to fine-tune both brightness and contrast for optimal viewing quality.

## 📊 MEDIUM PRIORITY (Medium Effort / Medium Value)
**Nice-to-have features for power users:**

- **Multiple OSD styles and themes**
  Create alternative OSD visual styles beyond the default progress bar, such as circular indicators, percentage displays, or minimalist designs. This would cater to different user preferences and provide visual variety while maintaining the same underlying brightness control functionality.

- **Monitor grouping and synchronization**
  Allow users to group multiple monitors together and apply brightness changes simultaneously across the entire group. This would be particularly useful for users with identical monitors or those who want consistent brightness across their entire workspace.

- **Brightness curves/gamma adjustment**
  Implement non-linear brightness curves that allow for more natural-looking brightness adjustments that account for human perception. Users could choose from preset curves like S-curve, exponential, or logarithmic, or create custom curves for specialized viewing conditions.

- **Color temperature adjustment**
  Extend DDC/CI control to include color temperature adjustments (VCP 0x14) alongside brightness control, allowing users to reduce blue light during evening hours. This would complement brightness control by providing a more comprehensive display management solution that reduces eye strain.

## 🛠️ LOW-MEDIUM PRIORITY (Medium-High Effort / Medium Value)
**Developer and advanced user features:**

- **Debug mode with detailed logging**
  *Largely implemented:* structured key-value debug logging of DDC/CI communication and state changes already exists via `env_logger` (`RUST_LOG=debug`). Remaining ideas: performance metrics, and making logs retrievable from release builds (no console) — the latter is tracked separately as an architecture-review finding (opt-in file logging under `%APPDATA%`).

- **Command-line interface for automation**
  Add a command-line interface that allows users to control brightness settings from scripts or the command line. This would enable automation scenarios and integration with existing workflow tools, supporting operations like setting specific brightness levels or applying presets.

- **Input source switching via hotkeys**
  Add support for switching monitor input sources (DisplayPort, HDMI, etc.) through hotkey combinations, extending beyond just brightness control. This would leverage additional DDC/CI VCP codes to provide comprehensive monitor management capabilities.

## 🌐 LOW PRIORITY (High Effort / Medium Value)
**Platform expansion and enterprise features:**

- **Linux support (X11 support via `XGrabKey` for hotkeys and `XComposite` for overlay)**
  Implement Linux support using X11 window system, utilizing `XGrabKey` for global hotkey registration and `XComposite` for creating transparent overlay windows. This would require creating a new platform module similar to the existing Windows implementation but adapted for X11 APIs and Linux-specific display management.

- **Linux support (Wayland support via layer-shell protocol for overlay and portal APIs for hotkeys)**
  Add Wayland compatibility by implementing the layer-shell protocol for overlay windows and using desktop portal APIs for hotkey registration. This would ensure the application works on modern Linux distributions that have moved away from X11 to Wayland compositors.

- **Linux support (DDC/CI via `/dev/i2c-*` devices or `ddcutil` integration)**
  Implement DDC/CI communication on Linux by either directly accessing `/dev/i2c-*` devices or integrating with the existing `ddcutil` command-line tool. This would require handling Linux-specific permissions and device access patterns while maintaining compatibility with the existing brightness control logic.

- **macOS support**
  Port the application to macOS, implementing platform-specific features like Touch Bar support for MacBook Pro models and integration with macOS menu bar. This would involve rewriting the platform abstraction layer to use macOS APIs for hotkey registration, overlay creation, and DDC/CI communication.

- **Windows 11 widget support**
  Create a Windows 11 widget that provides quick access to brightness controls and current monitor status directly from the widget panel. This would involve developing a widget-compatible interface that integrates with Windows 11's widget system while maintaining the core functionality.

- **Auto-update mechanism**
  Implement an automatic update system that checks for new versions and downloads/install them without user intervention. This would ensure users always have the latest features and bug fixes while maintaining security and stability of the update process.

- **Enterprise deployment packages**
  Create deployment packages specifically designed for enterprise environments, including MSI installers, group policy templates, and silent installation options. This would enable IT administrators to deploy the application across large organizations with centralized configuration management.

## 🔧 MAINTENANCE (no user-visible value; revisit periodically)
**Deliberate technical decisions that should not silently drift:**

- **`windows` crate upgrade (currently pinned at 0.52)**
  The pin is a deliberate stability choice (decided 2026-07): newer versions change many
  APIs the FFI layer touches (`HWND` ergonomics, `Result` signatures, cast smoothing), so
  an upgrade rewrites call sites across `src/platform/windows/`. Treat it as its own
  undertaking with a full manual test pass (DDC, OSD, overlay, tray, hotkeys, power
  events) — never as a side effect of another change. Value grows over time: newer
  versions eliminate workarounds the code currently carries.

## 📝 LOWEST PRIORITY (High Effort / Low Value)
**Marketing, documentation, and comprehensive rebranding:**

- **Complete app renaming and rebranding**
  Develop a comprehensive brand identity including a memorable name, visual style guide, and consistent messaging across all user-facing elements. This would involve updating all code references, documentation, and user interfaces to reflect the new brand identity while maintaining functionality.

- **Professional logo and icon design**
  Commission professional design services to create a distinctive logo and application icon that represents the product's purpose and appeals to the target audience. This would include creating multiple versions for different contexts (taskbar, system tray, splash screen, marketing materials) and ensuring they follow Windows design guidelines.

- **Comprehensive user documentation**
  Create detailed user guides, tutorials, and FAQ sections covering all features from basic usage to advanced configurations. This would include step-by-step instructions, troubleshooting guides, and video tutorials to help users maximize the application's capabilities.

- **Automated testing suite**
  Develop a comprehensive automated testing framework covering unit tests, integration tests, and end-to-end testing scenarios. This would ensure code quality, prevent regressions, and validate functionality across different Windows versions and hardware configurations.

- **Advanced internationalization (major language translations)**
  Implement complete translations for major languages including Spanish, German, Japanese, and Chinese, covering all user interface elements and documentation. This may ideally involve working with professional translators and native speakers to ensure accuracy and cultural appropriateness.
