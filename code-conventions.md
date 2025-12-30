# Rust Code Conventions Guide
## For the Brightness Control Tool Project

**Document Purpose:** This guide aggregates Rust coding conventions from official style guides, API guidelines, and best practices. It provides applicable conventions for our multi-monitor brightness control tool with DDC/CI, fullscreen overlay, and hotkey support.

---

## 1. Formatting Conventions

### 1.1 Indentation & Line Width

- **Indentation:** 4 spaces per level (never tabs)
- **Line Width:** Target 100 characters maximum
- **Block Indent Preferred:** Use block indent over visual indent for function arguments

```rust
// ✓ Preferred: Block indent with trailing commas
let result = expensive_function(
    argument_one,
    argument_two,
    argument_three,
);

// ✗ Avoid: Visual indent
let result = expensive_function(argument_one,
                                 argument_two,
                                 argument_three);
```

**Application:** Keeps diffs small and prevents excessive rightward drift as parameter names change.

### 1.2 Trailing Commas

- Use trailing commas in multi-line lists (function calls, arrays, struct literals)
- Improves diff readability and makes code refactoring easier

```rust
// ✓ Preferred
let monitors = vec![
    Monitor { id: 0, brightness: 75 },
    Monitor { id: 1, brightness: 50 },
];

// ✗ Avoid
let monitors = vec![
    Monitor { id: 0, brightness: 75 },
    Monitor { id: 1, brightness: 50 }
];
```

### 1.3 Attributes

- Put each attribute on its own line, indented to the item's level
- Format attributes with argument lists like function calls
- Prefer outer attributes (`#[...]`) over inner attributes (`#![...]`)
- Use single `#[derive(...)]` attribute per item (combine all derives into one)

```rust
// ✓ Preferred
#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct MonitorCapabilities {
    supports_ddc_ci: bool,
    min_brightness: u8,
    max_brightness: u8,
}

// ✗ Avoid
#[repr(C)] #[derive(Debug)] #[derive(Clone)]
struct MonitorCapabilities { ... }
```

### 1.4 Whitespace

- Use spaces around operators: `x + y`, `a = b`, `if x == y`
- Spaces after keywords: `if x`, `fn name()`, `for item in items`
- No space before function call parentheses: `function()`, not `function ()`
- Spaces inside braces for struct literals when on one line: `Foo { x, y }`

---

## 2. Naming Conventions

### 2.1 Casing Rules (RFC 430)

| Item Type | Convention | Examples |
|-----------|-----------|----------|
| **Types (structs, enums, traits)** | `UpperCamelCase` | `MonitorInfo`, `BrightnessLevel`, `DdcDevice` |
| **Functions & methods** | `snake_case` | `get_brightness()`, `set_overlay_opacity()`, `update_monitor_state()` |
| **Local variables & fields** | `snake_case` | `current_brightness`, `mouse_position`, `overlay_opacity` |
| **Constants** | `SCREAMING_SNAKE_CASE` | `DEFAULT_BRIGHTNESS`, `MAX_MONITORS`, `DDC_TIMEOUT_MS` |
| **Statics** | `SCREAMING_SNAKE_CASE` | `GLOBAL_STATE`, `HOTKEY_REGISTRY` |
| **Macro names** | `snake_case` | `debug_log!`, `assert_brightness!` |
| **Crate names** | `lowercase` (no `-rs` suffix) | `brightness_control`, `ddc_client` |
| **Module names** | `snake_case` | `hardware`, `overlay`, `hotkeys` |
| **Type parameters** | Concise `UpperCamelCase` or single letter | `T`, `E`, `M` |
| **Lifetimes** | Short lowercase | `'a`, 'ddc` |
| **Acronyms in UpperCamelCase** | Treat as one word | `Uuid` (not `UUID`), `Ddc` (not `DDC`) |
| **Acronyms in snake_case** | Lowercase all | `is_ddc_supported()`, `get_i2c_bus()` |

### 2.2 Project-Specific Naming

