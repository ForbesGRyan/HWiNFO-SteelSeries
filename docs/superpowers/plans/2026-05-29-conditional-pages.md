# Conditional Pages (Process-Gated) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let each custom page join the OLED rotation only while a named process is running (e.g. show an FPS page only while a game is open).

**Architecture:** A new `process_watch` module caches the set of running exe names and exposes pure matching helpers. The daemon computes the set of "active" pages each tick and rotates only through those, falling back to all pages when none qualify. The per-page condition is stored as a `show_when_running` key in each `PAGE{i}.Sensors` ini section, carried on `AppConfig.page_conditions`, and edited via a text field in the settings GUI.

**Tech Stack:** Rust, Tauri 2, `winapi` (toolhelp process enumeration), `rust-ini`, HTML/JS settings UI.

---

## File Structure

- **Create** `src-tauri/src/process_watch.rs` — `ProcessWatcher` (cached running-process set + periodic refresh) and the pure helpers `parse_condition`, `page_is_active`, `active_pages`.
- **Modify** `src-tauri/Cargo.toml` — add `tlhelp32` to the `winapi` features.
- **Modify** `src-tauri/src/main.rs` — declare `mod process_watch;`; extend the inline `AppConfig` literal.
- **Modify** `src-tauri/src/settings.rs` — add `page_conditions` field + `#[serde(default)]`; read `show_when_running` in `from_ini`.
- **Modify** `src-tauri/src/gui.rs` — write `show_when_running` in `apply_pages_sections`; extend the test `AppConfig` literal.
- **Modify** `src-tauri/src/daemon.rs` — hold a `ProcessWatcher`; rework `next_page_counter` + the tick rotation to use active pages; extend the test `AppConfig` literal.
- **Modify** `src-tauri/src/state.rs` — extend the mock `AppConfig` literal.
- **Modify** `src-tauri/ui/index.html` — per-page "Show only while running (exe)" input.

> **Note on field-addition ordering:** Adding `page_conditions` to `AppConfig` breaks every struct-literal construction at once. Task 2 therefore adds the field **and** updates all four literals (settings/daemon/gui/main/state) in a single task so the build stays green.

---

## Task 1: Pure process-matching helpers

**Files:**
- Create: `src-tauri/src/process_watch.rs`
- Modify: `src-tauri/src/main.rs` (add `mod process_watch;`)

- [ ] **Step 1: Declare the module**

In `src-tauri/src/main.rs`, add alongside the other `mod` lines (after line 23, `mod media;`):

```rust
mod process_watch;
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/process_watch.rs` with ONLY the pure helpers' tests and empty stubs:

