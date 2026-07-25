# `windows` Crate 0.52 → 0.62 Upgrade Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Lift the `windows` crate pin from 0.52 to 0.62 in one jump, with zero user-visible behavior change, then remove the workarounds 0.62 obsoletes and refresh the docs that encode the old pin.

**Architecture:** One atomic port commit (the version bump changes all FFI signatures at once — a half-ported tree does not compile), split across three plan tasks that each verify by falling `cargo check` error count. Then cleanup commits (each individually green), a docs commit, and a manual hardware-test handoff. The spec is `docs/superpowers/specs/2026-07-25-windows-crate-upgrade-design.md` — read it if any decision here seems surprising.

**Tech Stack:** Rust 2024 (MSRV 1.88, unchanged), `windows` crate 0.62, Windows-only binary, no async runtime.

## Global Constraints

- Dependency line becomes exactly `windows = { version = "0.62", features = [ ...unchanged 16 features... ] }` — the feature list needs **zero** edits.
- `rust-version = "1.88"` stays (0.62 needs only 1.82).
- **No user-visible behavior change.** In particular: the two child-control `CreateWindowExW` sites in `usage.rs` must NOT become fatal (see Task 4).
- Gates for every commit: `cargo fmt -- --check`, `cargo +stable clippy --all-targets -- -D warnings` (CURRENT stable — CI uses latest stable, an older local toolchain misses new lints), `cargo test`.
- Prefix EVERY cargo invocation with `CARGO_PROFILE_DEV_DEBUG='0'` (bash) / `$env:CARGO_PROFILE_DEV_DEBUG='0'` (PowerShell) — the D: drive is nearly full.
- Commit messages: terse, ≤ ~50 words, NO `Co-Authored-By`/`Generated with Claude` trailer (user-global rule overrides any session default).
- Code comments: self-contained domain terms only — no plan/spec/finding IDs. GitHub issue refs like `microsoft/windows-rs#3093` are allowed alongside a plain-language reason.
- Error-code conversion pattern stays `e.code().0.cast_unsigned()` everywhere (`HRESULT(pub i32)` is unchanged in 0.62); do not "improve" it.
- Structured logging rules apply (`docs/code-conventions.md` §7): `key:? = value` for Debug-format fields.

---

### Task 1: Preflight — baseline green on 0.52

**Files:** none modified.

- [ ] **Step 1: Verify branch and clean tree**

Run: `git branch --show-current && git status --porcelain`
Expected: `windows-crate-upgrade`; no output from status (the two untracked `docs/architecture-review.md` / `docs/research-findings.md` files may appear — leave them untouched, never `git add` them).

- [ ] **Step 2: Verify 0.52 baseline is green**

Run: `CARGO_PROFILE_DEV_DEBUG='0' cargo test 2>&1 | tail -5`
Expected: all tests pass. If this fails, STOP — the baseline is broken and the port must not start.

- [ ] **Step 3: Record the baseline test count**

Note the number of passed tests from Step 2 output (e.g. "NN passed"). Task 4 must reach the same number.

---

### Task 2: Port — pin bump, seam helpers, Send impl, BOOL moves, log-kv fix

The tree will NOT compile at the end of this task — that is expected. No commit here; the port commit lands at the end of Task 4. Verification is by falling error count.

**Files:**
- Modify: `Cargo.toml:9-15` (pin + comment)
- Modify: `src/platform/windows/mod.rs` (helpers; seam sites at :74, :80)
- Modify: `src/platform/windows/ddc.rs:16` (BOOL import), `:83-115` (Send impl)
- Modify: `src/main.rs:6` (BOOL import), `:609`, `:729` (`SetConsoleCtrlHandler`)
- Modify: `src/platform/windows/power.rs:198`, `:234` (log-kv)

**Interfaces:**
- Produces (consumed by Tasks 3 and 4): four `pub(crate)` helpers in `src/platform/windows/mod.rs`:
  - `pub(crate) fn hmonitor_from_isize(value: isize) -> HMONITOR`
  - `pub(crate) fn hmonitor_to_isize(handle: HMONITOR) -> isize`
  - `pub(crate) fn hwnd_from_isize(value: isize) -> HWND`
  - `pub(crate) fn hwnd_to_isize(handle: HWND) -> isize`
- Produces: `unsafe impl Send for PhysicalMonitor` in `ddc.rs` (keeps `DdcWorker: Send`, so `ddc_worker.rs:262 thread::spawn` still compiles in Task 3).

- [ ] **Step 1: Bump the pin and rewrite the dependency comment**

In `Cargo.toml`, replace lines 10–15 (comment + version; keep the feature array untouched):

```toml
# Windows API. Tracks the current minor; patch releases may drift freely.
# Minor/major bumps change FFI signatures across src/platform/windows/ and
# remain deliberate standalone undertakings with a full manual hardware test
# pass (DDC, OSD, overlay, tray, hotkeys, power events) — never a side effect
# of another change.
windows = { version = "0.62", features = [
```

