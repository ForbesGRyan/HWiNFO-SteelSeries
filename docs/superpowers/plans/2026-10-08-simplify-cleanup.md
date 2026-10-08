# Simplify & Cleanup Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Remove round-about data handling, dead code and test-only scaffolding from the app without changing what users see on the OLED or in the settings window.

**Architecture:** HWiNFO shared memory is parsed from a byte slice at fixed offsets into a cheap-to-clone `Snapshot` (shared names + a `Vec<f64>`). Config stays typed (`AppConfig.custom_sensors`) all the way to rendering; display frames are `Vec<String>` lines and become JSON only at the GameSense call. The daemon's outer loop is the only retry loop.

**Tech Stack:** Rust 2021, Tauri 2, winapi (file mapping), hidapi, gamesense 0.1.2, rust-ini, serde_json.

**Spec:** None. This plan implements the code-review findings from the 2026-10-08 session; the reasoning for each change is stated in the task that makes it.

## Global Constraints

- Windows-only. Work in the worktree `.claude/worktrees/simplify-cleanup` on branch `worktree-simplify-cleanup` (stacked on `feat/device-discovery`, PR #12).
- No new dependencies. Dependencies removed by this plan: `rusb`, `tauri-utils`, `dialoguer`, `strum`.
- `conf.ini` format is unchanged. Existing config files must load identically.
- The UI contract is unchanged: `AppConfig` JSON shape, `StatusPayload`, `OledFrame`, the `status`/`frame` events, and the error substring `not yet supported for direct USB` (matched in `src-tauri/ui/index.html`).
- `mouse_battery.rs` is out of scope apart from one dead function (Task 1).
- Prefix cargo commands with `rtk`. Use plain `git` (the worktree session rejects `rtk git`).
- Test command (run from the worktree root): `CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins`. Without the env var one test blocks on stdin.
- Baseline before Task 1: 540 passed, 1 failed. The one failure, `connect::tests::test_connect_hwinfo_bounded_returns_err_when_service_down`, fails only because HWiNFO is running on the dev machine. It is deleted in Task 4; until then it is the only acceptable failure.
- Do not kill the process `hwinfo-steelseries-oled` (no underscore): that is the user's real tray app. Test binaries are named `hwinfo_steelseries_oled-<hash>`.
- End every commit message with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

## Review Focus

1. **HWiNFO is closed and reopened while the app runs.** The app shows "Disconnected FROM HWiNFO" and then recovers by itself, no restart. Pinned by `test_render_recovers_after_stale` (Task 3) and the manual check in Task 8.
2. **A sensor is added or removed in HWiNFO while the app runs.** No panic; the new layout is picked up on the next pull. Pinned by `refresh_rebuilds_layout_when_reading_added` (Task 3).
3. **Shared memory is truncated or its header holds garbage counts.** `Hwinfo::new`/`pull` return `Err`; no out-of-bounds read, no hang. Pinned by `load_rejects_truncated_memory` and `load_rejects_garbage_counts` (Task 3).
4. **`conf.ini` exists but is malformed at startup.** The app reports the error and leaves the file untouched (the old wizard would overwrite it). Pinned by `load_or_create_config_errors_on_malformed_ini_and_keeps_file` (Task 2).
5. **`sensors_per_line` in `conf.ini` is larger than the slots allow (e.g. `4` → 20 slots, only 15 exist).** Today this panics the daemon thread on an index out of bounds. After Task 5 the extra slots render empty. Pinned by `test_format_custom_lines_survives_oversized_sensors_per_line` (Task 5).

## Deliberately not in this plan

- Emoji icon path and `IMG:` line prefix in `render.rs`: live behavior for hand-edited configs; removing it is a product decision.
- GameSense events `BLANK`/`WHITE`/`ERROR` are triggered but never bound. Suspected bug, not verified on hardware.
- `Daemon<R: Runtime>` stays generic: that is Tauri's own test pattern (`MockRuntime`) and costs nothing at runtime.
- `gg_connected`/`usb_connected`/`active_mode` stay as three fields: the UI reads all three.

## File Structure

| File | After this plan |
|---|---|
| `src-tauri/src/lib.rs` | Rewritten. `Hwinfo` (mapping + pull), `Snapshot` (names + values), slice-based parser, tests on synthetic buffers. |
| `src-tauri/src/settings.rs` | `AppConfig`/`WeatherConfig` parsing plus `load_or_create_config`. Wizard removed. |
| `src-tauri/src/utils.rs` | `run_sensors(&[CustomSensor], ..) -> Vec<Slot>`, `format_custom_lines(..) -> Vec<String>`. |
| `src-tauri/src/daemon.rs` | Summary formatting, `DisplayDriver` with two real impls, `Daemon` loop. No retry helpers, no ini access except `load_config`. |
| `src-tauri/src/connect.rs` | HID discovery/selection and single-attempt connects. |
| `src-tauri/src/gui.rs` | Tauri commands; preview uses typed sensors directly. |
| `src-tauri/src/main.rs` | Startup, tray, CLI flags. |
| `src-tauri/src/consts.rs` | Three constants. |
| `src-tauri/src/console_utils.rs` | `console_window` only. |

---

### Task 1: Delete dead code, unused dependencies and tracked build output

Pure deletion. Nothing here is reachable from the running app.

**Files:**
- Modify: `src-tauri/Cargo.toml`, `src-tauri/src/console_utils.rs`, `src-tauri/src/render.rs`, `src-tauri/src/mouse_battery.rs`, `src-tauri/src/connect.rs`, `src-tauri/src/consts.rs`, `src-tauri/src/daemon.rs`, `src-tauri/src/gui.rs`, `src-tauri/src/main.rs`
- Delete: `examples/steelseries-json.rs`
- Untrack: `target/.rustc_info.json`, `target/CACHEDIR.TAG`

**Interfaces:**
- Consumes: nothing.
- Produces: nothing new. Removes Tauri commands `request_sleep`, `request_wake`, `request_white_screen` (the UI never invokes them; the tray writes `SharedState.sleep_requested` directly).

- [ ] **Step 1: Confirm each target is unreferenced**

Run from the worktree root:

```bash
grep -rn "rusb\|tauri_utils" src-tauri/src
grep -n "request_sleep\|request_wake\|request_white_screen" src-tauri/ui/index.html
grep -rn "display_value_in_console\|to_page_major\|count_likely_battery_results\|HID_PRODUCT_ID\|HID_INTERFACE_NUMBER\|_arc_assert" src-tauri/src | grep -v "fn test_\|#\[test\]"
```

Expected: the first two print nothing. The third prints only definitions and uses inside `mod tests`. If anything else shows up, stop and report it instead of deleting.

- [ ] **Step 2: Remove unused dependencies**

In `src-tauri/Cargo.toml` delete these two lines:

```toml
rusb = "0.9"
tauri-utils = "2.0"
```

- [ ] **Step 3: Delete dead functions and the tests that exist only for them**

| File | Delete |
|---|---|
| `console_utils.rs` | `display_value_in_console`; the imports `console::Term`, `serde_json::Value`, `crate::consts::DISPLAY_LINES`; tests `test_display_value_json_structure`, `test_display_value_json_line_access`, `test_display_value_with_special_characters`, `test_display_value_with_empty_lines`, `test_display_value_numeric_content`, `test_display_value_in_console_executes`, `test_display_lines_constant_usage`, the comment banner above them, and any test-module import left unused |
| `render.rs` | `OledBuffer::to_page_major` with its doc comment and `#[allow(dead_code)]`; tests `test_to_page_major_length_and_ordering`, `test_to_page_major_empty_buffer_is_zeroes` and their banner comment |
| `mouse_battery.rs` | `count_likely_battery_results` with its doc comment and attribute; test `test_count_likely_battery_results` |
| `connect.rs` | consts `HID_PRODUCT_ID`, `HID_INTERFACE_NUMBER` with their attributes; tests `test_hid_vendor_id_is_steelseries`, `test_hid_product_id`, `test_hid_interface_number`, `test_hid_usage_page` (they assert constants equal themselves) |
| `consts.rs` | test `test_constants` |
| `daemon.rs` | `_arc_assert` with its comment and attribute; test `test_arc_assert_compiles`; the top-level `use std::sync::Arc;` |
| `gui.rs` | `set_sleep_command`, `request_sleep`, `request_wake`, `request_white_screen`; test `test_set_sleep_command_writes_state`; the three `request_*` assertions inside `test_command_wrappers_via_tauri_state`; `SleepCommand` from the top-level `use crate::state::{...}` only if the compiler reports it unused (`save_config_impl` still uses it) |
| `main.rs` | the three lines `gui::request_sleep,`, `gui::request_wake,`, `gui::request_white_screen,` inside `tauri::generate_handler![...]` |

- [ ] **Step 4: Delete the orphan example and untrack build output**

```bash
git rm examples/steelseries-json.rs
git rm --cached target/.rustc_info.json target/CACHEDIR.TAG
```

`/target/` is already in `.gitignore`.

- [ ] **Step 5: Build, lint, test**

```bash
rtk cargo build --manifest-path src-tauri/Cargo.toml
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
```

Expected: build succeeds, clippy reports no new warnings, tests show 1 failed (the known one) and 0 other failures.

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri Cargo.lock examples target
git commit -m "chore: remove dead code, unused deps and tracked build output

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 2: Replace the console wizard with a default config

The console wizard duplicates the settings window, blocks startup until HWiNFO is reachable, and overwrites a malformed `conf.ini`. On first run the app now writes a minimal config and opens the settings window.

**Files:**
- Modify: `src-tauri/src/settings.rs`, `src-tauri/src/main.rs`, `src-tauri/src/consts.rs`, `src-tauri/Cargo.toml`, `README.md`
- Test: `src-tauri/src/settings.rs` (`mod tests`)

**Interfaces:**
- Consumes: `AppConfig::from_ini(&Ini) -> Result<AppConfig, anyhow::Error>` (existing).
- Produces: `settings::load_or_create_config(path: &str) -> Result<(AppConfig, bool), anyhow::Error>`; the bool is `true` when the file was just created. Removes `settings::settings_create_config`, `consts::Style`. Adds `#[derive(Default)]` to `CustomSensor` (Task 5 relies on it).

- [ ] **Step 1: Write the failing tests**

Add to `mod tests` in `src-tauri/src/settings.rs`:

```rust
    fn temp_ini(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("hwinfo_ss_{}_{}.ini", name, std::process::id()));
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn load_or_create_config_writes_default_when_missing() {
        let path = temp_ini("first_run");
        let (cfg, created) = load_or_create_config(path.to_str().unwrap()).unwrap();
        assert!(created);
        assert!(cfg.is_summary);
        assert!(cfg.is_vertical);
        assert!(!cfg.direct_usb);
        // The file now exists and loads as an existing config.
        let (again, created_again) = load_or_create_config(path.to_str().unwrap()).unwrap();
        assert!(!created_again);
        assert!(again.is_summary);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_or_create_config_loads_existing_without_rewriting() {
        let path = temp_ini("existing");
        std::fs::write(&path, "[Main]\nstyle=Horizontal\n").unwrap();
        let (cfg, created) = load_or_create_config(path.to_str().unwrap()).unwrap();
        assert!(!created);
        assert!(cfg.is_summary);
        assert!(!cfg.is_vertical);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[Main]\nstyle=Horizontal\n");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_or_create_config_errors_on_malformed_ini_and_keeps_file() {
        let path = temp_ini("malformed");
        std::fs::write(&path, "[Main\nstyle=Vertical\n").unwrap();
        assert!(load_or_create_config(path.to_str().unwrap()).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[Main\nstyle=Vertical\n");
        let _ = std::fs::remove_file(&path);
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins load_or_create_config
```

Expected: compile error, `cannot find function load_or_create_config`.

- [ ] **Step 3: Delete the wizard**

In `src-tauri/src/settings.rs` delete everything from `/// Map the "Choose style" menu input` (`style_from_choice`) through the end of `settings_create_config`: `style_from_choice`, `direct_usb_from_choice`, `validate_lines`, `validate_sensors_per_line`, `pick_unit`, `validate_category_selection`, `validate_reading_selection`, `format_sensor_id`, `get_default_unit`, `configure_gpu_selection`, `configure_custom_sensors`, `settings_create_config`.

Delete these tests and helpers: `test_style_from_choice_valid`, `test_style_from_choice_invalid`, `test_direct_usb_from_choice_gamesense`, `test_direct_usb_from_choice_direct_usb`, `test_direct_usb_from_choice_invalid_defaults_to_gamesense`, `test_validate_lines_in_range`, `test_validate_lines_below_range_defaults_to_three`, `test_validate_lines_above_range_defaults_to_three`, `test_validate_sensors_per_line_in_range`, `test_validate_sensors_per_line_out_of_range`, `test_pick_unit_uses_input_when_non_empty`, `test_pick_unit_falls_back_to_default_when_empty`, `test_pick_unit_empty_default_and_empty_input`, `test_validate_category_selection_valid`, `test_validate_category_selection_out_of_range`, `test_validate_category_selection_empty_list`, `test_validate_reading_selection_valid`, `test_validate_reading_selection_out_of_range`, `test_format_sensor_id`, `test_get_default_unit_known_types`, `test_get_default_unit_unknown_types_empty`, `build_hwinfo_with_gpus`, `test_configure_gpu_selection_single_gpu_returns_ok_without_prompt`, `test_configure_gpu_selection_multi_gpu_errors_on_stdin_eof`, `test_configure_gpu_selection_no_gpu_returns_err`, `test_configure_custom_sensors_errors_when_category_out_of_range`, `test_configure_custom_sensors_errors_when_sensor_missing_from_hashmap`, `test_configure_custom_sensors_errors_on_stdin_eof`, `test_settings_create_config_errors_when_stdin_eof`.

Replace the import block at the top of the file with:

```rust
use crate::consts::{CUSTOM_SENSORS, DISPLAY_LINES};
use crate::render::FontSize;
use crate::weather::Units;
use anyhow::anyhow;
use ini::Ini;
use log::{debug, warn};
```

Remove any test-module import the compiler then reports as unused.

- [ ] **Step 4: Add `load_or_create_config`, derive `Default`, drop the `println!` noise**

Change the derive on `CustomSensor` to:

```rust
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CustomSensor {
```

In `AppConfig::from_ini`:
- delete `println!("AppConfig: Reading from ini...");`
- delete the whole `if main.is_none() { ... }` block (the `ok_or_else` below it already reports the error)
- replace `println!("AppConfig warning: 'style' key not found, defaulting to 'vertical'");` with `warn!("'style' key not found in conf.ini, defaulting to 'vertical'");`
- replace the `println!("AppConfig: Loaded main settings. ...", ...)` call with `debug!("Loaded main settings: is_summary={}, direct_usb={}", is_summary, direct_usb);`
- replace both five-field `CustomSensor { sensor: String::new(), ... }` padding literals inside the `while page_sensors.len() < k` loop with `CustomSensor::default()`

Add after the `impl AppConfig` block:

```rust
/// Load `path`, or on first run write a minimal config there. Returns the
/// config and whether it was just created, so the caller can open the settings
/// window. A file that exists but cannot be read or parsed is an error and is
/// left untouched.
pub fn load_or_create_config(path: &str) -> Result<(AppConfig, bool), anyhow::Error> {
    match Ini::load_from_file(path) {
        Ok(ini) => Ok((AppConfig::from_ini(&ini)?, false)),
        Err(ini::Error::Io(e)) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut ini = Ini::new();
            ini.with_section(Some("Main")).set("style", "Vertical");
            ini.write_to_file(path)
                .map_err(|e| anyhow!("Failed to write {}: {}", path, e))?;
            Ok((AppConfig::from_ini(&ini)?, true))
        }
        Err(e) => Err(anyhow!("Failed to read {}: {}", path, e)),
    }
}
```

- [ ] **Step 5: Use it in `main.rs`**

Delete `load_or_create_config_at` and `load_or_create_config`. Delete tests `test_load_or_create_config_at_loads_existing_ini` and `test_load_or_create_config_reads_existing_ini`.

Change the imports: `use settings::{settings_create_config, AppConfig};` becomes nothing at top level (delete the line), `use connect::connect_hwinfo;` is deleted, `use ini::Ini;` is deleted, and `warn` is dropped from `use log::{...}` if the compiler reports it unused. Add `use crate::settings::AppConfig;` inside `mod tests` (the `fresh_shared_for_test` helper needs it).

In `main()` replace

```rust
    handle_discovery_flags(&args);
    let open_settings = wants_open_settings(&args);

    info!("Starting HWiNFO-SteelSeries");
    let term = Term::stdout();

    let config = match load_or_create_config(&term) {
        Ok(c) => c,
        Err(e) => return Err(handle_fatal_error(&term, e)),
    };
```

with

```rust
    handle_discovery_flags(&args);

    info!("Starting HWiNFO-SteelSeries");
    let term = Term::stdout();

    let (config, first_run) = match settings::load_or_create_config("conf.ini") {
        Ok(loaded) => loaded,
        Err(e) => return Err(handle_fatal_error(&term, e)),
    };
    // First run has nothing configured yet: show the settings window.
    let open_settings = wants_open_settings(&args) || first_run;
```

- [ ] **Step 6: Remove `Style` and `dialoguer`**

Confirm `Style` is unused: `grep -rn "Style::\|consts::Style\|{Style" src-tauri/src` must print only lines in `consts.rs`. Then in `src-tauri/src/consts.rs` delete the `Style` enum, its `Display` impl and the whole `mod tests` (its remaining tests are all `Style` tests). The file ends up as:

```rust
pub const CUSTOM_SENSORS: usize = 15;
pub const DISPLAY_LINES: usize = 5;
pub const TICK_RATE: u64 = 500;
```

In `src-tauri/Cargo.toml` delete `dialoguer = "0.12"`.

- [ ] **Step 7: Update the README**

Run `grep -n -i "wizard\|config not found\|choose style\|first run\|first launch" README.md`. Replace each passage that describes the interactive console setup with:

```markdown
On first launch the app writes a default `conf.ini` (vertical summary, SteelSeries GG mode) and opens the settings window. Pick your layout, sensors and connection type there and press Save. `conf.ini` can also be edited by hand; use **Reload Config** in the tray menu afterwards.
```

- [ ] **Step 8: Run the tests**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Expected: the three new tests pass; 1 known failure; no new clippy warnings.

- [ ] **Step 9: Commit**

```bash
git add -A src-tauri Cargo.lock README.md
git commit -m "refactor: replace console wizard with default config and settings window

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 3: Rewrite the HWiNFO reader around `Snapshot`

Today `pull()` remaps the file mapping every tick, allocates three strings and copies 460 bytes per reading, and the daemon deep-clones the whole `Hwinfo` twice per tick to detect staleness. The header struct has no `#[repr]`, so it works only because of how rustc happens to order fields.

New design: one persistent mapping; fields read from a byte slice at fixed offsets (no `#[repr]` structs, bounds validated once per pull); names parsed once and shared through an `Arc`; values in a `Vec<f64>`; staleness from the header's `poll_time`.

**Files:**
- Rewrite: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/daemon.rs`, `src-tauri/src/utils.rs`, `src-tauri/src/gui.rs`, `src-tauri/src/state.rs`, `src-tauri/Cargo.toml`
- Test: `src-tauri/src/lib.rs` (`mod tests`), `src-tauri/src/daemon.rs` (`mod tests`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces (crate `hwinfo_steelseries_oled`):
  - `Hwinfo::new() -> anyhow::Result<Hwinfo>`
  - `Hwinfo::pull(&mut self) -> anyhow::Result<bool>` (true when HWiNFO has polled since the previous pull; values are refreshed either way)
  - `Hwinfo::snapshot(&self) -> &Snapshot`
  - `Snapshot: Clone + Default`
  - `Snapshot::from_readings(&[(&str, &str, f64)]) -> Snapshot` (sensor, reading, value)
  - `Snapshot::get(&self, sensor: &str, reading: &str) -> Option<f64>`
  - `Snapshot::find_first(&self, reading: &str) -> Option<f64>` (first match in HWiNFO's order)
  - `Snapshot::readings(&self) -> impl Iterator<Item = (&str, &str, f64)>`
- Produces (daemon): `Daemon::render(&mut self, hwinfo: &Snapshot, fresh: bool) -> Result<(), anyhow::Error>`, `is_stale(fresh: bool, count: &mut usize, limit: usize) -> bool`, `const STALE_LIMIT: usize = 5`.
- Removes: `HwinfoSensorsReadingElement`, `HwinfoSensorsSensorElement`, `Sensor`, `SensorReadingType`, `Hwinfo::find`, `Hwinfo::new_mock`, `bypass_pull_for_test`, `set_shared_memory_name_for_test`.

Behavior notes for the implementer:
- `find_first` used to walk a `HashMap` (arbitrary order). It now returns the first match in HWiNFO's own order, which is deterministic.
- A reading renamed inside HWiNFO is picked up on reconnect or **Reload Config**, not on the next tick. Adding or removing sensors is picked up on the next tick.

- [ ] **Step 1: Replace `src-tauri/src/lib.rs` with the new implementation and its tests**

Write this as the complete file:

```rust
//! HWiNFO shared-memory reader.
//!
//! HWiNFO publishes its sensors in the `Global\HWiNFO_SENS_SM2` file mapping:
//! a header, a table of sensor elements and a table of reading elements. The
//! structs are packed and little-endian, so fields are read from the mapped
//! bytes at fixed offsets rather than through `#[repr]` structs.

use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use winapi::ctypes::c_void;
use winapi::um::handleapi::CloseHandle;
use winapi::um::memoryapi::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ,
};
use winapi::um::winnt::MEMORY_BASIC_INFORMATION;

