use hidapi::HidApi;
use log::{debug, info, warn};
use std::ffi::CString;
use std::time::{Duration, Instant};

/// Known gaming mouse profiles with battery support
/// Format: (VendorID, ProductID, BatteryReportID, Name)
const MOUSE_PROFILES: &[(u16, u16, u8, &str)] = &[
    // Logitech
    (0x046d, 0xc539, 0x07, "Logitech G502 Lightspeed"),
    (0x046d, 0xc547, 0x07, "Logitech G502 X Plus"),
    (0x046d, 0xc548, 0x07, "Logitech G502 X Lightspeed"),
    (0x046d, 0xc092, 0x07, "Logitech G502 Proteus"),
    (0x046d, 0xc07d, 0x07, "Logitech G502 Core"),
    (0x046d, 0xc53f, 0x07, "Logitech G815"),
    // SteelSeries
    (0x1038, 0x1832, 0x06, "SteelSeries Aerox 3 Wireless"),
    (0x1038, 0x1830, 0x06, "SteelSeries Aerox 5 Wireless"),
    (0x1038, 0x1836, 0x06, "SteelSeries Aerox 9 Wireless"),
    // Razer
    (0x1532, 0x008c, 0x02, "Razer Viper Ultimate"),
    (0x1532, 0x007c, 0x02, "Razer DeathAdder V2 Pro"),
    (0x1532, 0x00aa, 0x02, "Razer Basilisk Ultimate"),
];

/// Pulsar mice that use USB control transfers for battery (not HID reports)
/// Format: (VendorID, ProductID, Name)
const PULSAR_MICE: &[(u16, u16, &str)] = &[
    (0x3710, 0x5406, "Pulsar 8Kdx Dongle"),  // Your mouse!
    (0x0e7e, 0x2011, "Pulsar X2V2 Mini Wireless"),
    (0x0e7e, 0x2012, "Pulsar X2H Wireless"),
    (0x0e7e, 0x2013, "Pulsar X2 Wireless"),
    (0x0e7e, 0x2014, "Pulsar X2A Wireless"),
];

/// Cached mouse device information
#[derive(Clone)]
struct CachedMouseDevice {
    vendor_id: u16,
    product_id: u16,
    path: CString,
    battery_report_id: u8,
}

/// Reads battery percentage from wireless gaming mice via HID
pub struct MouseBatteryReader {
    cached_value: String,
    last_read: Option<Instant>,
    cached_device: Option<CachedMouseDevice>,
    last_successful_read: Option<Instant>,
}

impl MouseBatteryReader {
    /// Create a new MouseBatteryReader
    pub fn new() -> Self {
        Self {
            cached_value: String::from("N/A"),
            last_read: None,
            cached_device: None,
            last_successful_read: None,
        }
    }

    /// Get battery percentage as a formatted string
    /// Returns cached value if less than 30 seconds old
    /// Returns "N/A" if mouse not found, HID API not available, or error occurs
    pub fn get_battery_percentage(&mut self, api: Option<&HidApi>) -> String {
        // If no HID API is available (e.g., using GameSense mode), return N/A
        let api = match api {
            Some(api) => api,
            None => {
                debug!("HID API not available, returning N/A for mouse battery");
                return String::from("N/A");
            }
        };
        // Check cache expiration (30 seconds)
        if let Some(last_read) = self.last_read {
            if last_read.elapsed() < Duration::from_secs(30) {
                debug!("Using cached mouse battery value: {}", self.cached_value);
                return self.cached_value.clone();
            }
        }

        // Clear device cache if no successful read for 5 minutes
        if let Some(last_success) = self.last_successful_read {
            if last_success.elapsed() > Duration::from_secs(300) {
                debug!("Clearing mouse battery device cache (5 min timeout)");
                self.cached_device = None;
            }
        }

        // Try to read battery
        match self.try_read_battery(api) {
            Ok(percentage) => {
                self.cached_value = format!("{}", percentage);
                self.last_read = Some(Instant::now());
                self.last_successful_read = Some(Instant::now());
                debug!("Mouse battery: {}%", percentage);
                self.cached_value.clone()
            }
            Err(e) => {
                debug!("Mouse battery read failed: {}", e);

                // If we have a cached device, try re-enumerating once
                if self.cached_device.is_some() {
                    warn!("Cached mouse device failed, re-enumerating...");
                    self.cached_device = None;

                    if let Ok(percentage) = self.try_read_battery(api) {
                        self.cached_value = format!("{}", percentage);
                        self.last_read = Some(Instant::now());
                        self.last_successful_read = Some(Instant::now());
                        return self.cached_value.clone();
                    }
                }

                // Give up and return N/A
                self.cached_value = String::from("N/A");
                self.last_read = Some(Instant::now());
                self.cached_value.clone()
            }
        }
    }