```rust
use std::collections::HashSet;

/// Split a `show_when_running` value into normalized exe-name tokens:
/// comma-separated, trimmed, lowercased, empties dropped.
pub fn parse_condition(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

/// A page is active if it has no condition, or if any of its tokens is in the
/// running set. `running` must contain lowercased exe filenames.
pub fn page_is_active(condition: &str, running: &HashSet<String>) -> bool {
    let tokens = parse_condition(condition);
    if tokens.is_empty() {
        return true;
    }
    tokens.iter().any(|t| running.contains(t))
}

/// Indices of pages that should be shown. `conditions[i]` is the raw
/// `show_when_running` for page `i` (missing entries treated as unconditional).
/// If nothing qualifies, returns all indices (never leaves the screen empty).
pub fn active_pages(
    conditions: &[String],
    page_count: usize,
    running: &HashSet<String>,
) -> Vec<usize> {
    let mut active: Vec<usize> = (0..page_count)
        .filter(|&i| {
            let cond = conditions.get(i).map(|s| s.as_str()).unwrap_or("");
            page_is_active(cond, running)
        })
        .collect();
    if active.is_empty() {
        active = (0..page_count).collect();
    }
    active
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(names: &[&str]) -> HashSet<String> {
        names.iter().map(|s| s.to_lowercase()).collect()
    }

    #[test]
    fn parse_condition_trims_lowercases_and_drops_empties() {
        assert_eq!(parse_condition(" Game.exe ,, LAUNCHER.exe ,"),
                   vec!["game.exe".to_string(), "launcher.exe".to_string()]);
        assert!(parse_condition("   ").is_empty());
        assert!(parse_condition("").is_empty());
    }

    #[test]
    fn page_is_active_empty_condition_is_always_true() {
        assert!(page_is_active("", &set(&[])));
        assert!(page_is_active("  ", &set(&["x.exe"])));
    }

    #[test]
    fn page_is_active_matches_case_insensitively() {
        assert!(page_is_active("Game.exe", &set(&["game.exe"])));
        assert!(!page_is_active("game.exe", &set(&["other.exe"])));
    }

    #[test]
    fn page_is_active_any_token_matches() {
        assert!(page_is_active("a.exe, b.exe", &set(&["b.exe"])));
        assert!(!page_is_active("a.exe, b.exe", &set(&["c.exe"])));
    }

    #[test]
    fn active_pages_mixes_conditional_and_unconditional() {
        // page0 unconditional, page1 needs game.exe (running), page2 needs x.exe (not)
        let conds = vec!["".to_string(), "game.exe".to_string(), "x.exe".to_string()];
        assert_eq!(active_pages(&conds, 3, &set(&["game.exe"])), vec![0, 1]);
    }

    #[test]
    fn active_pages_all_unconditional_returns_all() {
        let conds = vec!["".to_string(), "".to_string()];
        assert_eq!(active_pages(&conds, 2, &set(&[])), vec![0, 1]);
    }

    #[test]
    fn active_pages_falls_back_to_all_when_none_match() {
        let conds = vec!["a.exe".to_string(), "b.exe".to_string()];
        assert_eq!(active_pages(&conds, 2, &set(&["c.exe"])), vec![0, 1]);
    }

    #[test]
    fn active_pages_tolerates_short_conditions_slice() {
        // Only one condition provided but two pages: page1 treated as unconditional.
        let conds = vec!["x.exe".to_string()];
        assert_eq!(active_pages(&conds, 2, &set(&["x.exe"])), vec![0, 1]);
    }
}
```

- [ ] **Step 3: Run the tests to verify they pass**

Run: `cargo test process_watch::tests`
Expected: PASS (the helpers are already written; this task is the pure core, no Windows code yet).

> If the package name differs, use `cargo test process_watch::tests` from `src-tauri/`. Confirm with `cargo test 2>&1 | head` if unsure.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/process_watch.rs src-tauri/src/main.rs
git commit -m "feat(pages): add pure process-matching helpers"
```

---

## Task 2: Add `page_conditions` to AppConfig and read it from ini

**Files:**
- Modify: `src-tauri/src/settings.rs:84-105` (struct + default fn), `src-tauri/src/settings.rs:185-243` (`from_ini` body)
- Modify (literals): `src-tauri/src/daemon.rs:855`, `src-tauri/src/gui.rs:469`, `src-tauri/src/main.rs:604`, `src-tauri/src/state.rs:98`

- [ ] **Step 1: Write the failing test**

Add to `settings.rs`'s `#[cfg(test)] mod tests`:

```rust
    #[test]
    fn test_appconfig_reads_show_when_running_per_page() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("pages", "2");
        conf.with_section(Some("PAGE1.Sensors"))
            .set("sensor_0", "CPU;Temp");
        conf.with_section(Some("PAGE2.Sensors"))
            .set("show_when_running", "game.exe")
            .set("sensor_0", "GPU;Framerate");

        let config = AppConfig::from_ini(&conf).unwrap();
        assert_eq!(config.page_conditions.len(), 2);
        assert_eq!(config.page_conditions[0], "");
        assert_eq!(config.page_conditions[1], "game.exe");
    }

    #[test]
    fn test_appconfig_page_conditions_default_empty_when_absent() {
        let mut conf = Ini::new();
        conf.with_section(Some("Main"))
            .set("style", "Custom")
            .set("pages", "1");
        conf.with_section(Some("PAGE1.Sensors")).set("sensor_0", "CPU;Temp");
        let config = AppConfig::from_ini(&conf).unwrap();
        assert_eq!(config.page_conditions, vec!["".to_string()]);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test test_appconfig_reads_show_when_running_per_page`
Expected: FAIL to compile — `no field page_conditions on type AppConfig`.

- [ ] **Step 3: Add the field and default**

In `src-tauri/src/settings.rs`, add to the `AppConfig` struct (after the `font_sizes` field, around line 100):