const MAP_NAME: &str = "Global\\HWiNFO_SENS_SM2";

// Header: poll time, then (offset, element size, element count) per table.
const HDR_POLL_TIME: usize = 12;
const HDR_SENSORS: usize = 20;
const HDR_READINGS: usize = 32;
// Sensor element: user-visible name, ANSI and (newer HWiNFO) UTF-8.
const SENSOR_NAME: usize = 136;
const SENSOR_NAME_UTF8: usize = 264;
// Reading element.
const READING_SENSOR_INDEX: usize = 4;
const READING_LABEL: usize = 140;
const READING_VALUE: usize = 284;
const READING_LABEL_UTF8: usize = 316;
const STRING_LEN: usize = 128;

fn bytes_at<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N]> {
    bytes
        .get(offset..)
        .and_then(|rest| rest.get(..N))
        .and_then(|field| field.try_into().ok())
        .ok_or_else(|| anyhow!("HWiNFO shared memory truncated at offset {}", offset))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(bytes_at(bytes, offset)?) as usize)
}

/// One element table inside the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Section {
    offset: usize,
    size: usize,
    count: usize,
}

impl Section {
    /// Read a table descriptor and check the whole table lies inside `bytes`
    /// with elements of at least `min_size`, so later indexing cannot go out
    /// of bounds.
    fn parse(bytes: &[u8], offset: usize, min_size: usize) -> Result<Self> {
        let section = Section {
            offset: u32_at(bytes, offset)?,
            size: u32_at(bytes, offset + 4)?,
            count: u32_at(bytes, offset + 8)?,
        };
        let end = section
            .count
            .checked_mul(section.size)
            .and_then(|len| len.checked_add(section.offset));
        if section.size < min_size || end.is_none_or(|end| end > bytes.len()) {
            bail!(
                "HWiNFO shared memory layout invalid: {:?} in {} bytes",
                section,
                bytes.len()
            );
        }
        Ok(section)
    }

