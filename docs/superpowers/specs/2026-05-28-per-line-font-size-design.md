# Per-Line Font Size (Direct USB) — Design

**Date:** 2026-05-28
**Status:** Approved, pending implementation

## Summary

Let users pick a font size per display line for the **direct-USB** OLED render path.
Three presets per line: Small (9pt), Medium (12pt, default), Large (18pt).

## Scope & Constraints

- **Direct-USB only.** The GameSense path ships raw text strings to SteelSeries GG,
  which controls its own font — size is not controllable from this app there. The GUI
  control is therefore hidden unless "Direct USB" is selected.
- **Per line, not per sensor.** Sensors are collapsed into `line1..lineN` strings by
  `format_custom_value` (utils.rs) *before* reaching the renderer, so the line is the
  smallest unit the renderer sees. True per-sensor sizing would require a structured
  segment model + horizontal layout; explicitly out of scope.
- `DISPLAY_LINES = 5`, so up to 5 per-line sizes.
- Font source is the `profont` bitmap font crate, which offers only fixed point sizes
  (7/9/10/12/14/18/24) — hence a preset picker, not a free slider.
- Backwards compatible: missing config → all lines Medium (12pt) = current behavior.

## Components

### `FontSize` enum (render.rs or settings.rs)
- Variants: `Small`, `Medium`, `Large`.
- `from_str(&str) -> FontSize` (default Medium on unknown), `as_str(&self) -> &str`.
- `font(&self) -> &'static MonoFont<'static>` → PROFONT_9 / PROFONT_12 / PROFONT_18.
- `line_height(&self)` → font `character_size.height` + small inter-line gap.

### Config (`AppConfig`, settings.rs)
- New keys in `[Main]`: `font_line1 … font_line5`, values `small|medium|large`.
- Parsed into `font_sizes: [FontSize; DISPLAY_LINES]`, default `[Medium; 5]`.

### Render (`render.rs`)
- Signature: `render_text_to_oled(text: &str, x: i32, line_fonts: &[FontSize])`.
- Iterate `text.lines()`. For line index `i`, font = `line_fonts.get(i)` fallback Medium.
- Maintain a running `y` baseline cursor: draw at `y + ascent`, then advance by the
  line's `line_height()` rather than the fixed `10` start / `+12` step.
- Existing IMG: and emoji handling preserved per line.
- Overflow past y=64 clips harmlessly (already guarded in `set_pixel` / `draw_iter`).

### Call sites
- `daemon.rs::value_to_oled_buffer` → pass `&self.config.font_sizes`.
- `gui.rs` value preview → pass config sizes; "not connected"/error previews pass `&[Medium]`.
- `gui.rs::build_main_config` → write the 5 `font_lineN` keys.

### GUI (`index.html`)
- New "Line Font Sizes (Direct USB)" section: one Small/Medium/Large `<select>` per line
  (Line 1–5), ids `font-line1..5`.
- Visible only when the Direct USB checkbox is checked (reuse the existing
  show/hide pattern, e.g. `updateDevicePickerVisibility`).
- Wired into config load (set selects) and save (collect into `config.font_sizes`).

## Testing

- `FontSize::from_str` round-trips all three; unknown → Medium.
- `render_text_to_oled`: all-Small vs all-Large produce different buffers; a mixed slice
  `[Large, Small, Medium, ...]` renders without panic and lights pixels.
- Config: `[Main]` with the 5 keys loads into `font_sizes`; missing keys default Medium;
  `build_main_config` round-trips.
- Update existing `render_text_to_oled` test call sites for the new `line_fonts` arg.

## Out of Scope

- Per-sensor (sub-line) font sizing.
- Font sizing in GameSense mode.
- Free-form / arbitrary point sizes beyond the three presets.