- [ ] **Step 2: Confirm the breakage baseline**

Run: `CARGO_PROFILE_DEV_DEBUG='0' cargo check 2>&1 | grep -c "^error"`
Expected: roughly 89 errors (exact count may vary a few either way with rustc version). If the number is wildly different (< 50 or > 150), stop and investigate the dependency resolution (`cargo tree -p windows`).

- [ ] **Step 3: Add the seam helper pair to `mod.rs`**

Insert after the `CursorLocator` impl (after line 82), new section:

```rust
// ─────────────────────────────────────────────────────────────────────────────
// Handle ↔ isize Seam
// ─────────────────────────────────────────────────────────────────────────────
// `core/` carries monitor/window handles as plain `isize` (`MonitorHandle`,
// `TrayStatusHandle`) to stay platform-free and `Send`. Win32 handles are
// pointers, so every crossing of that seam converts here — keeping the
// int↔pointer casts (and their lints) in one place.

/// Rebuilds an `HMONITOR` from the `isize` form that crosses the `core` seam.
#[must_use]
pub(crate) fn hmonitor_from_isize(value: isize) -> HMONITOR {
    HMONITOR(std::ptr::with_exposed_provenance_mut(value.cast_unsigned()))
}

/// Flattens an `HMONITOR` into the `isize` form that crosses the `core` seam.
#[must_use]
pub(crate) fn hmonitor_to_isize(handle: HMONITOR) -> isize {
    handle.0.expose_provenance().cast_signed()
}

/// Rebuilds an `HWND` from the `isize` form stored in `TrayStatusHandle`.
#[must_use]
pub(crate) fn hwnd_from_isize(value: isize) -> HWND {
    HWND(std::ptr::with_exposed_provenance_mut(value.cast_unsigned()))
}

/// Flattens an `HWND` into the `isize` form stored in `TrayStatusHandle`.
#[must_use]
pub(crate) fn hwnd_to_isize(handle: HWND) -> isize {
    handle.0.expose_provenance().cast_signed()
}
```

(`with_exposed_provenance_mut` / `expose_provenance` are stable since 1.84 — inside our MSRV — and are the provenance-correct way to round-trip a pointer through an integer.)

- [ ] **Step 4: Use the helpers at the two `mod.rs` seam sites**

`mod.rs:74` — in `CursorLocator::monitor_under_cursor`:

```rust
get_monitor_under_cursor().map(|h| crate::core::controller::MonitorHandle(hmonitor_to_isize(h)))
```

`mod.rs:80` — in `CursorLocator::resolve_id`:

```rust
ddc::get_monitor_id(hmonitor_from_isize(handle.0))
```

- [ ] **Step 5: `unsafe impl Send for PhysicalMonitor` in `ddc.rs`**

Insert directly after the `PhysicalMonitor` struct definition (after line 89):

```rust
// SAFETY: A DDC physical-monitor handle is a process-scoped object with no
// thread affinity — any thread in this process may use or destroy it, so
// transferring ownership across threads cannot invalidate it. `Sync` is
// deliberately NOT implemented: access stays single-threaded (the DDC worker
// owns all instances). Since windows-rs 0.58, handle types implement neither
// `Send` nor `Sync` by design and callers who know their threading model opt
// in by wrapping — see microsoft/windows-rs#3093 and microsoft/windows-rs#3169.
unsafe impl Send for PhysicalMonitor {}
```

Note: the existing `const _: assert_send::<DdcMonitor>()` at `ddc.rs:576-579` is the regression test for exactly this — it must compile again once this impl exists.

- [ ] **Step 6: Move the `BOOL` imports**

`ddc.rs:16`: change

```rust
use windows::Win32::Foundation::{BOOL, HANDLE, HWND, LPARAM, RECT};
```

to

```rust
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, RECT};
use windows::core::BOOL;
```

`main.rs:6`: change

```rust
use windows::Win32::Foundation::{BOOL, FALSE, TRUE};
```

to

```rust
use windows::core::BOOL;
```

(`TRUE`/`FALSE` still exist in `Foundation`, but after Step 7 nothing in `main.rs` uses them — if the `ctrl_handler` function body still returns `TRUE`/`FALSE`, replace those with `BOOL::from(true)` / `BOOL::from(false)` rather than re-importing.)

- [ ] **Step 7: `SetConsoleCtrlHandler` takes `bool` now**

`main.rs:609`: `SetConsoleCtrlHandler(Some(ctrl_handler), TRUE)` → `SetConsoleCtrlHandler(Some(ctrl_handler), true)`
`main.rs:729`: `SetConsoleCtrlHandler(Some(ctrl_handler), FALSE)` → `SetConsoleCtrlHandler(Some(ctrl_handler), false)`

- [ ] **Step 8: Fix the two handle log-kv sites in `power.rs`**