For the brightness control tool:

```rust
// Types
pub struct Monitor { ... }
pub struct DdcClient { ... }
pub struct OverlayState { ... }
pub enum BrightnessMode { Hardware, Software }

// Functions
pub fn get_monitors() -> Vec<Monitor>
pub fn set_brightness(&mut self, value: u8) -> Result<()>
pub fn update_osd_overlay(opacity: f32) -> Result<()>

// Constants
const DEFAULT_BRIGHTNESS: u8 = 100;
const MAX_MONITORS: usize = 16;
const OVERLAY_TRANSITION_MS: u64 = 150;

// Modules
mod hardware { ... }  // DDC/CI and hardware control
mod overlay { ... }   // Fullscreen overlay and rendering
mod hotkeys { ... }   // Hotkey registration and handling
mod config { ... }    // Configuration and state management
```

### 2.3 Constructor Naming

- General constructor: `new()` or `new_with_details()`
- Conversion constructors: `from_*` (e.g., `from_monitor_handle()`)
- Builder-like: `with_*()`

```rust
impl DdcClient {
    pub fn new() -> Self { ... }
    pub fn with_timeout(timeout_ms: u64) -> Self { ... }
    pub fn from_monitor_handle(handle: MonitorHandle) -> Result<Self> { ... }
}
```

### 2.4 Iterator/Collection Methods (RFC 199)

For methods that return iterators:

```rust
impl MonitorList {
    pub fn iter(&self) -> Iter { ... }            // &self -> Iterator<&Item>
    pub fn iter_mut(&mut self) -> IterMut { ... } // &mut self -> Iterator<&mut Item>
    pub fn into_iter(self) -> IntoIter { ... }    // self -> Iterator<Item>
}
```

### 2.5 Conversion Methods (C-CONV)

| Method Prefix | Cost | Ownership |
|---------------|------|-----------|
| `as_*` | Free | borrowed → borrowed |
| `to_*` | Expensive | borrowed → borrowed/owned |
| `into_*` | Variable | owned → owned |

```rust
impl BrightnessValue {
    pub fn as_percent(&self) -> f32 { ... }              // Cheap, borrowed
    pub fn to_hardware_code(&self) -> u8 { ... }         // Expensive conversion
    pub fn into_ddc_vcp_code(self) -> DdcVcpCode { ... } // Takes ownership
}
```

### 2.6 Getter/Setter Naming (C-GETTER)

- **No `get_` prefix** for simple getters (Rust convention differs from many languages)
- Use `mut_` suffix for mutable access

```rust
pub struct Monitor {
    brightness: u8,
    monitor_id: u32,
}

impl Monitor {
    // ✓ Correct: No get_ prefix
    pub fn brightness(&self) -> u8 { self.brightness }
    pub fn brightness_mut(&mut self) -> &mut u8 { &mut self.brightness }
    pub fn monitor_id(&self) -> u32 { self.monitor_id }
    
    // ✗ Avoid
    pub fn get_brightness(&self) -> u8 { ... }
}
```

**Exception:** Use `get()` when there's a single, obvious thing to get (e.g., `Cell::get()`).

---

## 3. Project Structure

### 3.1 Directory Layout

Recommended structure for the brightness control tool:

```
brightness-control/
├── Cargo.toml
├── Cargo.lock
├── .gitignore
├── README.md
├── src/
│   ├── main.rs                 # Entry point and CLI
│   ├── lib.rs                  # Library root
│   ├── error.rs                # Error types
│   ├── hardware/
│   │   ├── mod.rs              # Hardware module
│   │   ├── ddc_client.rs        # DDC/CI protocol implementation
│   │   ├── monitor.rs           # Monitor detection and management
│   │   └── vcp_codes.rs         # VCP code constants and helpers
│   ├── overlay/
│   │   ├── mod.rs              # Overlay module
│   │   ├── window.rs            # Window creation and management
│   │   ├── renderer.rs          # Rendering and opacity control
│   │   └── osd.rs               # OSD (On-Screen Display) logic
│   ├── hotkeys/
│   │   ├── mod.rs              # Hotkey module
│   │   ├── registry.rs          # Windows hotkey registration
│   │   └── listener.rs          # Hotkey event listener
│   ├── config/
│   │   ├── mod.rs              # Configuration module
│   │   └── settings.rs          # Settings and state persistence
│   └── ui/
│       ├── mod.rs              # UI module
│       └── tray.rs              # System tray integration (optional)
├── tests/
│   ├── integration_tests.rs
│   └── hardware_tests.rs
└── examples/
    ├── basic_brightness.rs
    └── monitor_detection.rs
```

