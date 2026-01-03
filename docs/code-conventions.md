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

### FFI calls should be wrapped in safe functions as close to their point of use as possible

FFI calls should be wrapped in safe functions as close to their point of use as possible, typically within the feature module that requires them. This promotes locality of reasoning and keeps domain-specific logic together.

Shared or common FFI wrappers may be extracted into a common module (e.g., `platform/windows/mod.rs` or a dedicated `ffi.rs`) to avoid duplication and provide reusable utilities like RAII handle wrappers.

### Windows Crate (v0.52+) Specifics

We use the `windows` crate which generates idiomatic Rust bindings. Follow these rules:

1.  **Result over BOOL**: Newer bindings return `windows::core::Result<()>` or `Result<T>` instead of `BOOL`. Use `?` operator (with `.map_err` to convert to `BrightnessError`) instead of checking `.as_bool()`.
2.  **Slices over Pointers**: APIs taking arrays now often accept slices (`&[T]`) instead of `pointer` + `length`.
3.  **Missing Debug**: Many generated FFI structs (like `PHYSICAL_MONITOR`) do **not** implement `Debug`.
    *   *Do not* derive `Debug` on structs containing them.
    *   Implement `std::fmt::Debug` manually.
    *   **Caution**: Packed structs require copying fields to local variables before referencing them in `debug_struct`.
4.  **Raw Pointers**: Use `&raw const` and `&raw mut` instead of casting references (`&mut x as *mut _`).
    ```rust
    // Good
    Function(&raw mut my_struct);

    // Avoid (triggers clippy::borrow_as_ptr)
    Function(&mut my_struct as *mut _);
    ```

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
- Use `.cast_unsigned()` or `.cast_signed()` for sign changes (e.g., `HRESULT` to `u32`).
- For `f32` to integer, ensure the value is clamped/rounded before casting.

### Purity
Annotate pure functions (constructors, getters, calculations) with `#[must_use]` to ensure their results aren't accidentally discarded.
- **Exception**: Do not annotate functions returning `Result` (redundant).

---

## 6. Error Handling

Use a centralized error module (`src/error.rs`) with a project-specific error enum and type alias:

```rust
pub type Result<T> = std::result::Result<T, BrightnessError>;
```

---

## 7. Logging

See `architecture.md` section 8 for what events to log at each level. This section covers **how** to write log statements.

### Level Selection Guidelines

When choosing a log level, ask these questions:

| Question | If Yes → Level |
|----------|----------------|
| Has something broken that requires intervention? | `error!` |
| Is the application unable to continue this operation? | `error!` |
| Did something unexpected happen, but we recovered? | `warn!` |
| Is this a deprecated/discouraged code path? | `warn!` |
| Would an operator find this useful in production? | `info!` |
| Is this a significant state change or milestone? | `info!` |
| Is this helpful for debugging during development? | `debug!` |
| Is this ultra-detailed, stepping-through-code level? | `trace!` |

**Key distinction:** `error!` means "something is broken and needs fixing." `warn!` means "something is unusual but we handled it."

### Structured Logging

Prefer key-value pairs over string interpolation for machine-parseable logs:

```rust
// Preferred: structured fields
// Note: Strings/objects need `log::as_display!` or `log::as_debug!` as `String` does not implement ToValue
log::info!(monitor_id = log::as_display!(monitor.id()), brightness = new_value; "Brightness adjusted");
log::error!(vcp_code = 0x10, attempts = 3; "DDC write failed");

// Acceptable: simple messages without dynamic data
log::info!("Application started");

// Avoid: embedding structured data in format strings
log::info!("Brightness adjusted: monitor={}, value={}", monitor.id(), new_value);
```

### Field Naming

Use `snake_case` for all log field names. Common patterns:

| Field | Usage |
|-------|-------|
| `monitor_id` | Monitor identification |
| `brightness` | Brightness value (0-100) |
| `vcp_code` | DDC VCP code |
| `error_code` | Windows API error code |
| `attempts` | Retry attempt count |
| `duration_ms` | Operation timing |

### Never Log Sensitive Data

Never log personally identifiable information (PII) or secrets:
- User paths containing usernames (sanitize or omit)
- Serial numbers in production logs (use `debug!` level only)
- Any credentials or API keys

### Antipatterns

- **Don't log the same event at multiple levels** — pick one appropriate level
- **Don't log the same event in multiple places** — this often happens when both a function and its caller log the same error. Follow the "log at handling, not occurrence" rule:
  - Functions returning `Result` should *not* log errors — just return `Err(...)`
  - The *caller* who handles/recovers from the error should log it
  - Exception: Final error handlers (e.g., `main()`, message loop) should always log unhandled errors
- **Don't use `error!` for expected failures** — use `warn!` for recoverable situations
- **Don't use `debug!`/`trace!` in hot paths without need** — even disabled log macros have some overhead from argument evaluation

---

## Quick Checklist

- [ ] `cargo fmt` passes
- [ ] `cargo clippy` shows no warnings
- [ ] `cargo test` passes
- [ ] Public items have doc comments
- [ ] Unsafe code is isolated with safe wrappers
- [ ] Windows handles use RAII wrappers
- [ ] Log statements use appropriate levels (see section 7)
- [ ] No PII or secrets in log output
