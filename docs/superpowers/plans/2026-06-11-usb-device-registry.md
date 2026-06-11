# Direct-USB Supported-Device Registry Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the single hardcoded direct-USB packet format with a static registry of supported devices (PIDs, screen size, packet protocol), make the render pipeline resolution-aware, and surface unsupported devices in the GUI as "not yet supported".

**Architecture:** A new `devices.rs` module owns a compile-time `SUPPORTED_DEVICES` table and a `Protocol` enum whose `build_packets()` produces the full HID frame for a device. `OledBuffer` gains runtime `width`/`height`. `connect.rs` only connects registry devices; the daemon carries the matched registry entry in `OledClient::Hid` and renders at its dimensions. Frame/preview payloads carry `{width, height, pixels}` so the GUI canvas adapts.

**Tech Stack:** Rust (Tauri app in `src-tauri/`), `hidapi`, `embedded-graphics`, vanilla-JS frontend in `src-tauri/ui/index.html`.

**Spec:** `docs/superpowers/specs/2026-06-11-usb-device-registry-design.md`

**Notes for the implementer:**
- All `cargo` commands run from the repo root (`C:\Users\Ryan\code\HWiNFO-SteelSeries`).
- `rtk` (token-filter wrapper) is NOT installed on this machine — use plain `cargo`/`git`.
- The spec says "the settings.rs wizard lists supported devices only" — verified during planning: the console wizard never enumerates devices (it only asks GameSense vs Direct USB; device selection happens in the GUI). **No wizard change is needed.**
- Verified protocol facts (apex-tux `apex-hardware/src/usb.rs`):
  - PID→model: `0x1610` Apex Pro, `0x1612` Apex 7, `0x1614` Apex Pro TKL, `0x1618` Apex 7 TKL, `0x161C` Apex 5 — all "Legacy" protocol, 128×40.
  - Legacy packet: one 641-byte feature report = `0x61` + 640-byte SSD1306 page-major bitmap.
  - Gen3 PIDs (`0x1640`, `0x1644`, `0x1646`) are out of scope.

---

### Task 1: Resolution-aware `OledBuffer`

`OledBuffer` stores `width`/`height` and a `Vec<u8>` instead of `[u8; 1024]`. Constructor becomes `new(width, height)`. Internal layout stays column-major pages (per column, `height/8` bytes, bit 0 = top of page).

**Files:**
- Modify: `src-tauri/src/render.rs` (struct + impls + every `OledBuffer::new()` in tests)
- Modify: `src-tauri/src/daemon.rs` (call sites + `OledBuffer { data: ... }` literals)
- Modify: `src-tauri/src/gui.rs` (test call site)
- Modify: `src-tauri/src/state.rs` (init call site)

- [ ] **Step 1: Write failing tests for the new constructor and 128×40 bounds**

Add to the `tests` module in `src-tauri/src/render.rs`:

```rust
#[test]
fn test_new_sizes_buffer_for_dimensions() {
    let b64 = OledBuffer::new(128, 64);
    assert_eq!(b64.width, 128);
    assert_eq!(b64.height, 64);
    assert_eq!(b64.data.len(), 128 * 64 / 8); // 1024

    let b40 = OledBuffer::new(128, 40);
    assert_eq!(b40.data.len(), 128 * 40 / 8); // 640
}

#[test]
fn test_set_pixel_respects_instance_bounds() {
    let mut b40 = OledBuffer::new(128, 40);
    b40.set_pixel(0, 39, true); // in bounds
    b40.set_pixel(0, 40, true); // out of bounds for 40-tall: no-op, no panic
    b40.set_pixel(127, 0, true);
    b40.set_pixel(128, 0, true); // no-op

    // (0, 39): column 0, page 4, bit 7
    assert_eq!(b40.data[4], 0x80);
    // (127, 0): column 127, page 0, bit 0; 5 pages per column
    assert_eq!(b40.data[127 * 5], 0x01);
}

#[test]
fn test_get_chunk_uses_instance_pages() {
    let mut b40 = OledBuffer::new(128, 40);
    b40.set_pixel(2, 0, true);
    let chunk = b40.get_chunk(0, 4); // 4 columns × 5 pages
    assert_eq!(chunk.len(), 20);
    assert_eq!(chunk[2 * 5], 0x01);
}

#[test]
fn test_buffer_clone_is_deep() {
    let mut a = OledBuffer::new(128, 64);
    a.set_pixel(1, 1, true);
    let b = a.clone();
    assert_eq!(a, b);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml render::tests::test_new_sizes -- --nocapture`
Expected: COMPILE ERROR (`new` takes 0 args, no `width` field) — that is the failure signal for this step.

- [ ] **Step 3: Rewrite the `OledBuffer` struct and impls**

In `src-tauri/src/render.rs`, replace the struct, `new`, `set_pixel`, `get_chunk`, `DrawTarget`, and `OriginDimensions` (currently around lines 69–131):

