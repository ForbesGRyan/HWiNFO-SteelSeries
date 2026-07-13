# USB Device Discovery Report Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One-click path from "unsupported SteelSeries device detected" to a prefilled GitHub issue containing the USB details needed to add a registry entry.

**Architecture:** A new pure-Rust module `report.rs` formats a markdown device report and builds a prefilled GitHub issue URL from `ReportInterface` values. `connect.rs` gains an all-interfaces enumerator (VID filter only). `gui.rs` exposes two Tauri commands (`get_device_report`, `open_url`); the plain-JS frontend in `ui/index.html` shows "Request device support" / "Copy device report" buttons when unsupported devices are present.

**Tech Stack:** Rust (Tauri 2, hidapi 2.6), `open` crate (new dep) for launching the default browser, vanilla JS frontend.

**Spec:** `docs/superpowers/specs/2026-07-13-usb-device-discovery-design.md`

## Global Constraints

- Repo/issue base URL: `https://github.com/ForbesGRyan/HWiNFO-SteelSeries` — exact string, used in URL builder and the open-URL allowlist.
- Issue URL length ceiling: **7000** chars; over that, body becomes the stub `{unsupported summary}\n\nPaste the copied device report here.`
- Issue query params: `labels=device-support`, `title=[Device support] {device label}`, `body={report}` — all percent-encoded.
- Report includes **all** VID `0x1038` interfaces, not just usage page `0xFFC0`.
- Serial numbers never appear in the report — only a yes/no presence flag.
- OS line is the literal `Windows` (no version detection, no new crates for it).
- Test runs: use module filters (`cargo test --bins report`). The full suite has 2 environment-dependent failures when HWiNFO/SteelSeries GG are running on the dev machine — ignore those two only.
- Before final commit of the branch: `cargo fmt` and `cargo clippy --bins -- -D warnings` must be clean.
- Commit after every task. Commit messages end with `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`.

---

### Task 1: `report.rs` — data model, unsupported detection, device label

**Files:**
- Create: `src-tauri/src/report.rs`
- Modify: `src-tauri/src/main.rs` (add `mod report;` next to the existing `mod devices;` declaration)

**Interfaces:**
- Produces: `pub struct ReportInterface { product: String, manufacturer: String, product_id: u16, interface_number: i32, usage_page: u16, usage: u16, has_serial: bool, oled_capable: bool, supported: Option<&'static str> }` (all fields `pub`)
- Produces: `pub fn report_interface_from_parts(product: &str, manufacturer: &str, product_id: u16, interface_number: i32, usage_page: u16, usage: u16, has_serial: bool) -> ReportInterface`
- Produces: `pub fn unsupported_detected(interfaces: &[ReportInterface]) -> Vec<(String, u16)>` (deduped by PID, product string or `""`)
- Produces: `pub fn device_label(interfaces: &[ReportInterface]) -> String`
- Consumes: `crate::devices::find_supported`, `crate::connect::HID_USAGE_PAGE`

- [ ] **Step 1: Create module with failing tests**

Create `src-tauri/src/report.rs`:

```rust
//! Device-support report: markdown report + prefilled GitHub issue URL for
//! unsupported SteelSeries devices. Everything here is pure and unit-tested;
//! HID enumeration stays in connect.rs / gui.rs.

/// One VID-0x1038 HID interface, reduced to the fields the report needs.
#[derive(Debug, Clone)]
pub struct ReportInterface {
    pub product: String,
    pub manufacturer: String,
    pub product_id: u16,
    pub interface_number: i32,
    pub usage_page: u16,
    pub usage: u16,
    /// Serial presence only — the serial itself never enters the report.
    pub has_serial: bool,
    pub oled_capable: bool,
    /// Registry display name when the PID is supported.
    pub supported: Option<&'static str>,
}

/// Build a ReportInterface from raw descriptor fields, deriving the
/// OLED-capable and registry-supported flags.
pub fn report_interface_from_parts(
    product: &str,
    manufacturer: &str,
    product_id: u16,
    interface_number: i32,
    usage_page: u16,
    usage: u16,
    has_serial: bool,
) -> ReportInterface {
    ReportInterface {
        product: product.to_string(),
        manufacturer: manufacturer.to_string(),
        product_id,
        interface_number,
        usage_page,
        usage,
        has_serial,
        oled_capable: usage_page == crate::connect::HID_USAGE_PAGE,
        supported: crate::devices::find_supported(product_id).map(|d| d.name),
    }
}

/// Unsupported devices, deduped by PID, as (product string, PID) pairs —
/// the shape `connect::unsupported_devices_error` expects.
pub fn unsupported_detected(interfaces: &[ReportInterface]) -> Vec<(String, u16)> {
    let mut out: Vec<(String, u16)> = Vec::new();
    for i in interfaces {
        if i.supported.is_none() && !out.iter().any(|(_, pid)| *pid == i.product_id) {
            out.push((i.product.clone(), i.product_id));
        }
    }
    out
}

/// Issue-title label for the first unsupported device.
pub fn device_label(interfaces: &[ReportInterface]) -> String {
    match unsupported_detected(interfaces).first() {
        Some((name, pid)) => {
            let shown = if name.is_empty() { "unknown device" } else { name };
            format!("{} (PID 0x{:04X})", shown, pid)
        }
        None => "unknown device".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(product: &str, pid: u16, usage_page: u16) -> ReportInterface {
        report_interface_from_parts(product, "SteelSeries", pid, 1, usage_page, 0x0001, true)
    }

    #[test]
    fn test_from_parts_flags_supported_and_oled() {
        let i = iface("Nova Pro", 0x12E0, 0xFFC0);
        assert!(i.oled_capable);
        assert_eq!(i.supported, Some("Arctis Nova Pro Wireless"));

        let g3 = iface("Apex Gen3 TKL", 0x1640, 0xFFC0);
        assert!(g3.oled_capable);
        assert_eq!(g3.supported, None);

        let other = iface("Nova Pro", 0x12E0, 0x000C);
        assert!(!other.oled_capable);
    }

    #[test]
    fn test_unsupported_detected_dedupes_by_pid() {
        let ifaces = vec![
            iface("Apex Gen3 TKL", 0x1640, 0xFFC0),
            iface("Apex Gen3 TKL", 0x1640, 0x000C),
            iface("Nova Pro", 0x12E0, 0xFFC0),
        ];
        assert_eq!(
            unsupported_detected(&ifaces),
            vec![("Apex Gen3 TKL".to_string(), 0x1640)]
        );
    }

    #[test]
    fn test_device_label_first_unsupported_and_fallbacks() {
        let ifaces = vec![
            iface("Nova Pro", 0x12E0, 0xFFC0),
            iface("Apex Gen3 TKL", 0x1640, 0xFFC0),
            iface("", 0x1644, 0xFFC0),
        ];
        assert_eq!(device_label(&ifaces), "Apex Gen3 TKL (PID 0x1640)");

        let unnamed = vec![iface("", 0x1644, 0xFFC0)];
        assert_eq!(device_label(&unnamed), "unknown device (PID 0x1644)");

        let all_supported = vec![iface("Nova Pro", 0x12E0, 0xFFC0)];
        assert_eq!(device_label(&all_supported), "unknown device");
    }
}
```

In `src-tauri/src/main.rs`, find the `mod devices;` line and add below it:

```rust
mod report;
```

- [ ] **Step 2: Run tests to verify they fail to compile without the mod, then pass**