```rust
    #[serde(default)]
    pub page_conditions: Vec<String>,
```

- [ ] **Step 4: Populate it in `from_ini`**

In `AppConfig::from_ini`, the returned `Ok(Self { ... })` builds `custom_sensors` from a `for i in 1..=pages` loop. Add a sibling field that reads the condition per page. Insert this field into the `Ok(Self { ... })` literal (after `font_sizes,`):

```rust
            page_conditions: {
                let mut conds = Vec::with_capacity(pages);
                for i in 1..=pages {
                    let cond = config
                        .section(Some(format!("PAGE{}.Sensors", i)))
                        .and_then(|s| s.get("show_when_running"))
                        .unwrap_or("")
                        .trim()
                        .to_string();
                    conds.push(cond);
                }
                conds
            },
```

- [ ] **Step 5: Fix the four struct literals**

`AppConfig` is also built by hand in tests/mocks. In EACH of these, add the line `page_conditions: Vec::new(),` immediately before the `font_sizes:` line:

- `src-tauri/src/daemon.rs:855` (the `base_config()` helper)
- `src-tauri/src/gui.rs:469` (the `base_config()` helper)
- `src-tauri/src/main.rs:604` (the `SharedState::new(AppConfig { ... })`)
- `src-tauri/src/state.rs:98` (the `mock_config()` helper)

Example (daemon.rs), the literal becomes:

```rust
            custom_sensors: Vec::new(),
            weather: WeatherConfig::default(),
            page_conditions: Vec::new(),
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
```

- [ ] **Step 6: Run tests to verify they pass**

