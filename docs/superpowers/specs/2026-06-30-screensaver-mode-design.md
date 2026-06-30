# Screensaver mode — design

- **Date:** 2026-06-30
- **Status:** Approved (pending spec review)
- **Branch:** feat/usb-device-registry (or a new feat/screensaver branch)

## Summary

Add an idle-triggered **screensaver** to the OLED display whose purpose is **OLED
burn-in prevention**. The normal sensor display runs as today; after the PC has
had no keyboard/mouse input for N minutes, the screen switches to a **bouncing
clock** (12-hour `H:MM`, DVD-logo style motion). Any keyboard/mouse activity
returns it to the normal sensor display.

The animation is rendered with full pixel control on the **direct-USB** path. In
**GameSense** mode — where we can only hand SteelSeries three lines of text and
cannot position pixels — the idle state falls back to the existing **blank Sleep**
(burn-in is still prevented; there is just no animation).

## Goals

- Prevent burn-in by ensuring lit pixels move when the machine is idle.
- Keep the idle display glanceable (you can read the time across the room).
- Zero behavior change for existing users until they opt in.
- Small, isolated, independently testable units; no Windows calls in the testable core.

## Non-goals

- Smooth high-frame-rate animation (the daemon ticks at 500 ms; a slow drift is fine).
- GameSense pixel-positioned animation (constrained by SteelSeries' renderer).
- Detecting "idle" by sensor staleness (we use real system input idle).
- A general animation framework / multiple screensaver styles (one: bouncing clock).

## Decisions (from brainstorming)

| Question | Decision |
|---|---|
| Purpose | Burn-in prevention |
| Trigger | Idle-triggered: PC idle N min → animation; activity → back to sensors |
| Content | Bouncing clock (DVD-logo bounce) |
| Output scope | Direct-USB animates; GameSense falls back to blank Sleep when idle |
| Default timeout | 5 minutes (GUI seeds this when enabled) |
| Default state | OFF — `screensaver_idle_minutes` defaults to `0` (disabled) |
| Clock format | 12-hour, no am/pm, e.g. `3:42` |
| GUI | Include the settings control now |

## Behavior

1. Each daemon tick (500 ms), after the existing reload/sleep handling and the
   `is_sleeping`/`is_white_screen` early-return (so **manual Sleep/White always
   wins**), the daemon reads the system-wide idle time.
2. `should_screensave = config.screensaver_idle_minutes > 0
   && idle_ms >= screensaver_idle_minutes * 60_000`.
3. If `should_screensave`:
   - **Direct-USB:** advance the bounce, render the clock into an `OledBuffer` at
     `(x, y)`, send the frame, push frame + status to the GUI, set
     `screensaver_active = true`, advance `i`, and return early (skip the sensor
     build and HWiNFO disconnect check for this tick).
   - **GameSense:** if not already in screensaver, send blank once (reuse
     `OledClient::send_blank`); set `screensaver_active = true`; return early.
     Subsequent idle ticks do nothing (no re-blank spamming).
4. If not idle and `screensaver_active` was set, clear it (wake): the normal
   sensor render resumes on this tick (direct-USB) / next frame (GameSense).

Notes:
- While the screensaver is active, HWiNFO disconnect detection is paused (the user
  is away; this is acceptable and keeps the path simple).
- Manual Sleep/White, set via tray or GUI, short-circuit before the screensaver
  logic, so they continue to take precedence.

## Components

Each unit has one purpose, a clear interface, and is testable in isolation.

### 1. `src-tauri/src/idle.rs` (new) — system idle detection

```rust
/// Milliseconds since the last system-wide keyboard/mouse input.
pub trait IdleSource: Send {
    fn idle_ms(&self) -> u64;
}

/// Real Windows implementation: GetLastInputInfo + GetTickCount.
pub struct SystemIdle;
impl IdleSource for SystemIdle { fn idle_ms(&self) -> u64 { /* winapi */ } }

/// Pure, OS-independent decision (unit-tested without Windows).
pub fn should_screensave(idle_ms: u64, minutes: u64) -> bool {
    minutes > 0 && idle_ms >= minutes.saturating_mul(60_000)
}
```

- `SystemIdle::idle_ms` calls `GetLastInputInfo` (fills `LASTINPUTINFO.dwTime`, a
  `GetTickCount` timestamp) and computes `GetTickCount().wrapping_sub(dwTime)` to
  survive the ~49.7-day tick-count wraparound. Returns `0` if the call fails.
- Requires adding the `sysinfoapi` feature to the `winapi` dependency
  (`GetTickCount`); `winuser` (already enabled) provides `GetLastInputInfo`.

### 2. `src-tauri/src/screensaver.rs` (new) — the animation (no I/O)

```rust
pub struct Screensaver { x: i32, y: i32, vx: i32, vy: i32 }

impl Screensaver {
    pub fn new() -> Self;                 // start near center, vx/vy = (2, 1)
    /// Advance one step; reflect off edges keeping the text box in bounds.
    /// If text is wider/taller than the screen, that axis pins at 0 (no jitter).
    pub fn advance(&mut self, text_w: u32, text_h: u32, screen_w: u32, screen_h: u32);
    /// Render the time string at the current (x, y) into a fresh OledBuffer.
    pub fn render(&self, time_str: &str, font: FontSize, w: u32, h: u32) -> OledBuffer;
}

/// 12-hour H:MM with no am/pm, e.g. "3:42".
pub fn clock_text(now: &chrono::DateTime<chrono::Local>) -> String; // hour12() + minute()
```

- DVD bounce: `x += vx; y += vy;` then if the text box would cross an edge, clamp
  to the edge and negate that velocity component.
- `render` delegates to `render_text_at` (below). `clock_text` takes the time as a
  parameter so tests pass a fixed instant.

### 3. `src-tauri/src/render.rs` (small additions)

```rust
/// Draw `text` at an arbitrary (x, y) text baseline.
/// (Today's render_text_to_oled positions x but fixes y per text line.)
pub fn render_text_at(text: &str, x: i32, y: i32, font: FontSize, w: u32, h: u32) -> OledBuffer;

impl FontSize {
    pub fn char_width(&self) -> u32;   // monospace advance, from profont metrics
    pub fn char_height(&self) -> u32;  // glyph cell height, from profont metrics
}
```

- `char_width`/`char_height` come from the `PROFONT_9/12/18_POINT` font metrics
  already imported in `render.rs`; the screensaver multiplies `char_width` by the
  clock string length to size the bounce box.

### 4. `src-tauri/src/settings.rs` (`AppConfig`)

- New field:
  ```rust
  #[serde(default)]
  pub screensaver_idle_minutes: u64,
  ```
- Parse in `from_ini`: `main.get("screensaver_idle_minutes").and_then(|v|
  v.parse::<u64>().ok()).unwrap_or(0)` (missing/invalid → `0` = disabled).
- `#[serde(default)]` keeps old serialized configs and existing test constructors
  valid (field defaults to `0`). The test `AppConfig { .. }` literals in
  daemon.rs/state.rs gain `screensaver_idle_minutes: 0`.

### 5. `src-tauri/src/daemon.rs`

- `Daemon` gains: `screensaver: Screensaver`, `idle_source: Box<dyn IdleSource>`,
  `screensaver_active: bool`. `Daemon::new` defaults `idle_source` to
  `Box::new(SystemIdle)`; tests inject a mock.
- New pure helper kept testable:
  `fn screensaver_frame(saver: &mut Screensaver, time: &str, font: FontSize,
  size: (u32,u32)) -> OledBuffer` (advance + render), so a test can assert the
  buffer changes between ticks.
- `tick()` gains the branch described in **Behavior** (after the
  `is_sleeping || is_white_screen` early return, before the HWiNFO pull).

### 6. `src-tauri/src/state.rs` + GUI

- `SharedState` + `StatusPayload`: add `screensaver_active: bool` so the GUI can
  show a "Screensaver" indicator. Default `false`.
- `src-tauri/src/gui.rs` → `apply_main_section`: add
  `.set("screensaver_idle_minutes", config.screensaver_idle_minutes.to_string())`.
  `AppConfig`'s new serde field round-trips through `get_config`/`save_config`
  automatically.
- `src-tauri/ui/index.html`: add a "Screensaver (burn-in protection)" toggle and an
  idle-minutes number input, following the existing `page-time` pattern:
  - On load: set the input from `config.screensaver_idle_minutes` (toggle on when
    `> 0`).
  - On save: `config.screensaver_idle_minutes = toggle ? (minutes || 5) : 0`.
  - Enabling the toggle seeds the minutes field with `5` if empty.

## Config schema

```ini
[Main]
; 0 = disabled (default). When > 0, the OLED shows a bouncing clock after the
; PC has been idle (no keyboard/mouse) for this many minutes. Direct-USB only;
; in GameSense mode the screen blanks instead.
screensaver_idle_minutes = 5
```

## Testing plan

- **idle.rs:** `should_screensave` — `0` minutes always false; below/at/above
  threshold; `saturating_mul` guards overflow. (`SystemIdle` itself is not unit
  tested — it is a thin winapi shim.)
- **screensaver.rs:**
  - `advance` reflects velocity at each edge and never leaves bounds over many steps.
  - text larger than screen pins the axis instead of oscillating.
  - `render` lights pixels; position differs after `advance`.
  - `clock_text` formats a known instant to `H:MM` with no am/pm and no leading zero.
- **render.rs:** `render_text_at` lights pixels and differs by `y` offset;
  `char_width/char_height` are non-zero and ordered Small < Medium < Large.
- **daemon.rs:** with a mock `IdleSource`:
  - high idle + direct-USB → `screensaver_active` set, a frame is sent (MockDriver),
    sensor build skipped.
  - high idle + GameSense → blank sent once, not re-sent on the next idle tick.
  - low idle after active → flag cleared, normal sensor render resumes.
  - manual Sleep still short-circuits screensaver.
- **settings.rs:** parse present/missing/invalid `screensaver_idle_minutes`
  (→ value / 0 / 0).
- **gui.rs:** `apply_main_section` writes `screensaver_idle_minutes`; round-trips
  through save/get.

## Edge cases

- **Tick-count wraparound:** `wrapping_sub` in `SystemIdle::idle_ms`.
- **Tiny / non-128×64 screens (e.g. Apex 128×40):** bounce bounds use the live
  `display_size`; oversized text pins instead of jittering.
- **Mode switch / reload while active:** `reload()` reconnects and `save_config`
  issues `SleepCommand::Wake`; `screensaver_active` is recomputed each tick, so it
  self-corrects.
- **Slow motion:** at 500 ms/tick with `vx/vy = (2,1)`, drift is gentle and
  adequate for burn-in. Speeding up ticks during the screensaver is intentionally
  out of scope (would touch the main loop timing).

## Out of scope / future

- Multiple screensaver styles (logo, starfield, drifting sensors).
- GameSense coarse-bounce (cycling the clock across the three text lines).
- Faster animation frame rate during the screensaver.
- Dimming/brightness control (not exposed by the current send paths).
