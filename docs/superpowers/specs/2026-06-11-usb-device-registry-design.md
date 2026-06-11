# Direct-USB Supported-Device Registry — Design

**Date:** 2026-06-11
**Status:** Approved pending user review

## Problem

Direct-USB (HID) mode currently hardcodes one packet format — the Arctis Nova
Pro Wireless protocol — in `daemon.rs`, while device discovery in `connect.rs`
accepts *any* SteelSeries device (VID `0x1038`) exposing usage page `0xFFC0`.
Different SteelSeries OLED devices require different HID packets and have
different screen sizes, so connecting an unlisted device today would send it
wrong packets. New devices must be addable incrementally.

## Goals

1. A static registry of supported direct-USB devices, each with its PID(s),
   screen dimensions, and packet protocol.
2. Devices not in the registry are detected and shown in the GUI as
   "not yet supported" but cannot be connected in direct-USB mode.
3. Full resolution generalization now: render pipeline, previews, and GUI
   handle per-device screen sizes (128×64 and 128×40 initially).
4. First two registry families: Arctis Nova Pro Wireless and the legacy
   Apex OLED keyboards.

## Non-Goals

- Apex Gen3 protocols (PIDs `0x1640`, `0x1644`, `0x1646`; 642-byte and
  chunked variants) — future registry rows/variants.
- Font auto-scaling for short screens. Existing pixel clipping is safe;
  Large font on 128×40 simply clips.
- Config-driven or plugin packet formats. Registry is a compile-time table.

## Verified protocol facts

Source: [apex-tux](https://github.com/not-jan/apex-tux)
(`apex-hardware/src/usb.rs`) and current working code in this repo.

| | Arctis Nova Pro Wireless | Apex legacy (5/7/Pro, TKL) |
|---|---|---|
| PIDs | `0x12E0` | `0x1610`, `0x1612`, `0x1614`, `0x1618`, `0x161C` |
| Screen | 128×64 | 128×40 |
| Transport | `send_feature_report` | `send_feature_report` |
| Packets | 2 × 1024 bytes | 1 × 641 bytes |
| Header | `[0x06, 0x93, chunk_x, 0, 64, 64]` | `[0x61]` |
| Bitmap | column-major pages (8 vertical px/byte, bit 0 top), 64-column chunks | SSD1306 page-major: 5 pages × 128 bytes = 640 bytes |

Both transports use feature reports, so the existing `HidSender` trait is
unchanged. Exact Apex model-name↔PID mapping is confirmed against apex-tux
constants during implementation; on-hardware verification happens when the
user's Apex keyboard is connected.

## Design

### 1. New module `src-tauri/src/devices.rs`

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Two 1024-byte feature reports, header [0x06, 0x93, x, 0, w, h],
    /// column-major bitmap, 64-column chunks.
    NovaPro,
    /// One 641-byte feature report: 0x61 + 640-byte page-major bitmap.
    ApexLegacy,
}

#[derive(Debug)]
pub struct SupportedDevice {
    pub name: &'static str,          // shown in GUI picker
    pub product_ids: &'static [u16],
    pub width: u32,
    pub height: u32,
    pub protocol: Protocol,
}

pub static SUPPORTED_DEVICES: &[SupportedDevice] = &[
    // Arctis Nova Pro Wireless — 128×64, NovaPro
    // Apex legacy models — 128×40, ApexLegacy (one row per model name)
];

/// PID lookup. VID is always 0x1038 (enforced upstream by discovery filter).
pub fn find_supported(product_id: u16) -> Option<&'static SupportedDevice>;

