# Conditional Pages (Process-Gated) — Design

**Date:** 2026-05-29
**Status:** Approved for planning

## Problem

Custom mode rotates through all configured pages on a fixed timer. Users want
pages that appear only in a relevant context — e.g. a page showing FPS that
joins the rotation only while a game is running.

## Solution Overview

Each custom page gets an optional **process condition**: one or more exe names.
The page joins the normal rotation only while a matching process is running.
Pages with no condition behave exactly as today (always shown).

Decisions made during brainstorming:

- **Behavior:** Join rotation. A matched conditional page is one more page in
  the normal cycle; an unmatched one is skipped. No takeover, no priority.
- **Trigger:** Process running (anywhere on the system), not foreground focus.
- **Config surface:** `conf.ini` **and** the settings GUI.
- **Empty fallback:** If zero pages qualify (all conditional, none running),
  ignore conditions and rotate through every page so the screen never goes
  blank.

This feature controls **page visibility only**. The FPS value itself comes from
whatever framerate sensor the user places on the page (an HWiNFO/RTSS-fed
sensor). No new sensor type is added.

## Config Format

New key per `PAGE{i}.Sensors` section:

```ini
[PAGE2.Sensors]
show_when_running = game.exe
sensor_0 = GPU [#0]: ...;Framerate
```

- Empty or absent → unconditional page (always shown).
- Match is **case-insensitive exact** on the exe filename (`game.exe`).
- Comma-separated list allowed; page is active if **any** token matches:
  `show_when_running = game.exe, launcher.exe`

## Components

### New module: `src-tauri/src/process_watch.rs`

Separates Windows I/O from pure logic for testability.

**`ProcessWatcher`** (stateful, lives in `Daemon` alongside the other readers):
- Caches a `HashSet<String>` of lowercased running exe filenames.
- Refreshes via `CreateToolhelp32Snapshot` + `Process32FirstW`/`Process32NextW`
  at most every ~2 seconds (refresh keyed off the daemon tick counter, since
  `TICK_RATE` is 500 ms → refresh every ~4 ticks). Enumeration itself is thin
  and untested, matching the convention of the other Windows-backed readers.
- `running(&self) -> &HashSet<String>` returns the current cache.

**Pure helpers** (no Windows; fully unit-tested):
- `parse_condition(raw: &str) -> Vec<String>` — split on commas, trim,
  lowercase, drop empty tokens.
- `page_is_active(condition: &str, running: &HashSet<String>) -> bool` — empty
  condition → `true`; otherwise `true` if any parsed token is in `running`.
- `active_pages(conditions: &[String], running: &HashSet<String>) -> Vec<usize>`
  — indices of active pages. **If the result is empty, return all indices**
  (the fallback).

### `AppConfig` (`settings.rs`)

- Add `page_conditions: Vec<String>`, parallel to `custom_sensors` (index =
  page number). `#[serde(default)]` so existing serialized configs/state load
  cleanly.
- `from_ini`: read `show_when_running` from each `PAGE{i}.Sensors` section into
  `page_conditions[i-1]` (empty string when absent).

### Daemon rotation (`daemon.rs`)

- Each tick, compute `active = active_pages(&config.page_conditions,
  watcher.running())`.
- Rotation cycles over `active` instead of the raw `0..pages` range.
  `page_counter` is treated as a position within `active`; resolve the real
  page index as `active[page_counter % active.len()]`.
- Clamp `page_counter` when `active` shrinks (e.g. the game closes mid-cycle) so
  it never indexes past the end.
- `next_page_counter` reworked to advance within `active.len()` on the same
  timer interval as today (`page_time` × ticks-per-second).

### GUI (`gui.rs` + `ui/index.html`)

- `gui.rs::apply_pages_sections`: write `show_when_running` per page from
  `page_conditions`.
- `gui.rs` config round-trip carries `page_conditions` through save/load.
- `index.html`: one text input per page tab — *"Show only while running
  (exe):"* — bound to `config.page_conditions[activePage]`, persisted with the
  rest of the page config.

## Testing

Pure-function unit tests:
- `parse_condition`: trimming, lowercasing, comma splitting, empty-token drop.
- `page_is_active`: empty → true; single match; no match; case-insensitivity.
- `active_pages`: mixed conditional/unconditional; all-unconditional;
  fallback-to-all when nothing matches.
- `next_page_counter` rework: advancing within a shrunk active set; clamp on
  shrink.

Round-trip tests:
- `show_when_running` survives `ini → AppConfig → ini` via `settings.rs` and
  `gui.rs`, including the empty/absent case.

Untested (by convention, like the other readers):
- The raw toolhelp process enumeration in `ProcessWatcher`.

## Out of Scope

- A built-in FPS/RTSS sensor type. FPS comes from a user-selected sensor.
- Foreground-window detection (chosen trigger is process-running).
- Per-page priority or takeover behavior.

## Dependencies

No new crates. Process enumeration uses `winapi`/toolhelp already present in the
tree.
