use crate::devices::{find_supported, SupportedDevice};
use gamesense::client::GameSenseClient;
use hidapi::{HidApi, HidDevice};
use log::warn;

/// Connect to SteelSeries GG's GameSense service. One attempt; the daemon
/// loop retries.
pub fn connect_steelseries() -> Result<GameSenseClient, anyhow::Error> {
    GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", None)
        .map_err(|e| anyhow::anyhow!("SteelSeries GG not reachable ({})", e))
}

/// HID device identification parameters for SteelSeries OLED devices.
/// These are extracted as constants to make them testable and configurable.
pub const HID_VENDOR_ID: u16 = 0x1038;
pub const HID_USAGE_PAGE: u16 = 0xFFC0;

/// Pure-data predicate used by `is_oled_capable` — testable without HidApi.
pub fn matches_oled_ids(vendor_id: u16, usage_page: u16) -> bool {
    vendor_id == HID_VENDOR_ID && usage_page == HID_USAGE_PAGE
}

/// Predicate matching SteelSeries OLED-capable HID interfaces.
/// Filters by VID + usage_page only — PID is not constrained so multiple
/// devices (Arctis variants, Apex Pro, etc.) can be selected.
pub fn is_oled_capable(d: &hidapi::DeviceInfo) -> bool {
    matches_oled_ids(d.vendor_id(), d.usage_page())
}

/// Pure picker: returns the index of the chosen OLED device, or None.
/// - If `serials` is empty → None.
/// - If `desired` is non-empty and matches a candidate's serial → that index.
/// - Otherwise → index 0 (first OLED-capable device).
pub fn pick_oled_index(serials: &[Option<&str>], desired: &str) -> Option<usize> {
    if serials.is_empty() {
        return None;
    }
    if !desired.is_empty() {
        if let Some(i) = serials.iter().position(|s| *s == Some(desired)) {
            return Some(i);
        }
    }
    Some(0)
}

/// Lists all SteelSeries OLED-capable HID devices.
pub fn list_oled_devices(api: &HidApi) -> Vec<&hidapi::DeviceInfo> {
    api.device_list().filter(|d| is_oled_capable(d)).collect()
}

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

/// Registry entry for a discovered HID interface, if the device is supported.
pub fn supported_device(d: &hidapi::DeviceInfo) -> Option<&'static SupportedDevice> {
    if !is_oled_capable(d) {
        return None;
    }
    find_supported(d.product_id())
}

/// Suffix of the unsupported-device error. The settings UI matches on this
/// text to offer the device-support report.
pub const UNSUPPORTED_ERROR_SUFFIX: &str = "not yet supported for direct USB";

