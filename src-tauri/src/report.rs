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