`power.rs:198`: `log::debug!(hwnd = hwnd.0; "Power event listener window created and registered");` → `log::debug!(hwnd:? = hwnd; "Power event listener window created and registered");`
`power.rs:234`: `log::debug!(hwnd = self.hwnd.0; "Power event listener window destroyed");` → `log::debug!(hwnd:? = self.hwnd; "Power event listener window destroyed");`

(Raw `*mut c_void` implements no `log::kv::ToValue`; `:?` uses the handle's `Debug`. Cosmetic consequence, accepted by the spec: debug logs show pointers now.)

- [ ] **Step 9: Verify the error count fell**

Run: `CARGO_PROFILE_DEV_DEBUG='0' cargo check 2>&1 | grep -c "^error"`
Expected: noticeably below the Step 2 count (the remaining errors are the mechanical sweep of Tasks 3–4). No commit yet.

---

### Task 3: Port — mechanical sweep A (`mod.rs`, `ddc.rs`, `ddc_worker.rs`, `hotkey.rs`, `power.rs`)

Work file by file; after each file, `cargo check` should no longer report errors IN that file (errors in Task 4's files remain). The compiler is the worklist — the line numbers below are anchors from the trial compile, not an exhaustive contract.

**Files:**
- Modify: `src/platform/windows/mod.rs` (:59, :167, :223, :263, :325)
- Modify: `src/platform/windows/ddc.rs` (:55, :421, :507, :552)
- Modify: `src/platform/windows/ddc_worker.rs` (:176)
- Modify: `src/platform/windows/hotkey.rs` (:162, :218, :280-298, :327, :343, :398, :443, :446)
- Modify: `src/platform/windows/power.rs` (:165-196, :216, :230)

**Interfaces:**
- Consumes: the four seam helpers and the `Send` impl from Task 2.
- Produces: `SafeHwnd::is_valid()` / `SafeHandle::is_valid()` keep their existing public signatures (`pub fn is_valid(&self) -> bool`) — only bodies change.

**The recurring patterns (referenced by name below):**

- **P1 — null-check on a handle:** `h.0 == 0` / `h.0 != 0` no longer compiles (pointer vs integer). Replace with `h.is_invalid()` / `!h.is_invalid()`. This is the minimal compiling fix AND the cleanup the spec wanted — the compiler forces it into the port commit; there is no separate cleanup commit for it.
- **P2 — optional handle parameter:** parameters that accepted a "null" handle (`HWND::default()`, `HDC::default()`, `HHOOK::default()`, `HMENU::default()`) become `Option<T>` — pass `None`. Parameters that pass a REAL handle where the signature is now `Option<T>` — wrap in `Some(…)`.
- **P3 — `CreateWindowExW` returns `Result<HWND>`:** replace the call + manual null-check pair with `.map_err(…)?`:

```rust
let hwnd = unsafe {
    CreateWindowExW(/* args, with P2 applied to parent/menu/instance */)
}
.map_err(|e| BrightnessError::windows_api("CreateWindowExW", e.code().0.cast_unsigned()))?;
```

and DELETE the old `if hwnd.0 == 0 { return Err(last_error_as_brightness_error("CreateWindowExW")); }` block. (Error codes shift from raw last-error to HRESULT-wrapped — the spec accepts this; the crate captures the error at the correct instant, which is strictly more reliable than our deferred read.)
- **P4 — `HMODULE` vs `Option<HINSTANCE>`:** `GetModuleHandleW(None)` returns `HMODULE`; window-creation `hinstance` params want `Option<HINSTANCE>` → pass `Some(hinstance.into())`.
- **P5 — GDI object params:** `SelectObject`/`DeleteObject` (and `FillRect`'s brush) want `HGDIOBJ`; convert typed handles (`HBITMAP`, `HBRUSH`, `HFONT`) with `.into()`.

- [ ] **Step 1: `mod.rs`**

- `:59` `if hmonitor.0 == 0` → `if hmonitor.is_invalid()` (P1)
- `:167` `SafeHwnd::is_valid` body → `!self.hwnd.is_invalid()` (P1; `HWND::is_invalid()` is a null check, semantically identical to today's `!= 0`)
- `:223` `SafeHandle::is_valid` body → `!self.handle.is_invalid()` (P1; `HANDLE::is_invalid()` checks null AND `-1`, identical to today's pair of checks — including the unary-minus `E0600` fix)
- `:263` `MessageBoxW(HWND::default(), …)` → `MessageBoxW(None, …)` (P2)
- `:325` test: `let hwnd = HWND(0);` → `let hwnd = HWND::default();`

- [ ] **Step 2: `ddc.rs`**

- `:55` `EnumDisplayMonitors(HDC::default(), …)` → `EnumDisplayMonitors(None, …)` (P2)
- `:421` `SetupDiGetClassDevsW(…, HWND::default(), …)` → `SetupDiGetClassDevsW(…, None, …)` (P2)
- `:507` `SetupDiOpenDevRegKey(…, DICS_FLAG_GLOBAL, …)` → the scope parameter is plain `u32` now → `DICS_FLAG_GLOBAL.0`
- `:543-560` `RegQueryValueExW` returns raw `WIN32_ERROR` (not `Result`) since 0.54. Replace the tail of `read_edid_from_registry`:

```rust
        let result = RegQueryValueExW(
            hkey,
            value_name,
            None,
            Some(&raw mut data_type),
            Some(buffer.as_mut_ptr()),
            Some(&raw mut data_len),
        );

        result.ok().map_err(|e| {
            BrightnessError::windows_api("RegQueryValueExW", e.code().0.cast_unsigned())
        })?;
        Ok(buffer)
```

(`WIN32_ERROR::ok()` converts to `windows::core::Result<()>` and keeps today's HRESULT-wrapped error codes — behavior-preserving.) The earlier size-probing call at `:526` (`let _ = RegQueryValueExW(…)`) still compiles as-is; leave it. `RegCloseKey` inside `SafeHKey`'s drop also returns `WIN32_ERROR` now; a discarded `let _ =` stays valid.

- [ ] **Step 3: `ddc_worker.rs`**

- `:176` `self.handle_cache.insert(hmonitor.0, …)` → `self.handle_cache.insert(hmonitor_to_isize(hmonitor), …)` — import via `use super::hmonitor_to_isize;`. The `handle_cache: HashMap<isize, MonitorId>` field type stays. Fix any other `.0`-key lookup sites on that map the compiler reports the same way.
- `:262` `thread::spawn(move || worker.run())` must now compile again thanks to Task 2's `Send` impl — if it does not, STOP: a handle-bearing type is escaping the worker thread; re-read the spec's structural decision 1 before touching anything.

- [ ] **Step 4: `hotkey.rs`**

- `:162`, `:218` `CallNextHookEx(HHOOK::default(), …)` → `CallNextHookEx(None, …)` (P2)
- `:280-298` `CreateWindowExW`: apply P3; inside the argument list apply P2 (`HWND_MESSAGE` parent → `Some(HWND_MESSAGE)`, `HMENU::default()` → `None`, `hinstance` → `Some(hinstance.into())` per P4)
- `:327` `RegisterHotKey(self.hwnd, …)` → `RegisterHotKey(Some(self.hwnd), …)` (P2)
- `:343`, `:443` `UnregisterHotKey(self.hwnd, …)` → `UnregisterHotKey(Some(self.hwnd), …)` (P2)
- `:398` `GetMessageW(&raw mut msg, HWND::default(), 0, 0)` → `GetMessageW(&raw mut msg, None, 0, 0)` (P2)
- `:446` `self.hwnd.0 != 0` → `!self.hwnd.is_invalid()` (P1)

- [ ] **Step 5: `power.rs`**

- `:165-183` `CreateWindowExW`: apply P3 (delete the `:181-183` manual check); P2/P4 in the args (`HWND_MESSAGE` → `Some(HWND_MESSAGE)`, `HMENU::default()` → `None`, `hinstance` → `Some(hinstance.into())`)
- `:186` `RegisterSuspendResumeNotification(HANDLE(hwnd.0), …)` — still compiles (`hwnd.0` is `*mut c_void`, `HANDLE` wraps the same) — leave as-is unless the compiler objects
- `:216` `GetMessageW(…, HWND::default(), …)` → `GetMessageW(…, None, …)` (P2)
- `:230` `self.hwnd.0 != 0` → `!self.hwnd.is_invalid()` (P1)

- [ ] **Step 6: Verify these five files are clean**

Run: `CARGO_PROFILE_DEV_DEBUG='0' cargo check 2>&1 | grep "^error" -A2 | grep -oE "src[/\\\\][a-z_/\\\\]+\.rs" | sort -u`
Expected: only `osd.rs`, `osd_render.rs`, `overlay.rs`, `tray.rs`, `usage.rs` (and possibly `main.rs` follow-ons) remain. No commit yet.

---

### Task 4: Port — mechanical sweep B (`osd.rs`, `osd_render.rs`, `overlay.rs`, `tray.rs`, `usage.rs`) + the port commit

Same pattern names (P1–P5) as Task 3. This task ends with the tree fully green and THE single port commit.

**Files:**
- Modify: `src/platform/windows/osd.rs` (:184, :197, :327-337, :386, :523, :554, :598, :620, :629, :655)
- Modify: `src/platform/windows/osd_render.rs` (:46, :66, :97, :106, :126-135, :160, :173)
- Modify: `src/platform/windows/overlay.rs` (:130-139, :168, :385)
- Modify: `src/platform/windows/tray.rs` (:240, :319, :342, :347, :362, :407, :625, :635, :851, :866-921, :961, :977)
- Modify: `src/platform/windows/usage.rs` (:190-199, :223, :245-271, :287)
- Possibly modify: `src/main.rs` (follow-on errors now that the lib compiles)

**Interfaces:**
- Consumes: `hmonitor_from_isize`, `hwnd_from_isize`, `hwnd_to_isize` from Task 2 (import via `use super::…` in the respective files).
- `TrayStatusHandle(isize)` and `MonitorHandle(isize)` public shapes stay EXACTLY as they are.

- [ ] **Step 1: `osd.rs`**

- `:184` `hdc.0 != 0` → `!hdc.is_invalid()` (P1)
- `:197`, `:386`, `:523`, `:554`, `:598`, `:620`, `:629` — hwnd-taking calls (`BeginPaint`/`EndPaint`/`InvalidateRect`/`SetTimer`/`KillTimer`/`ShowWindow`/`SetWindowPos`/`GetClientRect`): apply P2 as the compiler directs (real handle → `Some(hwnd)`, null sentinel → `None`)
- `:327-337` window creation: P4 for the instance (note the existing `.unwrap_or_default()` produces `HMODULE`; wrap as `Some(instance.into())`), P3 for `CreateWindowExW` + delete the manual check
- `:655` `HMONITOR(handle.0)` → `hmonitor_from_isize(handle.0)`

- [ ] **Step 2: `osd_render.rs`**

- `:46`, `:97`, `:106`, `:135`, `:173` `FillRect`/`SelectObject`/`DeleteObject`: apply P5 (`.into()` on the typed GDI handle)
- `:66` `CreateFontW`: the charset/precision/quality/pitch args are newtypes now — replace raw numeric constants with the typed ones from `windows::Win32::Graphics::Gdi` (`DEFAULT_CHARSET: FONT_CHARSET`, `OUT_DEFAULT_PRECIS: FONT_OUTPUT_PRECISION`, `CLIP_DEFAULT_PRECIS: FONT_CLIP_PRECISION`, quality constants like `ANTIALIASED_QUALITY: FONT_QUALITY`, and the pitch/family combination as `FONT_PITCH_AND_FAMILY`). Preserve the numeric values the code passes today — look them up, don't guess a "nicer" constant.
- `:126-131`, `:160` `CreateCompatibleDC(dc)` → `CreateCompatibleDC(Some(dc))` (P2); `mem_dc.0 == 0` / `bitmap.0 == 0` → `.is_invalid()` (P1)

- [ ] **Step 3: `overlay.rs`**

- `:130-139` P4 + P3 (delete the manual check at `:134`)
- `:168` `SetWindowPos` insert-after arg (`HWND_TOPMOST`-style sentinel): wrap per P2 (`Some(HWND_TOPMOST)`)
- `:385` `HMONITOR(handle.0)` → `hmonitor_from_isize(handle.0)`

- [ ] **Step 4: `tray.rs`**

- `:240`, `:851` instance params: P4
- `:319` `GetDC(HWND::default())` → `GetDC(None)`; `:342` `CreateCompatibleDC(dc)` → `CreateCompatibleDC(Some(dc))` (P2)
- `:347`, `:362`, `:407` `SelectObject`/`DeleteObject`: P5
- `:625` `TrackPopupMenu` reserved param → `None`
- `:635` `PostMessageW(hwnd, …)` → `PostMessageW(Some(hwnd), …)` (P2)
- `:866-901` `CreateWindowExW`: P3 + P2 in args; delete the manual check
- `:921` `GetMessageW(…, HWND::default(), …)` → `(…, None, …)` (P2)
- `:961` `status_handle()`: `TrayStatusHandle(self.hwnd.as_raw().0)` → `TrayStatusHandle(hwnd_to_isize(self.hwnd.as_raw()))`
- `:977` `notify()`: `PostMessageW(HWND(self.0), …)` → `PostMessageW(Some(hwnd_from_isize(self.0)), …)`

- [ ] **Step 5: `usage.rs` — the per-site `CreateWindowExW` decisions (behavior-preserving)**

Three sites, three DIFFERENT treatments — do not unify them:

1. Main window `:179-199`: hard failure today → P3 (`.map_err(…)?`), delete the `:194-196` check. Instance arg per P4.
2. Static text `:223`: result ignored today (`let _static_hwnd = …`) → keep ignoring: `let _ = unsafe { CreateWindowExW(/* P2/P4-adjusted args */) };` — MUST NOT become `?`; a missing text control degrades the window, it does not kill it.
3. OK button `:245-271`: soft-checked today (`if btn_hwnd.0 != 0 { SetFocus(btn_hwnd); }`) → keep soft:

```rust
let btn_hwnd = CreateWindowExW(
    /* args as today, with: */
    Some(hwnd),                       // parent
    Some(HMENU(std::ptr::with_exposed_provenance_mut(ID_OK_BTN.cast_unsigned()))), // control ID as HMENU
    Some(hinstance.into()),
    None,
);
// …
if let Ok(btn) = btn_hwnd {
    let _ = SetFocus(Some(btn));      // SetFocus: Option<HWND> param, Result return
}
```

(If `ID_OK_BTN` is currently a different integer type than `isize`, adapt the cast with `cast_unsigned()`/`usize::try_from` — do NOT change the constant itself, it is matched against `WM_COMMAND` word values elsewhere.)

- `:265` `ShowWindow(hwnd, SW_SHOW)` and `:287` `IsWindow(self.hwnd.as_raw())`: apply P2 as the compiler directs (`Some(…)`).

- [ ] **Step 6: Full green**

Run: `CARGO_PROFILE_DEV_DEBUG='0' cargo check 2>&1 | tail -3`
Expected: zero errors. If `main.rs` shows follow-on errors now that the lib compiles (probe measured its 2 as a lower bound), fix them with the same patterns.

- [ ] **Step 7: Tests, fmt, clippy**

Run, in order:
1. `CARGO_PROFILE_DEV_DEBUG='0' cargo test 2>&1 | tail -5` — Expected: same pass count as Task 1 Step 3, zero failures.
2. `cargo fmt` then `cargo fmt -- --check` — Expected: clean.
3. `CARGO_PROFILE_DEV_DEBUG='0' cargo +stable clippy --all-targets -- -D warnings 2>&1 | tail -5` — Expected: clean. If new pointer-cast lints fire, fix them WHERE THEY FIRE (expected stragglers per the spec: the `ShellExecuteW` result casts in `main.rs:81/88` — these silently became pointer→int casts; if clippy objects, rewrite as `result.0.expose_provenance()` comparisons with the same numeric semantics; and the `HMENU` control-ID construction in `usage.rs`). Do not blanket-`allow` anything without a reason string.

- [ ] **Step 8: THE port commit**

```bash
git add -u && git add Cargo.lock
git commit -m "build: upgrade windows crate 0.52 -> 0.62

Handles are pointers now (is_invalid instead of .0 checks, no Send on
PHYSICAL_MONITOR -> explicit Send for PhysicalMonitor), handle params take
Option, CreateWindowExW returns Result, BOOL moved to windows::core.
isize<->handle seam helpers centralize the core boundary casts."
```

Verify nothing untracked slipped in: `git status --porcelain` shows only the two pre-existing untracked docs files.

---

### Task 5: Cleanup — stale comment + dead code

**Files:**
- Modify: `src/platform/windows/single_instance.rs:66-72`
- Modify: `src/platform/windows/mod.rs:115-120`

- [ ] **Step 1: Rewrite the stale `GetLastError` comment**

`single_instance.rs:66-72` currently justifies avoiding the crate's `GetLastError` because it "HRESULT-wraps the code" — true in 0.52, false since 0.54 (it returns raw `WIN32_ERROR` now). The load-bearing part (read last-error IMMEDIATELY) stays. Replace the comment block above `let last_error = get_last_error_code();` with:

```rust
    // Read the thread-local last error as the very next operation, before any
    // allocation, logging, or further FFI can overwrite it. `MUTEX_NAME` is a
    // compile-time constant, so constructing the name argument allocated nothing
    // whose drop could clobber the error. `get_last_error_code()` reads it via
    // std (raw error code, comparable to `ERROR_ALREADY_EXISTS`).
```

- [ ] **Step 2: Delete dead `last_error_is_success`**

Remove `mod.rs:115-120` (`last_error_is_success` — zero callers, pre-existing dead code confirmed by two independent reviews). Run `grep -rn "last_error_is_success" src/ tests/` first; expected: only the definition.

- [ ] **Step 3: Gates + commit**

Run the three gates (Global Constraints). Then:

```bash
git add -u
git commit -m "chore: fix stale GetLastError comment; drop dead last_error_is_success"
```

---

### Task 6: Cleanup — `SafeHandle` → `windows::core::Owned<HANDLE>`

Both adversarial reviews confirmed this swap is 1:1: `Free for HANDLE` performs the same guarded `CloseHandle` as `SafeHandle::drop`, and `single_instance.rs` is `SafeHandle`'s only consumer.

**Files:**
- Modify: `src/platform/windows/single_instance.rs:32-37`, `:76-83`
- Modify: `src/platform/windows/mod.rs` (delete `SafeHandle` type + its two tests)

**Interfaces:**
- `SafeHwnd` is NOT touched — it wraps `DestroyWindow`, not `CloseHandle`, and has multiple consumers.
- Produces: `SingleInstance` now holds `windows::core::Owned<HANDLE>`; its public API is unchanged.

- [ ] **Step 1: Swap the field**

`single_instance.rs`:

```rust
use windows::core::Owned;
// (drop the `SafeHandle` import from the `use super::…` line; keep get_last_error_code)

pub struct SingleInstance {
    // Held solely to keep the mutex handle open for the process lifetime; the
    // wrapped `Owned<HANDLE>` closes it on drop.
    #[allow(dead_code)]
    handle: Owned<HANDLE>,
}
```

and in `acquire()`:

```rust
            let guard = SingleInstance {
                // SAFETY: `CreateMutexW` returned success, so `handle` is a valid
                // handle we own and must close.
                handle: unsafe { Owned::new(handle) },
            };
```

- [ ] **Step 2: Delete `SafeHandle`**

Remove the `SafeHandle` struct, its impl, and its `Drop` from `mod.rs` (the block formerly at `:192-247`), plus the two tests `test_safe_handle_null_is_invalid` and any `SafeHandle` mention in `test_into_raw_prevents_drop` (that test uses `SafeHwnd` — keep it). Run `grep -rn "SafeHandle" src/ tests/`; expected: no hits afterwards.

- [ ] **Step 3: Gates + commit**

Run the three gates. Then:

```bash
git add -u
git commit -m "refactor: replace SafeHandle with windows::core::Owned<HANDLE>"
```

If the swap produces ANY semantic friction (e.g. `Owned`'s `Deref` shape fights the `#[allow(dead_code)]` hold-only pattern), STOP, keep `SafeHandle`, and record why in a comment — the spec explicitly allows this exit.

---

### Task 7: Docs refresh

**Files:**
- Modify: `docs/improvement-ideas.md:103-115` (Maintenance entry)
- Modify: `docs/code-conventions.md:122-139` (§3 "Windows Crate (v0.52+) Specifics")
- Modify: `CLAUDE.md` (the "`windows` crate v0.52" conventions bullet)
- Modify: `CHANGELOG.md` (`[Unreleased]` section)
- Modify: `src/platform/windows/ddc.rs:111`, `:437` (two "0.52+"-phrased comments)
- Check (modify only if factually wrong): `docs/architecture.md`

- [ ] **Step 1: Rewrite the Maintenance entry in `improvement-ideas.md`**

Replace the whole `**`windows` crate upgrade (currently pinned at 0.52)**` block (lines 103-115) with:

```markdown
- **`windows` crate version tracking (upgraded 0.52 → 0.62 on 2026-07-25)**
  The crate tracks the current minor; patch releases drift freely. Future
  minor/major bumps are routine maintenance, but breaking releases still get
  their own branch and a full manual hardware test pass (DDC, OSD, overlay,
  tray, hotkeys, power events) — never a side effect of another change. No
  standing revisit trigger: re-evaluate whenever upstream ships a breaking
  release that touches our API surface.
```

- [ ] **Step 2: Refresh `code-conventions.md` §3**

Retitle the section `### Windows Crate (v0.62) Specifics` and update the rules: keep rules 1 (Result over BOOL), 2 (slices), 3 (missing Debug — still true for `PHYSICAL_MONITOR`), 4 (raw pointers); ADD:

```markdown
5.  **Handles are pointers** (since 0.58): validity checks use `handle.is_invalid()`,
    never field comparisons like `.0 == 0`. Handle types implement neither `Send`
    nor `Sync`; a type that must cross threads either carries the handle as
    `isize` (see the seam helpers in `platform/windows/mod.rs`) or documents an
    explicit `unsafe impl Send` invariant.
6.  **Optional handle parameters**: parameters that accept "no window/DC/hook"
    take `Option<T>` — pass `None`, not `T::default()`.
7.  **`BOOL` lives in `windows::core`** (since 0.60); `TRUE`/`FALSE` remain in
    `Win32::Foundation`. Parameters that were `Into<BOOL>` are plain `bool` now.
8.  **RAII**: prefer `windows::core::Owned<T>` for handles whose cleanup is the
    crate-provided `Free` impl (e.g. `CloseHandle`); keep hand-rolled wrappers
    only where cleanup differs (e.g. `SafeHwnd` → `DestroyWindow`).
```

- [ ] **Step 3: Refresh the `CLAUDE.md` bullet**

Replace the `**`windows` crate v0.52**: …` bullet under "Project conventions" with:

```markdown
- **`windows` crate v0.62**: prefer the `Result`-returning bindings with `?` (+ `.map_err` to `BrightnessError`) over `BOOL` checks; handles are pointers — use `is_invalid()`, never `.0 == 0`, and note handles are not `Send`/`Sync` (core seam carries them as `isize`); optional handle params take `None`; `BOOL` lives in `windows::core`; use slices not ptr+len; many FFI structs lack `Debug` (impl manually, copy packed fields to locals first); use `&raw const`/`&raw mut` not `&x as *mut _`.
```

- [ ] **Step 4: `CHANGELOG.md` entry**

Under `[Unreleased]` → `### Changed` (create the subsection if absent):

```markdown
- Upgraded the `windows` crate from 0.52 to 0.62. No intended behavior change;
  debug-log output now shows window/monitor handles as pointers instead of
  integers.
```

- [ ] **Step 5: Sweep the two stale-reading comments in `ddc.rs`**

`:111` `// DestroyPhysicalMonitors takes a slice in windows-rs 0.52+` → `// DestroyPhysicalMonitors takes a slice of PHYSICAL_MONITOR entries.`
`:437` `// SetupDiEnumDeviceInfo returns Result<()> in windows 0.52+` → `// SetupDiEnumDeviceInfo returns Result<()>; Err(…) doubles as the end-of-list signal.`

- [ ] **Step 6: Check `architecture.md` for version-bound claims**

Run: `grep -n "0\.52\|windows crate\|windows-rs" docs/architecture.md`
Expected: update any hit that states a 0.52-era fact as current; leave everything else untouched.

- [ ] **Step 7: Gates + commit**

Run the three gates (comment-only code edits still must pass). Then:

```bash
git add -u
git commit -m "docs: refresh windows-crate guidance to 0.62; retire upgrade trigger"
```

---

### Task 8: Manual hardware-test checklist + handoff

**Files:**
- Create: `docs/superpowers/plans/2026-07-25-windows-crate-upgrade-manual-tests.md`

- [ ] **Step 1: Write the checklist file**

```markdown
# Manual hardware test pass — windows crate 0.52 → 0.62

Run on real hardware after the port branch is complete. Debug build
(`cargo run`) unless stated; one pass of everything before merge.

## Core (per docs/architecture.md "Integration Testing")
- [ ] DDC set/get: brightness up/down via hotkeys on EVERY connected monitor
      (hotkeys target the monitor under the cursor)
- [ ] OSD appears on the correct monitor, updates during adjustment,
      fades out
- [ ] Overlay: dim to 0% → black overlay engages; raise → overlay releases
- [ ] Tray: icon present; menu opens; live monitor status shown in menu
- [ ] Hotkeys: primary + secondary pairs; opt-in low-level hook for
      dedicated brightness keys (enable in config, test, disable again)
- [ ] Resume from sleep: sleep → wake → brightness cache refreshes
      (watch log for refresh)
- [ ] Monitor hot-plug: unplug/replug a monitor → topology updates
- [ ] Single instance: second launch shows the already-running notice and
      exits; first instance unaffected

## Paths the port demonstrably touches (beyond the standard list)
- [ ] Usage/instructions window: open it (tray menu), text renders, OK
      button has focus (Enter dismisses), window closes
- [ ] Ctrl+C in the debug console: clean shutdown (handler deregistered,
      no hang, exit code 0)
- [ ] Tray degraded-status PUSH path: unplug DDC (or otherwise force a
      DDC-degraded state) WITHOUT opening the menu → tray icon switches to
      the warning state on its own
- [ ] Supervised restarts: kill the DDC worker (e.g. induce a panic or use
      a debugger) → worker respawns, sets work again; same for the hotkey
      thread if force-killable
- [ ] Tray → "Open config file" and "Open data directory": both open; then
      one forced failure (temporarily rename config.json) → error handled,
      logged, app stays alive (ShellExecuteW result casts changed
      representation in 0.62)

## Targeted behavior checks (no compiler coverage)
- [ ] Single-instance detection still keys off ERROR_ALREADY_EXISTS
      (second launch, watch debug log for the last-error read)
- [ ] OSD error state (make a DDC set fail, e.g. unplugged monitor):
      error code shown/logged is plausible (codes may differ in detail
      from 0.52 — crate-captured vs deferred read — but must be sane)
- [ ] Debug logs: handle fields now print as pointers (cosmetic; confirm
      no log line panics or prints garbage)
```

- [ ] **Step 2: Commit**

```bash
git add docs/superpowers/plans/2026-07-25-windows-crate-upgrade-manual-tests.md
git commit -m "docs: add manual hardware test checklist for windows 0.62 upgrade"
```

- [ ] **Step 3: Final gate run + smoke test handoff**

1. Run all three gates one final time on the branch tip.
2. Run `CARGO_PROFILE_DEV_DEBUG='0' cargo build` and confirm it produces a debug binary.
3. Report to the user: the branch is code-complete; ask them to run the SMOKE test now (launch, hotkey up/down, OSD appears) and the full checklist file before merge. Hardware verification cannot be automated — the task is NOT done until the user has run it; hand off explicitly.

---

## Self-review notes (already applied)

- Spec coverage: pin bump + comment (T2), all three structural decisions (T2), mechanical sweep with per-site `usage.rs` decisions (T3/T4), is_invalid folded into port because the compiler forces it (documented in P1), stale comment + dead fn (T5), `Owned<HANDLE>` with friction exit (T6), all six doc targets incl. CLAUDE.md/CHANGELOG/ddc.rs comments (T7), full manual checklist incl. round-2 additions (T8).
- Deliberately NOT in any task (spec "deliberately kept"): manual `Debug` for `PhysicalMonitor`, the 21 `cast_unsigned()` sites, `MonitorHandle`/`TrayStatusHandle` shapes, `get_last_error_code()`.
- Line numbers are anchors from the 0.62 trial compile of this exact tree; if a number has drifted a line or two, trust the pattern + compiler, not the number.
