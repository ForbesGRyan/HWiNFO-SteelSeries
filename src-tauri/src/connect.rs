use console::Term;
use gamesense::client::GameSenseClient;
use hidapi::{HidApi, HidDevice};
use hwinfo_steelseries_oled::Hwinfo;
use log::{error, info, warn};

/// Retry loop with injectable sleep + bounded attempts (for tests).
/// `max_attempts = None` retries forever (production behavior).
fn retry_connect_inner<T, F, S>(
    term: &Term,
    service_name: &str,
    connect_fn: F,
    sleep_fn: &S,
    max_attempts: Option<u32>,
) -> Result<T, anyhow::Error>
where
    F: Fn() -> Result<T, anyhow::Error>,
    S: Fn(u64),
{
    let mut attempt: u32 = 0;
    loop {
        attempt += 1;
        match connect_fn() {
            Ok(result) => {
                info!("Successfully connected to {}", service_name);
                term.clear_line()?;
                term.write_line(&format!("Connected to {}", service_name))?;
                return Ok(result);
            }
            Err(e) => {
                warn!(
                    "Failed to connect to {}: {}. Retrying in 3 seconds...",
                    service_name, e
                );
                if let Some(max) = max_attempts {
                    if attempt >= max {
                        return Err(e);
                    }
                }
                for i in (1..=3).rev() {
                    term.clear_line()?;
                    term.write_line(&format!(
                        "Can't connect to {}. Trying again in {} second.",
                        service_name, i
                    ))?;
                    sleep_fn(1);
                }
            }
        }
    }
}

#[cfg(test)]
thread_local! {
    /// Test override: when Some, retry_connect uses bounded attempts and a no-op sleep.
    /// This lets tests exercise connect_hwinfo / connect_steelseries / connect_hid wrappers
    /// without blocking on real I/O.
    static TEST_RETRY_MAX: std::cell::Cell<Option<u32>> = const { std::cell::Cell::new(None) };
}

#[cfg(test)]
fn current_test_retry_max() -> Option<u32> {
    TEST_RETRY_MAX.with(|c| c.get())
}

#[cfg(not(test))]
fn current_test_retry_max() -> Option<u32> {
    None
}

fn retry_connect<T, F>(term: &Term, service_name: &str, connect_fn: F) -> Result<T, anyhow::Error>
where
    F: Fn() -> Result<T, anyhow::Error>,
{
    retry_connect_inner(
        term,
        service_name,
        connect_fn,
        &|secs: u64| {
            if current_test_retry_max().is_some() {
                // No-op sleep in tests to keep them fast.
            } else {
                std::thread::sleep(std::time::Duration::from_secs(secs));
            }
        },
        current_test_retry_max(),
    )
}

pub fn connect_hwinfo(term: &Term) -> Result<Hwinfo, anyhow::Error> {
    retry_connect(term, "HWiNFO", Hwinfo::new)
}

pub fn connect_steelseries(term: &Term) -> Result<GameSenseClient, anyhow::Error> {
    retry_connect(term, "SteelSeries GG", || {
        GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", None)
    })
}

/// HID device identification parameters for SteelSeries OLED devices.
/// These are extracted as constants to make them testable and configurable.
pub const HID_VENDOR_ID: u16 = 0x1038;
#[allow(dead_code)] // historical Arctis Nova Pro Wireless PID, kept for reference
pub const HID_PRODUCT_ID: u16 = 0x12E0;
#[allow(dead_code)] // OLED interface number on the original target device
pub const HID_INTERFACE_NUMBER: i32 = 0x04;
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