    /// Try to read battery from mouse device
    fn try_read_battery(&mut self, api: &HidApi) -> Result<u8, anyhow::Error> {
        // First, check if this is a Pulsar mouse using HID feature reports
        match Self::try_read_pulsar_battery_hidapi(api) {
            Ok(percentage) => {
                debug!("Read battery from Pulsar mouse: {}%", percentage);
                return Ok(percentage);
            }
            Err(e) => {
                debug!("Pulsar battery read failed (not a Pulsar or error): {}", e);
                // Continue to standard HID method
            }
        }

        // Try to use cached HID device
        if let Some(ref cached) = self.cached_device {
            debug!(
                "Attempting battery read from cached device: {:04x}:{:04x}",
                cached.vendor_id, cached.product_id
            );

            match self.read_battery_from_device(api, cached) {
                Ok(percentage) => return Ok(percentage),
                Err(e) => {
                    debug!("Cached device read failed: {}", e);
                    // Fall through to enumeration
                }
            }
        }

        // Enumerate devices to find a mouse
        debug!("Enumerating HID devices for gaming mice...");
        let device = self.find_mouse_device(api)?;

        // Cache the device
        self.cached_device = Some(device.clone());

        // Read battery from newly found device
        self.read_battery_from_device(api, &device)
    }

