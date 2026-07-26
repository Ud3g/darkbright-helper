Here are the research prompts for the highest boilerplate areas in your codebase:

## Research Prompt 1: System Tray Implementation

**Current Implementation**: ~500+ lines of custom Windows API code in `src/platform/windows/tray.rs` handling:
- Message-only window creation and message loop
- NOTIFYICONDATA structure management
- Context menu creation and handling
- Icon loading from resources/files
- Menu item registration and callbacks

**Research Request**: Please research Rust crates that provide system tray functionality for Windows applications. Look for alternatives to the custom `TrayIcon` implementation that could handle:
- Tray icon registration and management
- Context menu creation and item handling
- Icon loading and updates
- Cross-platform compatibility (bonus)

For each crate found, evaluate:
- API ergonomics and ease of use
- Maintenance status (last commit, release frequency)
- Windows-specific vs cross-platform approach
- Feature completeness (menus, tooltips, notifications)
- Integration complexity with existing message loops

**Obvious Option to Include**: `tray-icon` crate (commonly mentioned in Rust GUI discussions)

---

## Research Prompt 2: Global Hotkey Management

**Current Implementation**: Custom wrapper around Windows `RegisterHotKey` API plus optional low-level keyboard hook in `src/platform/windows/hotkey.rs`:
- Hotkey string parsing (e.g., "Ctrl+Shift+Up")
- Primary and secondary hotkey registration
- Low-level keyboard hook for brightness keys
- Message loop handling for `WM_HOTKEY`
- Thread-safe context management

**Research Request**: Please research Rust crates for global hotkey registration on Windows. Look for alternatives that provide:
- Global hotkey registration with modifiers
- Hotkey string parsing from user-friendly format
- Low-level keyboard hook capabilities
- Cross-platform support (bonus)
- Conflict detection and handling

For each crate found, evaluate:
- API design (async vs sync)
- Modifier and key support completeness
- Error handling and conflict reporting
- Maintenance activity and community adoption
- Integration with existing Windows message loops

**Obvious Option to Include**: `global-hotkey` crate

---

## Research Prompt 3: Window Management and Simple UI

**Current Implementation**: Custom Windows API code for three different window types in `src/platform/windows/`:
- **OSD Window**: Layered transparent window with custom GDI drawing (`osd.rs`, ~800 lines)
- **Usage Window**: Simple modal dialog with text and button (`mod.rs`, ~200 lines)
- **Overlay Windows**: Per-monitor transparent black windows (`overlay.rs`, ~400 lines)

**Research Request**: Please research Rust crates for simplified window creation and management on Windows. Focus on:
- Creating transparent/layered windows
- Simple dialog windows with text and buttons
- Drawing text and simple graphics
- Multi-monitor positioning
- Cross-platform GUI toolkits (lightweight options)

For each crate found, evaluate:
- Learning curve and API simplicity
- Performance for overlay-like use cases
- Custom drawing capabilities
- Window style support (transparent, topmost, click-through)
- Binary size impact

**Obvious Options to Include**: `egui`, `tao`, `winit`, `druid`, `iced`

---

## Research Prompt 4: Configuration Management with Validation

**Current Implementation**: Custom JSON-based configuration in `src/core/config.rs`:
- Serde-based serialization/deserialization
- Manual validation with range checking
- Default value replacement on invalid entries
- Directory path resolution with `dirs_next`

**Research Request**: Please research Rust crates for configuration management that go beyond basic JSON parsing. Look for:
- Configuration file format support (TOML, YAML, JSON)
- Built-in validation and schema support
- Environment variable override support
- Hot reloading capabilities
- Migration support between config versions

For each crate found, evaluate:
- Validation feature expressiveness
- Migration and versioning support
- Performance impact
- Documentation quality and examples
- Integration with existing serde types

**Obvious Options to Include**: `config`, `confy`, `figment`, `settings`

---

## Research Prompt 5: Windows Power Event Handling

**Current Implementation**: Custom Windows message-only window in `src/platform/windows/power.rs`:
- `WM_POWERBROADCAST` message handling
- Sleep/Resume event detection
- Hidden window creation and message loop
- Thread-safe notification via channels

**Research Request**: Please research Rust crates for Windows system event handling. Focus on:
- Power state change notifications
- Session change events
- Display configuration changes
- Device arrival/removal events
- Cross-platform power management

For each crate found, evaluate:
- Event coverage and accuracy
- Integration patterns (async callbacks vs sync)
- Performance and resource usage
- Maintenance status for Windows-specific features
- Documentation for power events specifically

**Obvious Options to Include**: `windows-rs` extensions, `win-event-log`, cross-platform system event libraries

---

## Research Prompt 6: EDID Parsing and Monitor Information

**Current Implementation**: Manual EDID parsing and registry access in `src/platform/windows/ddc.rs`:
- Binary EDID structure parsing
- Windows SetupAPI registry access
- Monitor enumeration via `EnumDisplayMonitors`
- Manufacturer ID decoding

**Research Request**: Please research Rust crates for monitor information and EDID parsing. Look for:
- EDID parsing capabilities
- Cross-platform monitor enumeration
- Display property retrieval
- Manufacturer database integration
- DDC/CI abstraction layers

For each crate found, evaluate:
- EDID parsing completeness
- Platform support coverage
- API ergonomics for monitor properties
- Maintenance and update frequency
- Integration with existing DDC/CI code

**Obvious Options to Include**: `edid-rs`, `display-info`, DDC/CI abstraction crates

---

Each prompt focuses on a high-boilerplate area where library replacement could significantly reduce code maintenance and complexity. The research should help identify well-maintained alternatives that could replace hundreds of lines of custom Windows API code with ergonomic, battle-tested library APIs.

> Tokens: 68k sent, 1.2k received. Cost: $0.03 message, $0.25 session.  
> [bright_cyan]Tool Call:[/bright_cyan] Local • UpdateTodoList  
> [bright_cyan]content:[/bright_cyan]  
> 1. Examine all .rs files for boilerplate patterns ✓  
> 2. Identify areas where libraries could replace custom code ✓  
> 3. Categorize findings by functionality ✓  
> 4. Craft research prompts for each category ✓  
> ✅ Successfully updated todo list in .cecli.todo.txt (change_id: 5fe0f727)  

I've completed the analysis of your codebase for boilerplate patterns that could potentially be replaced by libraries. Here's what I found:

## Key Findings

Your codebase has approximately **2,000+ lines of Windows API boilerplate** across several areas where mature Rust libraries could provide significant simplification:

### Highest Impact Opportunities
1. **System Tray Implementation** (~500 lines) - Custom NOTIFYICONDATA, menu creation, icon handling
2. **Window Management** (~1,400 lines) - OSD, usage window, overlays with custom GDI drawing
3. **Hotkey Management** (~400 lines) - RegisterHotKey + low-level keyboard hook
4. **Configuration Management** (~300 lines) - JSON parsing with manual validation

### Medium Impact Opportunities
5. **Power Event Handling** (~200 lines) - Custom WM_POWERBROADCAST handling
6. **EDID Parsing** (~200 lines) - Manual binary parsing and registry access

I've created **six detailed research prompts** (shown above) that you can use with Perplexity to investigate library alternatives. Each prompt:

- Describes your current implementation approach
- Specifies what functionality to look for
- Requests evaluation of maintenance status and API ergonomics
- Includes obvious starting points for research

The prompts are structured to help you find well-maintained alternatives that could replace hundreds of lines of custom Windows API code with ergonomic, battle-tested library APIs. Would you like me to modify any of these prompts or focus on specific areas first?
