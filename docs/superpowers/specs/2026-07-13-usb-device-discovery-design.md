# USB Device Discovery Report — Design

**Date:** 2026-07-13
**Status:** Approved pending user review

## Problem

Direct-USB mode only works for devices in the compile-time registry
(`devices.rs`). Users with unlisted SteelSeries OLED devices (e.g. Apex Gen3,
issue #11) see "not yet supported for direct USB" and have no easy way to send
the maintainer the USB details needed to add a registry row (PID, interface
layout, usage pages). Today that information arrives ad hoc in issue comments,
if at all.

## Goals

1. One-click path from "unsupported device detected" to a prefilled GitHub
   issue containing everything needed to add a registry entry.
2. Report covers **all** VID `0x1038` interfaces (not just usage page
   `0xFFC0`), so the maintainer can see which interface is the OLED endpoint.
3. Clipboard fallback: a "Copy report" action that works offline and when the
   URL would be too long.
4. Report generation is pure Rust and unit-tested, matching codebase style.

## Non-Goals

- HID report-descriptor dumps (needs device open + big payloads). Future
  enhancement if interface tables prove insufficient.
- Automatic protocol detection or config-driven device support.
- GitHub issue *forms* (structured YAML templates with prefilled fields);
  plain `title`/`body`/`labels` query params suffice.
- Non-SteelSeries vendors.

## Design

### 1. Interface enumeration — `connect.rs`

New `list_steelseries_interfaces(api: &HidApi) -> Vec<&hidapi::DeviceInfo>`
beside `list_oled_devices()`: filters by VID `0x1038` only (no usage-page
filter). Existing functions unchanged.

### 2. Report model + formatting — new module `src-tauri/src/report.rs`

```rust
pub struct ReportInterface {
    pub product: String,        // USB product string ("" if absent)
    pub manufacturer: String,
    pub product_id: u16,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
    pub has_serial: bool,       // presence only; serial itself is omitted (privacy)
    pub oled_capable: bool,     // usage_page == 0xFFC0
    pub supported: Option<&'static str>, // registry name if PID supported
}
```

Pure functions:

- `format_device_report(app_version: &str, os_info: &str, interfaces: &[ReportInterface]) -> String`
  — markdown: app version, Windows version line, one table row per interface
  (PID hex, interface#, usage page/usage hex, OLED-capable, supported name),
  and a closing "Detected but unsupported: X (PID 0x…)" summary matching the
  wording of `unsupported_devices_error`.
- `device_report_issue_url(report: &str, device_label: &str) -> (String, bool)`
  — returns the URL plus a `truncated` flag; builds
  `https://github.com/ForbesGRyan/HWiNFO-SteelSeries/issues/new?labels=device-support&title=[Device support] {device_label}&body={report}`
  with percent-encoding. If the full URL exceeds **7000 chars**, the body is
  replaced by a short stub: the unsupported-device summary plus
  *"Paste the copied device report here."* (the UI copies the full report to
  the clipboard alongside opening the URL in that case).
- `device_label(interfaces: &[ReportInterface]) -> String` — first unsupported
  interface's product string (or `unknown device`) + PID hex, deduped by PID.

`ReportInterface` construction from `hidapi::DeviceInfo` +
`devices::find_supported` lives in one thin non-test-only helper; everything
downstream of it is pure.

OS info: the literal string `"Windows"` (this is a Windows-only app; no new
crate for version detection — users state their Windows version in the issue
if it matters).

### 3. Tauri command — `gui.rs`

```rust
#[command]
pub fn get_device_report() -> Result<DeviceReportPayload, String>
// DeviceReportPayload { report: String, issue_url: String,
//                       has_unsupported: bool, url_truncated: bool }
```

Enumerates via `list_steelseries_interfaces`, maps to `ReportInterface`,
calls the pure functions. HID init failure → `Err(msg)` (same pattern as
`list_hid_devices`).

New command `open_url(url: String)` using the `open` crate (tiny, Windows
`ShellExecute` under the hood). Guard: only URLs starting with
`https://github.com/ForbesGRyan/HWiNFO-SteelSeries/` are accepted, so the
command can't be used as a generic launcher.

### 4. Frontend — `ui/index.html`

- Device picker: greyed-out unsupported rows get a **"Request support"**
  link.
- Status banner: when the status error contains
  `not yet supported for direct USB`, show the same button next to it.
- Click → `get_device_report()`; then `open_url(issue_url)`. If
  `url_truncated`, copy full report to clipboard first and toast
  "Full report copied — paste it into the issue body."
- **"Copy report"** secondary action → clipboard via
  `navigator.clipboard.writeText`, falling back to a hidden
  `<textarea>` + `document.execCommand('copy')` if the API is unavailable.
- If `open_url` fails (no browser handler), copy report + URL to clipboard
  and show the error in the status line.
- No button rendered when `has_unsupported` is false.

### 5. Repo side

Add label `device-support` to the GitHub repo (manual, one-time). No issue
template file needed.

## Error handling summary

| Failure | Behavior |
|---|---|
| HID API init fails | Button click shows error string in status line |
| No unsupported devices | Buttons absent |
| URL > 7000 chars | Stub body + auto-copy full report |
| Browser open fails | Copy report+URL to clipboard, status message |

## Testing

Unit tests (pure, no HID), in `report.rs`:

- Golden markdown for a known interface set (supported + unsupported mix).
- Unsupported summary line matches `unsupported_devices_error` wording.
- URL encoding of spaces/brackets/newlines; `labels=device-support` present.
- Truncation: long report → stub body, `url_truncated` flag; short report →
  full body.
- `device_label` dedupes multiple interfaces of one PID; falls back to
  `unknown device`.

Manual: machine with Nova Pro (supported) → no button; simulate unsupported
via a test build with Nova Pro PID removed from registry → full flow to
prefilled issue page.