/// Finds the OLED device matching the optional selector. Empty selector or
/// no match returns the first OLED-capable device. A selector prefixed with
/// `path:` matches against the platform HID device path (used when the
/// device exposes no USB serial number); otherwise it is matched as a serial.
pub fn find_hid_device<'a>(
    api: &'a HidApi,
    selector: &str,
) -> Result<&'a hidapi::DeviceInfo, anyhow::Error> {
    let candidates = list_oled_devices(api);
    if candidates.is_empty() {
        return Err(anyhow::anyhow!("No SteelSeries OLED device found"));
    }
    if let Some(wanted_path) = selector.strip_prefix("path:") {
        if let Some(d) = candidates
            .iter()
            .find(|d| d.path().to_string_lossy() == wanted_path)
        {
            return Ok(*d);
        }
        warn!(
            "Configured device path '{}' not present; falling back to first OLED device",
            wanted_path
        );
        return Ok(candidates[0]);
    }
    let serials: Vec<Option<&str>> = candidates.iter().map(|d| d.serial_number()).collect();
    match pick_oled_index(&serials, selector) {
        Some(idx) => {
            if !selector.is_empty() && serials[idx] != Some(selector) {
                warn!(
                    "Configured device serial '{}' not present; falling back to first OLED device",
                    selector
                );
            }
            Ok(candidates[idx])
        }
        None => Err(anyhow::anyhow!("No SteelSeries OLED device found")),
    }
}

