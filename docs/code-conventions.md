# Rust Code Conventions
## Brightness Control Tool

This document contains **project-specific** conventions only. Standard Rust conventions (naming, formatting, error handling) are assumed known and enforced by tooling.

---

## 1. Tooling

All code must pass these checks before commit:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
```

### rustfmt.toml

```toml
edition = "2024"
max_width = 100
hard_tabs = false
tab_spaces = 4
```

### Clippy Configuration

In `Cargo.toml`:

```toml
[lints.clippy]
all = "warn"
pedantic = "warn"
```

---

## 2. Project Structure

See `architecture.md` for the specific directory structure.

**Principles:**
- One responsibility per module
- Default to private; use `pub` only when needed
- Use `pub(crate)` for internal-only sharing between modules

---

## 3. Windows FFI Safety

Since this project uses Win32 APIs for DDC/CI, hotkeys, and overlays:

### Isolate unsafe code

Keep `unsafe` at module boundaries with safe wrappers:

```rust
// Safe public API
pub fn register_hotkey(&mut self, id: i32, modifiers: u32, key: u32) -> Result<()> {
    // unsafe contained here, not exposed to callers
    unsafe {
        if RegisterHotKey(self.hwnd, id, modifiers, key) == 0 {
            return Err(Error::HotkeyRegistrationFailed(GetLastError()));
        }
    }
    Ok(())
}
```

### Wrap handles in RAII types

Windows handles should be wrapped in structs that implement `Drop`:

```rust
pub struct MonitorHandle {
    handle: HMONITOR,
}

impl Drop for MonitorHandle {
    fn drop(&mut self) {
        // Cleanup if needed
    }
}
```

### Separate FFI bindings

Put raw Win32 bindings in dedicated modules (e.g., `platform/windows/ffi.rs`).

---

## 4. Documentation

Document all public items with `///` doc comments. Include:
- Brief description
- `# Arguments` for non-obvious parameters
- `# Returns` for non-trivial return values  
- `# Errors` listing when `Result::Err` is returned

---

## 5. Error Handling

Use a centralized error module (`src/error.rs`) with a project-specific error enum and type alias:

```rust
pub type Result<T> = std::result::Result<T, BrightnessError>;
```

---

## Quick Checklist

- [ ] `cargo fmt` passes
- [ ] `cargo clippy` shows no warnings
- [ ] `cargo test` passes
- [ ] Public items have doc comments
- [ ] Unsafe code is isolated with safe wrappers
- [ ] Windows handles use RAII wrappers
