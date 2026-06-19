# Design: Split GDI drawing layer out of `osd.rs`

**Date:** 2026-06-19
**Status:** Approved (pending written-spec review)
**Scope:** Single refactoring — no behavior change.

## Motivation

`src/platform/windows/osd.rs` is 1,088 lines and mixes four concerns:

1. **DPI / metrics scaling** — `OsdMetrics`, `for_dpi`, `BASE_*` constants, `get_monitor_dpi`, `update_osd_metrics`.
2. **Thread-local render state** — `OSD_STATE` / `OSD_METRICS` thread-locals, `OsdRenderState`, `update_osd_state`, `with_metrics`.
3. **GDI drawing primitives** (`osd.rs:235-666`, ~400 lines) — `paint_osd`, `draw_brightness_bars`, `draw_hardware_section`, `draw_overlay_section`, `draw_icon`, `draw_single_bar`, `draw_percentage_text`, `draw_error_message`, `create_icon_font`, `create_osd_font`.
4. **`OsdWindow` lifecycle / public API** — class registration, window creation, positioning, the `OsdWindow` struct with `show`/`hide`/`update`/`show_error`.

This file is the top structural hotspot in the codebase and #2 by git churn (52 touches). High-churn × high-complexity is where decay concentrates. Concern #3 — the drawing layer — is cleanly separable: it depends only on an `HDC`, the render state, and the metrics, and never calls window-lifecycle functions. Isolating it also contains the hand-balanced GDI cleanup (the manual `CreateSolidBrush`/`DeleteObject` and `SelectObject`/restore pairing, previously flagged as "Cosmetic #2").

## Goal

Extract concern #3 into a new sibling module `osd_render.rs`, and while moving it, replace the manual GDI resource bookkeeping with safe wrappers (RAII where a handle is held; a helper function where it is not). Concerns #1, #2, #4 stay in `osd.rs`.

Out of scope (deferred, see "Future Work"): bundling the repeated `with_metrics` tuples / long draw signatures into a layout struct, and lifting layout geometry into `core/`.

## File layout

Follows the existing flat convention under `platform/windows/` — specifically the `ddc.rs` / `ddc_worker.rs` precedent (one concern split into a sibling file with a shared prefix, not a subdirectory).

```
platform/windows/
  osd.rs          # Lifecycle + Window + Metrics + thread-local State (concerns #1, #2, #4)
  osd_render.rs   # GDI drawing primitives + RAII wrappers (concern #3)  ← NEW
  ...
```

Declared in `platform/windows/mod.rs` as `mod osd_render;` (crate-private — only `osd.rs` draws; nothing else needs the primitives).

## Interface between `osd.rs` and `osd_render.rs`

**Pure functions, explicit parameters.** `osd_render` has *zero* knowledge of thread-locals. `osd.rs` snapshots the thread-local state once and passes plain data in. This mirrors the codebase's "message-passing with single ownership / explicit data" principle (`architecture.md` §State Management) and keeps concern #2 (render-state plumbing) encapsulated in `osd.rs`.

The thread-local `OSD_STATE` is *not* removed — it remains necessary because `wnd_proc` is a system callback that cannot carry context. It simply stays where it belongs (the window/callback concern in `osd.rs`); only its snapshot crosses the boundary.

```rust
// osd.rs — wnd_proc, on WM_PAINT:
let state   = OSD_STATE.with(|s| s.borrow().clone());
let metrics = OSD_METRICS.with(|m| *m.borrow());
unsafe { osd_render::paint(hdc, &client_rect, &state, &metrics) };

// osd_render.rs — single entry point visible to osd.rs:
pub(super) unsafe fn paint(hdc: HDC, client_rect: &RECT, state: &OsdRenderState, metrics: &OsdMetrics);
```

`paint` is `pub(super)`, not `pub`: the module is private, so a `pub` item exposing the `pub(super)` `OsdRenderState` would trigger `private_interfaces` (deny) and `unreachable_pub` (pedantic). It is `unsafe fn` because it dereferences a caller-supplied raw `HDC` — this preserves the safety contract the original `unsafe fn paint_osd` carried at the module boundary. The private `draw_*` helpers, by contrast, become safe `fn` (they are only ever called with the controlled back-buffer DC).

`GetClientRect` is a window query, not drawing, so it moves to the caller (`wnd_proc` / `paint_osd` in `osd.rs`), which passes the resulting `client_rect` in. `osd_render::paint` does no window-handle queries — it only draws into the DC it is given. The early-return-on-empty-rect guard (`GetClientRect` failure) therefore lives in `osd.rs`.

### Type ownership

`OsdMetrics` (already `pub`) and `OsdRenderState` stay defined in `osd.rs`. `OsdMetrics` is genuinely shared — `osd.rs` positioning logic also reads `width`/`height`/`bottom_margin`, so it cannot move into the render module. `osd_render` imports both via `use super::osd::{OsdMetrics, OsdRenderState}` (both are children of `platform::windows`, so the sibling is reached through `super::osd::`, not `super::`). `OsdRenderState` is made visible to the render module (`pub(super)` or module-level `pub(crate)` as needed); it does not become part of the crate's public API.