### 3.2 Module Organization Principles

1. **One file per module** when module is small (e.g., `hardware.rs`, `error.rs`)
2. **Directory with `mod.rs`** when grouping related submodules:
   - `hardware/mod.rs` declares submodules
   - `hardware/ddc_client.rs` contains implementation
   - `hardware/monitor.rs` contains implementation

3. **Keep modules small and focused**
   - Single responsibility principle
   - Each module should have 1-2 main purposes
   - Avoid kitchen-sink utility modules

### 3.3 Public vs Private (Visibility)

- Use `pub` for public API surfaces
- Use `pub(crate)` to expose only within the crate (not public to users)
- Default to private; make public only when needed
- Document public items with doc comments (`///`)

```rust
// Private - internal implementation
fn calculate_ddc_checksum(data: &[u8]) -> u8 { ... }

// Public to whole crate
pub(crate) fn validate_vcp_code(code: u8) -> bool { ... }

// Public API - document it!
/// Retrieves the brightness level from the monitor via DDC/CI.
/// 
/// # Returns
/// The current brightness value (0-255).
pub fn get_brightness(&self) -> Result<u8> { ... }
```

### 3.4 Error Handling Module

Create a centralized `error.rs`:

```rust
// src/error.rs
use std::fmt;

#[derive(Debug)]
pub enum BrightnessError {
    DdcCommunicationFailed(String),
    MonitorNotFound,
    HotkeyRegistrationFailed(u32),
    OverlayCreationFailed,
    InvalidBrightnessValue(u8),
}

impl fmt::Display for BrightnessError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Self::DdcCommunicationFailed(msg) => write!(f, "DDC communication error: {}", msg),
            Self::MonitorNotFound => write!(f, "Monitor not found"),
            Self::HotkeyRegistrationFailed(code) => write!(f, "Failed to register hotkey (code: {})", code),
            Self::OverlayCreationFailed => write!(f, "Failed to create overlay window"),
            Self::InvalidBrightnessValue(val) => write!(f, "Invalid brightness value: {}", val),
        }
    }
}

impl std::error::Error for BrightnessError {}

pub type Result<T> = std::result::Result<T, BrightnessError>;
```

---

## 4. Code Style Best Practices

### 4.1 Documentation Comments

Use `///` for public items, document API:

```rust
/// Manages DDC-CI communication with a single monitor.
/// 
/// This client handles the protocol layer for reading and writing VCP codes
/// to control monitor settings (brightness, contrast, etc.) via the DDC-CI
/// interface.
/// 
/// # Examples
/// 
/// ```no_run
/// use brightness_control::hardware::DdcClient;
/// 
/// let mut client = DdcClient::new()?;
/// let brightness = client.get_brightness()?;
/// client.set_brightness(150)?;
/// ```
pub struct DdcClient {
    // ...
}

/// Sets the monitor brightness level.
///
/// # Arguments
/// 
/// * `brightness` - Value from 0-255 (0 = off, 255 = maximum)
/// 
/// # Returns
/// 
/// Returns `Ok(())` on success, or `BrightnessError` if the DDC command fails.
/// 
/// # Errors
/// 
/// Returns `BrightnessError::DdcCommunicationFailed` if the DDC-CI protocol
/// communication fails with the monitor.
pub fn set_brightness(&mut self, brightness: u8) -> Result<()> {
    // ...
}
```

Use `//` for internal comments:

```rust
// Check if the monitor supports brightness control via VCP code 0x10
if !self.supports_vcp_code(0x10)? {
    return Err(BrightnessError::MonitorNotFound);
}
```

### 4.2 Variable Binding

Prefer `let` with clear naming over short abbreviations:

```rust
// ✓ Preferred - Clear intent
let current_brightness = monitor.get_brightness()?;
let overlay_opacity = 0.5;
let ddc_timeout_ms = 1000;

// ✗ Avoid - Too abbreviated
let cb = monitor.get_brightness()?;
let ov = 0.5;
let dto = 1000;
```

Use pattern matching for better readability:

```rust
// ✓ Preferred - Clear ownership transfer
let Monitor { id, brightness } = monitor;

// ✓ Preferred - Explicit unpacking
match get_monitor_list() {
    Ok(monitors) => process_monitors(monitors),
    Err(e) => eprintln!("Error: {}", e),
}
```

### 4.3 Function Length & Complexity

- Keep functions small and focused (ideally < 50 lines)
- Extract complex logic into helper functions
- Use type system to encode domain logic

```rust
// ✗ Avoid: Too much logic
pub fn update_all_monitors() {
    // 100+ lines of brightness adjustment, overlay updates, logging
}

// ✓ Preferred: Decomposed
pub fn update_all_monitors() -> Result<()> {
    let monitors = self.get_active_monitors()?;
    for monitor in monitors {
        self.update_monitor_brightness(&monitor)?;
    }
    self.refresh_osd_overlay()?;
    Ok(())
}

fn update_monitor_brightness(&mut self, monitor: &Monitor) -> Result<()> {
    // Focused logic
}

fn refresh_osd_overlay(&mut self) -> Result<()> {
    // Focused logic
}
```

### 4.4 Error Handling Pattern

Use `Result<T>` consistently for fallible operations:

```rust
// Functions that can fail should return Result
pub fn get_brightness(&mut self, monitor_id: u32) -> Result<u8> { ... }
pub fn set_brightness(&mut self, monitor_id: u32, value: u8) -> Result<()> { ... }

// Use ? operator for error propagation
pub fn sync_brightness(&mut self) -> Result<()> {
    let current = self.get_brightness(0)?;  // ? propagates error
    let target = self.config.target_brightness;
    
    if current != target {
        self.set_brightness(0, target)?;
    }
    
    Ok(())
}

// Use match or if let for recovery
match register_hotkeys(&keys) {
    Ok(_) => println!("Hotkeys registered"),
    Err(e) => eprintln!("Warning: {}", e),
}
```

### 4.5 Immutability by Default

Declare variables as immutable unless mutation is needed:

```rust
// ✓ Preferred - Immutable by default
let brightness = 100;
let monitor_list = get_monitors()?;

// When mutation is needed - explicit
let mut overlay = Overlay::new()?;
overlay.set_opacity(0.75);
overlay.show()?;

// Function parameters - mut only when needed
pub fn update_brightness(&mut self, value: u8) -> Result<()> {
    // &mut self because we need to modify internal state
}
```

### 4.6 Type Safety & Domain Modeling

Use the type system to express domain constraints:

```rust
// ✓ Preferred - Type-safe brightness value
#[derive(Debug, Clone, Copy)]
pub struct BrightnessValue {
    value: u8,  // 0-255 guaranteed by type
}

impl BrightnessValue {
    pub fn new(value: u8) -> Result<Self> {
        Ok(BrightnessValue { value })
    }
    
    pub fn as_percentage(&self) -> f32 {
        (self.value as f32 / 255.0) * 100.0
    }
}

// Usage
let brightness = BrightnessValue::new(150)?;  // Type-safe
monitor.set_brightness(brightness)?;

// ✗ Avoid - Raw values, no validation
pub fn set_brightness(&mut self, brightness: u8) -> Result<()> {
    // No guarantee value is in valid range
}
```