Run: `cargo test --bins report`
Expected: PASS, 3 tests (`test_from_parts_flags_supported_and_oled`, `test_unsupported_detected_dedupes_by_pid`, `test_device_label_first_unsupported_and_fallbacks`). (Test and implementation land together here because the module is new; the failing-first checkpoint is meaningless for code that doesn't compile without its own definitions.)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/report.rs src-tauri/src/main.rs
git commit -m "feat(report): device-report interface model, unsupported detection, label

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 2: `report.rs` — markdown report formatting

**Files:**
- Modify: `src-tauri/src/report.rs`
- Modify: `src-tauri/src/connect.rs` (only if `unsupported_devices_error` is not already `pub` — it is, at `connect.rs:157`; no change expected)

**Interfaces:**
- Consumes: `ReportInterface`, `unsupported_detected` (Task 1), `crate::connect::unsupported_devices_error`
- Produces: `pub fn format_device_report(app_version: &str, os_info: &str, interfaces: &[ReportInterface]) -> String`

- [ ] **Step 1: Write the failing golden test**

Append to the `tests` module in `src-tauri/src/report.rs`:

```rust
    #[test]
    fn test_format_device_report_golden() {
        let ifaces = vec![
            iface("Apex Gen3 TKL", 0x1640, 0xFFC0),
            iface("Nova Pro", 0x12E0, 0xFFC0),
        ];
        let report = format_device_report("0.2.2", "Windows", &ifaces);
        let expected = "\
### Device report

- App version: 0.2.2
- OS: Windows

| Product | Manufacturer | PID | Interface | Usage page | Usage | Serial? | OLED-capable | Supported |
|---|---|---|---|---|---|---|---|---|
| Apex Gen3 TKL | SteelSeries | 0x1640 | 1 | 0xFFC0 | 0x0001 | yes | yes | no |
| Nova Pro | SteelSeries | 0x12E0 | 1 | 0xFFC0 | 0x0001 | yes | yes | Arctis Nova Pro Wireless |

Detected Apex Gen3 TKL (PID 0x1640) — not yet supported for direct USB
";
        assert_eq!(report, expected);
    }

    #[test]
    fn test_format_device_report_no_unsupported_omits_summary() {
        let ifaces = vec![iface("Nova Pro", 0x12E0, 0xFFC0)];
        let report = format_device_report("0.2.2", "Windows", &ifaces);
        assert!(!report.contains("not yet supported"));
        assert!(report.ends_with("Arctis Nova Pro Wireless |\n"));
    }

    #[test]
    fn test_format_device_report_empty_product_shows_unknown() {
        let ifaces = vec![iface("", 0x1644, 0x000C)];
        let report = format_device_report("0.2.2", "Windows", &ifaces);
        assert!(report.contains("| unknown device | SteelSeries | 0x1644 | 1 | 0x000C | 0x0001 | yes | no | no |"));
        assert!(report.contains("Detected unknown device (PID 0x1644) — not yet supported for direct USB"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --bins report`
Expected: FAIL to compile — `format_device_report` not found.

- [ ] **Step 3: Implement `format_device_report`**

Add above the `tests` module in `src-tauri/src/report.rs`:

```rust
/// Markdown device report: app/OS lines, one table row per interface, and
/// (when applicable) the same unsupported-device summary the connect error
/// uses.
pub fn format_device_report(
    app_version: &str,
    os_info: &str,
    interfaces: &[ReportInterface],
) -> String {
    let mut out = String::new();
    out.push_str("### Device report\n\n");
    out.push_str(&format!("- App version: {}\n", app_version));
    out.push_str(&format!("- OS: {}\n\n", os_info));
    out.push_str(
        "| Product | Manufacturer | PID | Interface | Usage page | Usage | Serial? | OLED-capable | Supported |\n",
    );
    out.push_str("|---|---|---|---|---|---|---|---|---|\n");
    for i in interfaces {
        let product = if i.product.is_empty() {
            "unknown device"
        } else {
            &i.product
        };
        out.push_str(&format!(
            "| {} | {} | 0x{:04X} | {} | 0x{:04X} | 0x{:04X} | {} | {} | {} |\n",
            product,
            i.manufacturer,
            i.product_id,
            i.interface_number,
            i.usage_page,
            i.usage,
            if i.has_serial { "yes" } else { "no" },
            if i.oled_capable { "yes" } else { "no" },
            i.supported.unwrap_or("no"),
        ));
    }
    let detected = unsupported_detected(interfaces);
    if !detected.is_empty() {
        out.push('\n');
        out.push_str(&crate::connect::unsupported_devices_error(&detected));
        out.push('\n');
    }
    out
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bins report`
Expected: PASS, 6 tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/report.rs
git commit -m "feat(report): markdown device report with interface table

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 3: `report.rs` — percent-encoding, issue URL with truncation, URL allowlist

**Files:**
- Modify: `src-tauri/src/report.rs`

**Interfaces:**
- Produces: `pub const ISSUE_URL_MAX: usize = 7000;`
- Produces: `pub const REPO_URL: &str = "https://github.com/ForbesGRyan/HWiNFO-SteelSeries";`
- Produces: `fn encode_uri_component(s: &str) -> String` (private)
- Produces: `pub fn device_report_issue_url(report: &str, device_label: &str, fallback_body: &str) -> (String, bool)` — URL plus `truncated` flag
- Produces: `pub fn is_allowed_external_url(url: &str) -> bool`

- [ ] **Step 1: Write the failing tests**

Append to the `tests` module:

```rust
    #[test]
    fn test_encode_uri_component() {
        assert_eq!(encode_uri_component("abc-XYZ_0.9~"), "abc-XYZ_0.9~");
        assert_eq!(
            encode_uri_component("[Device support] Apex (PID 0x1640)"),
            "%5BDevice%20support%5D%20Apex%20%28PID%200x1640%29"
        );
        assert_eq!(encode_uri_component("a\nb&c=d"), "a%0Ab%26c%3Dd");
    }

    #[test]
    fn test_issue_url_short_report_not_truncated() {
        let (url, truncated) =
            device_report_issue_url("short report", "Apex Gen3 TKL (PID 0x1640)", "fallback");
        assert!(!truncated);
        assert!(url.starts_with(
            "https://github.com/ForbesGRyan/HWiNFO-SteelSeries/issues/new?labels=device-support&title="
        ));
        assert!(url.contains("%5BDevice%20support%5D%20Apex%20Gen3%20TKL"));
        assert!(url.ends_with("&body=short%20report"));
        assert!(url.len() <= ISSUE_URL_MAX);
    }

    #[test]
    fn test_issue_url_long_report_truncates_to_fallback() {
        let long_report = "x".repeat(ISSUE_URL_MAX);
        let (url, truncated) = device_report_issue_url(
            &long_report,
            "Apex Gen3 TKL (PID 0x1640)",
            "Detected Apex Gen3 TKL (PID 0x1640) — not yet supported for direct USB",
        );
        assert!(truncated);
        assert!(url.len() <= ISSUE_URL_MAX);
        assert!(url.contains("Paste%20the%20copied%20device%20report%20here."));
        assert!(!url.contains("xxxx"));
    }

    #[test]
    fn test_is_allowed_external_url() {
        assert!(is_allowed_external_url(
            "https://github.com/ForbesGRyan/HWiNFO-SteelSeries/issues/new?labels=device-support"
        ));
        assert!(!is_allowed_external_url("https://github.com/evil/repo"));
        assert!(!is_allowed_external_url("https://example.com/"));
        assert!(!is_allowed_external_url("file:///C:/Windows/System32/calc.exe"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --bins report`
Expected: FAIL to compile — `encode_uri_component`, `device_report_issue_url`, `is_allowed_external_url`, `ISSUE_URL_MAX` not found.

- [ ] **Step 3: Implement**

Add above the `tests` module:

```rust
/// Ceiling for the prefilled-issue URL; beyond it the body is stubbed and
/// the UI copies the full report to the clipboard instead.
pub const ISSUE_URL_MAX: usize = 7000;
pub const REPO_URL: &str = "https://github.com/ForbesGRyan/HWiNFO-SteelSeries";

/// RFC 3986 percent-encoding: everything except unreserved characters.
fn encode_uri_component(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

/// Prefilled new-issue URL. Returns (url, truncated). When the full report
/// pushes the URL past ISSUE_URL_MAX, the body becomes `fallback_body` plus
/// a paste instruction, and `truncated` is true.
pub fn device_report_issue_url(
    report: &str,
    device_label: &str,
    fallback_body: &str,
) -> (String, bool) {
    let title = format!("[Device support] {}", device_label);
    let base = format!(
        "{}/issues/new?labels=device-support&title={}",
        REPO_URL,
        encode_uri_component(&title)
    );
    let full = format!("{}&body={}", base, encode_uri_component(report));
    if full.len() <= ISSUE_URL_MAX {
        return (full, false);
    }
    let stub = format!("{}\n\nPaste the copied device report here.", fallback_body);
    (
        format!("{}&body={}", base, encode_uri_component(&stub)),
        true,
    )
}

/// Only this repo's GitHub pages may be opened via the open_url command,
/// so it cannot be used as a generic launcher.
pub fn is_allowed_external_url(url: &str) -> bool {
    url.starts_with("https://github.com/ForbesGRyan/HWiNFO-SteelSeries/")
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --bins report`
Expected: PASS, 10 tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/report.rs
git commit -m "feat(report): prefilled issue URL with truncation and URL allowlist

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 4: enumeration + Tauri commands (`connect.rs`, `gui.rs`, `main.rs`, `Cargo.toml`)

**Files:**
- Modify: `src-tauri/src/connect.rs` (beside `list_oled_devices`, ~line 138)
- Modify: `src-tauri/src/gui.rs` (beside `list_hid_devices`, ~line 325)
- Modify: `src-tauri/src/main.rs` (`generate_handler!` list, line 289)
- Modify: `src-tauri/Cargo.toml` (add `open` dependency)

**Interfaces:**
- Consumes: `report_interface_from_parts`, `format_device_report`, `device_report_issue_url`, `device_label`, `unsupported_detected`, `is_allowed_external_url` (Tasks 1–3), `connect::unsupported_devices_error`
- Produces: `connect::is_steelseries_vendor(vendor_id: u16) -> bool`, `connect::list_steelseries_interfaces(api: &HidApi) -> Vec<&hidapi::DeviceInfo>`
- Produces: Tauri commands `gui::get_device_report() -> Result<DeviceReportPayload, String>` and `gui::open_url(url: String) -> Result<(), String>`; `DeviceReportPayload { report, issue_url, has_unsupported, url_truncated }` (serde `Serialize`)

- [ ] **Step 1: Write the failing test for the vendor predicate**

In `src-tauri/src/connect.rs`, append to the existing `tests` module:

```rust
    #[test]
    fn test_is_steelseries_vendor() {
        assert!(is_steelseries_vendor(0x1038));
        assert!(!is_steelseries_vendor(0x046D));
    }
```

Run: `cargo test --bins connect::tests::test_is_steelseries_vendor`
Expected: FAIL to compile — `is_steelseries_vendor` not found.

- [ ] **Step 2: Implement enumerator in `connect.rs`**

Add directly below `list_oled_devices` (after `connect.rs:141`):

```rust
/// Pure vendor predicate — testable without HidApi.
pub fn is_steelseries_vendor(vendor_id: u16) -> bool {
    vendor_id == HID_VENDOR_ID
}

/// Lists ALL SteelSeries HID interfaces (no usage-page filter), for the
/// device-support report. `list_oled_devices` remains the connect filter.
pub fn list_steelseries_interfaces(api: &HidApi) -> Vec<&hidapi::DeviceInfo> {
    api.device_list()
        .filter(|d| is_steelseries_vendor(d.vendor_id()))
        .collect()
}
```

Run: `cargo test --bins connect::tests::test_is_steelseries_vendor`
Expected: PASS.

- [ ] **Step 3: Add the `open` dependency**

In `src-tauri/Cargo.toml` `[dependencies]`, after the `log = "0.4"` line, add:

```toml
open = "5"
```

Run: `cargo check --bins`
Expected: compiles; `open` downloads and builds.

- [ ] **Step 4: Implement the Tauri commands in `gui.rs`**

Add below the `list_hid_devices` command (after `gui.rs:347`):

```rust
#[derive(Debug, Clone, Serialize)]
pub struct DeviceReportPayload {
    pub report: String,
    pub issue_url: String,
    pub has_unsupported: bool,
    pub url_truncated: bool,
}

#[command]
pub fn get_device_report() -> Result<DeviceReportPayload, String> {
    let api = hidapi::HidApi::new().map_err(|e| format!("HID API init failed: {}", e))?;
    let interfaces: Vec<crate::report::ReportInterface> =
        crate::connect::list_steelseries_interfaces(&api)
            .iter()
            .map(|d| {
                crate::report::report_interface_from_parts(
                    d.product_string().unwrap_or(""),
                    d.manufacturer_string().unwrap_or(""),
                    d.product_id(),
                    d.interface_number(),
                    d.usage_page(),
                    d.usage(),
                    d.serial_number().is_some_and(|s| !s.is_empty()),
                )
            })
            .collect();

    let report = crate::report::format_device_report(
        env!("CARGO_PKG_VERSION"),
        "Windows",
        &interfaces,
    );
    let detected = crate::report::unsupported_detected(&interfaces);
    let fallback = crate::connect::unsupported_devices_error(&detected);
    let (issue_url, url_truncated) = crate::report::device_report_issue_url(
        &report,
        &crate::report::device_label(&interfaces),
        &fallback,
    );
    Ok(DeviceReportPayload {
        report,
        issue_url,
        has_unsupported: !detected.is_empty(),
        url_truncated,
    })
}

#[command]
pub fn open_url(url: String) -> Result<(), String> {
    if !crate::report::is_allowed_external_url(&url) {
        return Err(format!("URL not allowed: {}", url));
    }
    open::that(&url).map_err(|e| format!("Failed to open browser: {}", e))
}
```

- [ ] **Step 5: Register the commands**

In `src-tauri/src/main.rs:289`, extend the handler list:

```rust
        .invoke_handler(tauri::generate_handler![
            gui::get_status,
            gui::get_config,
            gui::save_config,
            gui::get_live_preview,
            gui::list_sensors,
            gui::list_hid_devices,
            gui::preview_config,
            gui::request_sleep,
            gui::request_wake,
            gui::request_white_screen,
            gui::get_device_report,
            gui::open_url,
        ])
```

(If the current list has entries beyond line 299 not shown here, keep them — only append the two new commands before `])`.)

- [ ] **Step 6: Verify build and module tests**

Run: `cargo check --bins && cargo test --bins -- report:: connect::`
Expected: check clean; all `report` and `connect` module tests PASS (the two
filters after `--` are libtest OR-filters).

- [ ] **Step 7: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/connect.rs src-tauri/src/gui.rs src-tauri/src/main.rs
git commit -m "feat(gui): get_device_report and guarded open_url commands

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 5: frontend — report buttons in `ui/index.html`

**Files:**
- Modify: `src-tauri/ui/index.html`

**Interfaces:**
- Consumes: Tauri commands `get_device_report` → `{report, issue_url, has_unsupported, url_truncated}` and `open_url(url)`; existing `refreshDeviceList()` (~line 981), `renderStatus(s)` (~line 492), `showError(msg)` (~line 1133), `invoke` helper already used throughout.

No Rust tests here; verification is a manual smoke test (Step 4). Keep JS style identical to surrounding code (plain functions, `invoke`, `getElementById`).

- [ ] **Step 1: Add the button row markup**

In the Status card, directly after the `error-banner` div (`<div class="error-banner" id="error-banner"></div>`, line 334), add:

```html
    <div id="device-report-row" style="display:none; margin-top:6px; gap:8px;">
        <button id="btn-request-support" type="button">Request device support</button>
        <button id="btn-copy-report" type="button">Copy device report</button>
    </div>
```

- [ ] **Step 2: Add the JS wiring**

Near `refreshDeviceList()` (after its closing brace, ~line 1040), add:

```javascript
        // --- Device-support report (unsupported direct-USB devices) ---
        let deviceReportAvailable = false;

        function updateDeviceReportRow(fromError) {
            const row = document.getElementById('device-report-row');
            row.style.display = (deviceReportAvailable || fromError) ? 'flex' : 'none';
        }

        async function copyText(text) {
            try {
                await navigator.clipboard.writeText(text);
            } catch (_) {
                const ta = document.createElement('textarea');
                ta.value = text;
                document.body.appendChild(ta);
                ta.select();
                document.execCommand('copy');
                ta.remove();
            }
        }

        function flashButton(btn, label) {
            const orig = btn.textContent;
            btn.textContent = label;
            setTimeout(() => { btn.textContent = orig; }, 2000);
        }

        document.getElementById('btn-request-support').onclick = async () => {
            try {
                const r = await invoke('get_device_report');
                if (r.url_truncated) {
                    await copyText(r.report);
                    flashButton(document.getElementById('btn-copy-report'),
                        'Report copied — paste into issue body');
                }
                await invoke('open_url', { url: r.issue_url });
            } catch (e) {
                try {
                    const r = await invoke('get_device_report');
                    await copyText(r.report + '\n\n' + r.issue_url);
                    showError('Could not open browser — report and URL copied to clipboard');
                } catch (e2) {
                    showError('Device report failed: ' + e2);
                }
            }
        };

        document.getElementById('btn-copy-report').onclick = async () => {
            try {
                const r = await invoke('get_device_report');
                await copyText(r.report);
                flashButton(document.getElementById('btn-copy-report'), 'Copied ✓');
            } catch (e) {
                showError('Device report failed: ' + e);
            }
        };
```

- [ ] **Step 3: Drive visibility from both signals**

a) In `refreshDeviceList()`, right before the closing brace (after the `select.value = ...` line, ~line 1039), add:

```javascript
            deviceReportAvailable = devices.some(d => !d.supported);
            updateDeviceReportRow(false);
```

b) In `renderStatus(s)` (~line 502), extend the banner block:

```javascript
            const banner = document.getElementById('error-banner');
            if (s.last_error) { banner.textContent = s.last_error; banner.classList.add('visible'); }
            else { banner.classList.remove('visible'); }
            updateDeviceReportRow(!!(s.last_error && s.last_error.includes('not yet supported for direct USB')));
```

(The `banner` lines already exist — only the `updateDeviceReportRow(...)` call is new.)

- [ ] **Step 4: Manual smoke test**

Run: `cargo run -- --settings`
Expected with only supported hardware (Nova Pro): no button row visible after enabling Direct USB and the device list refresh. Then click **Copy device report** via devtools (`document.getElementById('btn-copy-report').click()` after forcing `document.getElementById('device-report-row').style.display='flex'`): clipboard receives a markdown report whose table lists the Nova Pro interfaces with `Supported = Arctis Nova Pro Wireless`.
Expected unsupported flow (no unsupported hardware on the dev machine): temporarily verify by URL — run `document.getElementById('btn-request-support').click()` the same way; the browser must open `github.com/ForbesGRyan/HWiNFO-SteelSeries/issues/new` with label, title, and body prefilled (title will read `[Device support] unknown device` since nothing is unsupported — that's expected in this simulation).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/ui/index.html
git commit -m "feat(ui): request-support and copy-report buttons for unsupported devices

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

---

### Task 6: final verification and branch cleanup

**Files:**
- Modify: none expected (fixes only if checks fail)

- [ ] **Step 1: Format and lint**

Run: `cargo fmt && cargo clippy --bins -- -D warnings`
Expected: no diffs, no warnings. Fix anything reported and amend into a `style:` commit if needed.

- [ ] **Step 2: Full test suite**

Run: `cargo test --bins`
Expected: all tests pass except the 2 known environment-dependent failures (they assert HWiNFO/GG connections *fail* and the dev machine has both running). Any other failure must be fixed before proceeding.

- [ ] **Step 3: Commit any remaining changes**

```bash
git status
git add -A src-tauri
git commit -m "style: fmt/clippy cleanup for device discovery report

Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>"
```

(Skip the commit if the tree is clean.)

- [ ] **Step 4: Repo-side label (manual, outside code)**

Create the `device-support` label once:

```bash
gh label create device-support --description "USB info for adding a device to the direct-USB registry" --color 0E8A16
```

Expected: label created (or "already exists" — fine either way).