Run: `cargo test -p hwinfo-steelseries`
Expected: PASS, build green (all literals updated).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/settings.rs src-tauri/src/daemon.rs src-tauri/src/gui.rs src-tauri/src/main.rs src-tauri/src/state.rs
git commit -m "feat(pages): add page_conditions to AppConfig, read show_when_running"
```

---

## Task 3: Persist `show_when_running` when saving config (GUI write path)

**Files:**
- Modify: `src-tauri/src/gui.rs:198-225` (`apply_pages_sections`)
- Test: `src-tauri/src/gui.rs` test module

- [ ] **Step 1: Write the failing test**

Add to `gui.rs`'s test module (near `test_apply_pages_sections_writes_sensors`):

```rust
    #[test]
    fn test_apply_pages_sections_writes_show_when_running() {
        let mut config = base_config();
        config.custom_sensors = vec![
            vec![sensor("CPU;Temp")],
            vec![sensor("GPU;Framerate")],
        ];
        config.page_conditions = vec!["".to_string(), "game.exe".to_string()];

        let mut ini = Ini::new();
        apply_pages_sections(&mut ini, &config);

        // Unconditional page omits the key.
        assert!(ini
            .section(Some("PAGE1.Sensors"))
            .unwrap()
            .get("show_when_running")
            .is_none());
        // Conditional page writes it.
        assert_eq!(
            ini.section(Some("PAGE2.Sensors")).unwrap().get("show_when_running"),
            Some("game.exe")
        );
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test test_apply_pages_sections_writes_show_when_running`
Expected: FAIL — assertion on `PAGE2.Sensors` `show_when_running` is `None`.

- [ ] **Step 3: Implement the write**

In `apply_pages_sections`, inside the `for (i, page) in config.custom_sensors.iter().enumerate()` loop, right after `let mut section = ini.with_section(Some(section_name));`, add:

```rust
            if let Some(cond) = config.page_conditions.get(i) {
                let cond = cond.trim();
                if !cond.is_empty() {
                    section.set("show_when_running", cond);
                }
            }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test test_apply_pages_sections_writes_show_when_running`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/gui.rs
git commit -m "feat(pages): persist show_when_running when saving config"
```

---

## Task 4: Toolhelp process enumeration + `ProcessWatcher`

**Files:**
- Modify: `src-tauri/Cargo.toml:22` (winapi features)
- Modify: `src-tauri/src/process_watch.rs` (add `ProcessWatcher` + enumeration)

- [ ] **Step 1: Add the winapi feature**

In `src-tauri/Cargo.toml`, change line 22 to include `tlhelp32`:

```toml
winapi = {version="0.3", features=["handleapi", "memoryapi", "winnt", "wincon", "winuser", "tlhelp32"]}
```

- [ ] **Step 2: Write the failing test for the watcher's pure behavior**

Add to the `tests` module in `process_watch.rs`:

```rust
    #[test]
    fn watcher_starts_empty_and_accepts_injected_set() {
        let mut w = ProcessWatcher::new();
        assert!(w.running().is_empty());
        w.set_running_for_test(set(&["game.exe"]));
        assert!(w.running().contains("game.exe"));
    }

    #[test]
    fn watcher_refresh_is_rate_limited() {
        // After an injected set, an immediate refresh at a near tick must NOT
        // re-enumerate (which would clobber the injected value within the window).
        let mut w = ProcessWatcher::new();
        w.set_running_for_test(set(&["sentinel.exe"]));
        w.refresh(1); // first refresh records the tick but our window blocks re-enum? See note.
        // The injected sentinel proves no enumeration replaced it within the window.
        // (Enumeration only runs when due; set_running_for_test seeds last_refresh_tick.)
        assert!(w.running().contains("sentinel.exe"));
    }
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test watcher_starts_empty`
Expected: FAIL to compile — `ProcessWatcher` not found.

- [ ] **Step 4: Implement `ProcessWatcher` + enumeration**

Add to the top of `process_watch.rs` (after the existing `use` line):

```rust
/// Re-enumerate running processes at most this many daemon ticks apart.
/// TICK_RATE is 500 ms, so 4 ticks ≈ 2 seconds.
const REFRESH_TICKS: isize = 4;

/// Caches the set of running exe filenames (lowercased) and refreshes it on a
/// coarse interval so the per-tick page-gating check is cheap.
pub struct ProcessWatcher {
    running: HashSet<String>,
    last_refresh_tick: Option<isize>,
}

impl ProcessWatcher {
    pub fn new() -> Self {
        Self {
            running: HashSet::new(),
            last_refresh_tick: None,
        }
    }

    pub fn running(&self) -> &HashSet<String> {
        &self.running
    }

    /// Refresh the cache if at least `REFRESH_TICKS` have elapsed since the last
    /// enumeration. `tick` is the daemon's monotonic tick counter.
    pub fn refresh(&mut self, tick: isize) {
        let due = match self.last_refresh_tick {
            None => true,
            Some(t) => (tick - t).abs() >= REFRESH_TICKS,
        };
        if !due {
            return;
        }
        if let Some(set) = enumerate_processes() {
            self.running = set;
        }
        self.last_refresh_tick = Some(tick);
    }

    #[cfg(test)]
    pub fn set_running_for_test(&mut self, names: HashSet<String>) {
        self.running = names;
        self.last_refresh_tick = Some(0);
    }
}

impl Default for ProcessWatcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Enumerate running processes via the Win32 toolhelp snapshot. Returns
/// lowercased exe filenames, or `None` if the snapshot could not be taken.
#[cfg(windows)]
fn enumerate_processes() -> Option<HashSet<String>> {
    use std::os::windows::ffi::OsStringExt;
    use winapi::um::handleapi::{CloseHandle, INVALID_HANDLE_VALUE};
    use winapi::um::tlhelp32::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
        TH32CS_SNAPPROCESS,
    };

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snapshot == INVALID_HANDLE_VALUE {
            return None;
        }

        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        let mut set = HashSet::new();
        if Process32FirstW(snapshot, &mut entry) != 0 {
            loop {
                let len = entry
                    .szExeFile
                    .iter()
                    .position(|&c| c == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = std::ffi::OsString::from_wide(&entry.szExeFile[..len])
                    .to_string_lossy()
                    .to_lowercase();
                if !name.is_empty() {
                    set.insert(name);
                }
                if Process32NextW(snapshot, &mut entry) == 0 {
                    break;
                }
            }
        }
        CloseHandle(snapshot);
        Some(set)
    }
}

#[cfg(not(windows))]
fn enumerate_processes() -> Option<HashSet<String>> {
    None
}
```

> Note: `set_running_for_test` seeds `last_refresh_tick = Some(0)`, so the rate-limit test's `refresh(1)` is within the window (`|1-0| < 4`) and returns early without enumerating — the injected sentinel survives, proving the gate works.

- [ ] **Step 5: Run tests to verify pass**

Run: `cargo test process_watch`
Expected: PASS (all pure + watcher tests).

- [ ] **Step 6: Verify the crate builds with the new winapi feature**

Run: `cargo build`
Expected: builds clean (no missing-symbol errors from toolhelp).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/src/process_watch.rs
git commit -m "feat(pages): add ProcessWatcher with toolhelp enumeration"
```

---

## Task 5: Rotate over active pages in the daemon

**Files:**
- Modify: `src-tauri/src/daemon.rs:298-309` (`next_page_counter`), `src-tauri/src/daemon.rs:396-441` (struct + `new`), `src-tauri/src/daemon.rs:678-705` (tick rotation)
- Test: `src-tauri/src/daemon.rs` test module

- [ ] **Step 1: Write the failing tests for the reworked `next_page_counter`**

The signature's third param changes meaning from `pages` to `active_len`. Replace the existing `next_page_counter` tests (`test_next_page_counter_*`) with these (they assert the new clamp behavior; numeric results are unchanged when the active set is full):

```rust
    #[test]
    fn test_next_page_counter_advances_at_interval() {
        // TICK_RATE = 500 → ticks_per_second = 2 → interval = page_time*2
        assert_eq!(next_page_counter(10, 5, 3, 0), 1);
        assert_eq!(next_page_counter(20, 5, 3, 1), 2);
        assert_eq!(next_page_counter(30, 5, 3, 2), 0); // wraps
    }

    #[test]
    fn test_next_page_counter_keeps_when_not_at_boundary() {
        assert_eq!(next_page_counter(5, 5, 3, 0), 0);
        assert_eq!(next_page_counter(1, 5, 3, 1), 1);
    }

    #[test]
    fn test_next_page_counter_zero_tick_does_not_advance() {
        assert_eq!(next_page_counter(0, 5, 3, 0), 0);
    }

    #[test]
    fn test_next_page_counter_handles_zero_active_or_page_time() {
        assert_eq!(next_page_counter(10, 5, 0, 0), 0);
        assert_eq!(next_page_counter(10, 0, 3, 1), 1);
        assert_eq!(next_page_counter(10, -1, 3, 1), 1);
    }

    #[test]
    fn test_next_page_counter_clamps_when_active_set_shrinks() {
        // current points past the end (active set shrank from 3 to 2) → clamp.
        assert_eq!(next_page_counter(5, 5, 2, 2), 1); // not a boundary: clamp only
        assert_eq!(next_page_counter(10, 5, 2, 2), 0); // boundary: clamp(→1) then advance→0
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test test_next_page_counter_clamps_when_active_set_shrinks`
Expected: FAIL — current impl does not clamp (`next_page_counter(5,5,2,2)` returns 2).

- [ ] **Step 3: Rework `next_page_counter`**

Replace the existing function (`src-tauri/src/daemon.rs:298-309`) with:

```rust
/// Advance the rotation index within the *active* page set. `current` is an
/// index into the active list; it is clamped when the active set shrinks (e.g.
/// a gated page drops out mid-cycle) and advanced once per `page_time` window.
fn next_page_counter(i: isize, page_time: isize, active_len: usize, current: usize) -> usize {
    if active_len == 0 {
        return 0;
    }
    let current = current.min(active_len - 1);
    if page_time <= 0 {
        return current;
    }
    let ticks_per_second = 1000 / TICK_RATE as isize;
    let interval = page_time * ticks_per_second;
    if interval > 0 && i != 0 && i % interval == 0 {
        (current + 1) % active_len
    } else {
        current
    }
}
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test test_next_page_counter`
Expected: PASS (all five tests).

- [ ] **Step 5: Add the watcher to the `Daemon` struct**

In `src-tauri/src/daemon.rs`:

Add the import near the other `use crate::...` lines (top of file):

```rust
use crate::process_watch::{active_pages, ProcessWatcher};
```

Add a field to `struct Daemon` (after `weather_reader`, around line 414):

```rust
    process_watcher: ProcessWatcher,
```

Initialize it in `Daemon::new`'s returned `Self { ... }` (after `weather_reader,`, around line 437):

```rust
            process_watcher: ProcessWatcher::new(),
```

- [ ] **Step 6: Rework the tick rotation to use active pages**

In `Daemon::tick`, replace the custom-mode rotation block (`src-tauri/src/daemon.rs:678-690`, the `if !self.config.is_summary { let next = next_page_counter(...) ... }`) and the subsequent `build_display_value` / `page_event_name` usage. Replace from the rotation block down to the `oled.trigger_frame(...)` call with:

```rust
        // Advance the rotation over the *active* (process-gated) page set.
        let display_page = if self.config.is_summary {
            0
        } else {
            self.process_watcher.refresh(self.i.0);
            let active = active_pages(
                &self.config.page_conditions,
                self.config.pages,
                self.process_watcher.running(),
            );
            self.page_counter =
                next_page_counter(self.i.0, self.config.page_time, active.len(), self.page_counter);
            let resolved = active.get(self.page_counter).copied().unwrap_or(0);
            debug!(
                "Active pages {:?}, rotation idx {} → page {}",
                active,
                self.page_counter,
                resolved + 1
            );
            resolved
        };

        let value = build_display_value(
            &self.config,
            hwinfo,
            &self.pages_vec,
            display_page,
            &mut self.mouse_battery_reader,
            &mut self.media_reader,
            &self.weather_reader,
            self.hid_api.as_ref(),
        )?;

        let buffer = value_to_oled_buffer(&value, &self.config.font_sizes);
        let event_name = page_event_name(display_page);
        oled.trigger_frame(&event_name, self.i.0, &value, &buffer)?;
```

> This removes the old standalone `let value = build_display_value(... self.page_counter ...)`, `let event_name = page_event_name(self.page_counter)` lines — they are replaced by the block above. `page_counter` now indexes the active list; `display_page` is the real page index used everywhere downstream.

- [ ] **Step 7: Write an integration test for gated rotation**

Add to the `daemon.rs` test module:

```rust
    #[test]
    fn test_daemon_tick_skips_unmatched_conditional_page() {
        let mut d = daemon_for_tests();
        d.config.is_summary = false;
        d.config.pages = 2;
        d.config.page_time = 1;
        // Page 2 gated on a process that is NOT running.
        d.config.page_conditions = vec!["".to_string(), "definitely-not-running.exe".to_string()];
        let mut hw = build_hwinfo(&[("S", "R", 1.0)]);
        hw.bypass_pull_for_test = true;
        d.hwinfo = Some(hw);
        install_mock_driver(&mut d);
        d.pages_vec = vec![ini::Properties::new(), ini::Properties::new()];
        // Force the watcher to a known empty set so page 2 stays gated out.
        d.process_watcher.set_running_for_test(std::collections::HashSet::new());

        // Run several ticks across multiple rotation intervals.
        for _ in 0..6 {
            let _ = d.tick();
        }
        // Only page 1 is active → rotation index never leaves the single active page.
        assert_eq!(d.page_counter, 0);
    }
```

- [ ] **Step 8: Run the daemon tests to verify pass**

Run: `cargo test daemon`
Expected: PASS, including the existing happy-path/custom-mode rotation tests (which use empty `page_conditions` → all pages active → unchanged behavior).

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/daemon.rs
git commit -m "feat(pages): rotate only over process-active pages"
```

---

## Task 6: Settings GUI field for the per-page condition

**Files:**
- Modify: `src-tauri/ui/index.html` (page-tab UI around lines 420-460 and the per-page render/save logic near lines 837-1060)

> This task is UI wiring with no Rust test harness (the project has no JS tests). Verify manually by launching the settings window.

- [ ] **Step 1: Add the input element**

In `src-tauri/ui/index.html`, find the page controls block (near the `page-time` input, ~line 423). Add, just below the page-time row:

```html
        <label for="page-condition">Show only while running (exe)</label>
        <input type="text" id="page-condition" placeholder="e.g. game.exe (blank = always)">
```

- [ ] **Step 2: Ensure the config object always carries `page_conditions`**

In the JS, find `ensurePage(idx)` (~line 511). After it ensures `config.custom_sensors[idx]` exists, also ensure the conditions array:

```javascript
        function ensurePageCondition(idx) {
            if (!Array.isArray(config.page_conditions)) config.page_conditions = [];
            while (config.page_conditions.length <= idx) config.page_conditions.push('');
        }
```

- [ ] **Step 3: Load the field when switching/rendering a page**

Find where the active page is rendered (the page-tab click handler / `renderLines`, ~line 848 and ~line 894 where config loads). Add a helper and call it whenever `activePage` changes and on initial config load:

```javascript
        function renderPageCondition() {
            ensurePageCondition(activePage);
            const el = document.getElementById('page-condition');
            if (el) el.value = config.page_conditions[activePage] || '';
        }
```

Call `renderPageCondition();` in the page-tab click handler (right after `activePage = i;`) and once after the config is loaded (near where `document.getElementById('pages').value = config.pages` is set, ~line 894).

- [ ] **Step 4: Write the field back on edit**

Where the other inputs register `change` listeners (~line 1026-1040), add:

```javascript
        document.getElementById('page-condition').addEventListener('input', () => {
            ensurePageCondition(activePage);
            config.page_conditions[activePage] = document.getElementById('page-condition').value.trim();
        });
```

- [ ] **Step 5: Include it in the saved config**

Find the save handler that assembles the config before `invoke('save_config', ...)` (~line 1057, where `config.pages` / `config.page_time` are read from the DOM). Ensure `config.page_conditions` is sized to the page count so trailing pages persist correctly:

```javascript
                for (let i = 0; i < config.pages; i++) ensurePageCondition(i);
                config.page_conditions = config.page_conditions.slice(0, config.pages);
```

- [ ] **Step 6: Manual verification**

Run: `cargo tauri dev` (or the project's normal dev launch), open Settings → Custom mode.
Expected:
- Each page tab shows the "Show only while running (exe)" field.
- Typing `game.exe` on page 2, saving, then reopening shows the value retained.
- Inspect `conf.ini`: `[PAGE2.Sensors]` contains `show_when_running=game.exe`.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/ui/index.html
git commit -m "feat(pages): settings UI field for per-page process condition"
```

---

## Task 7: Documentation + full verification

**Files:**
- Modify: `README.md` (custom-mode configuration section), `CLAUDE.md` (Special Sensor Features / config notes)

- [ ] **Step 1: Document the feature in README.md**

Add a short subsection under the Custom Mode configuration docs describing `show_when_running`:

```markdown
#### Conditional Pages

Add `show_when_running` to any `[PAGEn.Sensors]` section to show that page only
while a process is running. Useful for a game-only FPS page.

```ini
[PAGE2.Sensors]
show_when_running = game.exe
sensor_0 = ...
```

- Case-insensitive, matches the exe filename.
- Comma-separated for multiple matches: `show_when_running = game.exe, launcher.exe`.
- Pages with no `show_when_running` always show.
- If no conditional page currently matches, all pages rotate as a fallback.
```

- [ ] **Step 2: Note it in CLAUDE.md**

Under "Configuration System" → "Custom Mode", add a bullet:

```markdown
- **Conditional pages**: `show_when_running="exe[,exe...]"` in a PAGE section gates
  that page to only appear while a matching process runs (see `process_watch.rs`).
```

- [ ] **Step 3: Full test + lint sweep**

Run: `cargo test -p hwinfo-steelseries`
Expected: all PASS.

Run: `cargo clippy -- -D warnings`
Expected: no warnings (matches the repo's clippy-clean convention).

Run: `cargo fmt`
Expected: no diff after formatting (or commit the formatting).

- [ ] **Step 4: Commit**

```bash
git add README.md CLAUDE.md
git commit -m "docs: document conditional (process-gated) pages"
```

---

## Self-Review Notes

- **Spec coverage:** config format → Tasks 2/3/6; ProcessWatcher + pure helpers → Tasks 1/4; daemon rotation + fallback → Task 5; GUI field → Task 6; testing → embedded per task; docs → Task 7. All spec sections covered.
- **Type consistency:** `active_pages(conditions, page_count, running)`, `page_is_active(condition, running)`, `parse_condition(raw)`, `ProcessWatcher::{new, running, refresh, set_running_for_test}`, and `next_page_counter(i, page_time, active_len, current)` are used identically wherever referenced.
- **Fallback:** implemented once, in `active_pages` (returns all indices when nothing qualifies), and relied on by the daemon — no duplicate fallback logic.
```