```rust
/// A buffer for a SteelSeries OLED screen. Layout is column-major pages:
/// for each column, `height/8` bytes; within a byte, bit 0 is the topmost pixel.
#[derive(Debug, Clone, PartialEq)]
pub struct OledBuffer {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl OledBuffer {
    pub fn new(width: u32, height: u32) -> Self {
        debug_assert!(height % 8 == 0, "OLED height must be a multiple of 8");
        Self {
            width,
            height,
            data: vec![0u8; (width * height / 8) as usize],
        }
    }

    /// Bytes per column (= height / 8).
    fn pages(&self) -> usize {
        (self.height / 8) as usize
    }

    pub fn set_pixel(&mut self, x: u32, y: u32, on: bool) {
        if x >= self.width || y >= self.height {
            return;
        }
        let pages = self.pages();
        let idx = x as usize * pages + (y / 8) as usize;
        let bit = (y % 8) as u8;

        if on {
            self.data[idx] |= 1 << bit;
        } else {
            self.data[idx] &= !(1 << bit);
        }
    }

    pub fn get_chunk(&self, x_offset: u8, width: u8) -> Vec<u8> {
        let pages = self.pages();
        let mut chunk = Vec::with_capacity(width as usize * pages);
        for x in x_offset..(x_offset + width) {
            let start = x as usize * pages;
            chunk.extend_from_slice(&self.data[start..start + pages]);
        }
        chunk
    }
}

impl DrawTarget for OledBuffer {
    type Color = BinaryColor;
    type Error = core::convert::Infallible;

    fn draw_iter<I>(&mut self, pixels: I) -> Result<(), Self::Error>
    where
        I: IntoIterator<Item = Pixel<Self::Color>>,
    {
        for Pixel(point, color) in pixels.into_iter() {
            if point.x >= 0
                && point.x < self.width as i32
                && point.y >= 0
                && point.y < self.height as i32
            {
                self.set_pixel(point.x as u32, point.y as u32, color.is_on());
            }
        }
        Ok(())
    }
}

impl OriginDimensions for OledBuffer {
    fn size(&self) -> Size {
        Size::new(self.width, self.height)
    }
}
```

- [ ] **Step 4: Update every call site (mechanical)**

- `src-tauri/src/render.rs`: `render_text_to_oled` body: `OledBuffer::new()` → `OledBuffer::new(128, 64)` (made dynamic in Task 3). Every test calling `OledBuffer::new()` → `OledBuffer::new(128, 64)`.
- `src-tauri/src/state.rs:68`: `oled_buffer: OledBuffer::new()` → `OledBuffer::new(128, 64)`.
- `src-tauri/src/daemon.rs`:
  - `white_buffer()` (line ~149): `OledBuffer::new()` → `OledBuffer::new(128, 64)` (generalized in Task 5).
  - `send_blank` (line ~253): `OledBuffer::new()` → `OledBuffer::new(128, 64)`.
  - Lines ~668 and ~715: `s.oled_buffer = OledBuffer { data: buffer.data }` → `s.oled_buffer = buffer.clone()`.
  - All test call sites `OledBuffer::new()` → `OledBuffer::new(128, 64)`.
- `src-tauri/src/gui.rs:1195` (test): `OledBuffer::new()` → `OledBuffer::new(128, 64)`.

- [ ] **Step 5: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS (including the four new tests).

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/render.rs src-tauri/src/daemon.rs src-tauri/src/gui.rs src-tauri/src/state.rs
git commit -m "refactor(render): resolution-aware OledBuffer with runtime dimensions"
```

---

### Task 2: `to_page_major()` serialization for the Apex protocol

**Files:**
- Modify: `src-tauri/src/render.rs`

- [ ] **Step 1: Write failing tests**

Add to the `tests` module in `src-tauri/src/render.rs`:

```rust
#[test]
fn test_to_page_major_length_and_ordering() {
    let mut b = OledBuffer::new(128, 40);
    // (0,0) → page 0, column 0, bit 0 → out[0] = 0x01
    b.set_pixel(0, 0, true);
    // (5,9) → page 1, column 5, bit 1 → out[1*128 + 5] = 0x02
    b.set_pixel(5, 9, true);
    // (127,39) → page 4, column 127, bit 7 → out[4*128 + 127] = 0x80
    b.set_pixel(127, 39, true);

    let out = b.to_page_major();
    assert_eq!(out.len(), 640);
    assert_eq!(out[0], 0x01);
    assert_eq!(out[128 + 5], 0x02);
    assert_eq!(out[4 * 128 + 127], 0x80);
}

