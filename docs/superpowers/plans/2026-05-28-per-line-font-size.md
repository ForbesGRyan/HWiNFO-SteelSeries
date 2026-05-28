# Per-Line Font Size (Direct USB) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users pick a font size (Small/Medium/Large) per display line for the direct-USB OLED render path.

**Architecture:** A new `FontSize` enum in `render.rs` maps each preset to a `profont` `MonoFont` plus baseline/advance constants. `render_text_to_oled` gains a `line_fonts: &[FontSize]` parameter and stacks lines with a running baseline cursor so mixed sizes don't overlap. `AppConfig` carries `font_sizes: [FontSize; DISPLAY_LINES]`, persisted as `[Main] font_line1..5` INI keys and surfaced in the Tauri GUI as a section gated to direct-USB mode.

**Tech Stack:** Rust, `embedded-graphics`, `profont` 0.7, `rust-ini`, Tauri, vanilla JS/HTML.

**Constraints baked in:**
- Direct-USB only (GameSense path ships raw text; GG owns the font).
- Per line (5 lines), not per sensor — sensors are collapsed to `lineN` strings before rendering.
- Pure-Medium config must reproduce today's pixel output exactly (no regression for existing users).
- Backwards compatible: missing keys → all Medium.

---

## File Structure

- `src-tauri/src/render.rs` — **new** `FontSize` enum (owns font + layout constants); rework `render_text_to_oled`.
- `src-tauri/src/settings.rs` — add `font_sizes` to `AppConfig`, parse from INI, default helper.
- `src-tauri/src/gui.rs` — write keys in `apply_main_section`; update preview call sites + test `base_config`.
- `src-tauri/src/daemon.rs` — thread `font_sizes` through `value_to_oled_buffer`; update callers + test `base_config`.
- `src-tauri/src/state.rs`, `src-tauri/src/main.rs` — add field to `AppConfig` literals.
- `src-tauri/ui/index.html` — new "Line Font Sizes (Direct USB)" section + load/save/visibility wiring.
- `README.md` — document the feature.

---

## Task 1: `FontSize` enum in render.rs

**Files:**
- Modify: `src-tauri/src/render.rs:9` (imports) and append enum after imports
- Test: `src-tauri/src/render.rs` (existing `#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing tests**

Add these tests inside `mod tests` in `src-tauri/src/render.rs` (after the last test, before the closing `}` at line 643):

```rust
#[test]
fn test_fontsize_from_config_str_known_values() {
    assert_eq!(FontSize::from_config_str("small"), FontSize::Small);
    assert_eq!(FontSize::from_config_str("medium"), FontSize::Medium);
    assert_eq!(FontSize::from_config_str("large"), FontSize::Large);
}

#[test]
fn test_fontsize_from_config_str_case_and_whitespace() {
    assert_eq!(FontSize::from_config_str("  LARGE "), FontSize::Large);
}

#[test]
fn test_fontsize_from_config_str_unknown_defaults_medium() {
    assert_eq!(FontSize::from_config_str("huge"), FontSize::Medium);
    assert_eq!(FontSize::from_config_str(""), FontSize::Medium);
}

#[test]
fn test_fontsize_as_str_roundtrips() {
    for fs in [FontSize::Small, FontSize::Medium, FontSize::Large] {
        assert_eq!(FontSize::from_config_str(fs.as_str()), fs);
    }
}

#[test]
fn test_fontsize_default_is_medium() {
    assert_eq!(FontSize::default(), FontSize::Medium);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib render::tests::test_fontsize`
Expected: FAIL — `cannot find type/value FontSize in this scope`.

- [ ] **Step 3: Add the enum**

In `src-tauri/src/render.rs`, change the profont import at line 9 from:

```rust
use profont::PROFONT_12_POINT;
```

to:

```rust
use embedded_graphics::mono_font::MonoFont;
use profont::{PROFONT_12_POINT, PROFONT_18_POINT, PROFONT_9_POINT};
```

Then add this immediately after the imports (before `pub struct OledBuffer`):

```rust
/// Font size preset for a single OLED display line (direct-USB render path).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FontSize {
    Small,
    Medium,
    Large,
}

impl Default for FontSize {
    fn default() -> Self {
        FontSize::Medium
    }
}