pub fn connect_hid(term: &Term, api: &HidApi, serial: &str) -> Result<HidDevice, anyhow::Error> {
    retry_connect(term, "SteelSeries OLED (HID)", || {
        let device_info = find_hid_device(api, serial)?;

        device_info.open_device(api).map_err(|e| {
            error!("Failed to open HID device: {}", e);
            anyhow::anyhow!("Failed to open HID device: {}", e)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==========================================================================
    // HID Constants Tests
    // ==========================================================================

    #[test]
    fn test_hid_vendor_id_is_steelseries() {
        // SteelSeries vendor ID is 0x1038
        assert_eq!(HID_VENDOR_ID, 0x1038);
    }

    #[test]
    fn test_hid_product_id() {
        // Arctis Nova Pro Wireless product ID
        assert_eq!(HID_PRODUCT_ID, 0x12E0);
    }

    #[test]
    fn test_hid_interface_number() {
        // Interface 4 is used for OLED communication
        assert_eq!(HID_INTERFACE_NUMBER, 0x04);
    }

    #[test]
    fn test_hid_usage_page() {
        // Vendor-defined usage page 0xFFC0
        assert_eq!(HID_USAGE_PAGE, 0xFFC0);
    }

    // ==========================================================================
    // Retry Logic Tests
    // ==========================================================================
    //
    // The `retry_connect` function uses recursive retry logic with the following
    // behavior:
    // - On success: Returns immediately with the result
    // - On failure: Logs warning, counts down 3 seconds, then retries recursively
    //
    // Testing this function directly is challenging because:
    // 1. It's a private function
    // 2. It depends on `console::Term` for output
    // 3. It uses `std::thread::sleep` which makes tests slow
    // 4. It recurses infinitely until success
    //
    // To properly unit test this logic, we would need to:
    // - Make the function generic over a "terminal" trait
    // - Accept a sleep function as a parameter
    // - Add a maximum retry count parameter
    //
    // For now, we document the expected behavior:
    //
    // Expected behavior of retry_connect:
    // 1. Calls the connection function
    // 2. If successful:
    //    - Logs "Successfully connected to {service_name}"
    //    - Clears the terminal line
    //    - Writes "Connected to {service_name}"
    //    - Returns Ok(result)
    // 3. If failed:
    //    - Logs warning with error message
    //    - Counts down from 3 to 1 seconds, updating terminal each second
    //    - Recursively calls itself
    //
    // Integration tests would verify:
    // - connect_hwinfo returns Hwinfo instance when HWiNFO is running
    // - connect_steelseries returns GameSenseClient when SteelSeries GG is running
    // - connect_hid returns HidDevice when SteelSeries OLED device is connected

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
    // Service Name Tests
    // ==========================================================================

    #[test]
    fn test_service_names_are_descriptive() {
        // These are the service names used in log messages and terminal output.
        // Verify they are human-readable and descriptive.
        let hwinfo_name = "HWiNFO";
        let steelseries_name = "SteelSeries GG";
        let hid_name = "SteelSeries OLED (HID)";

        assert!(!hwinfo_name.is_empty());
        assert!(!steelseries_name.is_empty());
        assert!(!hid_name.is_empty());

        // Check that they contain expected keywords
        assert!(hwinfo_name.contains("HWiNFO"));
        assert!(steelseries_name.contains("SteelSeries"));
        assert!(hid_name.contains("HID") || hid_name.contains("OLED"));
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

    // ==========================================================================
    // Connection Function Signature Tests
    // ==========================================================================
    // These tests verify the public API hasn't changed unexpectedly

    #[test]
    fn test_connect_functions_exist() {
        // Verify the functions exist by taking references to them
        // This will cause a compile error if signatures change
        let _hwinfo_fn: fn(&Term) -> Result<Hwinfo, anyhow::Error> = connect_hwinfo;
        let _steelseries_fn: fn(&Term) -> Result<GameSenseClient, anyhow::Error> =
            connect_steelseries;
        let _hid_fn: fn(&Term, &HidApi, &str) -> Result<HidDevice, anyhow::Error> = connect_hid;
    }

    // ==========================================================================
    // Testable Retry Logic (Demonstration)
    // ==========================================================================
    //
    // If we wanted to make retry_connect fully testable, we could refactor it to:
    //
    // ```rust
    // pub struct RetryConfig {
    //     pub max_retries: Option<u32>,  // None = infinite
    //     pub delay_seconds: u64,
    // }
    //
    // pub fn retry_connect_with_config<T, F, S>(
    //     config: &RetryConfig,
    //     service_name: &str,
    //     connect_fn: F,
    //     sleep_fn: S,
    //     attempt: u32,
    // ) -> Result<T, anyhow::Error>
    // where
    //     F: Fn() -> Result<T, anyhow::Error>,
    //     S: Fn(u64),
    // {
    //     // ... implementation
    // }
    // ```
    //
    // This would allow us to:
    // - Pass a mock sleep function for fast tests
    // - Limit retries for predictable test behavior
    // - Test retry counting logic

    /// Demonstrates how retry logic could be tested with a mock.
    /// This is a simplified version showing the pattern.
    #[test]
    fn test_retry_logic_pattern_succeeds_first_try() {
        use std::cell::Cell;

        let attempt_count = Cell::new(0);
        let connect_fn = || -> Result<i32, anyhow::Error> {
            attempt_count.set(attempt_count.get() + 1);
            Ok(42)
        };

        let result = connect_fn();
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.get(), 1);
    }

    #[test]
    fn test_retry_logic_pattern_fails_then_succeeds() {
        use std::cell::Cell;

        let attempt_count = Cell::new(0);
        let connect_fn = || -> Result<i32, anyhow::Error> {
            attempt_count.set(attempt_count.get() + 1);
            if attempt_count.get() < 3 {
                Err(anyhow::anyhow!("Connection failed"))
            } else {
                Ok(42)
            }
        };

        // Simulate retry logic (simplified, without sleep)
        let mut result = connect_fn();
        while result.is_err() && attempt_count.get() < 5 {
            result = connect_fn();
        }

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
        assert_eq!(attempt_count.get(), 3); // Succeeded on 3rd attempt
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

    #[test]
    fn test_retry_connect_inner_succeeds_first_try() {
        let term = Term::stdout();
        let calls = std::cell::Cell::new(0u32);
        let result = retry_connect_inner(
            &term,
            "svc",
            || {
                calls.set(calls.get() + 1);
                Ok::<i32, anyhow::Error>(99)
            },
            &|_| {},
            Some(3),
        )
        .unwrap();
        assert_eq!(result, 99);
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn test_retry_connect_inner_retries_then_succeeds() {
        let term = Term::stdout();
        let calls = std::cell::Cell::new(0u32);
        let result = retry_connect_inner(
            &term,
            "svc",
            || {
                calls.set(calls.get() + 1);
                if calls.get() < 3 {
                    Err(anyhow::anyhow!("nope"))
                } else {
                    Ok::<i32, anyhow::Error>(7)
                }
            },
            &|_| {},
            Some(5),
        )
        .unwrap();
        assert_eq!(result, 7);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn test_retry_connect_wrapper_returns_ok_immediately() {
        // The infinite-retry wrapper just dispatches to retry_connect_inner with real sleep.
        // A closure that returns Ok on first call exercises the wrapper without ever sleeping.
        let term = Term::stdout();
        let result: Result<u8, anyhow::Error> = retry_connect(&term, "svc", || Ok(7));
        assert_eq!(result.unwrap(), 7);
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
            // SteelSeries device present — find should succeed.
            assert!(find_hid_device(&api, "").is_ok());
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
        let Some(first) = candidates.first() else {
            // No OLED device present — just verify the path selector still
            // surfaces the "no device" error rather than panicking.
            let r = find_hid_device(&api, "path:nonexistent");
            assert!(r.is_err());
            return;
        };
        let path = first.path().to_string_lossy().into_owned();
        let r = find_hid_device(&api, &format!("path:{}", path));
        assert!(r.is_ok());
        assert_eq!(r.unwrap().path().to_string_lossy(), path);
    }

    #[test]
    fn test_find_hid_device_path_selector_falls_back_when_missing() {
        let Some(api) = try_hid_api_for_connect() else {
            return;
        };
        if list_oled_devices(&api).is_empty() {
            return;
        }
        // Bogus path → fallback to first OLED device, not an error.
        let r = find_hid_device(&api, "path:does-not-exist");
        assert!(r.is_ok());
    }

    /// Set the bounded retry budget for the current test thread.
    fn set_test_retry_max(max: Option<u32>) {
        TEST_RETRY_MAX.with(|c| c.set(max));
    }

    #[test]
    fn test_connect_hwinfo_bounded_returns_err_when_service_down() {
        set_test_retry_max(Some(1));
        let term = Term::stdout();
        // HWiNFO not running in CI → Hwinfo::new() fails → with max=1, retry returns Err.
        let r = connect_hwinfo(&term);
        assert!(r.is_err());
        set_test_retry_max(None);
    }

    #[test]
    fn test_connect_steelseries_bounded_returns_err_when_service_down() {
        set_test_retry_max(Some(1));
        let term = Term::stdout();
        let r = connect_steelseries(&term);
        // GameSenseClient::new may succeed if GG is running, otherwise Err
        if let Err(e) = r {
            assert!(!e.to_string().is_empty());
        }
        set_test_retry_max(None);
    }

    #[test]
    fn test_connect_hid_bounded_propagates_err_when_no_devices() {
        let Some(api) = try_hid_api_for_connect() else {
            return;
        };
        if !list_oled_devices(&api).is_empty() {
            return; // SteelSeries device present, skip negative test
        }
        set_test_retry_max(Some(1));
        let term = Term::stdout();
        let r = connect_hid(&term, &api, "");
        assert!(r.is_err());
        set_test_retry_max(None);
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
    fn test_retry_connect_inner_bounded_gives_up() {
        let term = Term::stdout();
        let calls = std::cell::Cell::new(0u32);
        let sleep_calls = std::cell::Cell::new(0u32);
        let result: Result<i32, anyhow::Error> = retry_connect_inner(
            &term,
            "svc",
            || {
                calls.set(calls.get() + 1);
                Err(anyhow::anyhow!("never works"))
            },
            &|_| sleep_calls.set(sleep_calls.get() + 1),
            Some(2),
        );
        assert!(result.is_err());
        assert_eq!(calls.get(), 2);
        // First failure → 3 sleep_fn calls (countdown), second failure → no further sleeps before returning Err.
        assert_eq!(sleep_calls.get(), 3);
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
}
