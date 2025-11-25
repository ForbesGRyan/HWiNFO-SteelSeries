# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

HWiNFO-SteelSeries is a Windows-only Rust application that displays real-time hardware monitoring data from HWiNFO on SteelSeries OLED screens (Arctis Pro Wireless, Apex Pro, etc.). It reads sensor data from HWiNFO's shared memory and sends it to SteelSeries devices via their GameSense API.

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

### Module Structure

- **main.rs**: Application entry point and main event loop
  - Initializes system tray with embedded icon
  - Connects to HWiNFO and SteelSeries services
  - Loads/creates configuration (conf.ini)
  - Main loop: polls HWiNFO data, formats it, sends to OLED devices
  - Handles disconnection detection and error recovery

- **lib.rs**: HWiNFO shared memory interface
  - Uses Windows API to map HWiNFO's `Global\HWiNFO_SENS_SM2` shared memory
  - Parses sensor and reading structures from shared memory
  - Provides `get()`, `find()`, and `find_first()` methods for sensor access
  - Main data structures: `Hwinfo`, `Sensor`, `HwinfoSensorsReadingElement`, `HwinfoSensorsSensorElement`

- **steelseries.rs**: SteelSeries GameSense API wrapper
  - Creates screen handlers for OLED display (3 lines of text)
  - Uses `gamesense` crate for API communication

- **settings.rs**: Configuration wizard and INI file management
  - Interactive setup for Summary (Vertical/Horizontal) or Custom modes
  - Handles GPU selection when multiple GPUs detected
  - Defines `AppConfig` struct for parsed configuration

- **connect.rs**: Connection handlers with retry logic
  - Provides `connect_hwinfo()` and `connect_steelseries()` functions
  - Both functions retry every 3 seconds until successful connection

- **utils.rs**: Sensor data processing utilities
  - `run_sensors()`: Reads sensor values, handles special sensors (CLOCK, BLANK), applies unit conversions
  - `format_custom_value()`: Formats sensor data for OLED display

- **console_utils.rs**: Windows console window management
  - Controls console visibility (auto-hide in release mode)
  - Display helpers for debug output

- **consts.rs**: Application constants
  - `TICK_RATE`: 1000ms (update frequency)
  - `CUSTOM_SENSORS`: 9 (max sensors, 3 lines × 3 sensors)
  - `DISPLAY_LINES`: 3
  - `Style` enum: Vertical, Horizontal, Custom

### Data Flow

1. **Initialization**: Connect to HWiNFO shared memory → Connect to SteelSeries GG → Load/create config
2. **Main Loop** (every 1000ms):
   - Pull fresh data from HWiNFO via `hwinfo.pull()`
   - Check for disconnection (data unchanged for 5 cycles)
   - If Summary mode: fetch pre-defined CPU/GPU/MEM sensors, format as Vertical or Horizontal layout
   - If Custom mode: read user-configured sensors from PAGE sections, cycle through pages
   - Send formatted JSON to SteelSeries via `trigger_event_frame()`
3. **Error Handling**: Fatal errors show console window, display error chain, wait for user input

### Special Sensor Features

- **CLOCK**: Displays current time (12-hour format with am/pm)
- **BLANK**: Empty sensor slot for spacing
- **RTSS**: Framerate from RivaTuner Statistics Server (if installed)
- **Unit Conversion**: `convert_X="MB/GB"` divides value by 1024 (for memory sensors)

### Configuration System

Config file: `conf.ini` (auto-generated on first run)

**Summary Modes**:
- Vertical: 3-column layout (CPU/GPU/MEM with temp, usage, memory stats)
- Horizontal: 3-row layout (one component per row)
- Both support decimal places and GPU selection

**Custom Mode**:
- Up to 9 sensors (3 lines × 1-3 sensors per line)
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

All modules have comprehensive unit tests:
- **main.rs**: Summary formatting tests (vertical/horizontal, with/without decimals)
- **lib.rs**: HWiNFO sensor lookup methods (`get()`, `find()`, `find_first()`), equality checks
- **settings.rs**: Configuration parsing, defaults, edge cases
- **utils.rs**: Custom value formatting, empty labels, multi-sensor layouts
- **consts.rs**: Constant values, Style enum display

Tests use mock data structures to avoid requiring actual HWiNFO/SteelSeries connections.

## Logging

Uses `env_logger` crate. Set `RUST_LOG` environment variable:
```bash
RUST_LOG=debug cargo run  # Verbose logging
RUST_LOG=info cargo run   # Default level
```

## Important Implementation Notes

- **Console auto-hide**: In release builds, console window auto-hides after 500ms. Shown again on HWiNFO disconnection or fatal errors.
- **Disconnection detection**: If HWiNFO data unchanged for 5 cycles (5 seconds), display "Disconnected FROM HWiNFO" on OLED.
- **System tray**: Icon embedded from `assets/hwinfo-steelseries-icon.ico`. Right-click menu has "Exit" option.
- **Page cycling**: In Custom mode with multiple pages, `page_counter` increments every `page_time` seconds (cycle wraps using modulo).
- **Equality checks**: `Hwinfo`, `Sensor`, and `HwinfoSensorsReadingElement` implement `PartialEq` for disconnection detection (comparing old vs new data).
- **Memory safety**: Shared memory view is properly unmapped in `Hwinfo::drop()` and after each `pull()`.
- **No mutex needed**: HWiNFO's SM2 format doesn't require mutex synchronization (unlike the older SM format).

## Dependencies

Key external crates:
- **gamesense** (0.1.2): SteelSeries GameSense API client
- **winapi**: Windows API bindings for shared memory and console management
- **ini** (rust-ini): Configuration file parsing
- **dialoguer**: Interactive prompts for setup wizard
- **tray-icon**: System tray integration
- **chrono**: CLOCK special sensor
- **image**: Embedded icon loading
- **anyhow**: Error handling
- **log** / **env_logger**: Logging infrastructure

## Common Modification Scenarios

**Adding a new special sensor** (like CLOCK or BLANK):
1. Add case in `utils.rs::run_sensors()` to detect sensor name
2. Populate `labels[k]`, `units[k]`, `values[k]` with desired output
3. Update README.md configuration examples

**Changing update frequency**:
- Modify `TICK_RATE` in `consts.rs` (value is in milliseconds)

**Adding a 4th display line**:
1. Update `DISPLAY_LINES` in `consts.rs`
2. Update `CUSTOM_SENSORS` if needed (e.g., 12 for 4 lines × 3 sensors)
3. Modify `page_handler()` in `steelseries.rs` to accept 4 labels
4. Update formatting logic in `utils.rs::format_custom_value()`

**Changing disconnection threshold**:
- In `main.rs::run_application()`, change the `limit` parameter in `check_hwinfo_connection()` call (currently 5 cycles)