impl FontSize {
    /// Parse a config string; unknown/empty → Medium.
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "small" => FontSize::Small,
            "large" => FontSize::Large,
            _ => FontSize::Medium,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            FontSize::Small => "small",
            FontSize::Medium => "medium",
            FontSize::Large => "large",
        }
    }

    fn font(&self) -> &'static MonoFont<'static> {
        match self {
            FontSize::Small => &PROFONT_9_POINT,
            FontSize::Medium => &PROFONT_12_POINT,
            FontSize::Large => &PROFONT_18_POINT,
        }
    }

    /// Baseline y for the first line at this size. Medium=10 reproduces the
    /// original fixed layout so existing configs render identically.
    fn first_baseline(&self) -> i32 {
        match self {
            FontSize::Small => 8,
            FontSize::Medium => 10,
            FontSize::Large => 16,
        }
    }

    /// Vertical step added before drawing each subsequent line. Medium=12
    /// reproduces the original `y += 12` spacing.
    fn line_advance(&self) -> i32 {
        match self {
            FontSize::Small => 10,
            FontSize::Medium => 12,
            FontSize::Large => 20,
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd src-tauri && cargo test --lib render::tests::test_fontsize`
Expected: PASS (5 tests). `font()`/`first_baseline()`/`line_advance()` are unused for now — that's fine, they're consumed in Task 4; if clippy complains about dead code at this commit, ignore (resolved in Task 4).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/render.rs
git commit -m "feat(render): add FontSize enum (Small/Medium/Large)"
```

---

## Task 2: `AppConfig.font_sizes` field + INI parsing

**Files:**
- Modify: `src-tauri/src/settings.rs:1` (import), `:83-97` (struct), `:167-216` (from_ini), add default helper
- Modify struct literals: `src-tauri/src/gui.rs:445-457`, `src-tauri/src/daemon.rs:830-840`, `src-tauri/src/state.rs:89-99`, `src-tauri/src/main.rs:576-587`
- Test: `src-tauri/src/settings.rs` tests

- [ ] **Step 1: Write the failing tests**

Add to `src-tauri/src/settings.rs` `mod tests` (near the other `from_ini` tests):

```rust
#[test]
fn test_appconfig_font_sizes_parsed() {
    use crate::render::FontSize;
    let mut ini = Ini::new();
    ini.with_section(Some("Main"))
        .set("style", "vertical")
        .set("font_line1", "large")
        .set("font_line2", "small")
        .set("font_line3", "medium");
    let config = AppConfig::from_ini(&ini).unwrap();
    assert_eq!(config.font_sizes[0], FontSize::Large);
    assert_eq!(config.font_sizes[1], FontSize::Small);
    assert_eq!(config.font_sizes[2], FontSize::Medium);
}

#[test]
fn test_appconfig_font_sizes_default_medium_when_missing() {
    use crate::render::FontSize;
    let mut ini = Ini::new();
    ini.with_section(Some("Main")).set("style", "vertical");
    let config = AppConfig::from_ini(&ini).unwrap();
    assert!(config.font_sizes.iter().all(|f| *f == FontSize::Medium));
    assert_eq!(config.font_sizes.len(), crate::consts::DISPLAY_LINES);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd src-tauri && cargo test --lib settings::tests::test_appconfig_font_sizes`
Expected: FAIL — `no field font_sizes on type AppConfig`.

- [ ] **Step 3: Update the import**

`src-tauri/src/settings.rs:1`, change:

```rust
use crate::consts::{Style, CUSTOM_SENSORS};
```

to:

```rust
use crate::consts::{Style, CUSTOM_SENSORS, DISPLAY_LINES};
use crate::render::FontSize;
```

- [ ] **Step 4: Add the struct field + default helper**

In `src-tauri/src/settings.rs`, add the field to `AppConfig` (after the `weather` field at line 96):

```rust
    #[serde(default)]
    pub weather: WeatherConfig,
    #[serde(default = "default_font_sizes")]
    pub font_sizes: [FontSize; DISPLAY_LINES],
}

fn default_font_sizes() -> [FontSize; DISPLAY_LINES] {
    [FontSize::Medium; DISPLAY_LINES]
}
```

(Replace the existing `    #[serde(default)]\n    pub weather: WeatherConfig,\n}` block — keep `weather` as-is, just append the new field before the closing brace and add the helper fn after the struct.)

- [ ] **Step 5: Parse it in `from_ini`**

In `src-tauri/src/settings.rs`, just before the `Ok(Self {` at line 167, add:

```rust
        let font_sizes = {
            let mut arr = [FontSize::Medium; DISPLAY_LINES];
            for (i, slot) in arr.iter_mut().enumerate() {
                if let Some(v) = main.get(format!("font_line{}", i + 1)) {
                    *slot = FontSize::from_config_str(v);
                }
            }
            arr
        };
```

Then add `font_sizes,` to the returned struct literal — insert it right after `weather: WeatherConfig::from_ini(config),` (line 215):

```rust
            weather: WeatherConfig::from_ini(config),
            font_sizes,
        })
```

- [ ] **Step 6: Add the field to all four `AppConfig` struct literals**

Each of these constructs `AppConfig { ... }` and must gain `font_sizes`. Add the line immediately after the `weather: ...` line in each:

`src-tauri/src/gui.rs:456` (test `base_config`) — after `weather: WeatherConfig::default(),`:
```rust
            weather: WeatherConfig::default(),
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
        }
```

`src-tauri/src/daemon.rs` (test `base_config`, ~line 840) — find its `weather:` line and add after it:
```rust
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
```

`src-tauri/src/state.rs` (test `mock_config`, ~line 99) — after its `weather:` line:
```rust
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
```

`src-tauri/src/main.rs` (~line 587) — after its `weather:` line:
```rust
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
```

> If any of these literals does NOT currently set `weather`, add `font_sizes` right before the closing `}` of the literal instead. Use `grep -n "AppConfig {" src-tauri/src/*.rs` and inspect each.

- [ ] **Step 7: Run the build + tests**

Run: `cd src-tauri && cargo test --lib settings::tests::test_appconfig_font_sizes`
Expected: PASS. Also run `cargo build` and confirm no "missing field font_sizes" errors anywhere.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/settings.rs src-tauri/src/gui.rs src-tauri/src/daemon.rs src-tauri/src/state.rs src-tauri/src/main.rs
git commit -m "feat(config): add per-line font_sizes to AppConfig"
```

---

## Task 3: Persist `font_lineN` keys in `apply_main_section`

**Files:**
- Modify: `src-tauri/src/gui.rs:168-180`
- Test: `src-tauri/src/gui.rs` tests

- [ ] **Step 1: Write the failing test**

Add to `src-tauri/src/gui.rs` `mod tests`:

```rust
#[test]
fn test_apply_main_section_writes_font_sizes() {
    use crate::render::FontSize;
    let mut config = base_config();
    config.font_sizes[0] = FontSize::Large;
    config.font_sizes[1] = FontSize::Small;
    let mut ini = Ini::new();
    apply_main_section(&mut ini, &config);
    let main = ini.section(Some("Main")).unwrap();
    assert_eq!(main.get("font_line1"), Some("large"));
    assert_eq!(main.get("font_line2"), Some("small"));
    assert_eq!(main.get("font_line3"), Some("medium"));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd src-tauri && cargo test --lib gui::tests::test_apply_main_section_writes_font_sizes`
Expected: FAIL — `main.get("font_line1")` returns `None`.

- [ ] **Step 3: Write the font keys**

In `src-tauri/src/gui.rs`, at the end of `apply_main_section` (after the existing `.set("gpu", &config.gpu);` chain closes at line 179), add:

```rust
    {
        let mut sec = ini.with_section(Some("Main"));
        for (i, fs) in config.font_sizes.iter().enumerate() {
            sec.set(format!("font_line{}", i + 1), fs.as_str());
        }
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cd src-tauri && cargo test --lib gui::tests::test_apply_main_section_writes_font_sizes`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/gui.rs
git commit -m "feat(config): persist font_lineN keys in [Main]"
```

---

## Task 4: Per-line rendering in `render_text_to_oled`

**Files:**
- Modify: `src-tauri/src/render.rs:101-127` (function), update all in-crate callers + tests
- Modify callers: `src-tauri/src/daemon.rs:84-98` and `:646`, `:686`; `src-tauri/src/gui.rs:350`, `:378`, `:385`
- Test: `src-tauri/src/render.rs`, `src-tauri/src/daemon.rs`

- [ ] **Step 1: Write the failing tests (render.rs)**

Add to `src-tauri/src/render.rs` `mod tests`:

```rust
#[test]
fn test_render_small_vs_large_differ() {
    let small = render_text_to_oled("Hello", 0, &[FontSize::Small]);
    let large = render_text_to_oled("Hello", 0, &[FontSize::Large]);
    assert_ne!(small.data, large.data);
}

#[test]
fn test_render_mixed_line_fonts_no_panic_and_lit() {
    let buf = render_text_to_oled(
        "Big\nsmall\nmed",
        0,
        &[FontSize::Large, FontSize::Small, FontSize::Medium],
    );
    assert!(buf.data.iter().any(|&b| b != 0));
}

#[test]
fn test_render_fewer_fonts_than_lines_falls_back_medium() {
    // Only one font provided for three lines; must not panic.
    let buf = render_text_to_oled("a\nb\nc", 0, &[FontSize::Small]);
    assert!(buf.data.iter().any(|&b| b != 0));
}
```

- [ ] **Step 2: Update existing render tests to the new signature, then run to confirm failure**

In `src-tauri/src/render.rs` tests, add `, &[]` as the third argument to every existing `render_text_to_oled(...)` call. Specifically these call sites:
- `render_text_to_oled("", 0)` → `render_text_to_oled("", 0, &[])`
- `render_text_to_oled("Hello", 0)` → `render_text_to_oled("Hello", 0, &[])`
- `render_text_to_oled("Line1\nLine2\nLine3", 0)` → add `, &[]`
- `render_text_to_oled("🔥 Hot", 0)` → add `, &[]`
- `render_text_to_oled("Test", 0)` → add `, &[]`
- `render_text_to_oled("A", 0)` (both, in `test_render_text_with_x_offset`) → add `, &[]`
- `render_text_to_oled("A", 50)` → add `, &[]`
- `render_text_to_oled(&text, 0)` (in `test_render_text_img_directive_invokes_loader`) → add `, &[]`

Run: `cd src-tauri && cargo test --lib render::tests::test_render`
Expected: FAIL — `render_text_to_oled` takes 2 arguments (the function itself isn't updated yet).

- [ ] **Step 3: Rework the function**

Replace the body of `render_text_to_oled` in `src-tauri/src/render.rs` (lines 101-127) with:

```rust
pub fn render_text_to_oled(text: &str, x: i32, line_fonts: &[FontSize]) -> OledBuffer {
    let mut buffer = OledBuffer::new();

    let mut y = 0;
    for (i, line) in text.lines().enumerate() {
        let fs = line_fonts.get(i).copied().unwrap_or(FontSize::Medium);
        if i == 0 {
            y = fs.first_baseline();
        } else {
            y += fs.line_advance();
        }

        let style = MonoTextStyle::new(fs.font(), BinaryColor::On);
        let mut current_x = x;

        if let Some(rest) = line.strip_prefix("IMG:") {
            let path = rest.trim();
            let img_y = (y - 10).max(0) as u32;
            let _ = load_image_to_buffer(path, &mut buffer, current_x as u32, img_y);
        } else {
            if let Some(icon_data) = get_emoji_icon(line) {
                let raw_image = ImageRaw::<BinaryColor>::new(icon_data, 8);
                let image = Image::new(&raw_image, Point::new(current_x, y - 8));
                let _ = image.draw(&mut buffer);
                current_x += 10;
            }

            let clean_line: String = line.chars().filter(|c| !c.is_emoji()).collect();
            let _ = Text::new(clean_line.trim(), Point::new(current_x, y), style).draw(&mut buffer);
        }
    }

    buffer
}
```

Note: the top-level `use profont::PROFONT_12_POINT;` (now part of the merged import from Task 1) is still referenced via `FontSize::font()`, so no further import change is needed here. The old standalone `let style = MonoTextStyle::new(&PROFONT_12_POINT, ...)` line is removed — `PROFONT_12_POINT` is now only used inside `FontSize::font()`.

- [ ] **Step 4: Update daemon `value_to_oled_buffer` signature + callers**

In `src-tauri/src/daemon.rs`, add the import (top of file, near `use crate::render::{render_text_to_oled, OledBuffer};` at line 5):

```rust
use crate::render::{render_text_to_oled, FontSize, OledBuffer};
```

Change the function signature + final call (lines 84 and 97):

```rust
fn value_to_oled_buffer(value: &Value, font_sizes: &[FontSize]) -> OledBuffer {
```

and the last line of that function:

```rust
    render_text_to_oled(&text, 0, font_sizes)
```

Update the two production callers:
- Line ~646: `let buffer = value_to_oled_buffer(&value);` → `let buffer = value_to_oled_buffer(&value, &self.config.font_sizes);`
- Line ~686: `let buffer = value_to_oled_buffer(&value);` → `let buffer = value_to_oled_buffer(&value, &self.config.font_sizes);`

Update the two daemon tests that call it:
- `test_value_to_oled_buffer_populates_text`: `value_to_oled_buffer(&v)` → `value_to_oled_buffer(&v, &[])`
- `test_value_to_oled_buffer_with_non_object_value`: `value_to_oled_buffer(&v)` → `value_to_oled_buffer(&v, &[])`

- [ ] **Step 5: Update gui.rs preview call sites**

In `src-tauri/src/gui.rs` `preview_config_impl`:
- Line 350 (`"HWiNFO not\nconnected"`): `render_text_to_oled("HWiNFO not\nconnected", 0)` → `render_text_to_oled("HWiNFO not\nconnected", 0, &[])`
- Line 378 (`"Preview error:..."`): add `, &[]`
- Line 385 (real preview): `render_text_to_oled(&value_to_preview_text(&value), 0)` → `render_text_to_oled(&value_to_preview_text(&value), 0, &config.font_sizes)`

- [ ] **Step 6: Run the full test suite**

Run: `cd src-tauri && cargo test`
Expected: PASS, including the new render tests and all updated call sites. No compile errors.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/render.rs src-tauri/src/daemon.rs src-tauri/src/gui.rs
git commit -m "feat(render): per-line font sizes with stacked baseline layout"
```

---

## Task 5: GUI section in index.html

**Files:**
- Modify: `src-tauri/ui/index.html` — markup after the Weather section (~line 399), and JS in `updateUI` (~853), `updateDevicePickerVisibility` (~897), `btn-save` (~989).

- [ ] **Step 1: Add the markup**

In `src-tauri/ui/index.html`, immediately after the Weather section's closing `</div>` (the one at line 399, right before `<div id="custom-sensors-section" ...>`), insert:

```html
    <div id="font-size-section" class="section">
        <span class="section-title">Line Font Sizes (Direct USB)</span>
        <div class="grid">
            <label for="font-line1">Line 1</label>
            <select id="font-line1" class="font-line-select"></select>
            <label for="font-line2">Line 2</label>
            <select id="font-line2" class="font-line-select"></select>
            <label for="font-line3">Line 3</label>
            <select id="font-line3" class="font-line-select"></select>
            <label for="font-line4">Line 4</label>
            <select id="font-line4" class="font-line-select"></select>
            <label for="font-line5">Line 5</label>
            <select id="font-line5" class="font-line-select"></select>
        </div>
    </div>
```

- [ ] **Step 2: Add helpers + populate/load in JS**

In `src-tauri/ui/index.html`, add these functions next to `updateWeatherFieldsVisibility` (after line 882):

```javascript
        const FONT_LINES = 5;
        function populateFontSelects() {
            for (let i = 1; i <= FONT_LINES; i++) {
                const sel = document.getElementById('font-line' + i);
                if (sel.options.length) continue;
                for (const v of ['small', 'medium', 'large']) {
                    const opt = document.createElement('option');
                    opt.value = v;
                    opt.textContent = v.charAt(0).toUpperCase() + v.slice(1);
                    sel.appendChild(opt);
                }
            }
        }

        function updateFontSectionVisibility() {
            const on = !!document.getElementById('direct-usb').checked;
            document.getElementById('font-size-section').style.display = on ? 'block' : 'none';
        }
```

Then in `updateUI` (after the weather block, after line 853 `updateWeatherFieldsVisibility();`), add:

```javascript
            populateFontSelects();
            if (!Array.isArray(config.font_sizes) || config.font_sizes.length !== FONT_LINES) {
                config.font_sizes = ['medium', 'medium', 'medium', 'medium', 'medium'];
            }
            for (let i = 1; i <= FONT_LINES; i++) {
                document.getElementById('font-line' + i).value = config.font_sizes[i - 1] || 'medium';
            }
            updateFontSectionVisibility();
```

- [ ] **Step 3: Gate visibility on the direct-USB toggle**

In `updateDevicePickerVisibility` (line 897-901), add a call to the font section toggle so it tracks the same checkbox. Change the function to:

```javascript
        function updateDevicePickerVisibility() {
            const visible = !!document.getElementById('direct-usb').checked;
            document.getElementById('device-select-label').style.display = visible ? 'block' : 'none';
            document.getElementById('device-select').style.display = visible ? 'block' : 'none';
            updateFontSectionVisibility();
        }
```

(The `direct-usb` change listener at line 886 already calls `updateDevicePickerVisibility()`, so toggling the checkbox now also shows/hides the font section.)

- [ ] **Step 4: Collect values on save**

In the `btn-save` handler, after the `config.weather = { ... };` block (after line 989), add:

```javascript
            config.font_sizes = [];
            for (let i = 1; i <= FONT_LINES; i++) {
                config.font_sizes.push(document.getElementById('font-line' + i).value || 'medium');
            }
```

- [ ] **Step 5: Manual verification (no automated UI test)**

Run: `cd src-tauri && cargo build` to confirm the assets still compile in. Then `cargo run`, and in the GUI:
1. Confirm the "Line Font Sizes (Direct USB)" section is **hidden** when Direct USB is unchecked.
2. Check Direct USB → section appears with 5 dropdowns defaulting to Medium.
3. Set Line 1 = Large, Line 2 = Small, Save, click Reload → values persist.
4. Open `conf.ini` and confirm `[Main]` has `font_line1=large`, `font_line2=small`.
5. With Direct USB + Custom style, confirm the live preview reflects the larger/smaller lines.

State explicitly in your report whether the GUI was actually exercised or only built.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/ui/index.html
git commit -m "feat(gui): per-line font size selectors (direct-USB only)"
```

---

## Task 6: Documentation + final verification

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Document the feature**

In `README.md`, find the direct-USB / configuration documentation and add a short subsection describing per-line font sizes. Use this content (place it near the direct-USB or `[Main]` config docs):

```markdown
### Per-Line Font Size (Direct USB)

When using **Direct USB** output, each display line can use a different font size:

- **Small** (9pt), **Medium** (12pt, default), **Large** (18pt)

Set these in the GUI under "Line Font Sizes (Direct USB)" (visible only in Direct USB
mode), or directly in `conf.ini`:

```ini
[Main]
font_line1 = large
font_line2 = medium
font_line3 = small
font_line4 = medium
font_line5 = medium
```

Font size only affects the Direct USB render path. In GameSense mode, SteelSeries GG
controls the font and this setting has no effect. Large fonts reduce how many lines
fit on the 128x64 screen; lines past the bottom edge are clipped.
```

- [ ] **Step 2: Run the full verification suite**

```bash
cd src-tauri && cargo fmt && cargo test && cargo clippy --all-targets -- -D warnings
```

Expected: `cargo test` all green; `cargo clippy` no warnings (in particular, no dead-code warning for `FontSize::font`/`first_baseline`/`line_advance` now that Task 4 uses them).

- [ ] **Step 3: Commit**

```bash
git add README.md src-tauri/src
git commit -m "docs: document per-line font size for direct-USB"
```

---

## Self-Review Notes

- **Spec coverage:** FontSize enum (Task 1); config field + INI parse + default (Task 2); persistence (Task 3); render rework + all call sites + GameSense-untouched since only the bitmap path changes (Task 4); GUI section gated to direct-USB with load/save (Task 5); docs + tests (Tasks 1–6). All spec sections mapped.
- **No regression invariant:** Medium `first_baseline=10` / `line_advance=12` reproduce the original `y=10; y+=12` exactly, so pure-Medium configs render byte-identically (covered implicitly by the unchanged existing render tests, which now pass `&[]` → all Medium).
- **Type consistency:** `FontSize` (render.rs) used uniformly; `from_config_str`/`as_str` names stable across settings.rs, gui.rs; `font_sizes: [FontSize; DISPLAY_LINES]` consistent across all struct literals; `value_to_oled_buffer(&Value, &[FontSize])` matches all callers; JS `config.font_sizes` is a length-5 lowercase string array, matching serde `rename_all="lowercase"` + `[FontSize;5]`.
- **Backwards compat:** `#[serde(default = "default_font_sizes")]` covers missing JSON field; `from_config_str` defaults unknown→Medium; INI missing keys→Medium.
```