#[test]
fn test_to_page_major_empty_buffer_is_zeroes() {
    let b = OledBuffer::new(128, 64);
    let out = b.to_page_major();
    assert_eq!(out.len(), 1024);
    assert!(out.iter().all(|&byte| byte == 0));
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml render::tests::test_to_page_major`
Expected: COMPILE ERROR — `to_page_major` not found.

- [ ] **Step 3: Implement**

Add to `impl OledBuffer` in `src-tauri/src/render.rs`:

```rust
/// Serialize to SSD1306 page-major order: all columns of page 0, then
/// page 1, etc. (The internal layout is column-major pages.) Used by the
/// Apex legacy protocol.
pub fn to_page_major(&self) -> Vec<u8> {
    let pages = self.pages();
    let mut out = Vec::with_capacity(self.data.len());
    for page in 0..pages {
        for x in 0..self.width as usize {
            out.push(self.data[x * pages + page]);
        }
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml render::tests::test_to_page_major`
Expected: PASS (2 tests).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/render.rs
git commit -m "feat(render): SSD1306 page-major serialization for Apex protocol"
```

---

### Task 3: `render_text_to_oled` and `load_image_to_buffer` take target dimensions

**Files:**
- Modify: `src-tauri/src/render.rs`
- Modify: `src-tauri/src/daemon.rs:105-107` (`value_to_oled_buffer`)
- Modify: `src-tauri/src/gui.rs:365,398,409` (preview render calls)

- [ ] **Step 1: Write failing tests**

Add to the `tests` module in `src-tauri/src/render.rs`:

```rust
#[test]
fn test_render_text_at_explicit_dimensions() {
    let buf = render_text_to_oled("Hi", 0, &[], 128, 40);
    assert_eq!(buf.width, 128);
    assert_eq!(buf.height, 40);
    assert_eq!(buf.data.len(), 640);
    // Something was drawn
    assert!(buf.data.iter().any(|&b| b != 0));
}

#[test]
fn test_render_clips_safely_on_short_screen() {
    // 5 lines of Large font massively overflow 40px; must not panic.
    let buf = render_text_to_oled(
        "A\nB\nC\nD\nE",
        0,
        &[FontSize::Large; 5],
        128,
        40,
    );
    assert_eq!(buf.data.len(), 640);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml render::tests::test_render_text_at_explicit`
Expected: COMPILE ERROR — wrong number of arguments.

- [ ] **Step 3: Change signatures**

In `src-tauri/src/render.rs`:

```rust
pub fn render_text_to_oled(
    text: &str,
    x: i32,
    line_fonts: &[FontSize],
    width: u32,
    height: u32,
) -> OledBuffer {
    let mut buffer = OledBuffer::new(width, height);
    // ... body unchanged ...
}
```

In `load_image_to_buffer`, replace the hardcoded bounds (lines ~267 and ~271):

```rust
for y in 0..height {
    if y + y_off >= buffer.height {
        break;
    }
    for x in 0..width {
        if x + x_off >= buffer.width {
            break;
        }
        // ... unchanged ...
    }
}
```

(Rename the local image dims if they shadow — the decoded image's `(width, height)` from `gray.dimensions()` stays as-is; only the boundary checks change to `buffer.width` / `buffer.height`.)

- [ ] **Step 4: Update callers (all pass 128×64 for now)**

- `src-tauri/src/render.rs` tests: append `, 128, 64` to every existing `render_text_to_oled(...)` call.
- `src-tauri/src/daemon.rs:105`:

```rust
fn value_to_oled_buffer(value: &Value, font_sizes: &[FontSize], size: (u32, u32)) -> OledBuffer {
    render_text_to_oled(&value_to_text(value), 0, font_sizes, size.0, size.1)
}
```

  Both call sites in `tick()` (lines ~663 and ~703) pass `(128, 64)` for now (Task 5 makes this dynamic). Update any tests calling `value_to_oled_buffer`.
- `src-tauri/src/gui.rs` lines 365, 398, 409: append `, 128, 64` (Task 6 makes these dynamic).

- [ ] **Step 5: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/render.rs src-tauri/src/daemon.rs src-tauri/src/gui.rs
git commit -m "refactor(render): render_text_to_oled takes target dimensions"
```

---

### Task 4: `devices.rs` registry with `Protocol::build_packets`

Create the registry module. Move the packet-building logic out of `daemon.rs` into `Protocol::NovaPro`'s match arm; add the `ApexLegacy` arm. The daemon's three `OledClient::Hid` branches call `Protocol::NovaPro.build_packets(...)` temporarily (hardcoded until Task 5 threads the device through).

**Files:**
- Create: `src-tauri/src/devices.rs`
- Modify: `src-tauri/src/main.rs` (add `mod devices;` after `mod consts;`)
- Modify: `src-tauri/src/daemon.rs` (delete `build_hid_packet` / `build_hid_packets_for_buffer` + their tests; call the protocol)

- [ ] **Step 1: Create `src-tauri/src/devices.rs` with failing tests**

```rust
use crate::render::OledBuffer;

/// HID packet protocol family for a direct-USB OLED device. Each variant
/// owns the full frame encoding for its device family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Arctis Nova Pro (Wireless) base station: two 1024-byte feature
    /// reports, header [0x06, 0x93, chunk_x, 0, 64, height], column-major
    /// bitmap in 64-column chunks.
    NovaPro,
    /// Apex 5/7/Pro legacy keyboards: one 641-byte feature report,
    /// 0x61 followed by the 640-byte SSD1306 page-major bitmap.
    ApexLegacy,
}

/// One supported direct-USB device model.
#[derive(Debug)]
pub struct SupportedDevice {
    /// Display name shown in the GUI device picker.
    pub name: &'static str,
    pub product_ids: &'static [u16],
    pub width: u32,
    pub height: u32,
    pub protocol: Protocol,
}

/// Registry of devices supported in direct-USB mode. VID is always 0x1038
/// (enforced by the discovery filter in connect.rs). To add a device that
/// speaks an existing protocol, add a row. A new packet format needs a new
/// Protocol variant and a build_packets arm.
///
/// PID→model mapping verified against apex-tux (apex-hardware/src/usb.rs).
/// Apex Gen3 (0x1640/0x1644/0x1646) intentionally absent — different
/// protocols, not yet implemented.
pub static SUPPORTED_DEVICES: &[SupportedDevice] = &[
    SupportedDevice {
        name: "Arctis Nova Pro Wireless",
        product_ids: &[0x12E0],
        width: 128,
        height: 64,
        protocol: Protocol::NovaPro,
    },
    SupportedDevice {
        name: "Apex Pro",
        product_ids: &[0x1610],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex 7",
        product_ids: &[0x1612],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex Pro TKL",
        product_ids: &[0x1614],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex 7 TKL",
        product_ids: &[0x1618],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex 5",
        product_ids: &[0x161C],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
];

/// Look up the registry entry for a product ID.
pub fn find_supported(product_id: u16) -> Option<&'static SupportedDevice> {
    SUPPORTED_DEVICES
        .iter()
        .find(|d| d.product_ids.contains(&product_id))
}

impl Protocol {
    /// Build the complete HID feature-report sequence for one frame.
    pub fn build_packets(&self, buf: &OledBuffer) -> Vec<Vec<u8>> {
        match self {
            Protocol::NovaPro => [0u8, 64u8]
                .iter()
                .map(|&chunk_x| {
                    let bitmap = buf.get_chunk(chunk_x, 64);
                    let mut packet =
                        vec![0x06u8, 0x93, chunk_x, 0, 64, buf.height as u8];
                    packet.extend_from_slice(&bitmap);
                    packet.resize(1024, 0);
                    packet
                })
                .collect(),
            Protocol::ApexLegacy => {
                let mut packet = Vec::with_capacity(641);
                packet.push(0x61);
                packet.extend_from_slice(&buf.to_page_major());
                vec![packet]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_supported_nova_pro() {
        let d = find_supported(0x12E0).expect("Nova Pro in registry");
        assert_eq!(d.name, "Arctis Nova Pro Wireless");
        assert_eq!((d.width, d.height), (128, 64));
        assert_eq!(d.protocol, Protocol::NovaPro);
    }

    #[test]
    fn test_find_supported_apex_pro() {
        let d = find_supported(0x1610).expect("Apex Pro in registry");
        assert_eq!(d.name, "Apex Pro");
        assert_eq!((d.width, d.height), (128, 40));
        assert_eq!(d.protocol, Protocol::ApexLegacy);
    }

    #[test]
    fn test_find_supported_all_apex_legacy_pids() {
        for pid in [0x1610u16, 0x1612, 0x1614, 0x1618, 0x161C] {
            let d = find_supported(pid).unwrap_or_else(|| panic!("PID {pid:#06X} missing"));
            assert_eq!(d.protocol, Protocol::ApexLegacy);
            assert_eq!((d.width, d.height), (128, 40));
        }
    }

    #[test]
    fn test_find_supported_unknown_pid_is_none() {
        assert!(find_supported(0x9999).is_none());
        // Gen3 PIDs are deliberately unsupported for now
        assert!(find_supported(0x1640).is_none());
        assert!(find_supported(0x1644).is_none());
        assert!(find_supported(0x1646).is_none());
    }

    #[test]
    fn test_nova_pro_packets_layout() {
        let mut buf = OledBuffer::new(128, 64);
        buf.set_pixel(0, 0, true); // first byte of chunk 0
        buf.set_pixel(64, 0, true); // first byte of chunk 1

        let packets = Protocol::NovaPro.build_packets(&buf);
        assert_eq!(packets.len(), 2);
        for (i, p) in packets.iter().enumerate() {
            assert_eq!(p.len(), 1024);
            assert_eq!(p[0], 0x06);
            assert_eq!(p[1], 0x93);
            assert_eq!(p[2], if i == 0 { 0 } else { 64 }); // chunk_x
            assert_eq!(p[3], 0);
            assert_eq!(p[4], 64); // chunk width
            assert_eq!(p[5], 64); // screen height
            assert_eq!(p[6], 0x01); // pixel at top-left of this chunk
        }
    }

    #[test]
    fn test_apex_legacy_packet_layout() {
        let mut buf = OledBuffer::new(128, 40);
        buf.set_pixel(0, 0, true); // page 0, col 0 → payload byte 0
        buf.set_pixel(127, 39, true); // page 4, col 127 → last payload byte

        let packets = Protocol::ApexLegacy.build_packets(&buf);
        assert_eq!(packets.len(), 1);
        let p = &packets[0];
        assert_eq!(p.len(), 641);
        assert_eq!(p[0], 0x61);
        assert_eq!(p[1], 0x01); // (0,0)
        assert_eq!(p[640], 0x80); // (127,39): bit 7 of page 4
    }
}
```

- [ ] **Step 2: Register the module and verify tests fail then pass**

Add `mod devices;` to `src-tauri/src/main.rs` (after `mod consts;` at line 9).

Run: `cargo test --manifest-path src-tauri/Cargo.toml devices::`
Expected: PASS (module is self-contained; if it doesn't compile, fix before proceeding).

- [ ] **Step 3: Delete daemon's packet builders, call the protocol**

In `src-tauri/src/daemon.rs`:
- Delete `build_hid_packet` (lines ~140–146) and `build_hid_packets_for_buffer` (lines ~159–168).
- Delete their tests: `test_build_hid_packet_header_layout`, `test_build_hid_packets_for_buffer_produces_two_packets` (covered by `devices::tests` now).
- Add import: `use crate::devices::Protocol;`
- Replace the three `build_hid_packets_for_buffer(...)` calls:
  - `trigger_frame` Hid arm: `for packet in Protocol::NovaPro.build_packets(buffer) {`
  - `send_blank` Hid arm: `for packet in Protocol::NovaPro.build_packets(&OledBuffer::new(128, 64)) {`
  - `send_white` Hid arm: `for packet in Protocol::NovaPro.build_packets(&buffer) {`

(Still hardcoded NovaPro — Task 5 threads the real device through.)

- [ ] **Step 4: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/devices.rs src-tauri/src/main.rs src-tauri/src/daemon.rs
git commit -m "feat(devices): supported-device registry with per-protocol packet builders"
```

---

### Task 5: Connect only supported devices; daemon renders at device dimensions

`connect.rs` filters to registry devices and returns the matched entry. `OledClient::Hid` becomes a struct variant carrying it. The daemon stores the active display size and uses it everywhere.

**Files:**
- Modify: `src-tauri/src/connect.rs`
- Modify: `src-tauri/src/daemon.rs`

- [ ] **Step 1: Write failing tests in `connect.rs`**

Add to the `tests` module in `src-tauri/src/connect.rs`:

```rust
#[test]
fn test_unsupported_devices_error_lists_models() {
    let msg = unsupported_devices_error(&[
        ("SteelSeries Something".to_string(), 0x1234u16),
        ("".to_string(), 0xABCD),
    ]);
    assert!(msg.contains("SteelSeries Something (PID 0x1234)"));
    assert!(msg.contains("unknown device (PID 0xABCD)"));
    assert!(msg.contains("not yet supported for direct USB"));
}

#[test]
fn test_find_hid_device_returns_supported_entry_when_present() {
    let Some(api) = try_hid_api_for_connect() else {
        return;
    };
    match find_hid_device(&api, "") {
        Ok((info, supported)) => {
            // Whatever matched must be a registry device.
            assert!(supported.product_ids.contains(&info.product_id()));
        }
        Err(e) => {
            // No supported device on this machine — error must be one of
            // the two known shapes.
            let s = e.to_string();
            assert!(
                s.contains("No SteelSeries OLED") || s.contains("not yet supported"),
                "unexpected error: {s}"
            );
        }
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml connect::tests::test_unsupported`
Expected: COMPILE ERROR — `unsupported_devices_error` not found, `find_hid_device` returns a single value.

- [ ] **Step 3: Implement connect.rs changes**

In `src-tauri/src/connect.rs`, add imports and new code:

```rust
use crate::devices::{find_supported, SupportedDevice};

/// Registry entry for a discovered HID interface, if the device is supported.
pub fn supported_device(d: &hidapi::DeviceInfo) -> Option<&'static SupportedDevice> {
    if !is_oled_capable(d) {
        return None;
    }
    find_supported(d.product_id())
}

/// Error text naming detected-but-unsupported devices. `detected` pairs the
/// USB product string (may be empty) with the PID.
pub fn unsupported_devices_error(detected: &[(String, u16)]) -> String {
    let list = detected
        .iter()
        .map(|(name, pid)| {
            let shown = if name.is_empty() { "unknown device" } else { name };
            format!("{} (PID 0x{:04X})", shown, pid)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Detected {} — not yet supported for direct USB", list)
}
```

Rework `find_hid_device` to filter to supported devices and return the entry:

```rust
/// Finds a supported OLED device matching the optional selector. Empty
/// selector or no match returns the first supported device. A selector
/// prefixed with `path:` matches the platform HID device path; otherwise it
/// is matched as a serial. OLED-capable devices that are not in the
/// supported-device registry are never returned — if only unsupported
/// devices are present, the error names them.
pub fn find_hid_device<'a>(
    api: &'a HidApi,
    selector: &str,
) -> Result<(&'a hidapi::DeviceInfo, &'static SupportedDevice), anyhow::Error> {
    let oled_capable = list_oled_devices(api);
    if oled_capable.is_empty() {
        return Err(anyhow::anyhow!("No SteelSeries OLED device found"));
    }

    let candidates: Vec<&hidapi::DeviceInfo> = oled_capable
        .iter()
        .copied()
        .filter(|d| supported_device(d).is_some())
        .collect();

    if candidates.is_empty() {
        let detected: Vec<(String, u16)> = oled_capable
            .iter()
            .map(|d| {
                (
                    d.product_string().unwrap_or("").to_string(),
                    d.product_id(),
                )
            })
            .collect();
        return Err(anyhow::anyhow!(unsupported_devices_error(&detected)));
    }

    let with_entry = |d: &'a hidapi::DeviceInfo| {
        let entry = supported_device(d).expect("candidates are pre-filtered");
        (d, entry)
    };

    if let Some(wanted_path) = selector.strip_prefix("path:") {
        if let Some(d) = candidates
            .iter()
            .find(|d| d.path().to_string_lossy() == wanted_path)
        {
            return Ok(with_entry(d));
        }
        warn!(
            "Configured device path '{}' not present; falling back to first supported OLED device",
            wanted_path
        );
        return Ok(with_entry(candidates[0]));
    }

    let serials: Vec<Option<&str>> = candidates.iter().map(|d| d.serial_number()).collect();
    match pick_oled_index(&serials, selector) {
        Some(idx) => {
            if !selector.is_empty() && serials[idx] != Some(selector) {
                warn!(
                    "Configured device serial '{}' not present; falling back to first supported OLED device",
                    selector
                );
            }
            Ok(with_entry(candidates[idx]))
        }
        None => Err(anyhow::anyhow!("No SteelSeries OLED device found")),
    }
}
```

Update `connect_hid`:

```rust
pub fn connect_hid(
    term: &Term,
    api: &HidApi,
    serial: &str,
) -> Result<(HidDevice, &'static SupportedDevice), anyhow::Error> {
    retry_connect(term, "SteelSeries OLED (HID)", || {
        let (device_info, supported) = find_hid_device(api, serial)?;

        let device = device_info.open_device(api).map_err(|e| {
            error!("Failed to open HID device: {}", e);
            anyhow::anyhow!("Failed to open HID device: {}", e)
        })?;
        Ok((device, supported))
    })
}
```

Fix existing tests that destructure the old return types:
- `test_connect_functions_exist`: change the `_hid_fn` line to
  `let _hid_fn: fn(&Term, &HidApi, &str) -> Result<(HidDevice, &'static crate::devices::SupportedDevice), anyhow::Error> = connect_hid;`
- `test_find_hid_device_path_selector_matches_when_present`: the success arm becomes `let (info, _supported) = r.unwrap(); assert_eq!(info.path().to_string_lossy(), path);` — and the "device present" pre-check must use supported devices: replace `candidates.first()` with a filter via `supported_device`, returning early when none.
- `test_find_hid_device_no_devices_returns_err`: the present-device assertion becomes `assert!(find_hid_device(&api, "").is_ok() || find_hid_device(&api, "").err().map(|e| e.to_string().contains("not yet supported")).unwrap_or(false));` — simpler: when `list_oled_devices` is non-empty, accept either Ok or the "not yet supported" error.

- [ ] **Step 4: Thread the device through the daemon**

In `src-tauri/src/daemon.rs`:

Change the enum (line ~196):

```rust
enum OledClient {
    GameSense(GameSenseClient),
    Hid {
        sender: Box<dyn HidSender>,
        device: &'static crate::devices::SupportedDevice,
    },
}
```

Update the three protocol call sites:

```rust
// trigger_frame:
OledClient::Hid { sender, device } => {
    for packet in device.protocol.build_packets(buffer) {
        if let Err(e) = sender.send_feature_report(&packet) {
            error!("Failed to send HID frame: {}", e);
            return Err(anyhow!("HID send failed: {}", e));
        }
    }
}

// send_blank:
OledClient::Hid { sender, device } => {
    let blank = OledBuffer::new(device.width, device.height);
    for packet in device.protocol.build_packets(&blank) {
        let _ = sender.send_feature_report(&packet);
    }
}

// send_white:
OledClient::Hid { sender, device } => {
    let buffer = white_buffer(device.width, device.height);
    for packet in device.protocol.build_packets(&buffer) {
        let _ = sender.send_feature_report(&packet);
    }
}
```

Generalize `white_buffer`:

```rust
/// Pure helper: create an OledBuffer with every pixel turned on ("white" screen).
fn white_buffer(width: u32, height: u32) -> OledBuffer {
    let mut buffer = OledBuffer::new(width, height);
    for x in 0..width {
        for y in 0..height {
            buffer.set_pixel(x, y, true);
        }
    }
    buffer
}
```

(`send_white`'s GameSense arm doesn't use `white_buffer`; only the Hid arm calls it. Update any `white_buffer()` test call sites to `white_buffer(128, 64)`.)

Add `display_size` to the `Daemon` struct (after `page_counter`):

```rust
/// Active OLED dimensions: registry entry's size in direct-USB mode,
/// 128×64 for GameSense.
display_size: (u32, u32),
```

Initialize in `Daemon::new`: `display_size: (128, 64),`

In `connect_all` (line ~532):

```rust
if self.config.direct_usb {
    self.announce_connecting_direct_usb();

    let api = hidapi::HidApi::new().map_err(|e| anyhow!("HID API init failed: {}", e))?;
    let (device, supported) = connect_hid(&self.term, &api, &self.config.direct_usb_serial)?;
    info!(
        "Direct USB device: {} ({}x{})",
        supported.name, supported.width, supported.height
    );
    self.display_size = (supported.width, supported.height);
    self.oled = Some(Box::new(OledClient::Hid {
        sender: Box::new(device),
        device: supported,
    }));
    self.hid_api = Some(api);

    self.after_direct_usb_connected();
} else {
    self.display_size = (128, 64);
    // ... GameSense branch unchanged ...
```

In `tick()`, pass the size to both render calls (lines ~663 and ~703):

```rust
let buffer = value_to_oled_buffer(&value, &self.config.font_sizes, self.display_size);
```

Update daemon tests: every `OledClient::Hid(Box::new(...))` becomes

```rust
fn nova_device() -> &'static crate::devices::SupportedDevice {
    crate::devices::find_supported(0x12E0).expect("Nova Pro in registry")
}

// then:
let mut oled = OledClient::Hid {
    sender: Box::new(FakeHidSender::new()),
    device: nova_device(),
};
```

Add one new daemon test exercising the Apex path through the mock sender:

```rust
#[test]
fn test_oled_client_hid_apex_sends_single_641_byte_packet() {
    let fake = FakeHidSender::new();
    let calls = fake.calls.clone();
    let mut oled = OledClient::Hid {
        sender: Box::new(fake),
        device: crate::devices::find_supported(0x1610).expect("Apex Pro in registry"),
    };
    let buf = OledBuffer::new(128, 40);
    let value = json!({"line1": "x"});
    oled.trigger_frame("PAGE1", 0, &value, &buf).unwrap();

    let sent = calls.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].len(), 641);
    assert_eq!(sent[0][0], 0x61);
}
```

(Adapt to `FakeHidSender`'s actual construction — it already records packets in `self.calls`; if `calls` is not clonable/shared, follow the pattern of the existing `test_oled_client_hid_trigger_frame_sends_two_packets` test.)

- [ ] **Step 5: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/connect.rs src-tauri/src/daemon.rs
git commit -m "feat(connect,daemon): registry-gated direct-USB connect with per-device protocol dispatch"
```

---

### Task 6: Dimensions in shared state, frame payloads, and device listing

**Files:**
- Modify: `src-tauri/src/state.rs` (display size + `OledFrame` payload struct)
- Modify: `src-tauri/src/daemon.rs` (`push_frame`, write display size to state, `buffer_to_rgba_grayscale`)
- Modify: `src-tauri/src/gui.rs` (`buffer_to_pixels` → dims-aware; preview commands return `OledFrame`; `HidDeviceInfo` gains supported fields)

- [ ] **Step 1: Write failing tests**

In `src-tauri/src/state.rs` tests:

```rust
#[test]
fn test_new_initializes_display_size() {
    let state = SharedState::new(mock_config());
    assert_eq!(state.display_size, (128, 64));
}

#[test]
fn test_oled_frame_serializes() {
    let f = OledFrame {
        width: 128,
        height: 40,
        pixels: vec![0; 3],
    };
    let s = serde_json::to_string(&f).unwrap();
    assert!(s.contains("\"width\":128"));
    assert!(s.contains("\"height\":40"));
    assert!(s.contains("\"pixels\""));
}
```

In `src-tauri/src/gui.rs` tests (adapting the existing `test_buffer_to_pixels_size_and_mapping`):

```rust
#[test]
fn test_buffer_to_pixels_uses_buffer_dimensions() {
    let mut buf = OledBuffer::new(128, 40);
    buf.set_pixel(0, 0, true);
    let frame = buffer_to_frame(&buf);
    assert_eq!(frame.width, 128);
    assert_eq!(frame.height, 40);
    assert_eq!(frame.pixels.len(), 128 * 40);
    assert_eq!(frame.pixels[0], 255);
    assert_eq!(frame.pixels[1], 0);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml state::tests::test_new_initializes_display_size`
Expected: COMPILE ERROR — no `display_size` field, no `OledFrame`.

- [ ] **Step 3: Implement state.rs**

```rust
/// One rendered OLED frame for the GUI: grayscale pixel bytes (0 or 255),
/// row-major, length = width * height.
#[derive(Debug, Clone, Serialize)]
pub struct OledFrame {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}
```

Add to `SharedState`: `pub display_size: (u32, u32),` — initialize `display_size: (128, 64),` in `new()`.

- [ ] **Step 4: Implement daemon.rs payload changes**

- `buffer_to_rgba_grayscale(buf)` → rename/replace with a dims-aware version emitting `OledFrame` (keep it in daemon.rs):

```rust
fn buffer_to_frame_payload(buf: &OledBuffer) -> crate::state::OledFrame {
    let mut pixels = Vec::with_capacity((buf.width * buf.height) as usize);
    let pages = (buf.height / 8) as usize;
    for y in 0..buf.height {
        for x in 0..buf.width {
            let idx = x as usize * pages + (y / 8) as usize;
            let on = (buf.data[idx] & (1 << (y % 8))) != 0;
            pixels.push(if on { 255 } else { 0 });
        }
    }
    crate::state::OledFrame {
        width: buf.width,
        height: buf.height,
        pixels,
    }
}

fn push_frame(&self, buf: &OledBuffer) {
    let _ = self.app.emit("frame", buffer_to_frame_payload(buf));
}
```

- In `connect_all`, after setting `self.display_size`, also publish it (both branches — add to `after_direct_usb_connected`/`after_gamesense_connected` is wrong since they don't know the size; instead write it inline in `connect_all` right after each `self.display_size = ...` assignment):

```rust
let size = self.display_size;
self.write_state(|s| s.display_size = size);
```

- Update any tests referencing `buffer_to_rgba_grayscale`.

- [ ] **Step 5: Implement gui.rs changes**

Replace `buffer_to_pixels` with:

```rust
fn buffer_to_frame(buf: &crate::render::OledBuffer) -> crate::state::OledFrame {
    let mut pixels = Vec::with_capacity((buf.width * buf.height) as usize);
    let pages = (buf.height / 8) as usize;
    for y in 0..buf.height {
        for x in 0..buf.width {
            let idx = x as usize * pages + (y / 8) as usize;
            let on = (buf.data[idx] & (1 << (y % 8))) != 0;
            pixels.push(if on { 255 } else { 0 });
        }
    }
    crate::state::OledFrame {
        width: buf.width,
        height: buf.height,
        pixels,
    }
}
```

(Duplication with daemon's `buffer_to_frame_payload` — DRY: put ONE copy in `gui.rs` as `pub(crate) fn buffer_to_frame` and have daemon call `crate::gui::buffer_to_frame`. Delete the daemon copy.)

- `get_live_preview` / `preview_config` return `Result<crate::state::OledFrame, String>`; their `_impl` functions render at the live size:

```rust
fn get_live_preview_impl(shared: &Shared) -> Result<crate::state::OledFrame, String> {
    let g = shared.lock().map_err(|e| e.to_string())?;
    Ok(buffer_to_frame(&g.oled_buffer))
}
```

In `preview_config_impl`, read the size once at the top — `let (w, h) = { ...lock... g.display_size };` (fold into the existing lock that grabs `hwinfo_snapshot`) — and pass `w, h` to all three `render_text_to_oled` calls; return `buffer_to_frame(&buf)`.

- Extend `HidDeviceInfo`:

```rust
#[derive(Debug, Clone, Serialize)]
pub struct HidDeviceInfo {
    pub serial: String,
    pub product: String,
    pub manufacturer: String,
    pub vendor_id: u16,
    pub product_id: u16,
    pub interface_number: i32,
    pub path: String,
    /// True when the device is in the direct-USB supported-device registry.
    pub supported: bool,
    /// Registry display name when supported, empty otherwise.
    pub device_name: String,
    /// Screen size when supported, 0 otherwise.
    pub width: u32,
    pub height: u32,
}
```

In `list_hid_devices`, map via the registry:

```rust
.map(|d| {
    let entry = crate::connect::supported_device(d);
    HidDeviceInfo {
        serial: d.serial_number().unwrap_or("").to_string(),
        product: d.product_string().unwrap_or("").to_string(),
        manufacturer: d.manufacturer_string().unwrap_or("").to_string(),
        vendor_id: d.vendor_id(),
        product_id: d.product_id(),
        interface_number: d.interface_number(),
        path: d.path().to_string_lossy().into_owned(),
        supported: entry.is_some(),
        device_name: entry.map(|e| e.name.to_string()).unwrap_or_default(),
        width: entry.map(|e| e.width).unwrap_or(0),
        height: entry.map(|e| e.height).unwrap_or(0),
    }
})
```

- [ ] **Step 6: Run the full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/state.rs src-tauri/src/daemon.rs src-tauri/src/gui.rs
git commit -m "feat(gui,state): dimension-aware frame payloads and supported-device listing"
```

---

### Task 7: Frontend — adaptive canvas and unsupported-device labelling

**Files:**
- Modify: `src-tauri/ui/index.html`

- [ ] **Step 1: Make `renderFrame` consume `{width, height, pixels}`**

Replace the existing `renderFrame` (line ~464):

```javascript
function renderFrame(frame) {
    if (!frame || !frame.pixels || frame.pixels.length !== frame.width * frame.height) return;
    if (previewCanvas.width !== frame.width) previewCanvas.width = frame.width;
    if (previewCanvas.height !== frame.height) previewCanvas.height = frame.height;
    document.getElementById('preview-caption').textContent =
        `What is currently on the OLED (${frame.width}×${frame.height})`;
    const imgData = previewCtx.createImageData(frame.width, frame.height);
    for (let i = 0; i < frame.pixels.length; i++) {
        const v = frame.pixels[i];
        imgData.data[i * 4] = v;
        imgData.data[i * 4 + 1] = v;
        imgData.data[i * 4 + 2] = v;
        imgData.data[i * 4 + 3] = 255;
    }
    previewCtx.putImageData(imgData, 0, 0);
}
```

All three call sites (`listen('frame', ...)`, `get_live_preview`, `preview_config`) already pass the payload straight through — no changes needed there since the payload shape changed on the backend.

- [ ] **Step 2: Let the canvas keep aspect ratio**

Find the CSS rule sizing the preview canvas (currently `width: 384px; height: 192px;` around lines 129–130) and change the fixed height to auto so a 128×40 frame displays at 384×120:

```css
width: 384px;
height: auto;
```

- [ ] **Step 3: Grey out unsupported devices in the picker**

In `refreshDeviceList` (line ~973), inside the `for (const d of devices)` loop, prefer the registry name and disable unsupported entries. Replace the `const label = ...` line and add the disabled handling after `tail` is computed:

```javascript
const label = (d.supported && d.device_name)
    ? d.device_name
    : (d.product || `Unknown device ${d.product_id.toString(16)}`);
```

and after `opt.textContent = label + tail;` (or however the existing code joins them — keep its structure):

```javascript
if (!d.supported) {
    opt.disabled = true;
    opt.textContent += ' — not yet supported';
}
```

- [ ] **Step 4: Manual smoke check**

Run: `cargo build --manifest-path src-tauri/Cargo.toml` (compile only — UI is static HTML, no bundler).
Expected: builds clean. Visual verification happens on hardware at the end.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/ui/index.html
git commit -m "feat(ui): adaptive preview canvas and unsupported-device labels"
```

---

### Task 8: Final verification

**Files:** none new.

- [ ] **Step 1: Format and lint**

Run: `cargo fmt --manifest-path src-tauri/Cargo.toml`
Run: `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`
Expected: clean. Fix anything clippy raises; do not `#[allow]` without reason.

- [ ] **Step 2: Full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: PASS — note total test count vs. main (`git stash`-free comparison not needed; just confirm no test was deleted except the two moved daemon packet tests).

- [ ] **Step 3: On-hardware sanity (user has Nova Pro connected)**

Run: `cargo run --manifest-path src-tauri/Cargo.toml` with `direct_usb=true` in `conf.ini`.
Expected: log line `Direct USB device: Arctis Nova Pro Wireless (128x64)`; OLED renders as before; GUI live preview shows 128×64 caption.

- [ ] **Step 4: Commit any stragglers and report**

```bash
git status
git add -A && git commit -m "chore: fmt/clippy cleanup for device registry"
```

Only commit if fmt/clippy changed files.

---

## Self-review notes (already applied)

- Spec §1–§7 all map to Tasks 4, 1–3, 5, 5, 6–7, 5–6, 1–6 respectively; the wizard line in spec §5 is a planning-time no-op (wizard never listed devices) — documented in the header notes.
- Type threads checked: `find_hid_device → (DeviceInfo, &'static SupportedDevice)` → `connect_hid → (HidDevice, &'static SupportedDevice)` → `OledClient::Hid { sender, device }` → `device.protocol.build_packets(&OledBuffer)`; `OledFrame` is the single payload type for live frames and previews; `buffer_to_frame` lives in `gui.rs` only.
- `FakeHidSender` construction in the new Apex test must follow the existing fixture's API — flagged inline in Task 5 Step 4.