## RAII / cleanup design

Tool chosen per resource lifetime, per `code-conventions.md` §3 ("Wrap handles in RAII types"; "FFI calls wrapped … within the feature module that requires them"). All three live **private to `osd_render.rs`**.

### 1. Brush → `fill_rect` helper (no RAII)

A solid-color fill brush never outlives a single `FillRect`. Wrapping it in a Drop type is over-engineering; a free helper that creates → fills → deletes removes all ~6 manual brush dances:

```rust
/// Fills `rect` with a solid `color` (BGR). The brush never escapes this call.
fn fill_rect(hdc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(color));
        FillRect(hdc, &raw const *rect, brush);
        let _ = DeleteObject(brush);
    }
}
```

### 2. Font → `SelectedFont` guard (RAII)

A font handle *is* held across the draw (selected into the DC while text is rendered). Drop restores the previous object and deletes the font:

```rust
struct SelectedFont {
    hdc: HDC,
    font: HFONT,
    old: HGDIOBJ,
}

impl SelectedFont {
    fn new(hdc: HDC, font_size: i32, face: FontFace) -> Self { /* CreateFontW + SelectObject(old) */ }
}

impl Drop for SelectedFont {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);     // restore
            let _ = DeleteObject(self.font);      // delete
        }
    }
}
```

`FontFace` is a small enum (`Text` → "Segoe UI", `Emoji` → "Segoe UI Emoji") replacing the two near-identical `create_*_font` functions. `SetBkMode`/`SetTextColor`/`SetTextAlign` stay at the call sites (they are draw state, not resource ownership).

### 3. Memory DC + bitmap → `BackBuffer` guard (RAII)

Bundles the double-buffering in `paint`. Holds the memory DC, the compatible bitmap, and the previously selected bitmap; Drop restores and deletes both, in the correct order:

```rust
struct BackBuffer {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl BackBuffer {
    /// Returns `None` if any GDI allocation fails (matching today's early-return behavior).
    fn new(target: HDC, width: i32, height: i32) -> Option<Self> { ... }
    fn dc(&self) -> HDC { self.mem_dc }
    fn blit_to(&self, target: HDC) { /* BitBlt SRCCOPY */ }
}

impl Drop for BackBuffer {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.mem_dc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap);
            let _ = DeleteDC(self.mem_dc);
        }
    }
}
```

## Behavior preservation

This is a pure refactor. The following must be **identical** before/after:

- Pixel output for all states (compact, error, every brightness/overlay combination).
- Early-return-on-failure semantics: today `paint_osd` bails silently if `CreateCompatibleDC`/`CreateCompatibleBitmap` fail. `BackBuffer::new` returns `None`; `paint` returns early on `None`.
- Resource cleanup ordering (restore selected object before deleting it; delete bitmap before DC).
- All log lines (`trace!` in paint/draw functions) preserved with the same key-value fields.

## Testing

Drawing is GDI FFI and is **not** unit-testable cross-platform (consistent with the existing manual-integration approach for hardware-dependent behavior in `architecture.md` §Integration Testing). Verification:

- `cargo fmt -- --check`, `cargo clippy -- -D warnings`, and `cargo build` must pass (on a non-Windows host, target `x86_64-pc-windows-msvc`).
- Manual smoke test on Windows: trigger brightness up/down (hardware bar), overlay dimming (purple bar), and a DDC error (red bar + expanded error row); confirm the OSD renders identically and auto-hides.
- The only `unsafe fn` crossing the module boundary is `paint` (mirroring the original `unsafe fn paint_osd`); all private `draw_*` helpers are safe `fn`. `osd_render`'s only item visible to `osd.rs` is `pub(super) unsafe fn paint`.

## Future Work (not in this change)

- **Layout geometry → `core/`.** The pure arithmetic (bar widths, fill widths, centering) is platform-agnostic and, if a Linux backend is ever added, would be the genuinely shareable part. Lifting it into `core/` would also make it unit-testable. Deferred because it is a larger, separate change and not required to resolve this finding.
- **Shared `gdi.rs`.** If `overlay.rs` (which also does GDI) later wants the same wrappers, extract them into a shared module per `code-conventions.md` §3. Not done speculatively (YAGNI).

## Alignment with project docs

- `architecture.md` §State Management — explicit-data / single-ownership flow → pure-parameter interface (not thread-local reach-in).
- `code-conventions.md` §3 — "Isolate unsafe code", "Wrap handles in RAII types", "FFI wrapped within the feature module that requires them"; checklist items "Unsafe code is isolated with safe wrappers" and "Windows handles use RAII wrappers" are both satisfied.
- `CLAUDE.md` — "keep `unsafe`/FFI in `platform/windows/`", "Wrap Windows handles in RAII (Drop) types as close to point of use as possible".
