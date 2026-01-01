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

### Unsafe Blocks in Unsafe Functions (Rust 2024)

In Rust 2024, `unsafe fn` does not imply an `unsafe` block for its body. You must explicitly wrap unsafe operations in `unsafe { ... }` even inside an `unsafe fn`.

```rust
// Correct
unsafe fn my_unsafe_fn() {
    unsafe {
        ffi_call();
    }
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

### Windows Crate (v0.52+) Specifics

We use the `windows` crate which generates idiomatic Rust bindings. Follow these rules:

1.  **Result over BOOL**: Newer bindings return `windows::core::Result<()>` or `Result<T>` instead of `BOOL`. Use `?` operator (with `.map_err` to convert to `BrightnessError`) instead of checking `.as_bool()`.
2.  **Slices over Pointers**: APIs taking arrays now often accept slices (`&[T]`) instead of `pointer` + `length`.
3.  **Missing Debug**: Many generated FFI structs (like `PHYSICAL_MONITOR`) do **not** implement `Debug`.
    *   *Do not* derive `Debug` on structs containing them.
    *   Implement `std::fmt::Debug` manually.
    *   **Caution**: Packed structs require copying fields to local variables before referencing them in `debug_struct`.

---

## 4. Documentation

Document all public items with `///` doc comments. Include:
- Brief description
- `# Arguments` for non-obvious parameters
- `# Returns` for non-trivial return values
- `# Errors` listing when `Result::Err` is returned (Strictly enforced by `clippy::missing_errors_doc`)
- `# Panics` listing any potential panic conditions (Strictly enforced by `clippy::missing_panics_doc`)
- Use backticks around code elements (e.g., `Result`, `SomeType`, `SetupAPI`) to satisfy `clippy::doc_markdown`.

---

## 5. Coding Style & Linting

### Numeric Literals
Use underscores for readability in long numeric literals, especially hex colors/masks:
```rust
const COLOR: u32 = 0x00FF_FFFF;
```

### Casting
Minimize `as` casting.
- Use `u32::from(val)` for lossless conversions (`clippy::cast_lossless`).
- Use `try_from` for potential truncation.
- Use `.cast_unsigned()` or `.cast_signed()` for sign changes if intentional.

### Purity
Annotate pure functions (constructors, getters, calculations) with `#[must_use]` to ensure their results aren't accidentally discarded.

---

## 6. Error Handling

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