impl Protocol {
    /// Build the full HID packet sequence for one frame.
    pub fn build_packets(&self, buf: &OledBuffer) -> Vec<Vec<u8>>;
}
```

`build_hid_packet` and `build_hid_packets_for_buffer` move here from
`daemon.rs` (NovaPro arm). The ApexLegacy arm is `0x61` followed by the
640-byte `buf.to_page_major()` bitmap — 641 bytes exactly, no padding.

Adding a device that uses an existing protocol = one table row. A new packet
format = one enum variant + one `build_packets` match arm.

### 2. Resolution-aware `OledBuffer` (`render.rs`)

- Fields become `{ width: u32, height: u32, data: Vec<u8> }` with
  `data.len() == width * height / 8`; constructor `OledBuffer::new(width, height)`.
  Heights are multiples of 8 for all targets (64, 40).
- Derive `Clone`; replaces manual `OledBuffer { data: buffer.data }` copies.
- `set_pixel`, `DrawTarget::draw_iter`, `OriginDimensions::size`, `get_chunk`,
  and `load_image_to_buffer` use instance dims instead of 128/64 literals.
- New `to_page_major(&self) -> Vec<u8>`: SSD1306 ordering (page 0 across all
  columns, then page 1, …) for the Apex protocol.
- `render_text_to_oled(text, x, line_fonts)` gains target dimensions
  (signature: `render_text_to_oled(text, x, line_fonts, width, height)`).
  Line layout logic is unchanged; clipping handles overflow on short screens.

### 3. Discovery vs. connection (`connect.rs`)

- `is_oled_capable` (VID + usage page filter) is unchanged — discovery still
  lists every SteelSeries OLED-class interface so the GUI can show them.
- `find_hid_device` narrows candidates to registry-supported devices before
  applying the serial/`path:` selector. If OLED-capable devices exist but none
  is supported, the error names them:
  `"Detected <product> (PID 0xXXXX) — not yet supported for direct USB"`.
- `connect_hid` returns `(HidDevice, &'static SupportedDevice)` so the daemon
  knows dimensions and protocol. Retry behavior unchanged.

### 4. Daemon dispatch (`daemon.rs`)

- `OledClient::Hid` becomes a struct variant:
  `Hid { sender: Box<dyn HidSender>, device: &'static SupportedDevice }`.
- `trigger_frame`, `send_blank`, `send_white` call
  `device.protocol.build_packets(buffer)`; blank/white buffers are created at
  device dimensions (`OledBuffer::new(w, h)` / generalized `white_buffer(w, h)`).
- The daemon stores the active display dimensions and renders every frame at
  them. GameSense mode fixes them at 128×64.
- On HID connect, the daemon writes the active dims into shared state.

### 5. GUI and previews (`gui.rs`, `state.rs`, `ui/index.html`)

- `HidDeviceInfo` gains `supported: bool`, `device_name: String` (registry
  name, falling back to USB product string), `width: u32`, `height: u32`.
- Device picker: unsupported devices render greyed out with a
  "not yet supported" label and are unselectable.
- `SharedState` gains display dims (default 128×64; daemon updates on
  connect). `oled_buffer` initializes at those dims.
- Frame/preview payloads change from bare `Vec<u8>` to
  `{ width, height, pixels }` (`push_frame` event, `get_live_preview`,
  `preview_config`). `buffer_to_pixels` / `buffer_to_rgba_grayscale` iterate
  instance dims.
- `index.html`: canvas `width`/`height` and the caption text are set from the
  payload instead of hardcoded 128×64; the preview keeps its 3× CSS scale.
- `settings.rs` console wizard: the direct-USB device list shows supported
  devices only.

### 6. Error handling

- Wrong-packets-to-wrong-device is impossible by construction: the protocol is
  derived from the PID via the registry, never assumed.
- Unsupported-device connect attempts fail fast with a named, actionable error
  (device shows up in GUI with explanation rather than silently missing).
- All existing retry/disconnect behavior is unchanged.

### 7. Testing

- `devices.rs`: PID lookup hit/miss; golden packet tests per protocol —
  NovaPro: 2 packets, 1024 bytes, header layout, chunk x = 0/64;
  ApexLegacy: 1 packet, 641 bytes, `0x61` prefix, payload matches
  `to_page_major`.
- `render.rs`: `new(w, h)` sizing; `set_pixel` bounds at both 128×64 and
  128×40; `to_page_major` ordering against hand-computed pixels; clipping on
  40-px-tall buffer.
- `connect.rs`: supported-filter selection; unsupported-only error message.
- `gui.rs`: `HidDeviceInfo.supported` flag mapping; payload dims.
- All existing tests updated for the `OledBuffer::new(w, h)` signature
  (mechanical; tests pin 128×64 unless exercising 128×40).

## Migration notes

- `conf.ini` is unchanged — `direct_usb_serial` keeps its meaning; selection
  simply applies after the supported-filter.
- Behavior change: a non-registry SteelSeries OLED device that previously
  connected (and received NovaPro packets) now refuses with a clear error.
  This is intentional.