    fn elements<'a>(&self, bytes: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
        bytes[self.offset..self.offset + self.count * self.size].chunks_exact(self.size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Header {
    poll_time: i64,
    sensors: Section,
    readings: Section,
}

impl Header {
    fn parse(bytes: &[u8]) -> Result<Self> {
        Ok(Header {
            poll_time: i64::from_le_bytes(bytes_at(bytes, HDR_POLL_TIME)?),
            sensors: Section::parse(bytes, HDR_SENSORS, SENSOR_NAME + STRING_LEN)?,
            readings: Section::parse(bytes, HDR_READINGS, READING_VALUE + 8)?,
        })
    }
}

/// NUL-terminated string field; prefers the UTF-8 copy when the element is
/// large enough to have one.
fn name_at(element: &[u8], utf8: usize, ansi: usize) -> String {
    let field = element
        .get(utf8..utf8 + STRING_LEN)
        .or_else(|| element.get(ansi..ansi + STRING_LEN))
        .unwrap_or_default();
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Names of every reading, parsed once and shared between snapshots.
#[derive(Default)]
struct Layout {
    /// `(sensor, reading)` per reading, in HWiNFO's order.
    names: Vec<(String, String)>,
    /// sensor -> reading -> position in `names`.
    index: HashMap<String, HashMap<String, usize>>,
}

impl Layout {
    fn new(names: Vec<(String, String)>) -> Self {
        let mut index: HashMap<String, HashMap<String, usize>> = HashMap::new();
        for (i, (sensor, reading)) in names.iter().enumerate() {
            index
                .entry(sensor.clone())
                .or_default()
                .insert(reading.clone(), i);
        }
        Layout { names, index }
    }

    fn parse(bytes: &[u8], header: &Header) -> Self {
        let sensors: Vec<String> = header
            .sensors
            .elements(bytes)
            .map(|e| name_at(e, SENSOR_NAME_UTF8, SENSOR_NAME))
            .collect();
        let names = header
            .readings
            .elements(bytes)
            .map(|e| {
                let field = &e[READING_SENSOR_INDEX..READING_SENSOR_INDEX + 4];
                let sensor_index = u32::from_le_bytes(field.try_into().unwrap()) as usize;
                // An out-of-range sensor index keeps the reading's slot (so
                // values stay aligned) under an empty sensor name.
                (
                    sensors.get(sensor_index).cloned().unwrap_or_default(),
                    name_at(e, READING_LABEL_UTF8, READING_LABEL),
                )
            })
            .collect();
        Layout::new(names)
    }
}

/// Sensor readings at one moment. Cloning is cheap: the names are shared and
/// only the values are copied.
#[derive(Clone, Default)]
pub struct Snapshot {
    layout: Arc<Layout>,
    values: Vec<f64>,
}

impl Snapshot {
    /// Build a snapshot from `(sensor, reading, value)` triples.
    pub fn from_readings(readings: &[(&str, &str, f64)]) -> Self {
        let names = readings
            .iter()
            .map(|(sensor, reading, _)| (sensor.to_string(), reading.to_string()))
            .collect();
        Snapshot {
            layout: Arc::new(Layout::new(names)),
            values: readings.iter().map(|r| r.2).collect(),
        }
    }

    pub fn get(&self, sensor: &str, reading: &str) -> Option<f64> {
        let i = *self.layout.index.get(sensor)?.get(reading)?;
        self.values.get(i).copied()
    }

    /// Value of the first reading with this label, in HWiNFO's own order.
    pub fn find_first(&self, reading: &str) -> Option<f64> {
        let i = self.layout.names.iter().position(|(_, r)| r == reading)?;
        self.values.get(i).copied()
    }

    /// Every `(sensor, reading, value)`, in HWiNFO's own order.
    pub fn readings(&self) -> impl Iterator<Item = (&str, &str, f64)> + '_ {
        self.layout
            .names
            .iter()
            .zip(&self.values)
            .map(|((sensor, reading), value)| (sensor.as_str(), reading.as_str(), *value))
    }
}

fn read_values(bytes: &[u8], readings: Section, out: &mut Vec<f64>) {
    out.clear();
    out.extend(readings.elements(bytes).map(|e| {
        f64::from_le_bytes(e[READING_VALUE..READING_VALUE + 8].try_into().unwrap())
    }));
}

/// Parse a full snapshot from the mapped bytes.
fn load(bytes: &[u8]) -> Result<(Header, Snapshot)> {
    let header = Header::parse(bytes)?;
    let mut snapshot = Snapshot {
        layout: Arc::new(Layout::parse(bytes, &header)),
        values: Vec::new(),
    };
    read_values(bytes, header.readings, &mut snapshot.values);
    Ok((header, snapshot))
}

/// Update `snapshot` in place. Returns true when HWiNFO has polled since
/// `header` was read; the values are refreshed either way.
fn refresh(bytes: &[u8], header: &mut Header, snapshot: &mut Snapshot) -> Result<bool> {
    let new = Header::parse(bytes)?;
    if new.sensors != header.sensors || new.readings != header.readings {
        snapshot.layout = Arc::new(Layout::parse(bytes, &new));
    }
    read_values(bytes, new.readings, &mut snapshot.values);
    let fresh = new.poll_time != header.poll_time;
    *header = new;
    Ok(fresh)
}

/// A read-only view of HWiNFO's file mapping, unmapped on drop.
struct View {
    ptr: *mut c_void,
    len: usize,
}

// SAFETY: the view is a read-only mapping owned by this struct; nothing ties
// it to the thread that created it.
unsafe impl Send for View {}

impl View {
    fn open() -> Result<Self> {
        let name: Vec<u16> = OsStr::new(MAP_NAME).encode_wide().chain(once(0)).collect();
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if handle.is_null() {
            bail!("Failed to open shared memory object");
        }
        let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0) };
        // The view stays valid after the handle is closed.
        unsafe { CloseHandle(handle) };
        if ptr.is_null() {
            bail!("Failed to map view of shared memory");
        }
        let mut view = View { ptr, len: 0 };
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let written = unsafe {
            VirtualQuery(
                ptr,
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if written == 0 {
            bail!("Failed to query shared memory size");
        }
        view.len = info.RegionSize;
        Ok(view)
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` is a live mapping of `len` readable bytes until drop.
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe { UnmapViewOfFile(self.ptr) };
    }
}

/// Connection to a running HWiNFO's shared memory.
pub struct Hwinfo {
    view: View,
    header: Header,
    snapshot: Snapshot,
}

impl Hwinfo {
    pub fn new() -> Result<Self> {
        let view = View::open()?;
        let (header, snapshot) = load(view.bytes())?;
        Ok(Hwinfo {
            view,
            header,
            snapshot,
        })
    }

    /// Refresh the readings. Returns true when HWiNFO has polled since the
    /// previous pull.
    pub fn pull(&mut self) -> Result<bool> {
        refresh(self.view.bytes(), &mut self.header, &mut self.snapshot)
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_SIZE: usize = 44;

    /// Element sizes and name offsets: (sensor size, reading size, sensor name
    /// offset, reading label offset).
    type Shape = (usize, usize, usize, usize);
    const CURRENT: Shape = (392, 460, SENSOR_NAME_UTF8, READING_LABEL_UTF8);
    /// Older HWiNFO builds without the UTF-8 string copies.
    const LEGACY: Shape = (264, 316, SENSOR_NAME, READING_LABEL);

    fn put(buf: &mut [u8], offset: usize, bytes: &[u8]) {
        buf[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    /// Build a fake HWiNFO mapping. Readings are `(sensor index, label, value)`.
    fn memory_with(
        shape: Shape,
        poll_time: i64,
        sensors: &[&str],
        readings: &[(u32, &str, f64)],
    ) -> Vec<u8> {
        let (sensor_size, reading_size, sensor_name, reading_label) = shape;
        let sensor_off = HEADER_SIZE;
        let reading_off = sensor_off + sensors.len() * sensor_size;
        let mut buf = vec![0u8; reading_off + readings.len() * reading_size];
        put(&mut buf, HDR_POLL_TIME, &poll_time.to_le_bytes());
        let fields = [
            (HDR_SENSORS, sensor_off),
            (HDR_SENSORS + 4, sensor_size),
            (HDR_SENSORS + 8, sensors.len()),
            (HDR_READINGS, reading_off),
            (HDR_READINGS + 4, reading_size),
            (HDR_READINGS + 8, readings.len()),
        ];
        for (offset, value) in fields {
            put(&mut buf, offset, &(value as u32).to_le_bytes());
        }
        for (i, name) in sensors.iter().enumerate() {
            put(&mut buf, sensor_off + i * sensor_size + sensor_name, name.as_bytes());
        }
        for (i, (sensor, label, value)) in readings.iter().enumerate() {
            let element = reading_off + i * reading_size;
            put(&mut buf, element + READING_SENSOR_INDEX, &sensor.to_le_bytes());
            put(&mut buf, element + reading_label, label.as_bytes());
            put(&mut buf, element + READING_VALUE, &value.to_le_bytes());
        }
        buf
    }

    fn memory(poll_time: i64, sensors: &[&str], readings: &[(u32, &str, f64)]) -> Vec<u8> {
        memory_with(CURRENT, poll_time, sensors, readings)
    }

    #[test]
    fn load_reads_names_and_values() {
        let buf = memory(
            100,
            &["CPU [#0]", "GPU [#0]"],
            &[(0, "Total CPU Usage", 45.5), (1, "GPU Temperature", 72.0)],
        );
        let (header, snap) = load(&buf).unwrap();
        assert_eq!(header.poll_time, 100);
        assert_eq!(snap.get("CPU [#0]", "Total CPU Usage"), Some(45.5));
        assert_eq!(snap.get("GPU [#0]", "GPU Temperature"), Some(72.0));
        assert_eq!(snap.get("GPU [#0]", "Total CPU Usage"), None);
        assert_eq!(snap.get("Nope", "GPU Temperature"), None);
    }

    #[test]
    fn load_reads_non_ascii_names() {
        let buf = memory(1, &["Carte mère"], &[(0, "Température °C", 40.0)]);
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.get("Carte mère", "Température °C"), Some(40.0));
    }

    #[test]
    fn load_reads_legacy_layout_without_utf8_fields() {
        let buf = memory_with(LEGACY, 1, &["CPU"], &[(0, "Core Temp", 55.0)]);
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.get("CPU", "Core Temp"), Some(55.0));
    }

    #[test]
    fn find_first_uses_hwinfo_order() {
        let buf = memory(
            1,
            &["GPU [#0]", "GPU [#1]"],
            &[(1, "Fan", 1.0), (0, "GPU Temperature", 60.0), (1, "GPU Temperature", 70.0)],
        );
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.find_first("GPU Temperature"), Some(60.0));
        assert_eq!(snap.find_first("Missing"), None);
    }

    #[test]
    fn readings_iterates_in_order() {
        let buf = memory(1, &["A", "B"], &[(1, "y", 2.0), (0, "x", 1.0)]);
        let (_, snap) = load(&buf).unwrap();
        let all: Vec<_> = snap.readings().collect();
        assert_eq!(all, vec![("B", "y", 2.0), ("A", "x", 1.0)]);
    }

    #[test]
    fn refresh_is_fresh_only_when_poll_time_changes_but_always_updates_values() {
        let (mut header, mut snap) = load(&memory(100, &["S"], &[(0, "R", 1.0)])).unwrap();

        // Same poll time, new value: not fresh, value still updated.
        let fresh = refresh(&memory(100, &["S"], &[(0, "R", 2.0)]), &mut header, &mut snap).unwrap();
        assert!(!fresh);
        assert_eq!(snap.get("S", "R"), Some(2.0));

        let fresh = refresh(&memory(101, &["S"], &[(0, "R", 3.0)]), &mut header, &mut snap).unwrap();
        assert!(fresh);
        assert_eq!(snap.get("S", "R"), Some(3.0));
    }

    #[test]
    fn refresh_rebuilds_layout_when_reading_added() {
        let (mut header, mut snap) = load(&memory(1, &["S"], &[(0, "R", 1.0)])).unwrap();
        let grown = memory(2, &["S", "T"], &[(0, "R", 1.0), (1, "New", 9.0)]);
        refresh(&grown, &mut header, &mut snap).unwrap();
        assert_eq!(snap.get("T", "New"), Some(9.0));
        assert_eq!(snap.readings().count(), 2);
    }

    #[test]
    fn snapshot_clone_keeps_old_values_after_refresh() {
        let (mut header, mut snap) = load(&memory(1, &["S"], &[(0, "R", 1.0)])).unwrap();
        let before = snap.clone();
        refresh(&memory(2, &["S"], &[(0, "R", 5.0)]), &mut header, &mut snap).unwrap();
        assert_eq!(before.get("S", "R"), Some(1.0));
        assert_eq!(snap.get("S", "R"), Some(5.0));
    }

    #[test]
    fn load_rejects_truncated_memory() {
        let mut buf = memory(1, &["S"], &[(0, "R", 1.0)]);
        buf.truncate(buf.len() - 10);
        assert!(load(&buf).is_err());
        assert!(load(&buf[..20]).is_err());
        assert!(load(&[]).is_err());
    }

    #[test]
    fn load_rejects_garbage_counts() {
        let mut buf = memory(1, &["S"], &[(0, "R", 1.0)]);
        put(&mut buf, HDR_READINGS + 8, &u32::MAX.to_le_bytes());
        assert!(load(&buf).is_err());

        // Zero-sized elements must not be accepted either.
        let mut buf = memory(1, &["S"], &[(0, "R", 1.0)]);
        put(&mut buf, HDR_SENSORS + 4, &0u32.to_le_bytes());
        assert!(load(&buf).is_err());
    }

    #[test]
    fn reading_with_out_of_range_sensor_index_does_not_panic() {
        let buf = memory(1, &["S"], &[(7, "Orphan", 3.0), (0, "R", 1.0)]);
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.get("", "Orphan"), Some(3.0));
        assert_eq!(snap.get("S", "R"), Some(1.0));
    }

    #[test]
    fn empty_tables_load_as_empty_snapshot() {
        let (_, snap) = load(&memory(1, &[], &[])).unwrap();
        assert_eq!(snap.readings().count(), 0);
        assert_eq!(snap.find_first("anything"), None);
    }

    #[test]
    fn from_readings_builds_lookup() {
        let snap = Snapshot::from_readings(&[("CPU", "Temp", 65.0), ("GPU", "Temp", 72.0)]);
        assert_eq!(snap.get("CPU", "Temp"), Some(65.0));
        assert_eq!(snap.get("GPU", "Temp"), Some(72.0));
        assert_eq!(snap.find_first("Temp"), Some(65.0));
        assert_eq!(Snapshot::default().get("CPU", "Temp"), None);
    }
}
```

In `src-tauri/Cargo.toml` delete the line `strum = { version = "0.27", features = ["derive"] }`.

- [ ] **Step 2: Run the library tests**

```bash
rtk cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Expected: 13 passed, 0 failed. These tests do not touch the real HWiNFO mapping, so they pass whether or not HWiNFO is running. (The binary crate does not compile yet; that is the next steps.)

- [ ] **Step 3: Update `state.rs`, `utils.rs` and `gui.rs` to `Snapshot`**

`src-tauri/src/state.rs`: change `use hwinfo_steelseries_oled::Hwinfo;` to `use hwinfo_steelseries_oled::Snapshot;` and the field to `pub hwinfo_snapshot: Option<Snapshot>,`.

`src-tauri/src/utils.rs`: change `use hwinfo_steelseries_oled::Hwinfo;` to `use hwinfo_steelseries_oled::Snapshot;`, the parameter `hwinfo: &Hwinfo` to `hwinfo: &Snapshot`, and replace the lookup block

```rust
        let mut value = match hwinfo.get(sensor[0], sensor[1]) {
            Some(value) => {
                debug!("Successfully read sensor: {} / {}", sensor[0], sensor[1]);
                value
            }
            None => {
                error!("Sensor not found: {} / {}", sensor[0], sensor[1]);
                return Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Sensor not found:\n\t{}\n\t{}", sensor[0], sensor[1]),
                )));
            }
        }
        .value;
```

with

```rust
        let mut value = hwinfo.get(sensor[0], sensor[1]).ok_or_else(|| {
            error!("Sensor not found: {} / {}", sensor[0], sensor[1]);
            anyhow::anyhow!("Sensor not found:\n\t{}\n\t{}", sensor[0], sensor[1])
        })?;
```

Remove `debug` from `use log::{debug, error};` if the compiler reports it unused.

In the `utils.rs` test module replace the `use hwinfo_steelseries_oled::{...}` and `use std::collections::HashMap;` lines and the whole `build_hwinfo` helper with:

```rust
    fn build_hwinfo(entries: &[(&str, &str, f64)]) -> Snapshot {
        Snapshot::from_readings(entries)
    }
```

`src-tauri/src/gui.rs`: replace `sensors_from_hwinfo` with

```rust
/// Flatten a HWiNFO snapshot into a sorted list of SensorOption rows.
fn sensors_from_hwinfo(hw: &hwinfo_steelseries_oled::Snapshot) -> Vec<SensorOption> {
    let mut pairs: Vec<(&str, &str)> = hw
        .readings()
        .map(|(sensor, reading, _)| (sensor, reading))
        .collect();
    pairs.sort_unstable();
    pairs.dedup();
    pairs
        .into_iter()
        .map(|(category, reading)| SensorOption {
            category: category.to_string(),
            reading: reading.to_string(),
            full_id: format!("{};{}", category, reading),
            is_special: false,
            description: None,
        })
        .collect()
}
```

In the `gui.rs` test module replace the `hwinfo_steelseries_oled` / `HashMap` imports above `build_hwinfo` and the helper itself with:

```rust
    fn build_hwinfo(entries: &[(&str, &str)]) -> hwinfo_steelseries_oled::Snapshot {
        let triples: Vec<(&str, &str, f64)> = entries.iter().map(|(s, r)| (*s, *r, 1.0)).collect();
        hwinfo_steelseries_oled::Snapshot::from_readings(&triples)
    }
```

- [ ] **Step 4: Update `daemon.rs`**

Change `use hwinfo_steelseries_oled::Hwinfo;` to `use hwinfo_steelseries_oled::{Hwinfo, Snapshot};`.

Replace `fetch_summary_sensors` with:

```rust
fn fetch_summary_sensors(
    hwinfo: &Snapshot,
    gpu_name: &str,
) -> Result<SummarySensors, anyhow::Error> {
    let first = |reading: &str| {
        hwinfo
            .find_first(reading)
            .ok_or_else(|| anyhow!("{} not found", reading))
    };
    let gpu_temp = if gpu_name.is_empty() {
        first("GPU Temperature")?
    } else {
        hwinfo
            .get(gpu_name, "GPU Temperature")
            .ok_or_else(|| anyhow!("GPU Temperature not found"))?
    };
    Ok(SummarySensors {
        cpu_temp: first("CPU (Tctl/Tdie)")?,
        cpu_usage: first("Total CPU Usage")?,
        gpu_temp,
        gpu_usage: first("GPU Core Load")?,
        mem_used: first("Physical Memory Used")? / 1024.0,
        mem_free: first("Physical Memory Available")? / 1024.0,
        mem_load: first("Physical Memory Load")?,
    })
}
```

Replace `check_disconnect` with:

```rust
/// Ticks without new HWiNFO data before the display shows "Disconnected".
const STALE_LIMIT: usize = 5;

/// Count consecutive pulls without new HWiNFO data; true once `limit` is reached.
fn is_stale(fresh: bool, count: &mut usize, limit: usize) -> bool {
    *count = if fresh { 0 } else { (*count + 1).min(limit) };
    *count >= limit
}
```

In `build_display_value` change the parameter `hwinfo: &Hwinfo` to `hwinfo: &Snapshot`.

In `connect_all` replace

```rust
        let mut hwinfo = connect_hwinfo(&self.term)?;
        hwinfo.pull()?;
        self.hwinfo = Some(hwinfo);
```

with

```rust
        self.hwinfo = Some(connect_hwinfo(&self.term)?);
```

(`Hwinfo::new` already loads the first snapshot.)

In `tick`, replace everything from `let hwinfo = self` (the line after the `if self.is_sleeping || self.is_white_screen { return Ok(()); }` block) through the end of the function with:

```rust
        let mut hwinfo = self
            .hwinfo
            .take()
            .ok_or_else(|| anyhow!("hwinfo missing"))?;
        let fresh = hwinfo.pull()?;
        let result = self.render(hwinfo.snapshot(), fresh);
        // A stale view never recovers on its own: after HWiNFO restarts, the
        // old mapping stays frozen. Remap, keeping the old view if HWiNFO is
        // still gone.
        self.hwinfo = Some(if self.disconnect_count >= STALE_LIMIT {
            Hwinfo::new().unwrap_or(hwinfo)
        } else {
            hwinfo
        });
        result
    }

    /// Draw one frame from `hwinfo`. `fresh` is false when HWiNFO has not
    /// polled since the previous tick.
    fn render(&mut self, hwinfo: &Snapshot, fresh: bool) -> Result<(), anyhow::Error> {
        let oled = self.oled.as_mut().ok_or_else(|| anyhow!("oled missing"))?;
        let disconnected = is_stale(fresh, &mut self.disconnect_count, STALE_LIMIT);

        if let Ok(mut g) = self.state.lock() {
            g.hwinfo_snapshot = Some(hwinfo.clone());
        }

        if disconnected {
            warn!("HWiNFO disconnected");
            let value = disconnected_value();
            let buffer = value_to_oled_buffer(&value, &self.config.font_sizes, self.display_size);
            let _ = oled.trigger_frame("ERROR", self.i.0, &value, &buffer);

            self.write_state(|s| {
                s.hwinfo_connected = false;
                s.oled_buffer = buffer.clone();
                s.sensor_values = value_to_sensor_values(&value);
                s.last_error = Some("HWiNFO disconnected".to_string());
            });
            self.push_status();
            self.push_frame(&buffer);
            self.i += 1;
            return Ok(());
        }

        // Advance page counter (custom mode only)
        if !self.config.is_summary {
            let next = next_page_counter(
                self.i.0,
                self.config.page_time,
                self.config.pages,
                self.page_counter,
            );
            if next != self.page_counter {
                self.page_counter = next;
                debug!("Switching to page {}", self.page_counter + 1);
            }
        }

        let value = build_display_value(
            &self.config,
            hwinfo,
            &self.pages_vec,
            self.page_counter,
            &mut self.mouse_battery_reader,
            &mut self.media_reader,
            &self.weather_reader,
            self.hid_api.as_ref(),
        )?;

        let buffer = value_to_oled_buffer(&value, &self.config.font_sizes, self.display_size);
        let event_name = page_event_name(self.page_counter);
        oled.trigger_frame(&event_name, self.i.0, &value, &buffer)?;

        // Publish live dynamic-sensor snapshots so the settings preview can render
        // MEDIA_*/WEATHER_* with the same data the OLED shows (the GUI preview has
        // no access to the daemon's live readers otherwise).
        let media_snapshot = self.media_reader.snapshot();
        let weather_snapshot = self.weather_reader.current_info();

        self.write_state(|s| {
            s.hwinfo_connected = true;
            s.oled_buffer = buffer.clone();
            s.sensor_values = value_to_sensor_values(&value);
            s.last_error = None;
            s.media_info = media_snapshot;
            s.weather_info = weather_snapshot;
        });
        self.push_status();
        self.push_frame(&buffer);
        self.i += 1;
        Ok(())
    }