    /// Try to read battery from Pulsar mice using HID feature reports
    fn try_read_pulsar_battery_hidapi(api: &HidApi) -> Result<u8, anyhow::Error> {
        debug!("Looking for Pulsar mice via HID...");

        // Check all HID devices for Pulsar mice
        for device_info in api.device_list() {
            let vid = device_info.vendor_id();
            let pid = device_info.product_id();

            // Check if this is a Pulsar mouse
            for (pulsar_vid, pulsar_pid, name) in PULSAR_MICE {
                if vid == *pulsar_vid && pid == *pulsar_pid {
                    info!("Found Pulsar mouse via HID: {} ({:04x}:{:04x})", name, vid, pid);

                    // Try to open the device
                    match device_info.open_device(api) {
                        Ok(device) => {
                            // Try to read battery using HID feature reports
                            match Self::read_pulsar_battery_hid(&device) {
                                Ok(percentage) => {
                                    info!("Pulsar {} battery: {}%", name, percentage);
                                    return Ok(percentage);
                                }
                                Err(e) => {
                                    debug!("Failed to read battery from {}: {}", name, e);
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            debug!("Failed to open Pulsar device {}: {}", name, e);
                            continue;
                        }
                    }
                }
            }
        }

        Err(anyhow::anyhow!("No Pulsar mouse found"))
    }

    /// Read battery from a Pulsar mouse using HID feature reports
    fn read_pulsar_battery_hid(device: &hidapi::HidDevice) -> Result<u8, anyhow::Error> {
        // Build POWER command payload (17 bytes)
        // [Header, Command, 14 data bytes, Checksum]
        let mut request = [0u8; 17];
        request[0] = 0x08; // Report ID / Payload header
        request[1] = 0x04; // POWER command
        // Bytes 2-15 are zeros
        request[16] = 0x49; // Checksum: 0x55 - (0x08 + 0x04) = 0x49

        debug!("Sending Pulsar POWER command via feature report...");

        // Send feature report (hidapi will handle the Set Report control transfer)
        device.send_feature_report(&request)
            .map_err(|e| anyhow::anyhow!("Failed to send feature report: {}", e))?;

        debug!("Sent feature report, reading response...");

        // Read response via feature report (hidapi will handle the Get Report control transfer)
        let mut response = [0u8; 17];
        response[0] = 0x08; // Report ID

        let bytes_read = device.get_feature_report(&mut response)
            .map_err(|e| anyhow::anyhow!("Failed to read feature report: {}", e))?;

        debug!("Received {} bytes from Pulsar: {:02x?}", bytes_read, &response[..bytes_read]);

        // Battery percentage is at byte 6 (0-indexed, so 7th byte total)
        if bytes_read >= 7 {
            let battery = response[6];
            if battery <= 100 {
                debug!("Pulsar battery value: {}%", battery);
                return Ok(battery);
            }
        }

        Err(anyhow::anyhow!("Invalid battery response from Pulsar"))
    }

    /// Find a compatible gaming mouse in the HID device list
    fn find_mouse_device(&self, api: &HidApi) -> Result<CachedMouseDevice, anyhow::Error> {
        for device_info in api.device_list() {
            let vid = device_info.vendor_id();
            let pid = device_info.product_id();

            // Check if this device matches a known profile
            if let Some((_, _, report_id, name)) = MOUSE_PROFILES
                .iter()
                .find(|(v, p, _, _)| *v == vid && *p == pid)
            {
                debug!("Found compatible mouse: {} ({:04x}:{:04x})", name, vid, pid);

                return Ok(CachedMouseDevice {
                    vendor_id: vid,
                    product_id: pid,
                    path: device_info.path().to_owned(),
                    battery_report_id: *report_id,
                });
            }
        }

        Err(anyhow::anyhow!("No compatible gaming mouse found"))
    }

    /// Read battery from a specific cached device
    fn read_battery_from_device(
        &self,
        api: &HidApi,
        device: &CachedMouseDevice,
    ) -> Result<u8, anyhow::Error> {
        // Open the HID device
        let hid_device = api.open_path(&device.path)
            .map_err(|e| anyhow::anyhow!("Failed to open mouse device: {}", e))?;

        // Set read timeout to 100ms
        hid_device.set_blocking_mode(false)
            .map_err(|e| anyhow::anyhow!("Failed to set non-blocking mode: {}", e))?;

        // Prepare feature report request
        let mut buf = [0u8; 8];
        buf[0] = device.battery_report_id;

        // Request feature report
        let bytes_read = hid_device
            .get_feature_report(&mut buf)
            .map_err(|e| anyhow::anyhow!("Failed to read feature report: {}", e))?;

        debug!(
            "Read {} bytes from mouse battery report: {:02x?}",
            bytes_read, &buf[..bytes_read]
        );

        // Parse battery from response
        Self::parse_battery_report(&buf)
            .ok_or_else(|| anyhow::anyhow!("Failed to parse battery from report"))
    }

    /// Parse battery percentage from HID report buffer
    /// Common patterns:
    /// - Logitech: Byte 1 contains percentage (0-100)
    /// - Razer: Byte 2 contains percentage (0-100)
    /// - SteelSeries: Byte 1 contains percentage (0-100)
    fn parse_battery_report(data: &[u8]) -> Option<u8> {
        if data.len() < 3 {
            return None;
        }

        // Try common battery percentage locations
        // Most devices put percentage in byte 1 or 2
        let percentage_candidates = [data[1], data[2]];

        // First pass: look for non-zero valid percentages (preferred)
        for &candidate in &percentage_candidates {
            if candidate > 0 && candidate <= 100 {
                return Some(candidate);
            }
        }

        // Second pass: accept 0 if that's all we have (battery depleted)
        for &candidate in &percentage_candidates {
            if candidate <= 100 {
                return Some(candidate);
            }
        }

        None
    }
}

impl Default for MouseBatteryReader {
    fn default() -> Self {
        Self::new()
    }
}

/// Discovery mode: Find the battery report ID for a specific mouse
///
/// This function helps users discover which HID report ID contains battery information
/// for their specific mouse model. It tries all common report IDs and displays the results.
///
/// # Arguments
/// * `api` - HidApi instance
/// * `vendor_id` - Mouse vendor ID (e.g., 0x046d for Logitech)
/// * `product_id` - Optional mouse product ID (e.g., 0xc539 for G502). If None, searches all devices with matching VID
/// * `expected_battery` - Optional expected battery value for comparison
///
/// # Returns
/// List of (report_id, data, likely_battery_value) tuples for reports that returned data
pub fn discover_battery_report_id(
    api: &HidApi,
    vendor_id: u16,
    product_id: Option<u16>,
    expected_battery: Option<u8>,
) -> Result<Vec<(u8, Vec<u8>, Option<u8>)>, anyhow::Error> {
    if let Some(pid) = product_id {
        info!(
            "Starting battery report ID discovery for device {:04x}:{:04x}",
            vendor_id, pid
        );
    } else {
        info!(
            "Starting battery report ID discovery for all devices with VID {:04x}",
            vendor_id
        );
    }

    if let Some(expected) = expected_battery {
        info!("Expected battery value: {}%", expected);
        info!("Will highlight reports containing this value!");
    }
    info!("");

    // Find ALL matching devices (there may be multiple interfaces)
    let matching_devices: Vec<_> = api
        .device_list()
        .filter(|d| {
            if let Some(pid) = product_id {
                d.vendor_id() == vendor_id && d.product_id() == pid
            } else {
                d.vendor_id() == vendor_id
            }
        })
        .collect();

    if matching_devices.is_empty() {
        if let Some(pid) = product_id {
            return Err(anyhow::anyhow!(
                "Device {:04x}:{:04x} not found in HID device list",
                vendor_id,
                pid
            ));
        } else {
            return Err(anyhow::anyhow!(
                "No devices found with VID {:04x}",
                vendor_id
            ));
        }
    }

    info!("Found {} HID interface(s) matching criteria:", matching_devices.len());
    for (idx, dev) in matching_devices.iter().enumerate() {
        info!(
            "  Interface {}: {} (Manufacturer: {}) [{:04x}:{:04x}]",
            idx,
            dev.product_string().unwrap_or("Unknown Product"),
            dev.manufacturer_string().unwrap_or("Unknown Manufacturer"),
            dev.vendor_id(),
            dev.product_id()
        );
        info!(
            "    Interface: {}, Usage Page: 0x{:04x}, Usage: 0x{:04x}",
            dev.interface_number(),
            dev.usage_page(),
            dev.usage()
        );
    }
    info!("");

    let mut all_results = Vec::new();

    // Try each interface
    for (idx, device_info) in matching_devices.iter().enumerate() {
        info!(
            "Trying Interface {} (Interface #{}, Usage Page: 0x{:04x})...",
            idx,
            device_info.interface_number(),
            device_info.usage_page()
        );

        // Try to open this interface
        let hid_device = match device_info.open_device(api) {
            Ok(dev) => dev,
            Err(e) => {
                warn!("  Failed to open interface {}: {}", idx, e);
                continue;
            }
        };

        // Set non-blocking mode
        if let Err(e) = hid_device.set_blocking_mode(false) {
            warn!("  Failed to set non-blocking mode: {}", e);
            continue;
        }

        let mut interface_results = Vec::new();

        // Try FEATURE reports (0x01 to 0xFF - full range)
        info!("  Trying FEATURE reports 0x01 to 0xFF...");
        let mut reports_found = 0;

        for report_id in 0x01u8..=0xFFu8 {
            let mut buf = [0u8; 64];
            buf[0] = report_id;

            match hid_device.get_feature_report(&mut buf) {
                Ok(bytes_read) if bytes_read > 0 => {
                    reports_found += 1;
                    let data = buf[..bytes_read].to_vec();
                    let likely_battery = parse_likely_battery(&data);

                    // Check if this report contains the expected battery value
                    let contains_expected = if let Some(expected) = expected_battery {
                        data.iter().any(|&b| b == expected)
                    } else {
                        false
                    };

                    // Also check for common encodings of the expected value
                    let contains_encoded = if let Some(expected) = expected_battery {
                        let doubled = expected.saturating_mul(2); // Some devices store as percentage * 2
                        let hex_as_dec = u8::from_str_radix(&format!("{:02x}", expected), 10).unwrap_or(255);
                        data.iter().any(|&b| b == doubled || b == hex_as_dec)
                    } else {
                        false
                    };

                    // Show ALL reports with data, highlight matches
                    if contains_expected {
                        info!(
                            "    Feature 0x{:02x}: {} bytes - {:02x?} *** CONTAINS EXPECTED VALUE {}% ***",
                            report_id, bytes_read, &data[..bytes_read], expected_battery.unwrap()
                        );
                    } else if contains_encoded {
                        info!(
                            "    Feature 0x{:02x}: {} bytes - {:02x?} ** CONTAINS ENCODED VALUE ({}*2={} or other encoding) **",
                            report_id, bytes_read, &data[..bytes_read], expected_battery.unwrap(), expected_battery.unwrap() * 2
                        );
                    } else if let Some(battery) = likely_battery {
                        info!(
                            "    Feature 0x{:02x}: {} bytes - {:02x?} <- LIKELY BATTERY: {}%",
                            report_id, bytes_read, &data[..bytes_read], battery
                        );
                    } else {
                        info!(
                            "    Feature 0x{:02x}: {} bytes - {:02x?}",
                            report_id, bytes_read, &data[..bytes_read]
                        );
                    }

                    interface_results.push((report_id, data, likely_battery));
                }
                Ok(_) => {}
                Err(e) => {
                    debug!("    Feature 0x{:02x}: Error - {}", report_id, e);
                }
            }

            // Shorter delay for faster scanning
            std::thread::sleep(Duration::from_millis(5));
        }

        // Skip INPUT reports - feature reports are sufficient for battery discovery
        // (Input reports would require waiting and user interaction)
        debug!("  Skipping INPUT reports (feature reports cover most cases)");

        info!("  Feature report scan complete: {} reports returned data", reports_found);

        if !interface_results.is_empty() {
            info!(
                "  Interface {} found {} potential battery report(s)",
                idx,
                interface_results.len()
            );
            all_results.extend(interface_results);
        } else {
            info!("  Interface {} returned no battery data", idx);
        }
        info!("");
    }

    info!("-----------------------------------------------------------");
    info!("Discovery complete. Found {} report(s) with data across all interfaces.", all_results.len());

    if all_results.is_empty() {
        warn!("No reports returned data. This could mean:");
        warn!("  1. The mouse is in sleep mode - try moving it during discovery");
        warn!("  2. The device doesn't expose battery via standard HID reports");
        warn!("  3. The device uses a proprietary protocol (check manufacturer's SDK)");
        warn!("  4. Battery reporting requires a specific driver or software");
    } else {
        info!("Reports with likely battery values:");
        for (report_id, _, likely_battery) in &all_results {
            if let Some(battery) = likely_battery {
                info!("  - Report 0x{:02x}: {}%", report_id, battery);
            }
        }
    }

    Ok(all_results)
}

/// Brute force search for battery value across ALL HID devices and ALL report IDs
pub fn discover_all_devices_for_battery(
    api: &HidApi,
    expected_battery: u8,
) -> Result<(), anyhow::Error> {
    info!("=== BRUTE FORCE BATTERY DISCOVERY ===");
    info!("Searching ALL HID devices for battery value: {}%", expected_battery);
    info!("");

    let mut found_matches = Vec::new();

    // Enumerate all HID devices
    for device_info in api.device_list() {
        let vid = device_info.vendor_id();
        let pid = device_info.product_id();
        let product_name = device_info.product_string().unwrap_or("Unknown");
        let manufacturer = device_info.manufacturer_string().unwrap_or("Unknown");

        debug!("Checking device: {:04x}:{:04x} - {} ({})", vid, pid, product_name, manufacturer);

        // Try to open the device
        let device = match device_info.open_device(api) {
            Ok(dev) => dev,
            Err(e) => {
                debug!("  Cannot open device: {}", e);
                continue;
            }
        };

        // Try all feature report IDs from 0x01 to 0xFF
        for report_id in 0x01u8..=0xFFu8 {
            let mut buf = [0u8; 64];
            buf[0] = report_id;

            match device.get_feature_report(&mut buf) {
                Ok(bytes_read) if bytes_read > 0 => {
                    let data = &buf[..bytes_read];

                    // Check if this report contains the expected battery value
                    if data.iter().any(|&b| b == expected_battery) {
                        // Found a match!
                        let position = data.iter().position(|&b| b == expected_battery).unwrap();

                        info!("*** MATCH FOUND ***");
                        info!("  Device: {:04x}:{:04x} - {} ({})", vid, pid, product_name, manufacturer);
                        info!("  Report ID: 0x{:02x}", report_id);
                        info!("  Battery value {} found at byte position: {}", expected_battery, position);
                        info!("  Full report ({} bytes): {:02x?}", bytes_read, data);
                        info!("");

                        found_matches.push((vid, pid, report_id, position, product_name.to_string()));
                    }
                }
                Ok(_) => {} // No data
                Err(_) => {} // Error reading report
            }

            // Small delay to avoid overwhelming the device
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    info!("=== DISCOVERY COMPLETE ===");
    if found_matches.is_empty() {
        info!("No devices found containing battery value {}%", expected_battery);
        info!("This could mean:");
        info!("  1. Battery value is encoded differently (multiplied, inverted, etc.)");
        info!("  2. Battery is updated only periodically");
        info!("  3. Device requires a command before exposing battery");
    } else {
        info!("Found {} device(s) with battery value {}%:", found_matches.len(), expected_battery);
        for (vid, pid, report_id, position, name) in &found_matches {
            info!("  {:04x}:{:04x} ({}) - Report 0x{:02x}, byte {}", vid, pid, name, report_id, position);
        }
    }

    Ok(())
}

/// Try to identify which byte in the report looks like a battery percentage
fn parse_likely_battery(data: &[u8]) -> Option<u8> {
    if data.len() < 2 {
        return None;
    }

    // Skip the first byte (report ID echo)
    for &byte in &data[1..] {
        // Battery percentages are typically 0-100
        // Prioritize non-zero values
        if byte > 0 && byte <= 100 {
            return Some(byte);
        }
    }

    // If no non-zero value, check if we have a 0 (could be depleted battery)
    for &byte in &data[1..] {
        if byte == 0 {
            // Could be battery at 0%, but also could be padding
            // Only return if it's one of the first few bytes
            let index = data.iter().position(|&b| b == byte).unwrap();
            if index <= 3 {
                return Some(0);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    // ==================== parse_battery_report tests ====================

    #[test]
    fn test_parse_battery_report_logitech() {
        // Logitech format: [ReportID, Percentage, ...]
        let data = [0x07, 75, 0x00, 0x00];
        assert_eq!(MouseBatteryReader::parse_battery_report(&data), Some(75));
    }

    #[test]
    fn test_parse_battery_report_razer() {
        // Razer format: [ReportID, 0x00, Percentage, ...]
        let data = [0x02, 0x00, 100, 0x00];
        assert_eq!(MouseBatteryReader::parse_battery_report(&data), Some(100));
    }

    #[test]
    fn test_parse_battery_report_invalid() {
        // Invalid data (percentage > 100)
        let data = [0x07, 255, 200, 0x00];
        assert_eq!(MouseBatteryReader::parse_battery_report(&data), None);
    }

    #[test]
    fn test_parse_battery_report_too_short() {
        // Buffer too short
        let data = [0x07];
        assert_eq!(MouseBatteryReader::parse_battery_report(&data), None);
    }

    // ==================== new() and Default trait tests ====================

    #[test]
    fn test_new_reader_defaults() {
        let reader = MouseBatteryReader::new();
        assert_eq!(reader.cached_value, "N/A");
        assert!(reader.last_read.is_none());
        assert!(reader.cached_device.is_none());
    }

    #[test]
    fn test_default_trait_implementation() {
        // Test that Default::default() produces the same result as new()
        let reader_new = MouseBatteryReader::new();
        let reader_default = MouseBatteryReader::default();

        assert_eq!(reader_new.cached_value, reader_default.cached_value);
        assert_eq!(reader_new.last_read.is_none(), reader_default.last_read.is_none());
        assert_eq!(reader_new.cached_device.is_none(), reader_default.cached_device.is_none());
        assert_eq!(reader_new.last_successful_read.is_none(), reader_default.last_successful_read.is_none());
    }

    // ==================== get_battery_percentage tests ====================

    #[test]
    fn test_get_battery_percentage_with_none_hidapi() {
        // When HidApi is None, should return "N/A" immediately
        let mut reader = MouseBatteryReader::new();
        let result = reader.get_battery_percentage(None);
        assert_eq!(result, "N/A");
    }

    #[test]
    fn test_get_battery_percentage_returns_cached_value_unchanged() {
        // When HidApi is None, the cached_value should remain "N/A"
        let mut reader = MouseBatteryReader::new();
        reader.cached_value = String::from("50"); // Simulate a previous read

        // With None HidApi, it should return N/A (not the cached value)
        let result = reader.get_battery_percentage(None);
        assert_eq!(result, "N/A");
    }

    // ==================== parse_likely_battery tests ====================

    #[test]
    fn test_parse_likely_battery_with_valid_value() {
        // Valid battery value in data (after report ID)
        let data = [0x08, 85, 0x00, 0x00];
        assert_eq!(parse_likely_battery(&data), Some(85));
    }

    #[test]
    fn test_parse_likely_battery_with_multiple_valid_values() {
        // Multiple valid values - should return the first one after report ID
        let data = [0x08, 75, 50, 25, 0x00];
        assert_eq!(parse_likely_battery(&data), Some(75));
    }

    #[test]
    fn test_parse_likely_battery_with_100_percent() {
        // Full battery
        let data = [0x04, 100, 0x00];
        assert_eq!(parse_likely_battery(&data), Some(100));
    }

    #[test]
    fn test_parse_likely_battery_with_1_percent() {
        // Minimum non-zero battery
        let data = [0x04, 1, 0xFF, 0xFF];
        assert_eq!(parse_likely_battery(&data), Some(1));
    }

    #[test]
    fn test_parse_likely_battery_with_zero_first_position() {
        // Zero battery at position 1 (index 1) - should return Some(0)
        let data = [0x08, 0, 0xFF, 0xFF];
        assert_eq!(parse_likely_battery(&data), Some(0));
    }

    #[test]
    fn test_parse_likely_battery_no_valid_values() {
        // All values > 100 (except report ID)
        let data = [0x08, 255, 200, 150, 128];
        assert_eq!(parse_likely_battery(&data), None);
    }

    #[test]
    fn test_parse_likely_battery_empty_data() {
        // Empty data
        let data: [u8; 0] = [];
        assert_eq!(parse_likely_battery(&data), None);
    }

    #[test]
    fn test_parse_likely_battery_single_byte() {
        // Only report ID, no data
        let data = [0x08];
        assert_eq!(parse_likely_battery(&data), None);
    }

    #[test]
    fn test_parse_likely_battery_two_bytes_with_valid() {
        // Minimum valid data: report ID + battery
        let data = [0x08, 42];
        assert_eq!(parse_likely_battery(&data), Some(42));
    }

    #[test]
    fn test_parse_likely_battery_skips_first_byte() {
        // First byte is valid percentage but should be skipped (it's report ID)
        let data = [50, 200, 200, 200];
        // 50 is the report ID, all other values are > 100
        assert_eq!(parse_likely_battery(&data), None);
    }

    #[test]
    fn test_parse_likely_battery_prioritizes_nonzero() {
        // Zero first, then valid value - should return the non-zero value
        let data = [0x08, 0, 75, 0, 0];
        assert_eq!(parse_likely_battery(&data), Some(75));
    }

    // ==================== MOUSE_PROFILES constant validation ====================

    #[test]
    fn test_mouse_profiles_have_valid_vendor_ids() {
        // Verify all profiles have non-zero vendor IDs
        for (vid, _pid, _report_id, name) in MOUSE_PROFILES {
            assert!(*vid > 0, "Profile '{}' has invalid vendor ID 0", name);
        }
    }

    #[test]
    fn test_mouse_profiles_have_valid_product_ids() {
        // Verify all profiles have non-zero product IDs
        for (_vid, pid, _report_id, name) in MOUSE_PROFILES {
            assert!(*pid > 0, "Profile '{}' has invalid product ID 0", name);
        }
    }

    #[test]
    fn test_mouse_profiles_have_valid_report_ids() {
        // Verify all profiles have non-zero report IDs
        for (_vid, _pid, report_id, name) in MOUSE_PROFILES {
            assert!(*report_id > 0, "Profile '{}' has invalid report ID 0", name);
        }
    }

    #[test]
    fn test_mouse_profiles_have_names() {
        // Verify all profiles have non-empty names
        for (_vid, _pid, _report_id, name) in MOUSE_PROFILES {
            assert!(!name.is_empty(), "Profile has empty name");
        }
    }

    #[test]
    fn test_mouse_profiles_known_vendors() {
        // Verify known vendor IDs are correct
        let logitech_vid = 0x046d;
        let steelseries_vid = 0x1038;
        let razer_vid = 0x1532;

        let logitech_count = MOUSE_PROFILES.iter().filter(|(vid, _, _, _)| *vid == logitech_vid).count();
        let steelseries_count = MOUSE_PROFILES.iter().filter(|(vid, _, _, _)| *vid == steelseries_vid).count();
        let razer_count = MOUSE_PROFILES.iter().filter(|(vid, _, _, _)| *vid == razer_vid).count();

        assert!(logitech_count > 0, "Expected Logitech profiles");
        assert!(steelseries_count > 0, "Expected SteelSeries profiles");
        assert!(razer_count > 0, "Expected Razer profiles");
    }

    #[test]
    fn test_mouse_profiles_count() {
        // Verify we have a reasonable number of profiles
        assert!(MOUSE_PROFILES.len() >= 10, "Expected at least 10 mouse profiles, got {}", MOUSE_PROFILES.len());
    }

    // ==================== PULSAR_MICE constant validation ====================

    #[test]
    fn test_pulsar_mice_have_valid_vendor_ids() {
        // Verify all Pulsar mice have non-zero vendor IDs
        for (vid, _pid, name) in PULSAR_MICE {
            assert!(*vid > 0, "Pulsar mouse '{}' has invalid vendor ID 0", name);
        }
    }

    #[test]
    fn test_pulsar_mice_have_valid_product_ids() {
        // Verify all Pulsar mice have non-zero product IDs
        for (_vid, pid, name) in PULSAR_MICE {
            assert!(*pid > 0, "Pulsar mouse '{}' has invalid product ID 0", name);
        }
    }

    #[test]
    fn test_pulsar_mice_have_names() {
        // Verify all Pulsar mice have non-empty names
        for (_vid, _pid, name) in PULSAR_MICE {
            assert!(!name.is_empty(), "Pulsar mouse has empty name");
        }
    }

    #[test]
    fn test_pulsar_mice_count() {
        // Verify we have Pulsar mice defined
        assert!(PULSAR_MICE.len() >= 1, "Expected at least 1 Pulsar mouse profile, got {}", PULSAR_MICE.len());
    }

    #[test]
    fn test_pulsar_mice_known_vendor_ids() {
        // Pulsar uses multiple vendor IDs
        let valid_pulsar_vids = [0x3710, 0x0e7e];

        for (vid, _pid, name) in PULSAR_MICE {
            assert!(
                valid_pulsar_vids.contains(vid),
                "Pulsar mouse '{}' has unexpected vendor ID: 0x{:04x}",
                name,
                vid
            );
        }
    }

    // ==================== Integration test notes ====================
    // The following functions require actual HID hardware and cannot be unit tested:
    //
    // - try_read_battery(): Requires HidApi with real devices
    // - find_mouse_device(): Requires HidApi device enumeration
    // - read_battery_from_device(): Requires opening real HID device
    // - try_read_pulsar_battery_hidapi(): Requires Pulsar mouse hardware
    // - read_pulsar_battery_hid(): Requires Pulsar mouse hardware
    // - discover_battery_report_id(): Requires HidApi and real devices
    // - discover_all_devices_for_battery(): Requires HidApi and real devices
    //
    // These would need integration tests with mock HID devices or actual hardware.
}