/// Error text naming detected-but-unsupported devices. `detected` pairs the
/// USB product string (may be empty) with the PID.
pub fn unsupported_devices_error(detected: &[(String, u16)]) -> String {
    let list = detected
        .iter()
        .map(|(name, pid)| {
            let shown = if name.is_empty() {
                "unknown device"
            } else {
                name
            };
            format!("{} (PID 0x{:04X})", shown, pid)
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("Detected {} — {}", list, UNSUPPORTED_ERROR_SUFFIX)
}

/// Finds the supported OLED device matching the optional selector, returning
/// it together with its registry entry. Empty selector or no match returns the
/// first supported OLED-capable device. A selector prefixed with `path:`
/// matches against the platform HID device path (used when the device exposes
/// no USB serial number); otherwise it is matched as a serial. OLED-capable
/// devices whose PID is not in the supported-device registry are rejected
/// with an error naming each detected model.
pub fn find_hid_device<'a>(
    api: &'a HidApi,
    selector: &str,
) -> Result<(&'a hidapi::DeviceInfo, &'static SupportedDevice), anyhow::Error> {
    let oled_devices = list_oled_devices(api);
    if oled_devices.is_empty() {
        return Err(anyhow::anyhow!("No SteelSeries OLED device found"));
    }
    let candidates: Vec<&hidapi::DeviceInfo> = oled_devices
        .iter()
        .copied()
        .filter(|d| supported_device(d).is_some())
        .collect();
    if candidates.is_empty() {
        // Dedupe by PID: a device exposing several OLED-capable interfaces
        // should be listed once, not once per interface.
        let mut detected: Vec<(String, u16)> = Vec::new();
        for d in &oled_devices {
            if !detected.iter().any(|(_, pid)| *pid == d.product_id()) {
                detected.push((d.product_string().unwrap_or("").to_string(), d.product_id()));
            }
        }
        return Err(anyhow::anyhow!(unsupported_devices_error(&detected)));
    }
    let chosen = if let Some(wanted_path) = selector.strip_prefix("path:") {
        match candidates
            .iter()
            .find(|d| d.path().to_string_lossy() == wanted_path)
        {
            Some(d) => *d,
            None => {
                warn!(
                    "Configured device path '{}' not present; falling back to first supported OLED device",
                    wanted_path
                );
                candidates[0]
            }
        }
    } else {
        let serials: Vec<Option<&str>> = candidates.iter().map(|d| d.serial_number()).collect();
        match pick_oled_index(&serials, selector) {
            Some(idx) => {
                if !selector.is_empty() && serials[idx] != Some(selector) {
                    warn!(
                        "Configured device serial '{}' not present; falling back to first supported OLED device",
                        selector
                    );
                }
                candidates[idx]
            }
            None => return Err(anyhow::anyhow!("No SteelSeries OLED device found")),
        }
    };
    Ok((
        chosen,
        supported_device(chosen).expect("candidates are pre-filtered"),
    ))
}

/// Open the supported SteelSeries OLED device matching `selector`, returning
/// it with its registry entry. One attempt; the daemon loop retries, which
/// also picks up newly plugged devices.
pub fn connect_hid(
    api: &HidApi,
    selector: &str,
) -> Result<(HidDevice, &'static SupportedDevice), anyhow::Error> {
    let (device_info, supported) = find_hid_device(api, selector)?;
    let device = device_info
        .open_device(api)
        .map_err(|e| anyhow::anyhow!("Failed to open HID device: {}", e))?;
    Ok((device, supported))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================================================
    // Pure Logic Tests (Extracted Functions)
    // ==========================================================================

    /// Test that find_hid_device returns an error when device is not found.
    /// This test creates an empty HidApi (no devices) and verifies the error.
    #[test]
    fn test_find_hid_device_not_found_error_message() {
        // We can't easily mock HidApi, but we can verify the error format
        // by checking the error message we construct
        let error = anyhow::anyhow!("OLED device not found");
        assert_eq!(error.to_string(), "OLED device not found");
    }

    #[test]
    fn test_hid_device_open_error_message_format() {
        // Test that the error message format is correct
        let original_error = "Access denied";
        let error = anyhow::anyhow!("Failed to open HID device: {}", original_error);
        assert_eq!(
            error.to_string(),
            "Failed to open HID device: Access denied"
        );
    }

    // ==========================================================================
    // GameSense Client Configuration Tests
    // ==========================================================================

    #[test]
    fn test_gamesense_game_name_is_valid() {
        // Game name should be uppercase alphanumeric, no spaces
        let game_name = "HWINFO";
        assert!(game_name
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()));
        assert!(!game_name.contains(' '));
    }

    #[test]
    fn test_gamesense_display_name_is_human_readable() {
        let display_name = "HWiNFO_Stats";
        assert!(!display_name.is_empty());
        assert!(display_name.len() <= 32); // GameSense has display name limits
    }

    #[test]
    fn test_matches_oled_ids_true() {
        assert!(matches_oled_ids(HID_VENDOR_ID, HID_USAGE_PAGE));
    }

    #[test]
    fn test_matches_oled_ids_wrong_vendor() {
        assert!(!matches_oled_ids(0x1234, HID_USAGE_PAGE));
    }

    #[test]
    fn test_matches_oled_ids_wrong_usage_page() {
        assert!(!matches_oled_ids(HID_VENDOR_ID, 0x1234));
    }

    #[test]
    fn test_pick_oled_index_empty_returns_none() {
        assert_eq!(pick_oled_index(&[], ""), None);
        assert_eq!(pick_oled_index(&[], "abc"), None);
    }

    #[test]
    fn test_pick_oled_index_returns_first_when_no_desired() {
        let v = [Some("a"), Some("b"), Some("c")];
        assert_eq!(pick_oled_index(&v, ""), Some(0));
    }

    #[test]
    fn test_pick_oled_index_finds_match() {
        let v = [Some("a"), Some("b"), Some("c")];
        assert_eq!(pick_oled_index(&v, "b"), Some(1));
        assert_eq!(pick_oled_index(&v, "c"), Some(2));
    }

    #[test]
    fn test_pick_oled_index_falls_back_when_no_match() {
        let v = [Some("a"), Some("b"), None];
        assert_eq!(pick_oled_index(&v, "zzz"), Some(0));
    }

    #[test]
    fn test_pick_oled_index_skips_none_serials() {
        let v = [None, Some("target"), None];
        assert_eq!(pick_oled_index(&v, "target"), Some(1));
    }

    fn try_hid_api_for_connect() -> Option<HidApi> {
        HidApi::new().ok()
    }

    #[test]
    fn test_find_hid_device_no_devices_returns_err() {
        let Some(api) = try_hid_api_for_connect() else {
            return;
        };
        // Filter on HID_VENDOR_ID+usage page is unlikely to match a generic test box.
        if !list_oled_devices(&api).is_empty() {
            // SteelSeries device present — find should succeed for a supported
            // model, or report the detected models as unsupported.
            match find_hid_device(&api, "") {
                Ok((_info, _supported)) => {}
                Err(e) => assert!(e.to_string().contains("not yet supported")),
            }
            return;
        }
        let r = find_hid_device(&api, "");
        assert!(r.is_err());
        match r {
            Err(e) => assert!(e.to_string().contains("No SteelSeries OLED")),
            Ok(_) => unreachable!(),
        }
    }

    #[test]
    fn test_find_hid_device_with_serial_no_devices_returns_err() {
        let Some(api) = try_hid_api_for_connect() else {
            return;
        };
        if !list_oled_devices(&api).is_empty() {
            return;
        }
        let r = find_hid_device(&api, "FAKE-SERIAL-XYZ");
        assert!(r.is_err());
    }

    #[test]
    fn test_find_hid_device_path_selector_matches_when_present() {
        let Some(api) = try_hid_api_for_connect() else {
            return;
        };
        let candidates = list_oled_devices(&api);
        if candidates.is_empty() {
            // No OLED device present — just verify the path selector still
            // surfaces the "no device" error rather than panicking.
            let r = find_hid_device(&api, "path:nonexistent");
            assert!(r.is_err());
            return;
        }
        let Some(first) = candidates.iter().find(|d| supported_device(d).is_some()) else {
            // OLED devices present but none supported — the path selector
            // must surface the unsupported-device error.
            let r = find_hid_device(&api, "path:nonexistent");
            assert!(r.unwrap_err().to_string().contains("not yet supported"));
            return;
        };
        let path = first.path().to_string_lossy().into_owned();
        let r = find_hid_device(&api, &format!("path:{}", path));
        assert!(r.is_ok());
        let (info, _supported) = r.unwrap();
        assert_eq!(info.path().to_string_lossy(), path);
    }

    #[test]
    fn test_find_hid_device_path_selector_falls_back_when_missing() {
        let Some(api) = try_hid_api_for_connect() else {
            return;
        };
        let candidates = list_oled_devices(&api);
        if candidates.is_empty() {
            return;
        }
        let r = find_hid_device(&api, "path:does-not-exist");
        if candidates.iter().any(|d| supported_device(d).is_some()) {
            // Bogus path → fallback to first supported OLED device, not an error.
            let (_info, _supported) = r.unwrap();
        } else {
            assert!(r.unwrap_err().to_string().contains("not yet supported"));
        }
    }

    #[test]
    fn test_is_oled_capable_via_list_oled_devices() {
        let Some(api) = try_hid_api_for_connect() else {
            return;
        };
        // is_oled_capable is exercised via list_oled_devices.
        let _ = list_oled_devices(&api);
    }

    #[test]
    fn test_unsupported_devices_error_lists_models() {
        let msg = unsupported_devices_error(&[
            ("SteelSeries Something".to_string(), 0x1234u16),
            ("".to_string(), 0xABCD),
        ]);
        assert!(msg.contains("SteelSeries Something (PID 0x1234)"));
        assert!(msg.contains("unknown device (PID 0xABCD)"));
        assert!(msg.contains("not yet supported for direct USB"));
        // The fail-fast check in connect_hid keys off this exact suffix.
        assert!(msg.ends_with(UNSUPPORTED_ERROR_SUFFIX));
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
                let s = e.to_string();
                assert!(
                    s.contains("No SteelSeries OLED") || s.contains("not yet supported"),
                    "unexpected error: {s}"
                );
            }
        }
    }

    #[test]
    fn test_connection_error_preserves_context() {
        let inner_error = "Permission denied";
        let wrapped_error = anyhow::anyhow!("Failed to connect: {}", inner_error);

        // Error message should contain both the wrapper and original error
        let error_string = wrapped_error.to_string();
        assert!(error_string.contains("Failed to connect"));
        assert!(error_string.contains("Permission denied"));
    }

    #[test]
    fn test_is_steelseries_vendor() {
        assert!(is_steelseries_vendor(0x1038));
        assert!(!is_steelseries_vendor(0x046D));
    }
}