### 4.7 Constants vs Magic Numbers

Extract magic numbers into named constants:

```rust
// ✓ Preferred
const MIN_BRIGHTNESS: u8 = 0;
const MAX_BRIGHTNESS: u8 = 255;
const OVERLAY_FADE_DURATION_MS: u64 = 300;
const DDC_WRITE_TIMEOUT_MS: u64 = 5000;
const MAX_MONITORS: usize = 16;

pub fn set_brightness(&mut self, brightness: u8) -> Result<()> {
    if brightness > MAX_BRIGHTNESS {
        return Err(BrightnessError::InvalidBrightnessValue(brightness));
    }
    // ...
}

// ✗ Avoid
pub fn set_brightness(&mut self, brightness: u8) -> Result<()> {
    if brightness > 255 {  // What does 255 mean?
        return Err(...);
    }
    // ...
}
```

---

## 5. Windows-Specific Conventions

For Windows-specific code (DDC/CI, hotkeys, overlay):

### 5.1 FFI (Foreign Function Interface) Module

Organize Windows API bindings separately:

```rust
// src/platform/windows/ffi.rs - Win32 bindings
#![allow(non_snake_case)]  // Windows API doesn't follow Rust conventions

use winapi::shared::windef::HWND;
use winapi::um::winuser::{RegisterHotKey, UnregisterHotKey};

// src/platform/windows/mod.rs - Safe wrappers
pub mod ffi;
mod ddc;
mod hotkey;

pub use ddc::DdcClient;
pub use hotkey::HotkeyRegistry;
```

### 5.2 Handle Wrappers

Always wrap Windows handles in RAII types:

```rust
/// Safe wrapper around a Windows monitor handle
pub struct MonitorHandle {
    handle: HMONITOR,
}

impl Drop for MonitorHandle {
    fn drop(&mut self) {
        // Cleanup if needed
    }
}

impl MonitorHandle {
    pub fn new(handle: HMONITOR) -> Self {
        MonitorHandle { handle }
    }
}
```

### 5.3 Safe FFI Boundaries

Keep unsafe code at module boundaries:

```rust
// ✓ Preferred - Unsafe at boundary, safe API
pub fn register_hotkey(&mut self, id: i32, modifiers: u32, key: u32) -> Result<()> {
    unsafe {
        if RegisterHotKey(self.hwnd, id, modifiers, key) == 0 {
            return Err(BrightnessError::HotkeyRegistrationFailed(GetLastError()));
        }
    }
    Ok(())
}

// ✗ Avoid - Exposing unsafe to callers
pub unsafe fn register_hotkey_unsafe(&mut self, id: i32, ...) { ... }
```

---

## 6. Testing & Documentation

### 6.1 Test Module Organization

```rust
// In the main module file or separate test file
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_brightness_value_construction() {
        let brightness = BrightnessValue::new(128).unwrap();
        assert_eq!(brightness.as_percentage(), 50.2);
    }

    #[test]
    fn test_invalid_brightness_rejected() {
        assert!(BrightnessValue::new(256).is_err());
    }

    #[test]
    fn test_monitor_detection() {
        let monitors = get_monitors().expect("Should detect monitors");
        assert!(!monitors.is_empty());
    }
}
```

### 6.2 Example Code

Create examples in `examples/` directory:

```rust
// examples/basic_brightness.rs
use brightness_control::{Monitor, DdcClient};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut client = DdcClient::new()?;
    let monitors = client.detect_monitors()?;
    
    for monitor in monitors {
        println!("Monitor {}: brightness = {}", 
                 monitor.id(), 
                 monitor.get_brightness()?);
    }
    
    Ok(())
}
```

---

## 7. Performance Considerations

### 7.1 Avoiding Allocations

