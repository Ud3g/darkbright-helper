# OSD GDI Drawing-Layer Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extract the ~430-line GDI drawing layer out of `src/platform/windows/osd.rs` into a new sibling module `osd_render.rs`, replacing the hand-balanced GDI cleanup with safe wrappers, with zero behavior change.

**Architecture:** A pure-parameter drawing module. `osd.rs` keeps the window lifecycle, DPI metrics, and thread-local render state; it snapshots that state and calls one public entry point, `osd_render::paint(hdc, &client_rect, &state, &metrics)`. The drawing module knows nothing about thread-locals or windows — it only draws into a device context it is handed. GDI resource cleanup is moved from manual `DeleteObject`/`SelectObject` pairs into three private wrappers: a `fill_rect` helper (brushes), a `SelectedFont` RAII guard (fonts), and a `BackBuffer` RAII guard (double-buffering).

**Tech Stack:** Rust 2024, `windows` crate v0.52 (Win32 GDI), no new dependencies.

## Global Constraints

- **Platform:** Windows-only code; all FFI behind `src/platform/windows/`. This dev host is native Windows (`win32`), so `cargo build`/`clippy` run directly — no `--target` needed.
- **MSRV:** 1.87+.
- **Must pass before every commit:** `cargo fmt -- --check`, `cargo clippy -- -D warnings` (clippy `all` + `pedantic` are warn-by-default in `Cargo.toml`), `cargo build`.
- **Behavior preservation (this is a pure refactor):** identical pixel output for all states; identical early-return-on-failure semantics; identical GDI cleanup ordering (restore selected object *before* deleting it; delete bitmap before DC); all existing `log::trace!`/`warn!` lines preserved verbatim with the same key-value fields.
- **FFI style (`docs/code-conventions.md` §3):** isolate `unsafe` behind safe wrappers; wrap held handles in `Drop` types; keep wrappers within the feature module that uses them. In Rust 2024, `unsafe fn` bodies still need explicit `unsafe { }` blocks. Use `&raw const`/`&raw mut`, not `&x as *const _`. Prefer `u32::from`/`try_from` over `as`.
- **Comments:** no ephemeral planning labels (phase/task IDs) in code comments — state rationale in domain terms.
- **No unit tests for GDI:** drawing is FFI and is verified manually per `docs/architecture.md` §Integration Testing. Each task's "test cycle" is therefore `fmt` + `clippy` + `build` green, plus the manual Windows smoke test in Task 3.

---

### Task 1: Introduce GDI wrappers in `osd_render.rs` and rewrite `osd.rs` drawing functions to use them

This lands the RAII cleanup ("Cosmetic #2") with the wrappers in their final home. The drawing functions stay in `osd.rs` for now and still read thread-locals — only their *resource handling* changes. Tree stays green and behavior is identical.

**Files:**
- Create: `src/platform/windows/osd_render.rs`
- Modify: `src/platform/windows/mod.rs` (add module declaration, ~line 31)
- Modify: `src/platform/windows/osd.rs` (rewrite `paint_osd` 235-285, `draw_overlay_section` 385-452, `draw_icon` 454-480, `draw_single_bar` 508-553, `draw_percentage_text` 555-597, `draw_error_message` 599-640; delete `create_icon_font` 482-506 and `create_osd_font` 642-666)

**Interfaces:**
- Consumes: nothing from prior tasks.
- Produces (used by `osd.rs` in this task, made private in Task 2):
  - `pub(super) fn fill_rect(hdc: HDC, rect: &RECT, color: u32)`
  - `pub(super) enum FontFace { Text, Emoji }`
  - `pub(super) struct SelectedFont` with `pub(super) fn new(hdc: HDC, font_size: i32, face: FontFace) -> Self` (selects a font into `hdc`; `Drop` restores the previous object and deletes the font)
  - `pub(super) struct BackBuffer` with `pub(super) fn new(target: HDC, width: i32, height: i32) -> Option<Self>`, `pub(super) fn dc(&self) -> HDC`, `pub(super) fn blit_to(&self, target: HDC)` (`Drop` restores the old bitmap, deletes the bitmap, then deletes the DC)

- [ ] **Step 1: Create `osd_render.rs` with the three wrappers**

Create `src/platform/windows/osd_render.rs` with exactly this content:

```rust
//! GDI drawing primitives for the OSD window.
//!
//! This module owns the pure rendering layer: it draws into a device context
//! it is handed and knows nothing about window lifecycle or thread-local state.
//! GDI resource cleanup is handled by RAII guards (`SelectedFont`, `BackBuffer`)
//! and the `fill_rect` helper, so callers never balance `DeleteObject` by hand.

use windows::Win32::Foundation::{COLORREF, RECT};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CLIP_DEFAULT_PRECIS, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH, DeleteDC, DeleteObject, FF_DONTCARE,
    FW_NORMAL, FillRect, HBITMAP, HDC, HFONT, HGDIOBJ, OUT_DEFAULT_PRECIS, SRCCOPY, SelectObject,
};
use windows::core::w;

/// Fills `rect` with a solid `color` (BGR). The brush never escapes this call,
/// so no RAII wrapper is needed.
pub(super) fn fill_rect(hdc: HDC, rect: &RECT, color: u32) {
    unsafe {
        let brush = CreateSolidBrush(COLORREF(color));
        FillRect(hdc, &raw const *rect, brush);
        let _ = DeleteObject(brush);
    }
}

/// Selects which font face the OSD draws with.
pub(super) enum FontFace {
    /// Regular UI text ("Segoe UI").
    Text,
    /// Emoji glyphs ("Segoe UI Emoji").
    Emoji,
}

/// Creates an OSD font of the given pixel height and face.
fn create_font(font_size: i32, face: FontFace) -> HFONT {
    let face_name = match face {
        FontFace::Text => w!("Segoe UI"),
        FontFace::Emoji => w!("Segoe UI Emoji"),
    };
    unsafe {
        CreateFontW(
            font_size,
            0,
            0,
            0,
            FW_NORMAL.0.cast_signed(),
            0,
            0,
            0,
            u32::from(DEFAULT_CHARSET.0),
            u32::from(OUT_DEFAULT_PRECIS.0),
            u32::from(CLIP_DEFAULT_PRECIS.0),
            0,
            u32::from(DEFAULT_PITCH.0 | FF_DONTCARE.0),
            face_name,
        )
    }
}

/// RAII guard that selects a font into a device context and, on drop, restores
/// the previously selected object and deletes the font.
pub(super) struct SelectedFont {
    hdc: HDC,
    font: HFONT,
    old: HGDIOBJ,
}

impl SelectedFont {
    /// Creates a font of the given size/face and selects it into `hdc`.
    pub(super) fn new(hdc: HDC, font_size: i32, face: FontFace) -> Self {
        let font = create_font(font_size, face);
        let old = unsafe { SelectObject(hdc, font) };
        Self { hdc, font, old }
    }
}

impl Drop for SelectedFont {
    fn drop(&mut self) {
        unsafe {
            SelectObject(self.hdc, self.old);
            let _ = DeleteObject(self.font);
        }
    }
}

/// RAII double-buffer: a memory DC with a compatible bitmap selected into it.
/// On drop it restores the old bitmap, deletes the bitmap, then deletes the DC.
pub(super) struct BackBuffer {
    mem_dc: HDC,
    bitmap: HBITMAP,
    old_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
}

impl BackBuffer {
    /// Allocates a memory DC + compatible bitmap sized `width` x `height`,
    /// compatible with `target`. Returns `None` if any GDI allocation fails.
    pub(super) fn new(target: HDC, width: i32, height: i32) -> Option<Self> {
        unsafe {
            let mem_dc = CreateCompatibleDC(target);
            if mem_dc.0 == 0 {
                return None;
            }
            let bitmap = CreateCompatibleBitmap(target, width, height);
            if bitmap.0 == 0 {
                let _ = DeleteDC(mem_dc);
                return None;
            }
            let old_bitmap = SelectObject(mem_dc, bitmap);
            Some(Self {
                mem_dc,
                bitmap,
                old_bitmap,
                width,
                height,
            })
        }
    }

    /// The memory device context to draw into.
    pub(super) fn dc(&self) -> HDC {
        self.mem_dc
    }

    /// Copies the back buffer onto `target` at the origin.
    pub(super) fn blit_to(&self, target: HDC) {
        unsafe {
            let _ = BitBlt(target, 0, 0, self.width, self.height, self.mem_dc, 0, 0, SRCCOPY);
        }
    }
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

- [ ] **Step 2: Declare the module**

In `src/platform/windows/mod.rs`, add the declaration directly after `pub mod osd;` (line 31). It is crate-private — only `osd.rs` uses it:

```rust
pub mod osd;
mod osd_render;
```

- [ ] **Step 3: Rewrite `paint_osd` to use `BackBuffer` + `fill_rect`**

Replace the body of `paint_osd` (`osd.rs:235-285`) with this. Note the `OSD_STATE` snapshot and `draw_*` calls are unchanged from today; only the DC/bitmap/brush bookkeeping is replaced:

```rust
unsafe fn paint_osd(hwnd: HWND, hdc: HDC) {
    unsafe {
        log::trace!("Painting OSD");
        let mut rect = RECT::default();
        if GetClientRect(hwnd, &raw mut rect).is_err() {
            log::warn!("GetClientRect failed");
            return;
        }

        let width = rect.right - rect.left;
        let height = rect.bottom - rect.top;

        // Double-buffer to avoid flicker; the guard cleans up on drop.
        let Some(buffer) = osd_render::BackBuffer::new(hdc, width, height) else {
            return;
        };
        let mem_dc = buffer.dc();

        // Fill background
        osd_render::fill_rect(mem_dc, &rect, OSD_BACKGROUND_COLOR);

        // Get current state from thread-local storage
        let state = OSD_STATE.with(|s| s.borrow().clone());

        // Draw progress bar(s)
        draw_brightness_bars(mem_dc, &rect, &state);

        // Draw error message if in error state
        if state.is_error {
            draw_error_message(mem_dc, &rect, ERROR_MESSAGE);
        }

        // Copy to screen
        buffer.blit_to(hdc);
    }
}
```

- [ ] **Step 4: Rewrite `draw_single_bar` to use `fill_rect`**

Replace the two `CreateSolidBrush`/`FillRect`/`DeleteObject` blocks in `draw_single_bar` (`osd.rs:508-553`) with `fill_rect` calls. The full new body:

```rust
unsafe fn draw_single_bar(
    hdc: HDC,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    percent: u8,
    is_error: bool,
) {
    // Draw background
    let bg_rect = RECT {
        left: x,
        top: y,
        right: x + width,
        bottom: y + height,
    };
    osd_render::fill_rect(hdc, &bg_rect, BAR_BACKGROUND_COLOR);

    // Draw filled portion
    let fill_width = i32::try_from(i64::from(width) * i64::from(percent) / 100).unwrap_or(0);
    if fill_width > 0 {
        let fill_rect = RECT {
            left: x,
            top: y,
            right: x + fill_width,
            bottom: y + height,
        };
        let fill_color = if is_error {
            BAR_ERROR_COLOR
        } else {
            BAR_FILL_COLOR
        };
        osd_render::fill_rect(hdc, &fill_rect, fill_color);
    }
}
```

Note: the function no longer contains any `unsafe` operation directly (the `unsafe` is now inside `fill_rect`), so remove the `unsafe { ... }` wrapper inside it but keep the `unsafe fn` signature unchanged for now (it is changed in Task 2). If clippy flags `unused_unsafe`, that confirms the inner block must go.

- [ ] **Step 5: Rewrite `draw_overlay_section` to use `fill_rect`**

In `draw_overlay_section` (`osd.rs:385-452`), replace the two brush blocks (the background fill at 424-426 and the filled-portion at 439-441) with `fill_rect` calls. Change:

```rust
        let bg_brush = CreateSolidBrush(COLORREF(BAR_BACKGROUND_COLOR));
        FillRect(hdc, &raw const bg_rect, bg_brush);
        let _ = DeleteObject(bg_brush);
