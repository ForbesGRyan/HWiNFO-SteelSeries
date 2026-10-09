# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

HWiNFO-SteelSeries is a Windows-only Rust application that displays real-time hardware monitoring data from HWiNFO on SteelSeries OLED screens (Arctis Pro Wireless, Apex Pro, etc.). It reads sensor data from HWiNFO's shared memory and sends it to SteelSeries devices via the GameSense API or direct USB (HID). Runs as a Tauri tray app with a settings window.

## Build Commands

```bash
# Development build
cargo build
cargo run

# Release build (optimized)
cargo build --release

# Run tests
cargo test

# Format code (before commits)
cargo fmt

# Lint and check for issues
cargo clippy
```

## Packaging for Release

Use the provided scripts to create distributable packages:

```cmd
# Windows Batch
package-release.bat

# PowerShell (with optional skip build flag)
.\package-release.ps1
.\package-release.ps1 -SkipBuild
```

The icon is embedded at compile time, making the executable fully standalone.

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
3. Each tick: `Hwinfo::pull()` -> `build_display_lines()` (summary formatter or `run_sensors` + `format_custom_lines`) -> `Vec<String>` lines -> `DisplayDriver::trigger_frame` (JSON for GameSense, bitmap for HID).
4. No new HWiNFO data for `STALE_LIMIT` (5) ticks shows "Disconnected FROM HWiNFO" and remaps the shared memory until data returns.
5. The settings window saves `conf.ini` and sets `reload_requested`; the daemon reloads on its next tick or connect attempt.

### Special Sensor Features

- **CLOCK** / **DATE**: current time / date (optional `;FORMAT` strftime suffix)
- **BLANK**: Empty sensor slot for spacing
- **Unit Conversion**: `convert_X="MB/GB"` divides value by 1024 (for memory sensors)

### Configuration System

Config file: `conf.ini` (auto-generated on first run)

**Summary Modes**:
- Vertical: 3-column layout (CPU/GPU/MEM with temp, usage, memory stats)
- Horizontal: 3-row layout (one component per row)
- Both support decimal places and GPU selection

**Custom Mode**:
- Up to 15 sensors (5 lines x up to 3 sensors per line)
- Multi-page support with configurable page_time
- Sensor format: `"Category Name;Reading Name"` (e.g., `"GPU [#0]: NVIDIA GeForce RTX 3090;GPU Temperature"`)
- Each sensor has: label, unit, optional conversion

## Windows-Specific Considerations

This is a **Windows-only application** due to:
- HWiNFO's shared memory interface (Windows API: `OpenFileMappingW`, `MapViewOfFile`)
- Console window management (Windows API: `GetConsoleWindow`, `ShowWindow`)
- SteelSeries GG is Windows/macOS only (this app targets Windows)

All Windows API calls are in `lib.rs` (shared memory) and `console_utils.rs` (console visibility).

## Testing

Unit tests live in each module's `mod tests`. They need neither HWiNFO nor a device: `lib.rs` parses synthetic byte buffers, and the daemon is tested through `Daemon::render` with `Snapshot::from_readings` and Tauri's `MockRuntime`.

Run with `CARGO_TEST_HANDLE_FATAL=skip cargo test --manifest-path src-tauri/Cargo.toml` (one test would otherwise wait on stdin).

## Logging

Uses `env_logger` crate. Set `RUST_LOG` environment variable:
```bash
RUST_LOG=debug cargo run  # Verbose logging
RUST_LOG=info cargo run   # Default level
```

## Important Implementation Notes

- **Console auto-hide**: In release builds, console window auto-hides after 500ms. Shown again on fatal errors.
- **Disconnection detection**: HWiNFO's header poll time unchanged for 5 ticks shows "Disconnected FROM HWiNFO".
- **System tray**: Icon embedded from `assets/hwinfo-steelseries-icon.ico`. Right-click menu has "Exit" option.
- **Page cycling**: In Custom mode with multiple pages, `page_counter` increments every `page_time` seconds (cycle wraps using modulo).

## Dependencies

Key external crates:
- **gamesense** (0.1.2): SteelSeries GameSense API client
- **winapi**: Windows API bindings for shared memory and console management
- **ini** (rust-ini): Configuration file parsing
- **tauri**: window, tray icon, commands
- **chrono**: CLOCK special sensor
- **image**: Embedded icon loading
- **anyhow**: Error handling
- **log** / **env_logger**: Logging infrastructure

## Common Modification Scenarios

**Adding a new special sensor** (like CLOCK or BLANK):
1. Add a branch in `utils.rs::run_sensors()` that returns `Some(value)` (or `None` to hide the slot)
2. Add it to `SPECIAL_SENSORS` in `gui.rs` (or `WEATHER_SENSORS` for weather fields) so the settings picker lists it
3. Update README.md configuration examples

**Changing update frequency**:
- Modify `TICK_RATE` in `consts.rs` (value is in milliseconds)

**Changing disconnection threshold**:
- Change `STALE_LIMIT` in `daemon.rs`.