```rust
// ✓ Preferred - Use references where possible
pub fn process_monitor(&mut self, monitor: &Monitor) -> Result<()> {
    // ...
}

// For hotpath code
pub fn update_overlay(&mut self, monitors: &[Monitor]) -> Result<()> {
    // Iterate without cloning
    for monitor in monitors {
        self.update_single_overlay(monitor)?;
    }
    Ok(())
}
```

### 7.2 Lazy Initialization

For expensive resources like DDC clients:

```rust
pub struct BrightnessController {
    ddc_client: Option<DdcClient>,
    overlay: Option<Overlay>,
}

impl BrightnessController {
    pub fn get_ddc_client(&mut self) -> Result<&mut DdcClient> {
        if self.ddc_client.is_none() {
            self.ddc_client = Some(DdcClient::new()?);
        }
        Ok(self.ddc_client.as_mut().unwrap())
    }
}
```

---

## 8. Tools & Automation

### 8.1 Rustfmt Configuration

Create `.rustfmt.toml` in project root:

```toml
edition = "2021"
max_width = 100
hard_tabs = false
tab_spaces = 4
newline_style = "Auto"
use_small_heuristics = "Default"
merge_derives = true
reorder_imports = true
reorder_modules = true
remove_nested_parens = true
```

### 8.2 Clippy Lints

Add to `Cargo.toml`:

```toml
[lints.clippy]
all = "warn"
pedantic = "warn"
# Allow certain warnings for Windows-specific code
unsafe_code = "allow"  # FFI is inherently unsafe
```

Or use in code:

```rust
#![warn(clippy::all, clippy::pedantic)]
#![allow(clippy::module_name_repetitions)]  // Disable specific warnings
```

### 8.3 Pre-commit Checks

Run before committing:

```bash
# Format code
cargo fmt -- --check

# Lint with clippy
cargo clippy -- -D warnings

# Run tests
cargo test

# Check documentation
cargo doc --no-deps --document-private-items
```

---

## 9. Recommended Dependencies for Project

Based on conventions and project needs:

| Purpose | Crate | Notes |
|---------|-------|-------|
| Windows API | `winapi` or `windows` | Modern `windows` crate preferred |
| Error handling | `thiserror` or `anyhow` | Use `thiserror` for libraries |
| Logging | `log` + `env_logger` or `tracing` | `tracing` for async code |
| Configuration | `serde` + `toml` or `ron` | For config persistence |
| Async (if needed) | `tokio` | For hotkey listener threads |
| Testing | Built-in `#[test]` | Supplemented with `proptest` for property testing |
| Documentation | `cargo doc` | Built-in Rust documentation |

---

## 10. Summary Checklist

Before committing code:

- [ ] **Formatting**: `cargo fmt` passes
- [ ] **Linting**: `cargo clippy` shows no warnings
- [ ] **Testing**: `cargo test` passes all tests
- [ ] **Naming**: Follow RFC 430 conventions (UpperCamelCase types, snake_case functions)
- [ ] **Documentation**: Public items have `///` doc comments
- [ ] **Error Handling**: Use `Result<T>` for fallible operations
- [ ] **Module Organization**: Single responsibility per module
- [ ] **Immutability**: Variables immutable by default
- [ ] **FFI Safety**: Unsafe code isolated at boundaries
- [ ] **Constants**: Magic numbers extracted to named constants
- [ ] **Line Length**: No lines exceed 100 characters
- [ ] **Trailing Commas**: Multi-line lists have trailing commas
- [ ] **Comments**: Use `//` for internal logic, `///` for public API

---

## References

- [The Rust Style Guide](https://doc.rust-lang.org/nightly/style-guide/)
- [Rust API Guidelines](https://rust-lang.github.io/api-guidelines/)
- [RFC 430 - Naming Conventions](https://github.com/rust-lang/rfcs/blob/master/text/0430-finalizing-naming-conventions.md)
- [RFC 199 - Iterator Naming](https://github.com/rust-lang/rfcs/blob/master/text/0199-ownership-variants.md)

---

**Last Updated:** December 2025  
**Edition:** Rust 2021