```
to:
```rust
        osd_render::fill_rect(hdc, &bg_rect, BAR_BACKGROUND_COLOR);
```
and change:
```rust
            let fill_brush = CreateSolidBrush(COLORREF(OVERLAY_FILL_COLOR));
            FillRect(hdc, &raw const fill_rect, fill_brush);
            let _ = DeleteObject(fill_brush);
```
to:
```rust
            osd_render::fill_rect(hdc, &fill_rect, OVERLAY_FILL_COLOR);
```
Leave the rest of the function (layout math, `draw_percentage_text`, `draw_icon` calls) unchanged.

- [ ] **Step 6: Rewrite `draw_icon` to use `SelectedFont`**

Replace `draw_icon` (`osd.rs:454-480`) with:

```rust
unsafe fn draw_icon(hdc: HDC, x: i32, y: i32, icon: &str) {
    unsafe {
        let (font_size, bar_height) = with_metrics(|m| (m.font_size, m.bar_height));

        // Select an emoji font; the guard restores + deletes on drop.
        let _font = osd_render::SelectedFont::new(hdc, font_size, osd_render::FontFace::Emoji);

        // Set text properties
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));

        // Draw icon
        let wide_text: Vec<u16> = icon.encode_utf16().collect();
        let text_y = y + (bar_height - font_size) / 2;
        TextOutW(hdc, x, text_y, &wide_text);
    }
}
```

- [ ] **Step 7: Rewrite `draw_percentage_text` to use `SelectedFont`**

Replace `draw_percentage_text` (`osd.rs:555-597`) with:

```rust
unsafe fn draw_percentage_text(hdc: HDC, x: i32, y: i32, percent: u8, right_align: bool) {
    unsafe {
        let (font_size, bar_height) = with_metrics(|m| (m.font_size, m.bar_height));

        // Select the text font; the guard restores + deletes on drop.
        let _font = osd_render::SelectedFont::new(hdc, font_size, osd_render::FontFace::Text);

        // Set text properties
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(TEXT_COLOR));

        // Set text alignment
        let align = if right_align { TA_RIGHT } else { TA_LEFT };
        SetTextAlign(hdc, align);

        // Format and draw text
        let text = format!("{percent}%");
        let wide_text: Vec<u16> = text.encode_utf16().collect();

        // Center text vertically with the bar
        let text_y = y + (bar_height - font_size).saturating_div(2);
        TextOutW(hdc, x, text_y, &wide_text);

        // Restore default alignment (draw state, not a resource).
        SetTextAlign(hdc, TA_LEFT);
    }
}
```

- [ ] **Step 8: Rewrite `draw_error_message` to use `SelectedFont`**

Replace `draw_error_message` (`osd.rs:599-640`) with:

```rust
unsafe fn draw_error_message(hdc: HDC, client_rect: &RECT, message: &str) {
    unsafe {
        let (font_size, error_row_height) = with_metrics(|m| (m.font_size, m.error_row_height));

        // Select the text font; the guard restores + deletes on drop.
        let _font = osd_render::SelectedFont::new(hdc, font_size, osd_render::FontFace::Text);

        // Set text properties - use red color for error visibility
        SetBkMode(hdc, TRANSPARENT);
        SetTextColor(hdc, COLORREF(ERROR_TEXT_COLOR));

        // Convert message to wide string
        let wide_text: Vec<u16> = message.encode_utf16().collect();

        // Calculate position - centered horizontally, in footer area at bottom
        let width = client_rect.right - client_rect.left;
        // Approximate text width (~7 pixels per character at this font size)
        // Scaling approximation: original was 7px for 18pt font. Ratio ~0.38
        #[allow(clippy::cast_possible_truncation)]
        let approx_char_width = (f64::from(font_size) * 0.38).round() as i32;
        let approx_text_width = i32::try_from(wide_text.len()).unwrap_or(0) * approx_char_width;

        let x = (width - approx_text_width) / 2;
        // Position in error row area: bottom of window minus row height, centered vertically
        let y = client_rect.bottom - error_row_height + (error_row_height - font_size) / 2;

        TextOutW(hdc, x, y, &wide_text);
    }
}
```

- [ ] **Step 9: Delete the now-unused font factories**

Delete `create_icon_font` (`osd.rs:482-506`) and `create_osd_font` (`osd.rs:642-666`) entirely — their logic now lives in `osd_render::create_font`.

- [ ] **Step 10: Remove the imports that Task 1 makes unused in `osd.rs`**

The wrapper conversions move the inline FFI out of `osd.rs` *now* (not in Task 2), so these symbols become unused the moment Task 1 lands and **must be removed in this task** or the Task 1 clippy gate (`-D warnings` → `unused_imports` is an error) fails. Remove exactly these from the `use windows::Win32::Graphics::Gdi::{...}` group in `osd.rs`:

`BitBlt`, `CreateCompatibleBitmap`, `CreateCompatibleDC`, `CreateFontW`, `DeleteDC`, `DeleteObject`, `FillRect`, `SelectObject`, `SRCCOPY`, `HFONT`, `FW_NORMAL`, `CLIP_DEFAULT_PRECIS`, `DEFAULT_CHARSET`, `DEFAULT_PITCH`, `FF_DONTCARE`, `OUT_DEFAULT_PRECIS`.

Keep (still used in `osd.rs` during Task 1): `CreateSolidBrush` (window-class brush at `ensure_osd_class_registered`), the text-draw FFI `SetBkMode`/`SetTextColor`/`SetTextAlign`/`TextOutW`/`TA_LEFT`/`TA_RIGHT`/`TRANSPARENT` (the `draw_icon`/`draw_percentage_text`/`draw_error_message` bodies are still here — they leave in Task 2), plus all window/DC symbols. `COLORREF` and `RECT` come from the `Foundation` import line, not this group — leave them.

- [ ] **Step 11: Run fmt, clippy, build**

This is the **only** clippy gate for Task 1 — run it after all of Steps 1-10 have landed, not per-step. Between Step 2 (module declared) and Step 10 (import cleanup) the tree is transiently inconsistent: the wrappers are unused right after Step 2 (`dead_code`), and the old imports are unused right after Steps 3-9. Those transient warnings are expected and resolve by Step 10; do not chase them mid-task.

Run:
```bash
cargo fmt -- --check && cargo clippy -- -D warnings && cargo build
```
Expected: all green. One deterministic fixup: `draw_single_bar`'s body now contains no FFI (only `fill_rect` calls), so its inner `unsafe { }` block is `unused_unsafe` and must be removed — the Step 4 code already shows it without that block. **Do not** remove the `unsafe { }` blocks from `draw_overlay_section`, `draw_icon`, `draw_percentage_text`, `draw_error_message`, or `paint_osd`: those still call `unsafe fn` draw helpers and/or FFI (`SetBkMode`, `TextOutW`, `GetClientRect`), so their blocks remain live. If any unused-import warning beyond the Step 10 list appears, reconcile against Step 10 — do not guess.

- [ ] **Step 12: Commit**

```bash
git add src/platform/windows/osd_render.rs src/platform/windows/mod.rs src/platform/windows/osd.rs
git commit -m "$(cat <<'EOF'
refactor(osd): replace hand-balanced GDI cleanup with RAII wrappers