```

- [ ] **Step 5: Update the daemon tests**

In `mod tests` of `daemon.rs`:

Delete the imports `use hwinfo_steelseries_oled::{HwinfoSensorsReadingElement, HwinfoSensorsSensorElement, Sensor};` and `use std::collections::HashMap;`. Replace `build_hwinfo` and change the return type of `full_summary_hwinfo`:

```rust
    fn build_hwinfo(entries: &[(&str, &str, f64)]) -> Snapshot {
        Snapshot::from_readings(entries)
    }

    fn full_summary_hwinfo() -> Snapshot {
```

Delete: `test_check_disconnect_increments_on_match`, `test_check_disconnect_resets_on_change`, `test_check_disconnect_caps_at_limit`, `install_summary_hwinfo`, `install_bypass_hwinfo`, `test_daemon_tick_handles_disconnect_path`, `test_daemon_tick_happy_path_summary_renders_and_emits`, `test_daemon_tick_happy_path_advances_counter`, `test_daemon_tick_happy_path_custom_mode_advances_pages`, `test_daemon_tick_disconnect_after_many_unchanged_pulls`.

Add:

```rust
    #[test]
    fn test_is_stale_counts_up_to_limit_and_resets_on_fresh() {
        let mut count = 0;
        assert!(!is_stale(false, &mut count, 3));
        assert!(!is_stale(false, &mut count, 3));
        assert!(is_stale(false, &mut count, 3));
        // Capped: stays at the limit, does not overflow.
        assert!(is_stale(false, &mut count, 3));
        assert_eq!(count, 3);
        assert!(!is_stale(true, &mut count, 3));
        assert_eq!(count, 0);
    }

    #[test]
    fn test_render_summary_updates_state_and_counter() {
        let mut d = daemon_for_tests();
        install_mock_driver(&mut d);
        let i_before = d.i.0;

        d.render(&full_summary_hwinfo(), true).unwrap();

        assert_eq!(d.i.0, i_before + 1);
        let g = d.state.lock().unwrap();
        assert!(g.hwinfo_connected);
        assert!(!g.sensor_values.is_empty());
        assert!(g.hwinfo_snapshot.is_some());
        assert!(g.last_error.is_none());
    }

    #[test]
    fn test_render_without_driver_errors() {
        let mut d = daemon_for_tests();
        let err = d.render(&full_summary_hwinfo(), true).unwrap_err();
        assert!(format!("{}", err).contains("oled missing"));
    }

    #[test]
    fn test_render_custom_mode_advances_pages() {
        let mut d = daemon_for_tests();
        d.config.is_summary = false;
        d.config.pages = 2;
        d.config.page_time = 1; // TICK_RATE=500 → 2 ticks per second → switch every 2 ticks
        d.pages_vec = vec![ini::Properties::new(), ini::Properties::new()];
        install_mock_driver(&mut d);
        let hw = build_hwinfo(&[("S", "R", 1.0)]);

        for _ in 0..3 {
            d.render(&hw, true).unwrap();
        }
        assert_eq!(d.page_counter, 1);
    }

    #[test]
    fn test_render_shows_disconnected_after_stale_limit() {
        let mut d = daemon_for_tests();
        install_mock_driver(&mut d);
        let hw = full_summary_hwinfo();

        for _ in 0..STALE_LIMIT - 1 {
            d.render(&hw, false).unwrap();
        }
        assert!(d.state.lock().unwrap().hwinfo_connected);

        d.render(&hw, false).unwrap();
        let g = d.state.lock().unwrap();
        assert!(!g.hwinfo_connected);
        assert_eq!(g.last_error.as_deref(), Some("HWiNFO disconnected"));
    }

    #[test]
    fn test_render_recovers_after_stale() {
        let mut d = daemon_for_tests();
        install_mock_driver(&mut d);
        let hw = full_summary_hwinfo();
        for _ in 0..STALE_LIMIT {
            d.render(&hw, false).unwrap();
        }
        assert!(!d.state.lock().unwrap().hwinfo_connected);

        d.render(&hw, true).unwrap();
        let g = d.state.lock().unwrap();
        assert!(g.hwinfo_connected);
        assert!(g.last_error.is_none());
    }
```

`test_render_custom_mode_advances_pages`: ticks run with `i` = 0, 1, 2; the switch happens when `i % 2 == 0 && i != 0`, i.e. on the third call.

- [ ] **Step 6: Build and run all tests**

```bash
rtk cargo build --manifest-path src-tauri/Cargo.toml
rtk cargo test --manifest-path src-tauri/Cargo.toml --lib
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Expected: lib 13 passed; bins 1 known failure and nothing else; no new clippy warnings. If a test elsewhere still names a removed type (`Sensor`, `HwinfoSensorsReadingElement`, `new_mock`), replace its fixture with `Snapshot::from_readings(&[(sensor, reading, value)])`.

- [ ] **Step 7: Commit**

```bash
git add -A src-tauri Cargo.lock
git commit -m "refactor: parse HWiNFO shared memory into a cheap Snapshot

One persistent mapping, fixed-offset parsing with bounds checks, shared
names, poll_time-based staleness. Removes per-tick remapping, per-reading
allocations and the two per-tick Hwinfo deep clones.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 4: One retry loop

`retry_connect` loops forever inside `connect_all`, while `Daemon::run` already retries every 3 seconds. The inner loop blocks the daemon thread (reload and sleep requests are ignored while HWiNFO is absent), needs a string-match bail-out for unsupported devices, a test-only thread-local, an injectable sleep, and a `Term` threaded through to print a countdown to a hidden console.

**Files:**
- Modify: `src-tauri/src/connect.rs`, `src-tauri/src/daemon.rs`
- Test: `src-tauri/src/daemon.rs` (`mod tests`)

**Interfaces:**
- Consumes: `Hwinfo::new()` (Task 3), `AppConfig::from_ini` (existing).
- Produces:
  - `connect::connect_steelseries() -> Result<GameSenseClient, anyhow::Error>` (single attempt)
  - `connect::connect_hid(api: &HidApi, selector: &str) -> Result<(HidDevice, &'static SupportedDevice), anyhow::Error>` (single attempt)
  - `Daemon::load_config(&mut self) -> Result<(), anyhow::Error>`, `Daemon::take_reload_request(&self) -> bool`
- Removes: `connect::connect_hwinfo`, `retry_connect`, `retry_connect_inner`, `TEST_RETRY_MAX`, `current_test_retry_max`, `Daemon.term`.

- [ ] **Step 1: Write the failing test**

Add to `mod tests` in `daemon.rs`:

```rust
    #[test]
    fn test_take_reload_request_returns_and_clears_flag() {
        let d = daemon_for_tests();
        assert!(!d.take_reload_request());
        d.state.lock().unwrap().reload_requested = true;
        assert!(d.take_reload_request());
        assert!(!d.state.lock().unwrap().reload_requested);
        assert!(!d.take_reload_request());
    }
```

- [ ] **Step 2: Run it to verify it fails**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins take_reload_request
```

Expected: compile error, `no method named take_reload_request`.

- [ ] **Step 3: Simplify `connect.rs`**

Delete from the top of the file through `connect_steelseries`: `retry_connect_inner`, the `TEST_RETRY_MAX` thread-local, both `current_test_retry_max` functions, `retry_connect`, `connect_hwinfo`, `connect_steelseries`. Replace the imports and those functions with:

```rust
use crate::devices::{find_supported, SupportedDevice};
use gamesense::client::GameSenseClient;
use hidapi::{HidApi, HidDevice};
use log::warn;

/// Connect to SteelSeries GG's GameSense service. One attempt; the daemon
/// loop retries.
pub fn connect_steelseries() -> Result<GameSenseClient, anyhow::Error> {
    GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", None)
        .map_err(|e| anyhow::anyhow!("SteelSeries GG not reachable ({})", e))
}
```

Replace `connect_hid` and its doc comment with:

```rust
/// Open the supported SteelSeries OLED device matching `selector`, returning
/// it with its registry entry. One attempt; the daemon loop retries, which
/// also picks up newly plugged devices.
pub fn connect_hid(
    api: &HidApi,
    selector: &str,
) -> Result<(HidDevice, &'static SupportedDevice), anyhow::Error> {
    let (device_info, supported) = find_hid_device(api, selector)?;
    let device = device_info
        .open_device(api)
        .map_err(|e| anyhow::anyhow!("Failed to open HID device: {}", e))?;
    Ok((device, supported))
}
```

Change the doc comment on `UNSUPPORTED_ERROR_SUFFIX` to:

```rust
/// Suffix of the unsupported-device error. The settings UI matches on this
/// text to offer the device-support report.
```

In `mod tests` delete: `test_service_names_are_descriptive`, `test_connect_functions_exist`, `test_retry_logic_pattern_succeeds_first_try`, `test_retry_logic_pattern_fails_then_succeeds`, `test_retry_connect_inner_succeeds_first_try`, `test_retry_connect_inner_retries_then_succeeds`, `test_retry_connect_wrapper_returns_ok_immediately`, `test_retry_connect_inner_bounded_gives_up`, `set_test_retry_max`, `test_connect_hwinfo_bounded_returns_err_when_service_down`, `test_connect_steelseries_bounded_returns_err_when_service_down`, `test_connect_hid_bounded_propagates_err_when_no_devices`. Then delete any remaining test or import that no longer compiles because it names a deleted item (`Term`, `retry_connect`, `connect_hwinfo`).

- [ ] **Step 4: Update the daemon**

In `daemon.rs`:

Change the import to `use crate::connect::{connect_hid, connect_steelseries};` and delete `use console::Term;`. Delete the `term: Term,` field and its initializer `term: Term::stdout(),`.

In `connect_all` replace the three connect calls:

```rust
        self.hwinfo = Some(Hwinfo::new().map_err(|e| {
            anyhow!(
                "HWiNFO not reachable ({}). Is it running with Shared Memory Support enabled?",
                e
            )
        })?);
```

```rust
            let (device, supported) = connect_hid(&api, &self.config.direct_usb_serial)?;
```

```rust
            let mut gg = connect_steelseries()?;
```

Replace `reload` with:

```rust
    /// Re-read conf.ini into the daemon and shared state, restarting the
    /// weather reader so [Weather] changes take effect.
    fn load_config(&mut self) -> Result<(), anyhow::Error> {
        let ini =
            Ini::load_from_file("conf.ini").map_err(|e| anyhow!("Load conf.ini failed: {}", e))?;
        self.config = AppConfig::from_ini(&ini)?;

        self.weather_reader.stop();
        self.weather_reader = crate::weather::WeatherReader::spawn(&self.config.weather);

        let config = self.config.clone();
        self.write_state(|s| s.config = config);
        Ok(())
    }

    /// Whether a config reload was requested; clears the request.
    fn take_reload_request(&self) -> bool {
        self.state
            .lock()
            .map(|mut g| std::mem::take(&mut g.reload_requested))
            .unwrap_or(false)
    }

    fn reload(&mut self) -> Result<(), anyhow::Error> {
        info!("Daemon: reloading config");
        self.load_config()?;

        // Tear down the existing OLED connection, then reconnect with the new config.
        if let Some(mut oled) = self.oled.take() {
            let _ = oled.stop_heartbeat();
        }
        self.hid_api = None;
        self.write_state(|s| {
            s.gg_connected = false;
            s.usb_connected = false;
            s.active_mode = ActiveMode::Disconnected;
        });

        self.connect_all()
    }
```

In `run`, make the top of the outer loop honor a reload requested while nothing is connected (a config saved in the settings window while HWiNFO or the device is missing):

```rust
    fn run(&mut self) {
        loop {
            if self.take_reload_request() {
                if let Err(e) = self.load_config() {
                    self.record_connect_failure(&e);
                }
            }
            if let Err(e) = self.connect_all() {
                self.record_connect_failure(&e);
                thread::sleep(Duration::from_secs(3));
                continue;
            }
```

The rest of `run` is unchanged.

- [ ] **Step 5: Run the tests**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Expected: 0 failed (the previously known failure was deleted in Step 3); no new clippy warnings.

- [ ] **Step 6: Commit**

```bash
git add -A src-tauri
git commit -m "refactor: single-attempt connects; daemon loop is the only retry

Reload requests are now honored while disconnected.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 5: Typed sensors in, lines out

Config is parsed into `AppConfig.custom_sensors`, but the daemon re-reads `conf.ini` into `ini::Properties` and `run_sensors` looks up `sensor_N`/`label_N` string keys every tick; the GUI preview converts typed sensors back into fake `ini::Properties` to call it. Frames are built as `json!({"line1": ...})` and then unpacked again in three places. After this task `run_sensors` takes `&[CustomSensor]`, frames are `Vec<String>`, and JSON exists only where GameSense needs it.

**Files:**
- Modify: `src-tauri/src/utils.rs`, `src-tauri/src/daemon.rs`, `src-tauri/src/gui.rs`
- Test: the `mod tests` of the same three files

**Interfaces:**
- Consumes: `Snapshot` (Task 3), `CustomSensor: Default` (Task 2), `Daemon::load_config` keeps `self.config.custom_sensors` current (Task 4).
- Produces:
  - `utils::Slot { pub label: String, pub value: String, pub unit: String, pub icon: String }` (`Debug, Clone, Default, PartialEq`)
  - `utils::run_sensors(page: &[CustomSensor], decimal: bool, hwinfo: &Snapshot, mouse: &mut MouseBatteryReader, media: &mut MediaReader, weather: &WeatherReader, hid_api: Option<&hidapi::HidApi>) -> Result<Vec<Slot>, anyhow::Error>` (always `CUSTOM_SENSORS` slots)
  - `utils::format_custom_lines(sensors_per_line: u8, slots: &[Slot]) -> Vec<String>` (always `DISPLAY_LINES` lines)
  - `daemon::build_display_lines(config: &AppConfig, hwinfo: &Snapshot, page_counter: usize, mouse: &mut MouseBatteryReader, media: &mut MediaReader, weather: &WeatherReader, hid_api: Option<&hidapi::HidApi>) -> Result<Vec<String>, anyhow::Error>`
  - `DisplayDriver::trigger_frame(&mut self, event: &str, i: isize, lines: &[String], buffer: &OledBuffer) -> Result<(), anyhow::Error>`
- Removes: `format_custom_value`, `build_display_value`, `value_to_text`, `value_to_oled_buffer`, `value_to_sensor_values`, `disconnected_value`, `load_pages_from_ini`, `Daemon.pages_vec`, `Daemon::load_pages_for_config`, `gui::build_preview_props`, `gui::value_to_preview_text`, `gui::summary_preview_value`.

Behavior notes:
- A page whose `PAGEn.Sensors` section is missing used to make every tick fail with "Page n missing" (the old code skipped missing sections, shifting later pages). It now renders as an empty page, matching what `AppConfig::from_ini` already produces.
- A slot's icon is kept even when the sensor itself is hidden (e.g. `MEDIA_TITLE` with nothing playing). That matches current behavior.

- [ ] **Step 1: Write the failing tests in `utils.rs`**

In `mod tests` of `utils.rs`, replace `make_props` and `empty_buffers` with:

```rust
    /// Build a page from `("sensor_0", "...")`-style pairs.
    fn make_page(pairs: &[(&str, &str)]) -> Vec<CustomSensor> {
        let mut page = vec![CustomSensor::default(); CUSTOM_SENSORS];
        for (key, value) in pairs {
            let (field, index) = key.rsplit_once('_').expect("key like sensor_0");
            let slot = &mut page[index.parse::<usize>().expect("numeric index")];
            match field {
                "sensor" => slot.sensor = value.to_string(),
                "label" => slot.label = value.to_string(),
                "unit" => slot.unit = value.to_string(),
                "convert" => slot.convert = value.to_string(),
                "icon" => slot.icon = value.to_string(),
                other => panic!("unknown field {}", other),
            }
        }
        page
    }

    fn slots(entries: &[(&str, &str, &str, &str)]) -> Vec<Slot> {
        entries
            .iter()
            .map(|(label, value, unit, icon)| Slot {
                label: label.to_string(),
                value: value.to_string(),
                unit: unit.to_string(),
                icon: icon.to_string(),
            })
            .collect()
    }
```

Replace the four `test_format_custom_value_*` tests with:

```rust
    #[test]
    fn test_format_custom_lines_prepends_icon_token_when_set() {
        let lines = format_custom_lines(1, &slots(&[("CPU", "42", "", "cpu")]));
        assert!(lines[0].starts_with(&crate::render::icon_token("cpu")));
        assert!(lines[0].contains("CPU 42"));
    }

    #[test]
    fn test_format_custom_lines_no_token_when_icon_empty() {
        let lines = format_custom_lines(1, &slots(&[("CPU", "42", "", "")]));
        assert_eq!(lines[0], "CPU 42");
    }

    #[test]
    fn test_format_custom_lines_one_sensor_per_line() {
        let lines = format_custom_lines(
            1,
            &slots(&[
                ("L1", "1", "U1", ""),
                ("L2", "2", "U2", ""),
                ("L3", "3", "U3", ""),
                ("L4", "4", "U4", ""),
                ("L5", "5", "U5", ""),
            ]),
        );
        assert_eq!(lines, vec!["L1 1U1", "L2 2U2", "L3 3U3", "L4 4U4", "L5 5U5"]);
    }

    #[test]
    fn test_format_custom_lines_three_sensors_per_line() {
        let all: Vec<Slot> = (0..15)
            .map(|i| Slot {
                value: i.to_string(),
                ..Default::default()
            })
            .collect();
        let lines = format_custom_lines(3, &all);
        assert_eq!(lines, vec!["0 1 2", "3 4 5", "6 7 8", "9 10 11", "12 13 14"]);
    }

    #[test]
    fn test_format_custom_lines_always_returns_display_lines() {
        assert_eq!(format_custom_lines(1, &[]).len(), DISPLAY_LINES);
        assert_eq!(format_custom_lines(0, &slots(&[("A", "1", "", "")])).len(), DISPLAY_LINES);
    }

    #[test]
    fn test_format_custom_lines_survives_oversized_sensors_per_line() {
        // 4 per line × 5 lines = 20 positions, but only 15 slots exist.
        let all: Vec<Slot> = (0..15)
            .map(|i| Slot {
                value: i.to_string(),
                ..Default::default()
            })
            .collect();
        let lines = format_custom_lines(4, &all);
        assert_eq!(lines.len(), DISPLAY_LINES);
        assert_eq!(lines[0], "0 1 2 3");
        assert_eq!(lines[3], "12 13 14 ");
        assert_eq!(lines[4], "   ");
    }

    #[test]
    fn test_run_sensors_keeps_icon_and_pads_to_custom_sensors() {
        let page = vec![CustomSensor {
            sensor: "BLANK".into(),
            icon: "cpu".into(),
            ..Default::default()
        }];
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let result =
            run_sensors(&page, false, &hwinfo, &mut mouse, &mut media, &weather, None).unwrap();

        assert_eq!(result.len(), CUSTOM_SENSORS);
        assert_eq!(result[0].icon, "cpu");
        assert_eq!(result[1], Slot::default());
    }
```

Add `use crate::settings::CustomSensor;` to the test module if `use super::*;` does not already bring it in after Step 3.

- [ ] **Step 2: Run them to verify they fail**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins format_custom_lines
```

Expected: compile error, `cannot find function format_custom_lines` / `cannot find type Slot`.

- [ ] **Step 3: Rewrite `run_sensors` and `format_custom_value` in `utils.rs`**

Replace the imports at the top of the file and both functions (keep `safe_strftime` unchanged) with:

```rust
use chrono::{DateTime, Local};
use log::error;

use crate::consts::{CUSTOM_SENSORS, DISPLAY_LINES};
use crate::media::{MediaField, MediaReader};
use crate::mouse_battery::MouseBatteryReader;
use crate::settings::CustomSensor;
use crate::weather::{WeatherField, WeatherReader};
use hwinfo_steelseries_oled::Snapshot;
```

```rust
/// One resolved sensor position, ready for layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slot {
    pub label: String,
    pub value: String,
    pub unit: String,
    pub icon: String,
}

/// Resolve one page of configured sensors to display slots. Always returns
/// `CUSTOM_SENSORS` slots; unset or hidden sensors leave their slot's text
/// empty.
pub fn run_sensors(
    page: &[CustomSensor],
    decimal: bool,
    hwinfo: &Snapshot,
    mouse_battery_reader: &mut MouseBatteryReader,
    media_reader: &mut MediaReader,
    weather_reader: &WeatherReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<Vec<Slot>, anyhow::Error> {
    let mut slots = vec![Slot::default(); CUSTOM_SENSORS];

    for (configured, slot) in page.iter().zip(slots.iter_mut()) {
        slot.icon = configured.icon.clone();

        // Remove quotes if they exist (for backwards compatibility with old configs)
        let sensor_str = configured.sensor.trim_matches('"');
        if sensor_str.trim().is_empty() {
            continue;
        }
        let sensor: Vec<&str> = sensor_str.split(';').collect();

        // `None` hides the slot: label and unit stay empty too.
        let value: Option<String> = if sensor[0] == "BLANK" {
            Some(String::new())
        } else if sensor[0] == "CLOCK" || sensor[0] == "DATE" {
            let default_format = if sensor[0] == "CLOCK" {
                "%I:%M:%S %P"
            } else {
                "%Y-%m-%d"
            };
            let format = sensor
                .get(1)
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or(default_format);
            Some(safe_strftime(&Local::now(), format, default_format))
        } else if sensor[0] == "MOUSE_BATTERY" {
            Some(mouse_battery_reader.get_battery_percentage(hid_api))
        } else if let Some(field) = MediaField::from_sensor_name(sensor[0]) {
            // Hidden when nothing is playing.
            media_reader.get_media_field(field)
        } else if let Some(field) = WeatherField::from_sensor_name(sensor[0]) {
            // Hidden when there is no data yet or the field is unset.
            weather_reader.get_field(field)
        } else if sensor.len() < 2 {
            error!("Malformed sensor entry (missing ';'): {}", sensor_str);
            None
        } else {
            let mut value = hwinfo.get(sensor[0], sensor[1]).ok_or_else(|| {
                error!("Sensor not found: {} / {}", sensor[0], sensor[1]);
                anyhow::anyhow!("Sensor not found:\n\t{}\n\t{}", sensor[0], sensor[1])
            })?;
            if matches!(configured.convert.as_str(), "MB/GB" | "kb/mb" | "KB/MB") {
                value /= 1024.0;
            }
            Some(if decimal {
                format!("{:.1}", value)
            } else {
                format!("{:02.0}", value)
            })
        };

        if let Some(value) = value {
            slot.label = configured.label.clone();
            slot.unit = configured.unit.clone();
            slot.value = value;
        }
    }
    Ok(slots)
}

/// Lay the slots out as `DISPLAY_LINES` text lines, `sensors_per_line` slots
/// per line. Positions beyond the available slots render empty.
pub fn format_custom_lines(sensors_per_line: u8, slots: &[Slot]) -> Vec<String> {
    let per_line = sensors_per_line as usize;
    // Tighter spacing when three or more sensors share a line.
    let separator = if per_line >= 3 { " " } else { "  " };
    let empty = Slot::default();

    (0..DISPLAY_LINES)
        .map(|line| {
            (0..per_line)
                .map(|i| {
                    let slot = slots.get(line * per_line + i).unwrap_or(&empty);
                    let label = slot.label.trim();
                    let unit = slot.unit.trim();
                    let icon = slot.icon.trim();

                    // Leading icon token (drawn only on the direct-USB bitmap
                    // path; a harmless control char in GameSense text).
                    let prefix = if icon.is_empty() {
                        String::new()
                    } else {
                        crate::render::icon_token(icon)
                    };
                    // A space between label and value only when there is a label.
                    if label.is_empty() {
                        format!("{}{}{}", prefix, slot.value, unit)
                    } else {
                        format!("{}{} {}{}", prefix, label, slot.value, unit)
                    }
                })
                .collect::<Vec<_>>()
                .join(separator)
        })
        .collect()
}
```

Note on the malformed-entry and BLANK cases: in the old code a malformed entry left the slot empty, and BLANK kept label and unit with an empty value. Both are preserved (`None` vs `Some(String::new())`).

- [ ] **Step 4: Migrate the `test_run_sensors_*` tests in `utils.rs`**

Apply the same mechanical change to every existing `test_run_sensors_*` test. Example, before:

```rust
        let props = make_props(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "Out"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "Out");
        assert_eq!(units[0], "°F");
        assert_eq!(values[0], "72");
```

after:

```rust
        let props = make_page(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "Out"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();

        let slots =
            run_sensors(&props, false, &hwinfo, &mut mouse, &mut media, &weather, None).unwrap();

        assert_eq!(slots[0].label, "Out");
        assert_eq!(slots[0].unit, "°F");
        assert_eq!(slots[0].value, "72");
```

The rules, for each test:
1. `make_props(` → `make_page(`.
2. Delete the `let (mut labels, mut units, mut values) = empty_buffers();` line.
3. The call becomes `run_sensors(&props, <decimal>, &hwinfo, &mut mouse, &mut media, &weather, <hid_api>)` where `<decimal>` and `<hid_api>` are the values the old call passed in 6th and 10th position. Bind the `Ok` value as `slots`. Tests that asserted `.is_err()` keep asserting it on the returned `Result`.
4. `labels[i]` → `slots[i].label`, `units[i]` → `slots[i].unit`, `values[i]` → `slots[i].value`.
5. A test that built `ini::Properties` by hand instead of through `make_props` gets the same pairs passed to `make_page`.

- [ ] **Step 5: Update `daemon.rs`**

Imports: change to `use crate::utils::{format_custom_lines, run_sensors};`, `use serde_json::Value;` (drop `json`), and `use crate::consts::{DISPLAY_LINES, TICK_RATE};` (drop `CUSTOM_SENSORS`). Remove `use ini::Ini;` only if `load_config` no longer needs it (it does; keep it).

Replace `format_vertical_summary`, `format_horizontal_summary`, `value_to_text`, `value_to_oled_buffer`, `value_to_sensor_values` with:

```rust
fn format_vertical_summary(s: &SummarySensors, decimal: bool) -> Vec<String> {
    let precision = if decimal { 1 } else { 0 };
    let spacing = if decimal { " " } else { "   " };
    let spacing2 = if decimal { " " } else { "    " };
    vec![
        "CPU   GPU   MEM".to_string(),
        format!(
            "{:.p$}°{}{:.p$}°{}{:.p$}G",
            s.cpu_temp,
            spacing,
            s.gpu_temp,
            spacing,
            s.mem_used,
            p = precision
        ),
        format!(
            "{:.p$}%{}{:.p$}%{}{:.p$}G",
            s.cpu_usage,
            spacing2,
            s.gpu_usage,
            spacing2,
            s.mem_free,
            p = precision
        ),
    ]
}

fn format_horizontal_summary(s: &SummarySensors, decimal: bool) -> Vec<String> {
    let precision = if decimal { 1 } else { 0 };
    vec![
        format!("CPU {:.p$}° {:.p$}%", s.cpu_temp, s.cpu_usage, p = precision),
        format!("GPU {:.p$}° {:.p$}%", s.gpu_temp, s.gpu_usage, p = precision),
        format!("MEM {:.p$}G {:.p$}%", s.mem_used, s.mem_load, p = precision),
    ]
}

/// GameSense frame payload: `{"line1": ..., "line2": ...}`.
fn lines_to_frame(lines: &[String]) -> Value {
    Value::Object(
        lines
            .iter()
            .enumerate()
            .map(|(i, line)| (format!("line{}", i + 1), Value::String(line.clone())))
            .collect(),
    )
}

fn lines_to_oled_buffer(lines: &[String], font_sizes: &[FontSize], size: (u32, u32)) -> OledBuffer {
    render_text_to_oled(&lines.join("\n"), 0, font_sizes, size.0, size.1)
}

fn lines_to_sensor_values(lines: &[String]) -> Vec<SensorValue> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| SensorValue {
            label: format!("Line {}", i + 1),
            value: line.clone(),
        })
        .collect()
}
```

In the `DisplayDriver` trait, the forwarding `impl DisplayDriver for OledClient`, and `OledClient::trigger_frame`, change the parameter `value: &Value` to `lines: &[String]`. In `OledClient::trigger_frame` the GameSense arm becomes:

```rust
            OledClient::GameSense(client) => {
                client.trigger_event_frame(event, i, lines_to_frame(lines))?;
            }
```

In `OledClient::send_blank` and `send_white` replace the `json!({...})` literals:

```rust
                let blank = vec![String::new(); 3];
                client.trigger_event_frame("BLANK", 0, lines_to_frame(&blank))?;
```

```rust
                let white = vec!["████████████████".to_string(); 3];
                client.trigger_event_frame("WHITE", 0, lines_to_frame(&white))?;
```

Delete `disconnected_value`, `load_pages_from_ini`, and `build_display_value`. Add:

```rust
fn build_display_lines(
    config: &AppConfig,
    hwinfo: &Snapshot,
    page_counter: usize,
    mouse: &mut MouseBatteryReader,
    media: &mut MediaReader,
    weather: &crate::weather::WeatherReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<Vec<String>, anyhow::Error> {
    if config.is_summary {
        let sensors = fetch_summary_sensors(hwinfo, &config.gpu)?;
        return Ok(if config.is_vertical {
            format_vertical_summary(&sensors, config.decimal)
        } else {
            format_horizontal_summary(&sensors, config.decimal)
        });
    }
    let page = config
        .custom_sensors
        .get(page_counter)
        .ok_or_else(|| anyhow!("Page {} missing", page_counter))?;
    let slots = run_sensors(page, config.decimal, hwinfo, mouse, media, weather, hid_api)?;
    Ok(format_custom_lines(config.sensors_per_line, &slots))
}
```

In `Daemon`: delete the `pages_vec` field, its initializer, the `load_pages_for_config` method and its call in `connect_all`.

In `render`, the disconnected branch becomes:

```rust
        if disconnected {
            warn!("HWiNFO disconnected");
            let lines = ["Disconnected", "FROM", "HWiNFO"].map(String::from);
            let buffer = lines_to_oled_buffer(&lines, &self.config.font_sizes, self.display_size);
            let _ = oled.trigger_frame("ERROR", self.i.0, &lines, &buffer);

            self.write_state(|s| {
                s.hwinfo_connected = false;
                s.oled_buffer = buffer.clone();
                s.sensor_values = lines_to_sensor_values(&lines);
                s.last_error = Some("HWiNFO disconnected".to_string());
            });
```

and the normal path, from `let value = build_display_value(` through `oled.trigger_frame(...)?;`, becomes:

```rust
        let lines = build_display_lines(
            &self.config,
            hwinfo,
            self.page_counter,
            &mut self.mouse_battery_reader,
            &mut self.media_reader,
            &self.weather_reader,
            self.hid_api.as_ref(),
        )?;

        let buffer = lines_to_oled_buffer(&lines, &self.config.font_sizes, self.display_size);
        let event_name = page_event_name(self.page_counter);
        oled.trigger_frame(&event_name, self.i.0, &lines, &buffer)?;
```

with `s.sensor_values = lines_to_sensor_values(&lines);` in the `write_state` closure below it.

In `connect_all`, the GameSense branch builds the handler's line keys; leave that code as is.

- [ ] **Step 6: Update the daemon tests**

In `mod tests` of `daemon.rs`:

- Add `use crate::settings::CustomSensor;`.
- `MockDriver::trigger_frame`: parameter `_value: &Value` → `_lines: &[String]`.
- In `test_format_vertical_summary_no_decimal`, `test_format_vertical_summary_decimal`, `test_format_horizontal_summary_no_decimal`, `test_format_horizontal_summary_decimal`: `v["line1"]` → `v[0]`, `v["line2"]` → `v[1]`, `v["line3"]` → `v[2]`.
- Delete: `test_disconnected_value_has_three_lines`, `test_value_to_oled_buffer_populates_text`, `test_value_to_text_preserves_leading_blank_line`, `test_value_to_text_preserves_middle_blank_slot`, `test_value_to_text_all_present_unchanged`, `test_value_to_sensor_values_collects_lines`, `test_value_to_sensor_values_skips_missing_lines`, `test_value_to_oled_buffer_with_non_object_value`, `test_value_to_sensor_values_with_non_object`, `test_load_pages_from_ini_returns_existing_sections`, `test_load_pages_from_ini_empty_when_no_pages`, `test_load_pages_from_ini_zero_pages`, `test_load_pages_for_config_summary_clears_pages`, `test_load_pages_for_config_custom_with_no_conf_ini_yields_empty`, `test_build_display_value_summary_vertical`, `test_build_display_value_summary_horizontal`, `test_build_display_value_custom_missing_page_errors`, `test_build_display_value_custom_renders_blank_sensor`, `test_build_display_value_custom_run_sensors_propagates_err`.
- In `test_daemon_new_initializes_fields` delete the line `assert!(d.pages_vec.is_empty());`.
- In `test_render_custom_mode_advances_pages` replace `d.pages_vec = vec![ini::Properties::new(), ini::Properties::new()];` with `d.config.custom_sensors = vec![Vec::new(), Vec::new()];`.
- Add:

```rust
    fn lines_for(cfg: &AppConfig, hw: &Snapshot) -> Result<Vec<String>, anyhow::Error> {
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        build_display_lines(cfg, hw, 0, &mut mouse, &mut media, &weather, None)
    }

    #[test]
    fn test_build_display_lines_summary_vertical() {
        let lines = lines_for(&base_config(), &full_summary_hwinfo()).unwrap();
        assert_eq!(lines[0], "CPU   GPU   MEM");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_build_display_lines_summary_horizontal() {
        let mut cfg = base_config();
        cfg.is_vertical = false;
        let lines = lines_for(&cfg, &full_summary_hwinfo()).unwrap();
        assert_eq!(lines[0], "CPU 42° 50%");
    }

    #[test]
    fn test_build_display_lines_custom_missing_page_errors() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        let err = lines_for(&cfg, &build_hwinfo(&[])).unwrap_err();
        assert!(format!("{}", err).contains("Page 0 missing"));
    }

    #[test]
    fn test_build_display_lines_custom_renders_configured_sensor() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![CustomSensor {
            sensor: "CPU;Temp".into(),
            label: "X".into(),
            unit: "°".into(),
            ..Default::default()
        }]];
        let lines = lines_for(&cfg, &build_hwinfo(&[("CPU", "Temp", 42.0)])).unwrap();
        assert_eq!(lines[0], "X 42°");
        assert_eq!(lines.len(), crate::consts::DISPLAY_LINES);
    }

    #[test]
    fn test_build_display_lines_custom_propagates_missing_sensor() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![CustomSensor {
            sensor: "Nope;Missing".into(),
            ..Default::default()
        }]];
        assert!(lines_for(&cfg, &build_hwinfo(&[])).is_err());
    }

    #[test]
    fn test_lines_to_frame_numbers_keys_from_one() {
        let frame = lines_to_frame(&["A".to_string(), String::new(), "C".to_string()]);
        assert_eq!(frame["line1"], "A");
        assert_eq!(frame["line2"], "");
        assert_eq!(frame["line3"], "C");
        assert!(frame.get("line4").is_none());
    }

    #[test]
    fn test_lines_to_oled_buffer_keeps_blank_line_positions() {
        let with_gap = lines_to_oled_buffer(
            &[String::new(), "CPU".to_string()],
            &[],
            (128, 64),
        );
        let without_gap = lines_to_oled_buffer(&["CPU".to_string()], &[], (128, 64));
        assert!(with_gap.data.iter().any(|&b| b != 0));
        // A leading blank line pushes the text down: the buffers differ.
        assert_ne!(with_gap, without_gap);
    }

    #[test]
    fn test_lines_to_sensor_values_labels_each_line() {
        let values = lines_to_sensor_values(&["A".to_string(), "B".to_string()]);
        assert_eq!(values.len(), 2);
        assert_eq!(values[1].label, "Line 2");
        assert_eq!(values[1].value, "B");
    }
```

- In the `test_oled_client_*` tests that call `trigger_frame`, pass `&[]` (or `&["x".to_string()]`) where a `&Value` / `&json!(...)` was passed. These tests are deleted in Task 6; here they only need to compile.

- [ ] **Step 7: Update `gui.rs`**

Change imports to `use crate::utils::{format_custom_lines, run_sensors};`; drop `use serde_json::Value;` if the compiler reports it unused.

Replace `summary_preview_value` with:

```rust
/// Static summary placeholder for the preview pane.
fn summary_preview_lines(is_vertical: bool) -> Vec<String> {
    let lines = if is_vertical {
        ["CPU   GPU   MEM", "00°   00°   00G", "00%   00%   00G"]
    } else {
        ["CPU 00° 00%", "GPU 00° 00%", "MEM 00G 00%"]
    };
    lines.map(String::from).to_vec()
}
```

Delete `build_preview_props` and `value_to_preview_text`.

In `preview_config_impl`, replace everything from `let value: Value = if config.is_summary {` through the final `Ok(buffer_to_frame(&buf))` with:

```rust
    let lines = if config.is_summary {
        summary_preview_lines(config.is_vertical)
    } else {
        let hwinfo = match hwinfo_opt.as_ref() {
            Some(h) => h,
            None => {
                let buf = render_text_to_oled("HWiNFO not\nconnected", 0, &[], w, h);
                return Ok(buffer_to_frame(&buf));
            }
        };

        let page_sensors = config.custom_sensors.get(page).cloned().unwrap_or_default();

        let mut mb = MouseBatteryReader::new();
        // Seed the preview readers with the daemon's live snapshots so MEDIA_*/
        // WEATHER_* sensors render real data instead of always-blank placeholders.
        let mut media = MediaReader::with_cached_info(media_info);
        let weather = match weather_info {
            Some(info) => crate::weather::WeatherReader::with_cached_info(info),
            None => crate::weather::WeatherReader::disabled(),
        };

        // Only enumerate HID when the page shows the mouse battery; without it
        // the preview would always read "N/A".
        let hid_api = page_sensors
            .iter()
            .any(|s| s.sensor == "MOUSE_BATTERY")
            .then(|| hidapi::HidApi::new().ok())
            .flatten();

        match run_sensors(
            &page_sensors,
            config.decimal,
            hwinfo,
            &mut mb,
            &mut media,
            &weather,
            hid_api.as_ref(),
        ) {
            Ok(slots) => format_custom_lines(config.sensors_per_line, &slots),
            Err(e) => {
                let buf = render_text_to_oled(&format!("Preview error:\n{}", e), 0, &[], w, h);
                return Ok(buffer_to_frame(&buf));
            }
        }
    };

    let buf = render_text_to_oled(&lines.join("\n"), 0, &config.font_sizes, w, h);
    Ok(buffer_to_frame(&buf))
```

Keep the existing first statement of the function (the block that locks `shared` and extracts `hwinfo_opt`, `media_info`, `weather_info`, `(w, h)`).

In the `gui.rs` tests:
- Delete `test_build_preview_props_writes_all_fields`, `test_build_preview_props_writes_icon`, `test_build_preview_props_skips_empty_sensor_string`, `test_build_preview_props_caps_at_custom_sensors`, `test_value_to_preview_text_orders_lines`, `test_value_to_preview_text_preserves_blank_slots`, `test_value_to_preview_text_skips_non_string`, `test_value_to_preview_text_empty_object`, `test_value_to_preview_text_non_object_returns_empty`.
- Replace `test_summary_preview_value_vertical` and `test_summary_preview_value_horizontal` with:

```rust
    #[test]
    fn test_summary_preview_lines() {
        assert_eq!(summary_preview_lines(true)[0], "CPU   GPU   MEM");
        assert_eq!(summary_preview_lines(false)[0], "CPU 00° 00%");
        assert_eq!(summary_preview_lines(true).len(), 3);
    }
```

- [ ] **Step 8: Run the tests**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Expected: 0 failed; no new clippy warnings. `run_sensors` and `build_display_lines` have 7 parameters each, at clippy's limit.

- [ ] **Step 9: Commit**

```bash
git add -A src-tauri
git commit -m "refactor: typed sensors into run_sensors, display frames as lines

The daemon no longer re-reads conf.ini into ini::Properties and frames are
JSON only at the GameSense call. Fixes an out-of-bounds panic when
sensors_per_line exceeds the available slots.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 6: Two display drivers instead of three layers

`DisplayDriver` (trait) wraps `OledClient` (enum, plus a pure forwarding impl) which wraps `HidSender` (trait). Each trait has one production implementation. After this task `DisplayDriver` has two real implementations and the enum, the forwarding impl and `HidSender` are gone.

Tests removed with `HidSender`: the nine `FakeHidSender` tests. What they covered is a three-line send loop; packet contents stay covered by the `Protocol::build_packets` tests in `devices.rs`.

**Files:**
- Modify: `src-tauri/src/daemon.rs`
- Test: `src-tauri/src/daemon.rs` (`mod tests`)

**Interfaces:**
- Consumes: `lines_to_frame` and the `trigger_frame(.., lines: &[String], ..)` signature (Task 5); `connect_hid`, `connect_steelseries` (Task 4).
- Produces: `struct GameSenseDisplay(GameSenseClient)`, `struct HidDisplay { device: hidapi::HidDevice, model: &'static SupportedDevice }`, both `impl DisplayDriver`. `DisplayDriver::stop_heartbeat` gains a default no-op body.
- Removes: `OledClient`, `HidSender`.

- [ ] **Step 1: Replace the driver types**

In `daemon.rs` delete the `HidSender` trait, `impl HidSender for hidapi::HidDevice`, the `OledClient` enum, `impl DisplayDriver for OledClient` and `impl OledClient`. Replace the `DisplayDriver` trait and those items with:

```rust
/// Where frames go. Two implementations: SteelSeries GG (text lines) and
/// direct USB HID (bitmap).
trait DisplayDriver: Send {
    fn trigger_frame(
        &mut self,
        event: &str,
        i: isize,
        lines: &[String],
        buffer: &OledBuffer,
    ) -> Result<(), anyhow::Error>;
    fn send_blank(&mut self) -> Result<(), anyhow::Error>;
    fn send_white(&mut self) -> Result<(), anyhow::Error>;
    fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

struct GameSenseDisplay(GameSenseClient);

impl DisplayDriver for GameSenseDisplay {
    fn trigger_frame(
        &mut self,
        event: &str,
        i: isize,
        lines: &[String],
        _buffer: &OledBuffer,
    ) -> Result<(), anyhow::Error> {
        self.0.trigger_event_frame(event, i, lines_to_frame(lines))?;
        Ok(())
    }

    fn send_blank(&mut self) -> Result<(), anyhow::Error> {
        let blank = vec![String::new(); 3];
        self.0.trigger_event_frame("BLANK", 0, lines_to_frame(&blank))?;
        Ok(())
    }

    fn send_white(&mut self) -> Result<(), anyhow::Error> {
        let white = vec!["████████████████".to_string(); 3];
        self.0.trigger_event_frame("WHITE", 0, lines_to_frame(&white))?;
        Ok(())
    }

    fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
        self.0.stop_heartbeat()?;
        Ok(())
    }
}

struct HidDisplay {
    device: hidapi::HidDevice,
    model: &'static crate::devices::SupportedDevice,
}

impl HidDisplay {
    fn send(&self, buffer: &OledBuffer) -> Result<(), anyhow::Error> {
        for packet in self.model.protocol.build_packets(buffer) {
            self.device.send_feature_report(&packet).map_err(|e| {
                error!("Failed to send HID frame: {}", e);
                anyhow!("HID send failed: {}", e)
            })?;
        }
        Ok(())
    }
}

impl DisplayDriver for HidDisplay {
    fn trigger_frame(
        &mut self,
        _event: &str,
        _i: isize,
        _lines: &[String],
        buffer: &OledBuffer,
    ) -> Result<(), anyhow::Error> {
        self.send(buffer)
    }

    // Blank and white are best-effort: a failed send must not tear down the
    // connection when the user asks the screen to sleep.
    fn send_blank(&mut self) -> Result<(), anyhow::Error> {
        let _ = self.send(&OledBuffer::new(self.model.width, self.model.height));
        Ok(())
    }

    fn send_white(&mut self) -> Result<(), anyhow::Error> {
        let _ = self.send(&white_buffer(self.model.width, self.model.height));
        Ok(())
    }
}
```

In `connect_all` replace

```rust
            self.oled = Some(Box::new(OledClient::Hid {
                sender: Box::new(device),
                device: supported,
            }));
```

with

```rust
            self.oled = Some(Box::new(HidDisplay {
                device,
                model: supported,
            }));
```

and `self.oled = Some(Box::new(OledClient::GameSense(gg)));` with `self.oled = Some(Box::new(GameSenseDisplay(gg)));`.

- [ ] **Step 2: Remove the tests of the deleted layers**

In `mod tests` delete: `test_oled_client_send_blank_hid_variant_does_not_panic_on_invalid_device`, `test_oled_client_send_blank_gamesense_unsupported_in_tests`, `nova_device`, `FakeHidSender` (struct and both impls), and every test whose name starts with `test_oled_client_`. Remove `stop_heartbeat` from `impl DisplayDriver for MockDriver` only if you also remove the `stop_calls` field; otherwise leave `MockDriver` as is. Keep `test_white_buffer_all_pixels_on`.

- [ ] **Step 3: Add a test that the daemon drives whatever driver is installed**

Replace the two tests `test_daemon_handle_sleep_command_calls_driver_send_blank` and `test_daemon_handle_sleep_command_calls_driver_send_white` (they only asserted flags because the mock's counters were unreachable) with a mock that shares its counters:

```rust
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Calls {
        frames: AtomicUsize,
        blank: AtomicUsize,
        white: AtomicUsize,
        stop: AtomicUsize,
    }

    struct CountingDriver(Arc<Calls>);

    impl DisplayDriver for CountingDriver {
        fn trigger_frame(
            &mut self,
            _event: &str,
            _i: isize,
            _lines: &[String],
            _buffer: &OledBuffer,
        ) -> Result<(), anyhow::Error> {
            self.0.frames.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn send_blank(&mut self) -> Result<(), anyhow::Error> {
            self.0.blank.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn send_white(&mut self) -> Result<(), anyhow::Error> {
            self.0.white.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
            self.0.stop.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn test_daemon_drives_installed_display_driver() {
        let calls = Arc::new(Calls::default());
        let mut d = daemon_for_tests();
        d.oled = Some(Box::new(CountingDriver(calls.clone())));

        d.render(&full_summary_hwinfo(), true).unwrap();
        d.handle_sleep_command(SleepCommand::Sleep);
        d.handle_sleep_command(SleepCommand::White);
        d.disconnect_and_cleanup();

        assert_eq!(calls.frames.load(Ordering::Relaxed), 1);
        assert_eq!(calls.blank.load(Ordering::Relaxed), 1);
        assert_eq!(calls.white.load(Ordering::Relaxed), 1);
        assert_eq!(calls.stop.load(Ordering::Relaxed), 1);
        assert!(d.oled.is_none());
    }
```

- [ ] **Step 4: Run the tests**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Expected: 0 failed; no new clippy warnings.

- [ ] **Step 5: Commit**

```bash
git add -A src-tauri
git commit -m "refactor: GameSense and HID as two DisplayDriver impls

Removes the OledClient enum, its forwarding impl and the HidSender trait.

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 7: Inline one-line helpers; one pause flag

Small functions that exist so a test can call them, each used once. Inline them and delete the tests that only restated them. `is_sleeping` and `is_white_screen` are only ever read together (`a || b`), so they become one `paused` flag.

**Files:**
- Modify: `src-tauri/src/daemon.rs`, `src-tauri/src/main.rs`, `src-tauri/src/connect.rs`, `src-tauri/src/gui.rs`
- Test: the `mod tests` of the same files

**Interfaces:**
- Consumes: everything from Tasks 3 to 6.
- Produces: `Daemon.paused: bool`, `Daemon::record_failure(&self, what: &str, err: &anyhow::Error)`.
- Removes: `apply_sleep_flags`, `page_event_name`, the seven `announce_*`/`after_*` methods, `record_connect_failure`, `record_tick_failure`, `find_flag_position`, `has_required_arg_after`, `validate_expected_battery_strict`, `matches_oled_ids`, `is_steelseries_vendor`, `get_status_impl`, `get_config_impl`, `get_live_preview_impl`.

- [ ] **Step 1: `daemon.rs`, pause flag and status helpers**

Replace the fields `is_sleeping: bool,` and `is_white_screen: bool,` with:

```rust
    /// True while the display is asleep or showing the white screen; ticks
    /// draw nothing until a Wake command.
    paused: bool,
```

and their two initializers with `paused: false,`.

Delete `apply_sleep_flags` and `page_event_name`. Replace `handle_sleep_command` with:

```rust
    fn handle_sleep_command(&mut self, cmd: SleepCommand) {
        self.paused = cmd != SleepCommand::Wake;
        let Some(oled) = self.oled.as_mut() else {
            return;
        };
        let result = match cmd {
            SleepCommand::Sleep => oled.send_blank(),
            SleepCommand::White => oled.send_white(),
            SleepCommand::Wake => Ok(()),
        };
        if let Err(e) = result {
            error!("{:?} command failed: {}", cmd, e);
        }
    }
```

In `tick` replace `if self.is_sleeping || self.is_white_screen {` with `if self.paused {`.

In `render` replace

```rust
        let event_name = page_event_name(self.page_counter);
        oled.trigger_frame(&event_name, self.i.0, &lines, &buffer)?;
```

with

```rust
        let event_name = format!("PAGE{}", self.page_counter + 1);
        oled.trigger_frame(&event_name, self.i.0, &lines, &buffer)?;
```

Delete `announce_connecting_hwinfo`, `after_hwinfo_connected`, `announce_connecting_direct_usb`, `announce_connecting_gamesense`, `after_direct_usb_connected`, `after_gamesense_connected`, `record_connect_failure`, `record_tick_failure`. Add:

```rust
    /// Show a "Connecting to ..." message in the settings window.
    fn announce(&self, mode: Option<ActiveMode>, message: &str) {
        self.write_state(|s| {
            if let Some(mode) = mode {
                s.active_mode = mode;
            }
            s.last_error = Some(message.to_string());
        });
        self.push_status();
    }

    /// Log a failure and surface it in the settings window.
    fn record_failure(&self, what: &str, err: &anyhow::Error) {
        error!("Daemon: {}: {}", what, err);
        self.write_state(|s| {
            s.last_error = Some(format!("{}: {}", what, err));
        });
        self.push_status();
    }
```

Replace `connect_all` with:

```rust
    fn connect_all(&mut self) -> Result<(), anyhow::Error> {
        self.announce(None, "Connecting to HWiNFO...");
        self.hwinfo = Some(Hwinfo::new().map_err(|e| {
            anyhow!(
                "HWiNFO not reachable ({}). Is it running with Shared Memory Support enabled?",
                e
            )
        })?);
        self.write_state(|s| {
            s.hwinfo_connected = true;
            s.last_error = None;
        });
        self.push_status();

        if let Err(e) = self.media_reader.initialize() {
            warn!("MediaReader init failed: {}. Media sensors unavailable.", e);
        }

        if self.config.direct_usb {
            self.announce(
                Some(ActiveMode::DirectUsb),
                "Connecting to SteelSeries HID...",
            );

            let api = hidapi::HidApi::new().map_err(|e| anyhow!("HID API init failed: {}", e))?;
            let (device, supported) = connect_hid(&api, &self.config.direct_usb_serial)?;
            info!(
                "Direct USB device: {} ({}x{})",
                supported.name, supported.width, supported.height
            );
            self.set_display_size((supported.width, supported.height));
            self.oled = Some(Box::new(HidDisplay {
                device,
                model: supported,
            }));
            self.hid_api = Some(api);
        } else {
            self.set_display_size((128, 64));
            self.announce(
                Some(ActiveMode::GameSense),
                "Connecting to SteelSeries GG...",
            );

            let mut gg = connect_steelseries()?;
            let line_keys: Vec<String> =
                (1..=DISPLAY_LINES).map(|j| format!("line{}", j)).collect();
            let line_refs: Vec<&str> = line_keys.iter().map(|s| s.as_str()).collect();
            for i in 1..=self.config.pages {
                let handler = page_handler(3, &line_refs, None);
                gg.bind_event(&format!("PAGE{}", i), None, None, None, None, vec![handler])?;
            }
            gg.start_heartbeat();
            self.oled = Some(Box::new(GameSenseDisplay(gg)));
        }

        let direct_usb = self.config.direct_usb;
        self.write_state(|s| {
            s.usb_connected = direct_usb;
            s.gg_connected = !direct_usb;
            s.last_error = None;
        });
        self.push_status();
        Ok(())
    }

    fn set_display_size(&mut self, size: (u32, u32)) {
        self.display_size = size;
        self.write_state(|s| {
            s.display_size = size;
            s.oled_buffer = OledBuffer::new(size.0, size.1);
        });
    }
```

Update the callers: in `tick`, the reload-failure block becomes

```rust
        if reload {
            if let Err(e) = self.reload() {
                self.record_failure("Reload failed", &e);
            }
        }
```

and in `run`, `self.record_connect_failure(&e)` (both occurrences) becomes `self.record_failure("Connect failed", &e)` and `self.record_tick_failure(&e)` becomes `self.record_failure("Tick error", &e)`. The resulting status texts (`Reload failed: ...`, `Connect failed: ...`, `Tick error: ...`) are identical to today's.

- [ ] **Step 2: `daemon.rs` tests**

Delete: `test_apply_sleep_flags`, `test_page_event_name`, `test_announce_connecting_hwinfo_sets_status`, `test_after_hwinfo_connected_clears_error`, `test_announce_connecting_direct_usb_sets_mode`, `test_announce_connecting_gamesense_sets_mode`, `test_after_direct_usb_connected_sets_flags`, `test_after_gamesense_connected_sets_flags`, `test_record_connect_failure_writes_state`, `test_record_tick_failure_writes_state`, `test_shared_state_initial_disconnect_count_zero`, `test_mock_app_creates_handle`.

In `test_daemon_new_initializes_fields` replace the two lines `assert!(!d.is_sleeping);` and `assert!(!d.is_white_screen);` with `assert!(!d.paused);`.

Replace `test_daemon_handle_sleep_command_sets_flags`, `test_daemon_tick_processes_sleep_and_skips_when_sleeping`, `test_daemon_tick_processes_white_screen`, `test_daemon_tick_wake_unsets_flags` and `test_daemon_tick_reload_request_errors_when_no_conf` with:

```rust
    #[test]
    fn test_handle_sleep_command_sets_paused() {
        let mut d = daemon_for_tests();
        d.handle_sleep_command(SleepCommand::Sleep);
        assert!(d.paused);
        d.handle_sleep_command(SleepCommand::Wake);
        assert!(!d.paused);
        d.handle_sleep_command(SleepCommand::White);
        assert!(d.paused);
    }

    #[test]
    fn test_tick_drains_sleep_request_and_skips_while_paused() {
        let mut d = daemon_for_tests();
        d.state.lock().unwrap().sleep_requested = Some(SleepCommand::Sleep);
        // No hwinfo installed: tick would fail if it did not return early.
        assert!(d.tick().is_ok());
        assert!(d.paused);
        assert!(d.state.lock().unwrap().sleep_requested.is_none());
    }

    #[test]
    fn test_tick_wake_resumes() {
        let mut d = daemon_for_tests();
        d.paused = true;
        d.state.lock().unwrap().sleep_requested = Some(SleepCommand::Wake);
        // Resumed, so tick goes on and fails on the missing hwinfo.
        assert!(d.tick().is_err());
        assert!(!d.paused);
    }

    #[test]
    fn test_tick_clears_reload_request_even_when_reload_fails() {
        let mut d = daemon_for_tests();
        d.paused = true; // return right after the reload attempt
        d.state.lock().unwrap().reload_requested = true;
        assert!(d.tick().is_ok());
        assert!(!d.state.lock().unwrap().reload_requested);
    }

    #[test]
    fn test_record_failure_writes_status_text() {
        let d = daemon_for_tests();
        d.record_failure("Connect failed", &anyhow!("boom"));
        assert_eq!(
            d.state.lock().unwrap().last_error.as_deref(),
            Some("Connect failed: boom")
        );
    }

    #[test]
    fn test_announce_sets_mode_and_message() {
        let d = daemon_for_tests();
        d.announce(Some(ActiveMode::DirectUsb), "Connecting to SteelSeries HID...");
        let g = d.state.lock().unwrap();
        assert_eq!(g.active_mode, ActiveMode::DirectUsb);
        assert_eq!(g.last_error.as_deref(), Some("Connecting to SteelSeries HID..."));
    }
```

- [ ] **Step 3: `main.rs` CLI helpers**

Delete `find_flag_position`, `has_required_arg_after`, `validate_expected_battery_strict`. Replace `wants_open_settings`, `run_discover_all_battery`, `run_discover_mouse_battery` and `handle_discovery_flags` with:

```rust
/// Whether argv requests opening the settings window on launch (`--settings`).
fn wants_open_settings(args: &[String]) -> bool {
    args.iter().any(|a| a == "--settings")
}

/// Execute "--discover-all-battery" given argv and the flag's position.
/// On success returns the expected battery value used (for testing).
fn run_discover_all_battery(args: &[String], pos: usize) -> Result<u8, String> {
    let raw = args.get(pos + 1).ok_or_else(|| {
        format!(
            "Usage: {} --discover-all-battery <EXPECTED_BATTERY>",
            args[0]
        )
    })?;
    let expected = parse_expected_battery(Some(raw))
        .ok_or_else(|| "Expected battery must be 0-100".to_string())?;
    let hid_api =
        hidapi::HidApi::new().map_err(|e| format!("Failed to initialize HID API: {}", e))?;
    mouse_battery::discover_all_devices_for_battery(&hid_api, expected)
        .map_err(|e| e.to_string())?;
    Ok(expected)
}

/// Execute "--discover-mouse-battery" given argv and the flag's position.
fn run_discover_mouse_battery(
    args: &[String],
    pos: usize,
) -> Result<(u16, Option<u16>, Option<u8>), String> {
    let raw_vid = args.get(pos + 1).ok_or_else(|| {
        format!(
            "Usage: {} --discover-mouse-battery <VID> [PID] [EXPECTED]",
            args[0]
        )
    })?;
    let vid = parse_hex_u16(raw_vid).ok_or_else(|| "Invalid VID format".to_string())?;
    let pid = parse_optional_pid(args.get(pos + 2).map(String::as_str));
    let expected = parse_expected_battery(args.get(pos + 3).map(String::as_str));
    let hid_api = hidapi::HidApi::new().map_err(|e| format!("Failed to init HID API: {}", e))?;
    mouse_battery::discover_battery_report_id(&hid_api, vid, pid, expected)
        .map(|_| (vid, pid, expected))
        .map_err(|e| e.to_string())
}

/// Run a discovery flag if one is present, then exit.
fn handle_discovery_flags(args: &[String]) {
    let position = |flag: &str| args.iter().position(|a| a == flag);
    let result = if let Some(pos) = position("--discover-all-battery") {
        run_discover_all_battery(args, pos).map(|_| ())
    } else if let Some(pos) = position("--discover-mouse-battery") {
        run_discover_mouse_battery(args, pos).map(|_| ())
    } else {
        return;
    };
    match result {
        Ok(()) => std::process::exit(0),
        Err(e) => {
            eprintln!("{}", e);
            std::process::exit(1);
        }
    }
}
```

Check the two `mouse_battery::discover_*` call signatures against `src-tauri/src/mouse_battery.rs` at HEAD before building; keep the argument lists exactly as the current `main.rs` passes them.

In `mod tests` delete `test_find_flag_position_found_and_not_found`, `test_has_required_arg_after`, `test_validate_expected_battery_strict`. Replace `test_handle_discovery_flags_no_flags_returns_false` with:

```rust
    #[test]
    fn test_handle_discovery_flags_without_flags_returns() {
        // Returning at all is the assertion: with a discovery flag it would exit.
        handle_discovery_flags(&["app".to_string(), "--settings".to_string()]);
    }
```

- [ ] **Step 4: `connect.rs` predicates**

Delete `matches_oled_ids` and `is_steelseries_vendor` with their doc comments. Replace `is_oled_capable` and `list_steelseries_interfaces` with:

```rust
/// Predicate matching SteelSeries OLED-capable HID interfaces.
/// Filters by VID + usage_page only — PID is not constrained so multiple
/// devices (Arctis variants, Apex Pro, etc.) can be selected.
pub fn is_oled_capable(d: &hidapi::DeviceInfo) -> bool {
    d.vendor_id() == HID_VENDOR_ID && d.usage_page() == HID_USAGE_PAGE
}
```

```rust
/// Lists ALL SteelSeries HID interfaces (no usage-page filter), for the
/// device-support report. `list_oled_devices` remains the connect filter.
pub fn list_steelseries_interfaces(api: &HidApi) -> Vec<&hidapi::DeviceInfo> {
    api.device_list()
        .filter(|d| d.vendor_id() == HID_VENDOR_ID)
        .collect()
}
```

In `mod tests` delete `test_matches_oled_ids_true`, `test_matches_oled_ids_wrong_vendor`, `test_matches_oled_ids_wrong_usage_page`, `test_is_steelseries_vendor`.

- [ ] **Step 5: `gui.rs` trivial `_impl` twins**

Delete `get_status_impl`, `get_config_impl`, `get_live_preview_impl` and put their bodies in the commands:

```rust
#[command]
pub fn get_status(state: State<Shared>) -> Result<StatusPayload, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    Ok(g.status_payload())
}

#[command]
pub fn get_config(state: State<Shared>) -> Result<AppConfig, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    Ok(g.config.clone())
}

#[command]
pub fn get_live_preview(state: State<Shared>) -> Result<crate::state::OledFrame, String> {
    let g = state.lock().map_err(|e| e.to_string())?;
    Ok(buffer_to_frame(&g.oled_buffer))
}
```

`save_config_impl`, `list_sensors_impl` and `preview_config_impl` stay: they take a path or do real work, and their tests call them without a Tauri app.

In `mod tests` delete `test_get_status_impl_returns_payload`, `test_get_config_impl_returns_clone`, `test_get_live_preview_impl_returns_frame`. In `test_command_wrappers_via_tauri_state` strengthen the three corresponding lines so the behavior stays covered:

```rust
        let status = get_status(state.clone()).unwrap();
        assert!(!status.hwinfo_connected);
        assert_eq!(status.active_mode, ActiveMode::Disconnected);
        assert_eq!(get_config(state.clone()).unwrap().gpu, "GPU [#0]");
        let frame = get_live_preview(state.clone()).unwrap();
        assert_eq!((frame.width, frame.height), (128, 64));
        assert!(frame.pixels.iter().all(|p| *p == 0));
```

- [ ] **Step 6: Run the tests**

```bash
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
```

Expected: 0 failed; no new clippy warnings.

- [ ] **Step 7: Commit**

```bash
git add -A src-tauri
git commit -m "refactor: inline single-use helpers; one paused flag in the daemon

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```

---

### Task 8: Refresh CLAUDE.md and verify on real hardware

`CLAUDE.md` describes the pre-Tauri app (main loop in `main.rs`, 1000 ms tick, 9 sensors, 3 lines, `tray-icon` crate, console wizard). An agent reading it gets a wrong map.

**Files:**
- Modify: `CLAUDE.md`

**Interfaces:**
- Consumes: the final shape of every module from Tasks 1 to 7.
- Produces: documentation only.

- [ ] **Step 1: Replace the stale sections of `CLAUDE.md`**

Replace everything from `## Architecture` up to (not including) `## Windows-Specific Considerations` with:

```markdown
## Architecture

Cargo workspace; the app is the `src-tauri` package (a Tauri 2 app with a tray icon and a settings window in `src-tauri/ui/index.html`).

### Module Structure

- **lib.rs** (crate `hwinfo_steelseries_oled`): HWiNFO shared-memory reader. `Hwinfo` holds one persistent mapping of `Global\HWiNFO_SENS_SM2`; `pull()` re-reads values and reports whether HWiNFO has polled since the last call. `Snapshot` is the read-only view everything else uses (`get`, `find_first`, `readings`); cloning it copies only the values. Fields are parsed from the mapped bytes at fixed offsets with bounds checks.
- **main.rs**: startup, tray menu, `--settings` and `--discover-*` flags, Tauri command registration.
- **daemon.rs**: background thread. `run()` connects (retrying every 3 s), then `tick()`s every `TICK_RATE` ms: drain reload/sleep requests, `pull()`, `render()` a frame, publish state and events to the UI. `DisplayDriver` has two implementations: `GameSenseDisplay` (text lines to SteelSeries GG) and `HidDisplay` (bitmap over USB).
- **settings.rs**: `AppConfig` parsing from `conf.ini`; `load_or_create_config` writes a default on first run.
- **utils.rs**: `run_sensors` resolves a page of configured sensors (HWiNFO readings plus CLOCK, DATE, BLANK, MOUSE_BATTERY, MEDIA_*, WEATHER_*) into `Slot`s; `format_custom_lines` lays them out as text lines.
- **render.rs**: `OledBuffer` and text/icon rendering for the direct-USB bitmap path.
- **devices.rs**: registry of direct-USB devices and their packet protocols.
- **connect.rs**: HID device discovery and selection; single-attempt connects.
- **gui.rs**: Tauri commands for the settings window (config load/save, sensor list, preview, device report).
- **state.rs**: `SharedState` shared between the daemon and the commands.
- **report.rs**: device-support report and prefilled GitHub issue URL.
- **media.rs**, **weather.rs**, **mouse_battery.rs**: readers behind the special sensors.
- **steelseries.rs**: GameSense screen handler definition.
- **console_utils.rs**: show/hide the console window.
- **consts.rs**: `TICK_RATE` (500 ms), `CUSTOM_SENSORS` (15), `DISPLAY_LINES` (5).

### Data Flow

1. `main` loads `conf.ini` (or writes a default and opens the settings window), builds the tray, spawns the daemon.
2. The daemon connects to HWiNFO and to the display (SteelSeries GG or direct USB).
3. Each tick: `Hwinfo::pull()` → `build_display_lines()` (summary formatter or `run_sensors` + `format_custom_lines`) → `Vec<String>` lines → `DisplayDriver::trigger_frame` (JSON for GameSense, bitmap for HID).
4. No new HWiNFO data for `STALE_LIMIT` (5) ticks shows "Disconnected FROM HWiNFO" and remaps the shared memory until data returns.
5. The settings window saves `conf.ini` and sets `reload_requested`; the daemon reloads on its next tick or connect attempt.
```

In `## Testing` replace the body with:

```markdown
Unit tests live in each module's `mod tests`. They need neither HWiNFO nor a device: `lib.rs` parses synthetic byte buffers, and the daemon is tested through `Daemon::render` with `Snapshot::from_readings` and Tauri's `MockRuntime`.

Run with `CARGO_TEST_HANDLE_FATAL=skip cargo test --manifest-path src-tauri/Cargo.toml` (one test would otherwise wait on stdin).
```

In `## Important Implementation Notes` delete the bullets about `Equality checks`, `Memory safety` and `No mutex needed`, and change the disconnection bullet to: `- **Disconnection detection**: HWiNFO's header poll time unchanged for 5 ticks shows "Disconnected FROM HWiNFO".`

In `## Dependencies` delete the `dialoguer` and `tray-icon` bullets and add `- **tauri**: window, tray icon, commands`.

In `## Common Modification Scenarios` replace the "Adding a new special sensor" steps with:

```markdown
**Adding a new special sensor** (like CLOCK or BLANK):
1. Add a branch in `utils.rs::run_sensors()` that returns `Some(value)` (or `None` to hide the slot)
2. Add it to `SPECIAL_SENSORS` in `gui.rs` so the settings picker lists it
3. Update README.md configuration examples
```

and replace the "Changing disconnection threshold" entry with: `- Change `STALE_LIMIT` in `daemon.rs`.` Delete the "Adding a 4th display line" entry (there are already five).

- [ ] **Step 2: Full verification**

```bash
rtk cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
rtk cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets
rtk cargo test --manifest-path src-tauri/Cargo.toml --lib
CARGO_TEST_HANDLE_FATAL=skip rtk cargo test --manifest-path src-tauri/Cargo.toml --bins
rtk cargo build --manifest-path src-tauri/Cargo.toml --release
```

Expected: fmt clean (run `cargo fmt` and re-check if not), no clippy warnings, 0 test failures, release build succeeds.

- [ ] **Step 3: Manual check on the real device**

This needs the user: the tray app must be exited first, and only they can see the OLED. Ask them to exit the running tray app, then from the worktree root run `src-tauri` in debug with their existing config copied in:

```bash
cp ../../../conf.ini conf.ini 2>/dev/null; RUST_LOG=info cargo run --manifest-path src-tauri/Cargo.toml -- --settings
```

Ask them to confirm each line:

1. OLED shows the same content as before the change (custom pages cycle, or the summary layout).
2. Settings window: sensor picker lists HWiNFO sensors; live preview matches the OLED.
3. Save a change in settings: OLED updates within a second.
4. Close HWiNFO: OLED shows "Disconnected FROM HWiNFO" after about 3 seconds. Reopen HWiNFO: OLED recovers by itself.
5. Tray: Sleep Display blanks the screen; opening Settings wakes it.
6. Delete `conf.ini` in the worktree and start again: settings window opens, a default `conf.ini` exists.

Report any line that fails with the log output; do not mark the task done on a failure.

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: bring CLAUDE.md in line with the Tauri architecture

Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>"
```
