# Rust Code Conventions
## Brightness Control Tool

This document contains **project-specific** conventions only. Standard Rust conventions (naming, formatting, error handling) are assumed known and enforced by tooling.

---

## 1. Tooling

**`.github/workflows/ci.yml` is the authority on what must pass.** The list was previously
restated here and in `CONTRIBUTING.md` in two versions that both disagreed with it (only
`CLAUDE.md`'s copy matched): the one here omitted `--all-targets`, so following it linted no
tests at all and could pass locally while CI failed. Run what CI runs:

```bash
cargo fmt --all -- --check
cargo clippy --all-targets --locked -- -D warnings
cargo test --locked
cargo check --release --locked   # the only config where the hidden-console cfg compiles
```

CI additionally checks all targets against the MSRV pinned in `Cargo.toml`, runs `cargo audit`
against the RustSec advisory database (also weekly on a schedule, so a quiet repo still hears
about new advisories), and — on release only — fails if `cargo about generate` finds a
dependency licence outside the accepted list.

Lint levels and formatter settings live in `Cargo.toml` (`[lints.clippy]`, currently `all` and
`pedantic` at `warn`, promoted to errors by CI's `-D warnings`) and `rustfmt.toml`. Both are
enforced artifacts; this document does not restate their contents, because a copy is a second
thing to keep true.

---

## 2. Project Structure

See `architecture.md` for the specific directory structure.

**Principles:**
- One responsibility per module
- Default to private; use `pub` only when needed
- Use `pub(crate)` for internal-only sharing between modules

### Naming a controller seam

The suffix says what kind of boundary it is, and the three in use are not interchangeable:

- **`…Sink`** — a one-way effect the controller fires and does not wait on: `OsdSink`,
  `OverlaySink`, `SettingsSink`.
- **`…Port`** — a two-way channel whose result comes back later as a `BrightnessMessage`
  rather than as a return value: `DdcPort`, `HotkeyPort`.
- **A plain domain noun** for a query or a store, where neither of the above fits:
  `MonitorLocator`, `ConfigStore`.

Name the Windows implementation for what it *is*, not for the trait it satisfies —
`OsdWindow`, `DdcSupervisor`, `CursorLocator`, `WindowsConfigStore`. Two implementations
(`HotkeyPortImpl`, `SettingsSinkImpl`) are named after their trait instead, and two spellings
of the `impl` header are in use, fully qualified (`impl crate::core::controller::OsdSink for
…`) and imported. Both splits are cosmetic and neither is worth a sweeping rename; prefer the
domain noun and the imported form in new code.

### `pub` means "the binary or `tests/` names it"

This crate is a library plus a separate binary crate that consumes it, so `pub`
has a precise meaning here: named by `src/main.rs`, `tests/`, or the `lib.rs` doc
example — **or required by the signature of something they do name.** The second
half is not a loophole. `MonitorId`, `SaveResult`, the seam traits and the
`Config` sub-structs are all `pub` without appearing in the binary by name,
because they sit in the types it uses; making them `pub(crate)` would only move
the error to `private_interfaces`. Nothing beyond those two cases earns `pub`.

This is load-bearing rather than tidiness. Every `pub` item in a `pub mod` counts
as externally reachable, so rustc reports it as used no matter what — an
over-wide surface silently switches off `dead_code` for the whole crate. Two
consequences to keep in mind when adding code:

- Prefer `pub(crate) mod` for a new module, and reach the binary through a
  re-export in `platform/windows/mod.rs` if it needs one. A re-exported *type*
  keeps all of its `pub` methods reachable, so demote the methods the binary does
  not call.
- Some clippy lints (`unnecessary_wraps` among them) skip `pub` functions because
  the signature is API. Narrowing a function to `pub(crate)` is what lets them
  fire.

**There is no lint for this. The check is to demote and rebuild.**

`unreachable_pub` cannot help: it only fires on `pub` items inside *private*
modules, and the modules here are `pub mod`, so it stays silent by construction —
the very same reason `dead_code` goes blind. A recipe built on it sat in this
document for a long time and could never have reported anything.

What does work is asking the compiler directly. Narrow the item to `pub(crate)`
and build; it answers both questions at once:

- **"private type in public interface"** or `E0603` — the item is required after
  all, by a signature the binary or `tests/` reaches. Put the `pub` back, and say
  in a doc comment *which* public type drags it out, so the next reader does not
  repeat the experiment.
- **Clean build** — the narrowing was right. Keep it, and note that `dead_code`
  and lints like `unnecessary_wraps` (which skip `pub` fns, treating the signature
  as API) now apply to that item for the first time.

Batch it: demote every candidate at once, build, and put back only what the
compiler names. Run `cargo test` as well as `cargo clippy --all-targets`, because
`--all-targets` does not compile doc examples and a doc example is a consumer like
any other.

A rough candidate list is worth generating before that, but only as a starting
point: grep the `pub` items declared under `src/` (excluding `main.rs` and
`#[cfg(test)]` blocks) and subtract every name that appears in `src/main.rs`,
`tests/` or `src/lib.rs`. It over-reports badly, because it cannot see the
"required by a signature" case above — the last run of it flagged 28 items, of
which 19 were legitimate and 9 were genuinely too wide. Every type it flagged was
legitimate; every real hit was a function or a constant. Treat it as a list of
things to test, never as a verdict.

One more reason not to trust a green build here: widening visibility is silent.
Narrowing something that is needed fails loudly, but a `pub` that should have been
`pub(crate)` compiles, passes CI, and simply switches the checks off for that item.
Nothing will ever tell you. That asymmetry is why this is a periodic audit rather
than a gate.

---

## 3. Windows FFI Safety

Since this project uses Win32 APIs for DDC/CI, hotkeys, and overlays:

### Isolate unsafe code

Keep `unsafe` at module boundaries with safe wrappers. Copied from
`platform/windows/hotkey.rs`, because the examples in this document are the part most
likely to be imitated and so have to be real code rather than something plausible:

```rust
pub fn register_hotkey(
    &mut self,
    id: i32,
    modifiers: HOT_KEY_MODIFIERS,
    vk: VIRTUAL_KEY,
) -> Result<()> {
    unsafe {
        RegisterHotKey(Some(self.hwnd), id, modifiers, u32::from(vk.0)).map_err(|e| {
            BrightnessError::windows_api("RegisterHotKey", e.code().0.cast_unsigned())
        })?;
    }
    log::debug!(hotkey_id = id; "Registered hotkey");
    self.registered_ids.push(id);
    Ok(())
}
```

Four rules from further down this section at once: the `Result`-returning binding with `?`
rather than a `BOOL` check, `Some(hwnd)` for an optional handle parameter, `u32::from` and
`.cast_unsigned()` instead of `as`, and the error mapped into `BrightnessError` at the
boundary so callers never see a `windows` crate type.

### Write a `// SAFETY:` comment where the invariant is not local

Not every `unsafe` block needs one, and pretending otherwise is a large part of why so
few have one: there are ~290 `unsafe` blocks against 28 `// SAFETY:` comments, and which
ones are annotated tracks the file rather than the difficulty. Turning on
`clippy::undocumented_unsafe_blocks` would demand roughly 260 new comments, most of them
restating that a Win32 call was handed two integers — noise that teaches a reviewer to
skim past the handful that carry a real argument.

The line worth drawing is whether the block's correctness depends on something the block
itself does not show. Those get a `// SAFETY:` immediately above, naming the invariant
*and* what guarantees it:

- the provenance or lifetime of a raw pointer — the `lparam` round-trip through
  `EnumDisplayMonitors` in `ddc.rs` is the canonical case;
- a buffer whose length travels as a separate argument;
- a handle whose validity or thread affinity the compiler cannot see;
- a `transmute`, or a function pointer resolved at run time — the undocumented ordinals
  in `theme.rs`;
- the `hwnd`/`wParam`/`lParam` contract of a window procedure or other callback;
- an `unsafe impl` — `unsafe impl Send for PhysicalMonitor` in `ddc.rs` is the one
  load-bearing invariant in the crate, and carries the longest comment for that reason.

A block that only wraps a Win32 call taking scalars, string literals, or a handle an RAII
wrapper has already validated needs no comment; adding one there is what dilutes the
marker until nobody reads it.

`// SAFETY:` is reserved for `unsafe`. A cast that needs a range argument, or a lint
suppression that needs a reason, gets an ordinary comment — the marker is how a reviewer
finds the memory-safety claims in a 290-block codebase, and spending it on
`msg.wParam.0 as i32` empties it.

The two mechanical halves of this are gates, not prose: an exported `unsafe fn` must
carry a `# Safety` doc section (`clippy::missing_safety_doc`, already implied by
`clippy::all`), and an unsafe operation inside an `unsafe fn` body still needs its own
`unsafe { }` block (`unsafe_op_in_unsafe_fn`, denied in `Cargo.toml`).

### Wrap handles in RAII types

A handle that outlives a single expression — one that is stored in a struct, crosses a scope,
or crosses a thread — gets a wrapper whose `Drop` releases it, as `SafeHwnd` does in
`platform/windows/mod.rs`:

```rust
impl Drop for SafeHwnd {
    fn drop(&mut self) {
        if self.is_valid() {
            // SAFETY: We own this handle and it's valid.
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }
}
```

The rule is about *ownership*, not about every handle-typed value:

- A GDI object created and freed inside one painting block (the brushes and pens throughout
  `settings/dark.rs`) does not need a wrapper, and wrapping it would obscure the paint code.
  What the rule is really guarding against is an early `return` between the create and the
  free.
- A handle that needs no release at all — `HMONITOR`, for instance — gets no wrapper. A
  `Drop` with an empty body is worse than none: it claims an ownership that does not exist.
- Not every handle-shaped type is a handle. `core::controller::MonitorHandle` is deliberately
  a plain `isize` and deliberately has no `Drop`: it exists so the platform-agnostic core can
  carry a monitor's identity across a thread boundary, which a real handle type could not do.

### Allocating a `WM_APP` message

A custom window message is `WM_APP + n`, named `WM_<MODULE>_<PURPOSE>`, and defined in the
module that owns the window class receiving it — next to that class, not in a shared
constants file. `WM_APP + n` only has to be unique among the messages one class handles, so
modules do not coordinate their numbering; the ranges currently in use happen to be disjoint,
and nothing depends on that.

Two things do need care, and both are about delivery rather than the number:

- **Keep a module's values contiguous** if anything drains or filters them as a range. The
  settings messages are drained by a `PeekMessageW` range filter to reclaim heap payloads,
  and a compile-time assertion beside the constants enforces the span — extend the assertion
  deliberately when adding one, never work around it.
- **A message delivered by `PostThreadMessageW` is not a window message.** It arrives with a
  null `hwnd` and no window proc will ever see it, so it must be handled in the message loop
  body behind an explicit `msg.hwnd.is_invalid()` test, and its value must be unique across
  *every* window class hosted on that thread. List those classes in a comment next to the
  constant. There is exactly one such message today (`WM_APP_HOTKEY_WAKE`); see the
  Threading Model section of `docs/architecture.md` for the full allocation table and why the
  guard runs in the direction it does.

### FFI calls should be wrapped in safe functions as close to their point of use as possible

FFI calls should be wrapped in safe functions as close to their point of use as possible, typically within the feature module that requires them. This promotes locality of reasoning and keeps domain-specific logic together.

Shared or common FFI wrappers may be extracted into a common module (e.g., `platform/windows/mod.rs` or a dedicated `ffi.rs`) to avoid duplication and provide reusable utilities like RAII handle wrappers.

### Windows Crate Specifics

The `windows` crate generates idiomatic Rust bindings; the version in use is whatever
`Cargo.toml` pins, and its bump policy is a Maintenance Decision in `docs/architecture.md`.
These rules describe how the bindings behave, not one release of them:

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
5.  **Handles are pointers**: validity checks use `handle.is_invalid()`,
    never field comparisons like `.0 == 0`. Handle types implement neither `Send`
    nor `Sync`; a type that must cross threads either carries the handle as
    `isize` (see the seam helpers in `platform/windows/mod.rs`) or documents an
    explicit `unsafe impl Send` invariant.
6.  **Optional handle parameters**: parameters that accept "no window/DC/hook"
    take `Option<T>` — pass `None`, not `T::default()`.
7.  **`BOOL` lives in `windows::core`**; `TRUE`/`FALSE` remain in
    `Win32::Foundation`. Parameters that were `Into<BOOL>` are plain `bool` now.
8.  **RAII**: for new code, prefer `windows::core::Owned<T>` for handles whose
    cleanup is the crate-provided `Free` impl (e.g. `CloseHandle`) over writing
    a hand-rolled wrapper. `SafeHwnd` stays hand-rolled because its cleanup
    (`DestroyWindow`) doesn't fit the `Free` pattern — not every wrapper in the
    codebase is `Owned`-eligible yet, so absence of `Owned` isn't itself a
    signal something's wrong; see "Maintenance Decisions" in
    `docs/architecture.md` for known pre-`Owned` wrappers slated for
    opportunistic migration.

---

## 4. Documentation

Document all items with `///` doc comments, not only public ones — the de-facto practice
throughout the codebase, and rustc's `missing_docs` is *not* enabled, so this half is
convention rather than gate. Exempt: trait-impl boilerplate, serde `default_*` helpers, and
tests. Include:
- Brief description
- `# Arguments` for non-obvious parameters
- `# Returns` for non-trivial return values
- `# Errors` listing when `Result::Err` is returned (Strictly enforced by `clippy::missing_errors_doc`)
- `# Panics` listing any potential panic conditions (Strictly enforced by `clippy::missing_panics_doc`)
- Use backticks around code elements (e.g., `Result`, `SomeType`, `SetupAPI`) to satisfy `clippy::doc_markdown`.

---

## 5. Coding Style & Linting

Underscored numeric literals (`clippy::unreadable_literal`), `u32::from` over `as`
(`cast_lossless`), `try_from` where a value can truncate (`cast_possible_truncation`), and
`#[must_use]` on pure functions (`must_use_candidate`) are all pedantic lints that CI already
fails the build on. They are not restated here; the compiler is a better teacher than a list,
and a list is a second thing to keep true. (One clarification the lint does not offer: a
function returning `Result` gets no `#[must_use]` — `Result` already carries it.)

Two rules remain, because no lint expresses them:

**Clamp and round before casting a float to an integer.** `clippy::cast_possible_truncation`
can only be silenced at such a site, never satisfied, so the correctness argument has to be
made by the code: bound the value, `.round()` it, and only then cast — see the OSD's opacity
handling in `osd.rs` and the overlay's in `overlay.rs`.

**A lint suppression is narrow, is an `#[expect]`, and carries its reason.** Attach it to the
smallest item that needs it, never a module or the crate root, and say why the lint is wrong
*here* — "the truncation is bounded by a checked range", not "clippy complains". The reason
goes inline as `reason = "…"` when it fits in a phrase, and in a comment directly above when
it needs a sentence, which is the common case here.

`#[allow]` is not used: with `-D warnings` in CI, an `#[expect]` fails the build once its
lint stops firing, which is the only mechanism by which a suppression ever gets removed
again. An `#[allow]` whose reason has evaporated is invisible forever. Note that an
expectation is evaluated per build configuration — a lint that fires only under
`--all-targets` will report an unfulfilled expectation elsewhere, so check a suppression
against the full command set rather than one invocation.

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
// For types implementing Display, use `key:% = value` syntax
// For types implementing Debug, use `key:? = value` syntax
log::info!(monitor_id:% = monitor.id(), brightness = new_value; "Brightness adjusted");
log::error!(vcp_code = 0x10, attempts = 3; "DDC write failed");
log::debug!(message:? = msg; "Received message");

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

Enforced structurally where possible:
- `MonitorId`'s `Display` is serial-free by design, so `monitor_id:% = id` is
  safe at any level. The serial-bearing form is only available via the
  explicitly named `MonitorId::full_identity()` — call it only in `debug!`
  statements.
- Config errors (`ConfigRead`/`ConfigParse`/`ConfigWrite`/`ConfigFileOpen`)
  embed only the file name, not the absolute path, because they surface in
  warn/error logs. Log full paths at `debug!` only.

The debug-only rule is what makes the opt-in rolling log file safe at its
default `info` level: serials and paths stay out of the file unless the user
deliberately sets `logging.file_level` to `debug` for a diagnostic session.

### Antipatterns

- **Don't log the same event at multiple levels** — pick one appropriate level
- **Don't log the same event in multiple places** — this often happens when both a function and its caller log the same error. Follow the "log at handling, not occurrence" rule:
  - Functions returning `Result` should *not* log errors — just return `Err(...)`
  - The *caller* who handles/recovers from the error should log it
  - Exception: Final error handlers (e.g., `main()`, message loop) should always log unhandled errors
- **Don't use `error!` for expected failures** — use `warn!` for recoverable situations
- **Don't use `debug!`/`trace!` in hot paths without need** — even disabled log macros have some overhead from argument evaluation

---

## 8. Testing

`docs/architecture.md` §Testing says *what* is covered and where the unit/manual line falls.
This section is the mechanics: how a test in this codebase is built, so the next one looks
like the last 370.

### Fakes, not mocks

The controller is generic over seven seams, and each has a hand-written `Fake…` in the test
module of `core/controller.rs`. There are no mocking crates and, deliberately, **no
`[dev-dependencies]` at all**. That is not frugality — it is what keeps `core/` testable on
any host: the suite compiles and runs without a Windows target or a physical monitor, and
a mocking framework with a proc-macro or a platform-specific backend would quietly end that.

A fake is a plain struct that records what it was told and answers what it was configured to
answer:

```rust
#[derive(Default)]
struct FakeOverlay {
    updates: Vec<(MonitorId, u8)>,
    removed: Vec<MonitorId>,
    fail_update: bool,
}
```

- **Record into plain fields.** `Vec<…>` for calls worth inspecting in order, a counter for
  calls worth only counting. No `Arc<Mutex<…>>`, no `RefCell`, no interior mutability: the
  controller owns its seams exclusively and every method that records takes `&mut self`, so
  shared mutability would be modelling a concurrency that does not exist here — and would
  hide the single-owner property the whole design rests on. The seam methods that take
  `&self` are the pure queries (`is_visible`, `is_alive`, `monitor_under_cursor`), and a fake
  that only answers them needs no fields at all — `FakeLocator` is two configured values.
- **Inject failure with a flag**, named for what it breaks (`fail_send`, `fail_update`). When
  only the next call should fail, use the one-shot form: a `fail_next` field consumed through
  `std::mem::take`, as `FakeHotkeyPort` does.
- **Assert against the recorder**, never against log output. Logging is a side effect the
  tests do not own.

Shared setup lives beside the fakes as ordinary functions — `test_controller(base)`,
`test_id()`, `seed()`, `deliver_refresh()` — taking an explicit `Instant` rather than reading
the clock, which is why the watchdog and refresh timing tests are deterministic.

### On the Windows side, test the pure helpers

Thirteen of the `platform/windows/` files have tests, and with two exceptions they test only
pure helpers — parsing, formatting, layout arithmetic — never a live window or a real device.
The two exceptions are deliberate and are the whole list:

- `power.rs` calls `power_wnd_proc` directly to prove resume events reach the main thread.
- `ddc.rs` enumerates real monitors, written to pass on a headless machine by asserting only
  that the call succeeds.

Anything else that needs a window, a device, or a theme belongs in the manual checklists in
`docs/architecture.md`, not in a `#[test]`.

Filesystem tests build their own path with a helper that mixes in `process::id()` (see
`unique_path` in `config_store.rs`), because `cargo test` runs them in parallel within one
process and a fixed temp filename makes two tests fight over one file.

### Naming

Descriptive names in the present tense, stating the behaviour and its outcome:
`refresh_send_failure_aborts`, `periodic_refresh_gates_on_enumerated_not_readable`. Around
80 older tests still carry a `test_` prefix; rename one opportunistically when you are
already editing it, but **do not** do a sweeping rename — the churn costs more than the
inconsistency, and the newest modules are already prefix-free throughout.

---

## Quick Checklist

The four commands in section 1 pass — they are the mechanical half and the only half worth
a checklist item, since CI runs exactly them.

What the tooling cannot check, and a reviewer therefore has to:

- [ ] Every item is documented, private ones included — only trait-impl boilerplate,
      serde `default_*` helpers and tests are exempt
- [ ] `unsafe` is isolated behind safe wrappers, and a handle that outlives its expression
      has a `Drop` (see section 3 for what the rule does *not* cover)
- [ ] Any `unsafe` block whose correctness rests on something outside it — pointer
      provenance, a separately passed length, a handle's validity or thread affinity, a
      callback contract — has a `// SAFETY:` naming the invariant and its guarantor
- [ ] Log statements sit at the point of handling, at the level section 7 describes
- [ ] No PII: no serials or absolute paths above `debug!`
- [ ] Any new lint suppression is narrow, is an `#[expect]`, and says why