Add osd_render.rs with fill_rect (brushes), SelectedFont (font
select/restore), and BackBuffer (double-buffering) wrappers, and
rewrite the osd.rs drawing functions to use them. Removes the manual
DeleteObject/SelectObject bookkeeping. No behavior change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 2: Relocate the drawing functions into `osd_render.rs` with a pure-parameter interface

Move the (now-clean) drawing functions out of `osd.rs` and into `osd_render.rs`, converting them from thread-local reads (`with_metrics`, `OSD_STATE`) to explicit `&OsdMetrics` / `&OsdRenderState` parameters. Expose a single public `paint` entry point. `osd.rs` shrinks to lifecycle + metrics + state.

**Files:**
- Modify: `src/platform/windows/osd.rs` (move 6 draw functions out; move drawing-only constants out; change visibility of `OsdRenderState` and `OSD_BACKGROUND_COLOR`; rewrite `paint_osd`; drop now-unused imports)
- Modify: `src/platform/windows/osd_render.rs` (receive the moved functions + constants; make wrappers private; add imports)

**Interfaces:**
- Consumes (from Task 1): `fill_rect`, `FontFace`, `SelectedFont`, `BackBuffer` (now in the same module — become private).
- Consumes (from `osd.rs`, via `use super::osd::{...}`): `OsdMetrics` (already `pub`), `OsdRenderState` (made `pub(super)` this task), `OSD_BACKGROUND_COLOR` (made `pub(super)` this task).
- Produces (the module's only item visible to `osd.rs`): `pub(super) unsafe fn paint(hdc: HDC, client_rect: &RECT, state: &OsdRenderState, metrics: &OsdMetrics)`. It is `pub(super)` (not `pub`): a `pub fn` exposing the `pub(super)` `OsdRenderState` in its signature trips `private_interfaces` (a rustc warn-by-default lint → error under `-D warnings`). (`pub` would also be unreachable from outside the private module, but `unreachable_pub` is allow-by-default and not enabled here, so it is not the enforcing lint — `private_interfaces` is.) It is `unsafe` because it dereferences a raw `HDC` via FFI — the caller must pass a valid device context (preserving the safety contract the original `unsafe fn paint_osd` carried).
- Final private signatures inside `osd_render.rs`:
  - `fn draw_brightness_bars(hdc: HDC, client_rect: &RECT, state: &OsdRenderState, metrics: &OsdMetrics)`
  - `fn draw_hardware_section(hdc: HDC, client_rect: &RECT, bar_top: i32, hardware_brightness: u8, is_error: bool, metrics: &OsdMetrics)`
  - `fn draw_overlay_section(hdc: HDC, client_rect: &RECT, bar_top: i32, overlay_opacity: u8, metrics: &OsdMetrics)`
  - `fn draw_icon(hdc: HDC, x: i32, y: i32, icon: &str, metrics: &OsdMetrics)`
  - `fn draw_single_bar(hdc: HDC, x: i32, y: i32, width: i32, height: i32, percent: u8, is_error: bool)` (no metrics — pure geometry)
  - `fn draw_percentage_text(hdc: HDC, x: i32, y: i32, percent: u8, right_align: bool, metrics: &OsdMetrics)`
  - `fn draw_error_message(hdc: HDC, client_rect: &RECT, message: &str, metrics: &OsdMetrics)`

- [ ] **Step 1: Widen visibility of the shared types/constant in `osd.rs`**

In `osd.rs`, change three visibilities so the sibling module can read them:
- `struct OsdRenderState` (line 175) → `pub(super) struct OsdRenderState`, and mark each of its three fields `pub(super)` (`hardware_brightness`, `overlay_opacity`, `is_error`). Keep its existing `#[derive(Debug, Clone, Default)]` (line 174) untouched — `Clone` is needed for the `OSD_STATE` snapshot and `Default` for the thread-local initializer.
- `const OSD_BACKGROUND_COLOR` (line 64) → `pub(super) const OSD_BACKGROUND_COLOR`.

`OsdMetrics` and its fields are already `pub`, so no change there.

- [ ] **Step 2: Move the drawing-only constants into `osd_render.rs`**

Cut these constants from `osd.rs` and paste them near the top of `osd_render.rs` (after the imports). They are used only by drawing:
- `ERROR_MESSAGE` (line 61)
- `BAR_FILL_COLOR`, `OVERLAY_FILL_COLOR`, `BAR_BACKGROUND_COLOR`, `TEXT_COLOR`, `BAR_ERROR_COLOR`, `ERROR_TEXT_COLOR` (lines 73-84)
- `ICON_HARDWARE`, `ICON_OVERLAY` (lines 90-92)

Leave `OSD_BACKGROUND_COLOR` in `osd.rs` (it is also the window-class brush) — `osd_render` will reference it via `super::osd::OSD_BACKGROUND_COLOR`. Leave all `BASE_*`, dimension, and `BASE_FONT_SIZE` constants in `osd.rs` (metrics concern).

- [ ] **Step 3: Move the six drawing functions + `paint` entry into `osd_render.rs`**

Cut `draw_brightness_bars`, `draw_hardware_section`, `draw_overlay_section`, `draw_icon`, `draw_single_bar`, `draw_percentage_text`, and `draw_error_message` from `osd.rs` and paste them into `osd_render.rs`. Apply these mechanical transformations to each as you move it:

1. **Signature:** drop `unsafe fn` → `fn` for the seven private `draw_*` helpers — they are only ever called with the controlled `mem_dc` from `paint`, so a safe private fn with internal `unsafe { }` blocks is sound (the same pattern `set_osd_opacity` already uses for an HWND). The public-ish entry `paint` is the exception: it stays `unsafe fn` (see below), since it is the boundary that accepts an externally-supplied `HDC`. Add a trailing `metrics: &OsdMetrics` parameter to every function except `draw_single_bar` (see Interfaces block above for exact final signatures).
2. **Metrics reads:** replace each `with_metrics(|m| (...))` call with direct reads from the `metrics` parameter. Concretely:
   - In `draw_brightness_bars`: `let bar_top = with_metrics(|m| (m.height - m.bar_height) / 2);` → `let bar_top = (metrics.height - metrics.bar_height) / 2;`. Pass `metrics` through to `draw_overlay_section` and `draw_hardware_section`, and pass `state.overlay_opacity` / `state.hardware_brightness` / `state.is_error` as today.
   - In `draw_hardware_section` and `draw_overlay_section`: replace `let (padding, percent_text_width, icon_width, bar_gap, bar_height) = with_metrics(|m| (...));` with five direct bindings from `metrics` (e.g. `let padding = metrics.padding;` …). Pass `metrics` into the nested `draw_percentage_text` and `draw_icon` calls.
   - In `draw_icon`, `draw_percentage_text`, `draw_error_message`: replace `with_metrics(|m| (m.font_size, m.bar_height))` (or `m.error_row_height`) with `let font_size = metrics.font_size;` and `let bar_height = metrics.bar_height;` (resp. `metrics.error_row_height`).
3. **`unsafe` blocks:** keep an inner `unsafe { }` block around the remaining FFI calls (`SetBkMode`, `SetTextColor`, `SetTextAlign`, `TextOutW`) in the text functions. `draw_single_bar`, `draw_brightness_bars`, `draw_hardware_section`, and `draw_overlay_section` contain no direct FFI after Task 1 (only `fill_rect`/`draw_*` calls), so they need no `unsafe` block — if one remains, clippy's `unused_unsafe` will flag it; remove it.

Then add the entry point `paint` (this is the body that was in `osd.rs::paint_osd` minus the `GetClientRect` *and minus the `trace!("Painting OSD")` line*, both of which stay in `osd.rs::paint_osd` so the log order on a `GetClientRect` failure is unchanged):

```rust
/// Paints the OSD content into `hdc` using double-buffering.
///
/// `client_rect` is the window's client rectangle; `state` and `metrics` are
/// snapshots taken by the caller — this function reads no thread-local state.
///
/// # Safety
///
/// `hdc` must be a valid device context (e.g. from `BeginPaint`).
pub(super) unsafe fn paint(
    hdc: HDC,
    client_rect: &RECT,
    state: &OsdRenderState,
    metrics: &OsdMetrics,
) {
    let width = client_rect.right - client_rect.left;
    let height = client_rect.bottom - client_rect.top;

    // Double-buffer to avoid flicker; the guard cleans up on drop.
    let Some(buffer) = BackBuffer::new(hdc, width, height) else {
        return;
    };
    let mem_dc = buffer.dc();

    // Fill background
    fill_rect(mem_dc, client_rect, super::osd::OSD_BACKGROUND_COLOR);

    // Draw progress bar(s)
    draw_brightness_bars(mem_dc, client_rect, state, metrics);

    // Draw error message if in error state
    if state.is_error {
        draw_error_message(mem_dc, client_rect, ERROR_MESSAGE, metrics);
    }

    // Copy to screen
    buffer.blit_to(hdc);
}
```

- [ ] **Step 4: Add the type imports and demote the wrappers to private in `osd_render.rs`**

At the top of `osd_render.rs`, add:
```rust
use super::osd::{OsdMetrics, OsdRenderState};
```
Add the GDI imports the moved text functions need to the existing `use windows::Win32::Graphics::Gdi::{...}` group: `SetBkMode`, `SetTextAlign`, `SetTextColor`, `TA_LEFT`, `TA_RIGHT`, `TextOutW`, `TRANSPARENT`. (`COLORREF` and `RECT` are already imported in `osd_render.rs` from Task 1 Step 1 — no change needed there.)

Change the wrapper visibilities from `pub(super)` to private (plain `fn`/`struct`) for `fill_rect`, `FontFace`, `SelectedFont` (and its `new`), and `BackBuffer` (and its `new`/`dc`/`blit_to`) — they are now only used within this module. Keep `paint` as `pub(super) unsafe fn` (it is the one item `osd.rs` calls).

- [ ] **Step 5: Rewrite `paint_osd` in `osd.rs` as a thin bridge**

Replace the whole `paint_osd` function in `osd.rs` with this (it now only does the window query + state snapshot, then delegates):

```rust
/// Snapshots render state and metrics, then delegates drawing to `osd_render`.
///
/// # Safety
///
/// Must be called from within a `BeginPaint`/`EndPaint` block.
unsafe fn paint_osd(hwnd: HWND, hdc: HDC) {
    // Kept here (before the GetClientRect guard) so the trace order on a
    // GetClientRect failure is identical to the pre-refactor behavior.
    log::trace!("Painting OSD");
    let mut rect = RECT::default();
    if unsafe { GetClientRect(hwnd, &raw mut rect) }.is_err() {
        log::warn!("GetClientRect failed");
        return;
    }

    let state = OSD_STATE.with(|s| s.borrow().clone());
    let metrics = OSD_METRICS.with(|m| *m.borrow());
    unsafe { osd_render::paint(hdc, &rect, &state, &metrics) };
}
```

- [ ] **Step 6: Drop now-unused imports from `osd.rs`**

Only the text-draw FFI is left to remove — everything else became unused and was already removed in Task 1 Step 10. The `draw_icon`/`draw_percentage_text`/`draw_error_message` bodies move to `osd_render.rs` this task, so remove these from the `use windows::Win32::Graphics::Gdi::{...}` group in `osd.rs`: `SetBkMode`, `SetTextAlign`, `SetTextColor`, `TA_LEFT`, `TA_RIGHT`, `TextOutW`, `TRANSPARENT`. Keep `CreateSolidBrush` (used by `ensure_osd_class_registered`) and the window/DC symbols. `COLORREF` and `RECT` are in the `Foundation` import (a separate `use` line), not this Gdi group, and stay untouched (`COLORREF` is still used by `ensure_osd_class_registered`/`set_osd_opacity`; `RECT` by `paint_osd`). Do not guess — let clippy in Step 7 reconcile against this list.

- [ ] **Step 7: Run fmt, clippy, build**

Run:
```bash
cargo fmt -- --check && cargo clippy -- -D warnings && cargo build
```
Expected: all green. Resolve any remaining unused-import or `unused_unsafe` warnings per Steps 4 and 6. Confirm `osd_render`'s only `pub(super)` item is `paint` (everything else is module-private), and that no `private_interfaces`/`unreachable_pub` warning appears.

- [ ] **Step 8: Commit**

```bash
git add src/platform/windows/osd.rs src/platform/windows/osd_render.rs
git commit -m "$(cat <<'EOF'
refactor(osd): move GDI drawing layer into osd_render with pure interface

Relocate the drawing primitives out of osd.rs into osd_render.rs and
convert them from thread-local reads to explicit &OsdMetrics /
&OsdRenderState parameters. osd.rs keeps lifecycle, metrics, and the
thread-local state, snapshotting it once and calling osd_render::paint.
No behavior change.

Co-Authored-By: Claude Opus 4.8 (1M context) <noreply@anthropic.com>
EOF
)"
```

---

### Task 3: Manual Windows smoke test and finalize

Drawing cannot be unit-tested cross-platform; verify identical behavior by running the app, per `docs/architecture.md` §Integration Testing.

**Files:** none (verification only).

- [ ] **Step 1: Build and run with logging**

Run:
```bash
RUST_LOG=debug cargo run
```
Expected: app starts, tray icon appears, no panics.

- [ ] **Step 2: Verify the hardware brightness bar**

Press the brightness-up / brightness-down hotkey over a monitor. Expected: OSD appears bottom-center, the right (gold) bar fills left-to-right matching the percentage, the 🔆 icon and `NN%` text render, and the OSD auto-hides after the timeout (default 1000ms). Confirm it looks identical to before the refactor.

- [ ] **Step 3: Verify the overlay dimming bar**

Dim a monitor to 0% / sub-zero so overlay dimming engages. Expected: the left (purple) bar fills right-to-left, the 🕶 icon and `NN%` render. Identical to before.

- [ ] **Step 4: Verify the error state**

Trigger a DDC failure (e.g. unplug/disable a DDC-capable monitor and attempt an adjustment, or follow the manual error procedure in `docs/architecture.md`). Expected: OSD expands to the taller height, the bar turns red, and the row "DDC Error - Adjustment failed" appears centered. Identical to before.

- [ ] **Step 5: Verify multi-monitor + DPI**

With monitors at different scaling factors, trigger the OSD on each (move the mouse to the target monitor first). Expected: the OSD scales correctly per-monitor (size, font, bar dimensions) exactly as before.

- [ ] **Step 6: Finalize**

Invoke the `superpowers:finishing-a-development-branch` skill to choose how to integrate `refactor/osd-render-split` (merge to `main`, open a PR, etc.).

---

## Self-Review

**Spec coverage:**
- New sibling `osd_render.rs`, crate-private → Task 1 Steps 1-2. ✓
- Pure-parameter `paint(hdc, &client_rect, &state, &metrics)` interface → Task 2 Steps 3, 5. ✓
- `GetClientRect` stays in `osd.rs` → Task 2 Step 5. ✓
- Thread-local snapshot in `osd.rs`, none in `osd_render` → Task 2 Steps 3, 5. ✓
- `OsdMetrics`/`OsdRenderState` ownership + visibility → Task 2 Step 1. ✓
- `fill_rect` helper (brushes) → Task 1 Steps 1, 4, 5. ✓
- `SelectedFont` guard (fonts) + `FontFace` collapsing two font factories → Task 1 Steps 1, 6-9. ✓
- `BackBuffer` guard (double-buffering) → Task 1 Steps 1, 3. ✓
- Early-return-on-failure preserved (`BackBuffer::new` → `None`) → Task 1 Step 3 / Task 2 Step 3. ✓
- Cleanup ordering preserved (restore-before-delete; bitmap before DC) → `Drop` impls in Task 1 Step 1. ✓
- Log lines preserved → `trace!("Painting OSD")` stays in `osd.rs::paint_osd` before the `GetClientRect` guard so its order on failure is unchanged (Task 2 Step 5); other `trace!`/`warn!` lines kept verbatim in moved bodies. ✓
- `paint` visibility — `pub(super) unsafe fn`, not `pub`, to avoid `private_interfaces` (the enforcing lint) and to preserve the original `unsafe fn paint_osd` HDC-validity contract at the module boundary (Task 2 Interfaces, Step 3). ✓
- Import sequencing — the 16 GDI symbols the wrappers displace become unused in `osd.rs` at the end of Task 1 (not Task 2) and are removed in Task 1 Step 10; only the 7 text-draw symbols leave in Task 2 Step 6. Every intermediate commit passes `clippy -D warnings`. ✓
- Wrappers private to `osd_render.rs` → Task 2 Step 4. ✓
- Manual verification (no GDI unit tests) → Task 3. ✓
- Future work (layout→`core/`, shared `gdi.rs`) → intentionally excluded; not in any task. ✓

**Placeholder scan:** No TBD/TODO/"handle edge cases"/"similar to Task N". Moved-function bodies are specified by exact transformation rules against named line ranges, with full code for every net-new item. ✓

**Type consistency:** `paint`, `fill_rect`, `SelectedFont::new`, `FontFace::{Text,Emoji}`, `BackBuffer::{new,dc,blit_to}`, and the seven `draw_*` signatures are named identically in the Interfaces blocks and the step code. `OSD_BACKGROUND_COLOR` referenced as `super::osd::OSD_BACKGROUND_COLOR` from `osd_render`, consistent with its `pub(super)` declaration. ✓
