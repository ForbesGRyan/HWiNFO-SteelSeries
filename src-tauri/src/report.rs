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
/// a paste instruction, and `truncated` is true. The `<= ISSUE_URL_MAX`
/// ceiling holds by construction regardless of input size: the label is
/// capped before it reaches the title, and if the stubbed body still
/// overflows, the body collapses to a fixed, minimal instruction.
pub fn device_report_issue_url(
    report: &str,
    device_label: &str,
    fallback_body: &str,
) -> (String, bool) {
    let capped_label: String = device_label.chars().take(120).collect();
    let title = format!("[Device support] {}", capped_label);
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
    let stub_url = format!("{}&body={}", base, encode_uri_component(&stub));
    if stub_url.len() <= ISSUE_URL_MAX {
        return (stub_url, true);
    }
    (
        format!(
            "{}&body={}",
            base,
            encode_uri_component("Paste the copied device report here.")
        ),
        true,
    )
}

/// Only this repo's GitHub pages may be opened via the open_url command,
/// so it cannot be used as a generic launcher.
pub fn is_allowed_external_url(url: &str) -> bool {
    url.starts_with("https://github.com/ForbesGRyan/HWiNFO-SteelSeries/")
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
    fn test_issue_url_pathological_label_and_fallback_stays_under_max() {
        let report = "x".repeat(8000);
        let device_label = "y".repeat(500);
        let fallback_body = "z".repeat(8000);
        let (url, truncated) = device_report_issue_url(&report, &device_label, &fallback_body);
        assert!(truncated);
        assert!(url.len() <= ISSUE_URL_MAX);
        assert!(url.starts_with(
            "https://github.com/ForbesGRyan/HWiNFO-SteelSeries/issues/new?labels=device-support&title="
        ));
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
}
