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
    (0x3710, 0x5406, "Pulsar 8Kdx Dongle"), // Your mouse!
    (0x0e7e, 0x2011, "Pulsar X2V2 Mini Wireless"),
    (0x0e7e, 0x2012, "Pulsar X2H Wireless"),
    (0x0e7e, 0x2013, "Pulsar X2 Wireless"),
    (0x0e7e, 0x2014, "Pulsar X2A Wireless"),
];

/// Classification of a feature-report response against an expected battery value.
#[derive(Debug, PartialEq, Eq)]
enum ReportClass {
    ContainsExpected,
    ContainsEncoded,
    LikelyBattery(u8),
    Plain,
}

/// Match (vid, pid) against MOUSE_PROFILES; returns (battery_report_id, name) on hit.
fn match_mouse_profile(vid: u16, pid: u16) -> Option<(u8, &'static str)> {
    MOUSE_PROFILES
        .iter()
        .find(|(v, p, _, _)| *v == vid && *p == pid)
        .map(|(_, _, report_id, name)| (*report_id, *name))
}

/// Match (vid, pid) against PULSAR_MICE; returns mouse name on hit.
fn match_pulsar_mouse(vid: u16, pid: u16) -> Option<&'static str> {
    PULSAR_MICE
        .iter()
        .find(|(v, p, _)| *v == vid && *p == pid)
        .map(|(_, _, name)| *name)
}

/// Build the 17-byte Pulsar POWER feature-report request.
fn build_pulsar_power_request() -> [u8; 17] {
    let mut request = [0u8; 17];
    request[0] = 0x08; // Report ID / payload header
    request[1] = 0x04; // POWER command
    request[16] = 0x49; // checksum: 0x55 - (0x08 + 0x04)
    request
}

/// Parse Pulsar POWER response: battery percentage at byte index 6, valid if ≤ 100.
fn parse_pulsar_power_response(response: &[u8]) -> Option<u8> {
    if response.len() >= 7 && response[6] <= 100 {
        Some(response[6])
    } else {
        None
    }
}

/// Classify a feature-report response against an optional expected value.
fn classify_report_response(data: &[u8], expected: Option<u8>) -> ReportClass {
    if let Some(exp) = expected {
        if data.contains(&exp) {
            return ReportClass::ContainsExpected;
        }
        let doubled = exp.saturating_mul(2);
        let hex_as_dec = format!("{:02x}", exp).parse::<u8>().unwrap_or(255);
        if data.iter().any(|&b| b == doubled || b == hex_as_dec) {
            return ReportClass::ContainsEncoded;
        }
    }
    match parse_likely_battery(data) {
        Some(b) => ReportClass::LikelyBattery(b),
        None => ReportClass::Plain,
    }
}

/// Position of expected battery byte within a report payload.
fn find_battery_in_report(data: &[u8], expected: u8) -> Option<usize> {
    data.iter().position(|&b| b == expected)
}

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

            if let Some(name) = match_pulsar_mouse(vid, pid) {
                info!(
                    "Found Pulsar mouse via HID: {} ({:04x}:{:04x})",
                    name, vid, pid
                );

                match device_info.open_device(api) {
                    Ok(device) => match Self::read_pulsar_battery_hid(&device) {
                        Ok(percentage) => {
                            info!("Pulsar {} battery: {}%", name, percentage);
                            return Ok(percentage);
                        }
                        Err(e) => {
                            debug!("Failed to read battery from {}: {}", name, e);
                        }
                    },
                    Err(e) => {
                        debug!("Failed to open Pulsar device {}: {}", name, e);
                    }
                }
            }
        }

        Err(anyhow::anyhow!("No Pulsar mouse found"))
    }

    /// Read battery from a Pulsar mouse using HID feature reports
    fn read_pulsar_battery_hid(device: &hidapi::HidDevice) -> Result<u8, anyhow::Error> {
        let request = build_pulsar_power_request();
        device
            .send_feature_report(&request)
            .map_err(|e| anyhow::anyhow!("Failed to send feature report: {}", e))?;

        let mut response = [0u8; 17];
        response[0] = 0x08;
        let bytes_read = device
            .get_feature_report(&mut response)
            .map_err(|e| anyhow::anyhow!("Failed to read feature report: {}", e))?;

        debug!(
            "Received {} bytes from Pulsar: {:02x?}",
            bytes_read,
            &response[..bytes_read]
        );

        parse_pulsar_power_response(&response[..bytes_read])
            .ok_or_else(|| anyhow::anyhow!("Invalid battery response from Pulsar"))
    }

    /// Find a compatible gaming mouse in the HID device list
    fn find_mouse_device(&self, api: &HidApi) -> Result<CachedMouseDevice, anyhow::Error> {
        for device_info in api.device_list() {
            let vid = device_info.vendor_id();
            let pid = device_info.product_id();

            if let Some((report_id, name)) = match_mouse_profile(vid, pid) {
                debug!("Found compatible mouse: {} ({:04x}:{:04x})", name, vid, pid);
                return Ok(CachedMouseDevice {
                    vendor_id: vid,
                    product_id: pid,
                    path: device_info.path().to_owned(),
                    battery_report_id: report_id,
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
        let hid_device = api
            .open_path(&device.path)
            .map_err(|e| anyhow::anyhow!("Failed to open mouse device: {}", e))?;

        // Set read timeout to 100ms
        hid_device
            .set_blocking_mode(false)
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
            bytes_read,
            &buf[..bytes_read]
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
        percentage_candidates
            .iter()
            .find(|&&candidate| candidate <= 100)
            .copied()
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

    info!(
        "Found {} HID interface(s) matching criteria:",
        matching_devices.len()
    );
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
                    let class = classify_report_response(&data, expected_battery);

                    info!(
                        "    {}",
                        format_report_log(report_id, &data, &class, expected_battery)
                    );

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

        info!(
            "  Feature report scan complete: {} reports returned data",
            reports_found
        );

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
    info!(
        "Discovery complete. Found {} report(s) with data across all interfaces.",
        all_results.len()
    );

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
    info!(
        "Searching ALL HID devices for battery value: {}%",
        expected_battery
    );
    info!("");

    let mut found_matches = Vec::new();

    // Enumerate all HID devices
    for device_info in api.device_list() {
        let vid = device_info.vendor_id();
        let pid = device_info.product_id();
        let product_name = device_info.product_string().unwrap_or("Unknown");
        let manufacturer = device_info.manufacturer_string().unwrap_or("Unknown");

        debug!(
            "Checking device: {:04x}:{:04x} - {} ({})",
            vid, pid, product_name, manufacturer
        );

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
                    if let Some(position) = find_battery_in_report(data, expected_battery) {
                        for line in format_brute_force_match(
                            vid,
                            pid,
                            product_name,
                            manufacturer,
                            report_id,
                            position,
                            expected_battery,
                            data,
                        ) {
                            info!("{}", line);
                        }
                        info!("");

                        found_matches.push((
                            vid,
                            pid,
                            report_id,
                            position,
                            product_name.to_string(),
                        ));
                    }
                }
                Ok(_) => {}
                Err(_) => {}
            }

            // Small delay to avoid overwhelming the device
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    for line in format_brute_force_summary(&found_matches, expected_battery) {
        info!("{}", line);
    }

    Ok(())
}

/// Pure helper: format a per-report log line for `discover_battery_report_id`.
fn format_report_log(
    report_id: u8,
    data: &[u8],
    class: &ReportClass,
    expected: Option<u8>,
) -> String {
    match class {
        ReportClass::ContainsExpected => format!(
            "Feature 0x{:02x}: {} bytes - {:02x?} *** CONTAINS EXPECTED VALUE {}% ***",
            report_id, data.len(), data, expected.unwrap_or(0)
        ),
        ReportClass::ContainsEncoded => format!(
            "Feature 0x{:02x}: {} bytes - {:02x?} ** CONTAINS ENCODED VALUE ({}*2={} or other encoding) **",
            report_id, data.len(), data, expected.unwrap_or(0), expected.unwrap_or(0).wrapping_mul(2)
        ),
        ReportClass::LikelyBattery(b) => format!(
            "Feature 0x{:02x}: {} bytes - {:02x?} <- LIKELY BATTERY: {}%",
            report_id, data.len(), data, b
        ),
        ReportClass::Plain => format!(
            "Feature 0x{:02x}: {} bytes - {:02x?}",
            report_id, data.len(), data
        ),
    }
}

/// Pure helper: count likely-battery hits across discover results.
#[allow(dead_code)]
fn count_likely_battery_results(results: &[(u8, Vec<u8>, Option<u8>)]) -> usize {
    results.iter().filter(|(_, _, b)| b.is_some()).count()
}

/// Pure helper: format brute-force match lines for `discover_all_devices_for_battery`.
fn format_brute_force_match(
    vid: u16,
    pid: u16,
    product: &str,
    manufacturer: &str,
    report_id: u8,
    position: usize,
    expected_battery: u8,
    data: &[u8],
) -> Vec<String> {
    vec![
        "*** MATCH FOUND ***".to_string(),
        format!(
            "  Device: {:04x}:{:04x} - {} ({})",
            vid, pid, product, manufacturer
        ),
        format!("  Report ID: 0x{:02x}", report_id),
        format!(
            "  Battery value {} found at byte position: {}",
            expected_battery, position
        ),
        format!("  Full report ({} bytes): {:02x?}", data.len(), data),
    ]
}

/// Pure helper: format the summary lines for the brute-force discovery.
fn format_brute_force_summary(
    found_matches: &[(u16, u16, u8, usize, String)],
    expected_battery: u8,
) -> Vec<String> {
    let mut out = vec!["=== DISCOVERY COMPLETE ===".to_string()];
    if found_matches.is_empty() {
        out.push(format!(
            "No devices found containing battery value {}%",
            expected_battery
        ));
    } else {
        out.push(format!(
            "Found {} device(s) with battery value {}%:",
            found_matches.len(),
            expected_battery
        ));
        for (vid, pid, report_id, position, name) in found_matches {
            out.push(format!(
                "  {:04x}:{:04x} ({}) - Report 0x{:02x}, byte {}",
                vid, pid, name, report_id, position
            ));
        }
    }
    out
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
        assert_eq!(
            reader_new.last_read.is_none(),
            reader_default.last_read.is_none()
        );
        assert_eq!(
            reader_new.cached_device.is_none(),
            reader_default.cached_device.is_none()
        );
        assert_eq!(
            reader_new.last_successful_read.is_none(),
            reader_default.last_successful_read.is_none()
        );
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

        let logitech_count = MOUSE_PROFILES
            .iter()
            .filter(|(vid, _, _, _)| *vid == logitech_vid)
            .count();
        let steelseries_count = MOUSE_PROFILES
            .iter()
            .filter(|(vid, _, _, _)| *vid == steelseries_vid)
            .count();
        let razer_count = MOUSE_PROFILES
            .iter()
            .filter(|(vid, _, _, _)| *vid == razer_vid)
            .count();

        assert!(logitech_count > 0, "Expected Logitech profiles");
        assert!(steelseries_count > 0, "Expected SteelSeries profiles");
        assert!(razer_count > 0, "Expected Razer profiles");
    }

    #[test]
    fn test_mouse_profiles_count() {
        // Verify we have a reasonable number of profiles
        assert!(
            MOUSE_PROFILES.len() >= 10,
            "Expected at least 10 mouse profiles, got {}",
            MOUSE_PROFILES.len()
        );
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
        assert!(
            !PULSAR_MICE.is_empty(),
            "Expected at least 1 Pulsar mouse profile, got {}",
            PULSAR_MICE.len()
        );
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

    // ==================== match_mouse_profile tests ====================

    #[test]
    fn test_match_mouse_profile_known_logitech() {
        let (report_id, name) = match_mouse_profile(0x046d, 0xc539).unwrap();
        assert_eq!(report_id, 0x07);
        assert!(name.contains("G502 Lightspeed"));
    }

    #[test]
    fn test_match_mouse_profile_known_steelseries() {
        let (report_id, name) = match_mouse_profile(0x1038, 0x1832).unwrap();
        assert_eq!(report_id, 0x06);
        assert!(name.contains("Aerox 3"));
    }

    #[test]
    fn test_match_mouse_profile_known_razer() {
        let (report_id, _) = match_mouse_profile(0x1532, 0x008c).unwrap();
        assert_eq!(report_id, 0x02);
    }

    #[test]
    fn test_match_mouse_profile_unknown() {
        assert!(match_mouse_profile(0xdead, 0xbeef).is_none());
    }

    #[test]
    fn test_match_mouse_profile_matches_every_entry() {
        for (vid, pid, expected_report, expected_name) in MOUSE_PROFILES {
            let (rid, name) = match_mouse_profile(*vid, *pid).unwrap();
            assert_eq!(rid, *expected_report);
            assert_eq!(name, *expected_name);
        }
    }

    // ==================== match_pulsar_mouse tests ====================

    #[test]
    fn test_match_pulsar_mouse_known() {
        assert_eq!(
            match_pulsar_mouse(0x3710, 0x5406),
            Some("Pulsar 8Kdx Dongle")
        );
        assert!(match_pulsar_mouse(0x0e7e, 0x2011).is_some());
    }

    #[test]
    fn test_match_pulsar_mouse_unknown() {
        assert!(match_pulsar_mouse(0x046d, 0xc539).is_none()); // Logitech, not Pulsar
        assert!(match_pulsar_mouse(0x0000, 0x0000).is_none());
    }

    #[test]
    fn test_match_pulsar_mouse_matches_every_entry() {
        for (vid, pid, expected_name) in PULSAR_MICE {
            assert_eq!(match_pulsar_mouse(*vid, *pid), Some(*expected_name));
        }
    }

    // ==================== Pulsar request/response tests ====================

    #[test]
    fn test_build_pulsar_power_request_layout() {
        let req = build_pulsar_power_request();
        assert_eq!(req.len(), 17);
        assert_eq!(req[0], 0x08); // report ID
        assert_eq!(req[1], 0x04); // POWER command
        for b in &req[2..16] {
            assert_eq!(*b, 0);
        }
        assert_eq!(req[16], 0x49); // checksum
    }

    #[test]
    fn test_build_pulsar_power_request_checksum_invariant() {
        let req = build_pulsar_power_request();
        // Checksum = 0x55 - (header + command)
        assert_eq!(req[16], 0x55u8.wrapping_sub(req[0] + req[1]));
    }

    #[test]
    fn test_parse_pulsar_power_response_valid() {
        let mut resp = [0u8; 17];
        resp[6] = 85;
        assert_eq!(parse_pulsar_power_response(&resp), Some(85));
    }

    #[test]
    fn test_parse_pulsar_power_response_zero() {
        let resp = [0u8; 17];
        assert_eq!(parse_pulsar_power_response(&resp), Some(0));
    }

    #[test]
    fn test_parse_pulsar_power_response_full() {
        let mut resp = [0u8; 17];
        resp[6] = 100;
        assert_eq!(parse_pulsar_power_response(&resp), Some(100));
    }

    #[test]
    fn test_parse_pulsar_power_response_out_of_range() {
        let mut resp = [0u8; 17];
        resp[6] = 101;
        assert_eq!(parse_pulsar_power_response(&resp), None);
        resp[6] = 255;
        assert_eq!(parse_pulsar_power_response(&resp), None);
    }

    #[test]
    fn test_parse_pulsar_power_response_too_short() {
        let resp = [0u8; 6];
        assert_eq!(parse_pulsar_power_response(&resp), None);
    }

    // ==================== classify_report_response tests ====================

    #[test]
    fn test_classify_report_response_contains_expected() {
        // expected=75 appears directly in data
        let data = [0x08, 99, 75, 33];
        assert_eq!(
            classify_report_response(&data, Some(75)),
            ReportClass::ContainsExpected
        );
    }

    #[test]
    fn test_classify_report_response_contains_encoded_doubled() {
        // expected=50 → doubled=100 appears
        let data = [0x08, 200, 100, 33];
        assert_eq!(
            classify_report_response(&data, Some(50)),
            ReportClass::ContainsEncoded
        );
    }

    #[test]
    fn test_classify_report_response_likely_battery_with_no_expected() {
        let data = [0x08, 75, 0, 0];
        assert_eq!(
            classify_report_response(&data, None),
            ReportClass::LikelyBattery(75)
        );
    }

    #[test]
    fn test_classify_report_response_plain_no_battery_no_match() {
        // No expected match, no likely battery (all > 100)
        let data = [0x08, 200, 200, 200];
        assert_eq!(
            classify_report_response(&data, Some(50)),
            ReportClass::Plain
        );
    }

    #[test]
    fn test_classify_report_response_expected_takes_priority_over_likely() {
        // Even though byte 75 would be likely battery, the expected check fires first
        let data = [0x08, 75, 33];
        assert_eq!(
            classify_report_response(&data, Some(75)),
            ReportClass::ContainsExpected
        );
    }

    #[test]
    fn test_classify_report_response_saturating_double() {
        // expected=200, doubled=255 (saturated). 255 in data → encoded.
        let data = [0x08, 255, 99];
        assert_eq!(
            classify_report_response(&data, Some(200)),
            ReportClass::ContainsEncoded
        );
    }

    // ==================== find_battery_in_report tests ====================

    #[test]
    fn test_find_battery_in_report_found() {
        let data = [0x08, 0x00, 75, 0x00];
        assert_eq!(find_battery_in_report(&data, 75), Some(2));
    }

    #[test]
    fn test_find_battery_in_report_first_occurrence() {
        let data = [0x08, 50, 0x00, 50];
        assert_eq!(find_battery_in_report(&data, 50), Some(1));
    }

    #[test]
    fn test_find_battery_in_report_not_found() {
        let data = [0x08, 0x00, 0x00, 0x00];
        assert_eq!(find_battery_in_report(&data, 75), None);
    }

    #[test]
    fn test_find_battery_in_report_empty() {
        let data: [u8; 0] = [];
        assert_eq!(find_battery_in_report(&data, 50), None);
    }

    // ==================== get_battery_percentage cache / API paths ====================

    /// HidApi::new() can fail in environments without HID support; tests that need it
    /// should be skipped gracefully rather than failing.
    fn try_hid_api() -> Option<HidApi> {
        HidApi::new().ok()
    }

    #[test]
    fn test_get_battery_percentage_uses_cache_within_30s() {
        let Some(api) = try_hid_api() else { return };
        let mut reader = MouseBatteryReader::new();
        reader.cached_value = "42".to_string();
        reader.last_read = Some(Instant::now());
        // last_read < 30s → returns cached value, no HID enumeration
        let result = reader.get_battery_percentage(Some(&api));
        assert_eq!(result, "42");
    }

    #[test]
    fn test_get_battery_percentage_clears_stale_device_cache() {
        let Some(api) = try_hid_api() else { return };
        let mut reader = MouseBatteryReader::new();
        // Stale last_successful_read (>5min ago). Use checked_sub so this still runs
        // on CI runners whose monotonic clock hasn't been alive that long yet.
        let Some(stale) = Instant::now().checked_sub(Duration::from_secs(600)) else {
            return;
        };
        reader.last_successful_read = Some(stale);
        reader.cached_device = Some(CachedMouseDevice {
            vendor_id: 0xdead,
            product_id: 0xbeef,
            path: CString::new("fake").unwrap(),
            battery_report_id: 0x07,
        });
        // No matching device in real enumeration → returns N/A, also clears cached_device
        let result = reader.get_battery_percentage(Some(&api));
        assert_eq!(result, "N/A");
        assert!(reader.cached_device.is_none());
        assert!(reader.last_read.is_some());
    }

    #[test]
    fn test_get_battery_percentage_with_api_no_mouse_returns_na() {
        let Some(api) = try_hid_api() else { return };
        let mut reader = MouseBatteryReader::new();
        // No cached value/device → enumerates → no match → N/A
        let result = reader.get_battery_percentage(Some(&api));
        assert_eq!(result, "N/A");
        assert!(reader.last_read.is_some());
    }

    #[test]
    fn test_get_battery_percentage_re_enumerate_on_cached_failure() {
        let Some(api) = try_hid_api() else { return };
        let mut reader = MouseBatteryReader::new();
        // cached_device set but path invalid → read fails → re-enumerate path fires (still N/A)
        reader.cached_device = Some(CachedMouseDevice {
            vendor_id: 0x046d,
            product_id: 0xc539,
            path: CString::new("nonexistent-path-XYZ").unwrap(),
            battery_report_id: 0x07,
        });
        let result = reader.get_battery_percentage(Some(&api));
        assert_eq!(result, "N/A");
    }

    #[test]
    fn test_try_read_battery_no_devices_errors() {
        let Some(api) = try_hid_api() else { return };
        let mut reader = MouseBatteryReader::new();
        let r = reader.try_read_battery(&api);
        assert!(r.is_err());
    }

    #[test]
    fn test_try_read_pulsar_battery_hidapi_no_match_errors() {
        let Some(api) = try_hid_api() else { return };
        let r = MouseBatteryReader::try_read_pulsar_battery_hidapi(&api);
        assert!(r.is_err());
    }

    #[test]
    fn test_find_mouse_device_no_match() {
        let Some(api) = try_hid_api() else { return };
        let reader = MouseBatteryReader::new();
        let r = reader.find_mouse_device(&api);
        match r {
            Err(e) => assert!(e.to_string().contains("No compatible gaming mouse")),
            Ok(_) => panic!("expected error"),
        }
    }

    // ==================== discover_battery_report_id ====================

    #[test]
    fn test_discover_battery_report_id_unknown_vid_errors() {
        let Some(api) = try_hid_api() else { return };
        let r = discover_battery_report_id(&api, 0xdead, None, None);
        assert!(r.is_err());
    }

    #[test]
    fn test_discover_battery_report_id_unknown_vid_pid_errors() {
        let Some(api) = try_hid_api() else { return };
        let r = discover_battery_report_id(&api, 0xdead, Some(0xbeef), Some(50));
        assert!(r.is_err());
        let msg = r.unwrap_err().to_string();
        assert!(msg.contains("not found") || msg.contains("No devices"));
    }

    // ==================== discover_all_devices_for_battery ====================

    #[test]
    fn test_discover_all_devices_for_battery_returns_ok_with_no_matches() {
        let Some(api) = try_hid_api() else { return };
        // Use a value unlikely to appear in any feature report on test box (255)
        let r = discover_all_devices_for_battery(&api, 255);
        assert!(r.is_ok());
    }

    // ==================== CachedMouseDevice clone ====================

    #[test]
    fn test_cached_mouse_device_clone() {
        let d = CachedMouseDevice {
            vendor_id: 0x046d,
            product_id: 0xc539,
            path: CString::new("X").unwrap(),
            battery_report_id: 0x07,
        };
        let c = d.clone();
        assert_eq!(c.vendor_id, 0x046d);
        assert_eq!(c.product_id, 0xc539);
        assert_eq!(c.battery_report_id, 0x07);
        assert_eq!(c.path.as_bytes(), b"X");
    }

    // ==================== ReportClass derive coverage ====================

    #[test]
    fn test_report_class_debug_format() {
        let _ = format!("{:?}", ReportClass::ContainsExpected);
        let _ = format!("{:?}", ReportClass::ContainsEncoded);
        let _ = format!("{:?}", ReportClass::LikelyBattery(50));
        let _ = format!("{:?}", ReportClass::Plain);
    }

    #[test]
    fn test_format_report_log_contains_expected() {
        let s = format_report_log(0x07, &[1, 50, 0], &ReportClass::ContainsExpected, Some(50));
        assert!(s.contains("0x07"));
        assert!(s.contains("CONTAINS EXPECTED VALUE 50%"));
    }

    #[test]
    fn test_format_report_log_contains_encoded() {
        let s = format_report_log(0x02, &[200, 100], &ReportClass::ContainsEncoded, Some(50));
        assert!(s.contains("0x02"));
        assert!(s.contains("ENCODED VALUE"));
        assert!(s.contains("50*2=100"));
    }

    #[test]
    fn test_format_report_log_likely_battery() {
        let s = format_report_log(0x06, &[0, 85], &ReportClass::LikelyBattery(85), None);
        assert!(s.contains("LIKELY BATTERY: 85%"));
    }

    #[test]
    fn test_format_report_log_plain() {
        let s = format_report_log(0x10, &[1, 2, 3], &ReportClass::Plain, None);
        assert!(s.contains("0x10"));
        assert!(!s.contains("LIKELY"));
        assert!(!s.contains("EXPECTED"));
        assert!(!s.contains("ENCODED"));
    }

    #[test]
    fn test_format_brute_force_match_includes_all_fields() {
        let lines = format_brute_force_match(
            0x1234,
            0xabcd,
            "Mouse",
            "Vendor",
            0x07,
            2,
            50,
            &[1, 2, 50, 4],
        );
        assert!(lines.iter().any(|l| l == "*** MATCH FOUND ***"));
        assert!(lines
            .iter()
            .any(|l| l.contains("1234:abcd") && l.contains("Mouse")));
        assert!(lines.iter().any(|l| l.contains("0x07")));
        assert!(lines.iter().any(|l| l.contains("byte position: 2")));
    }

    #[test]
    fn test_format_brute_force_summary_empty() {
        let lines = format_brute_force_summary(&[], 50);
        assert!(lines.iter().any(|l| l.contains("DISCOVERY COMPLETE")));
        assert!(lines
            .iter()
            .any(|l| l.contains("No devices found containing battery value 50%")));
    }

    #[test]
    fn test_format_brute_force_summary_with_matches() {
        let matches = vec![
            (0x046d, 0xc539, 0x07, 1, "G502".to_string()),
            (0x1038, 0x1832, 0x06, 2, "Aerox".to_string()),
        ];
        let lines = format_brute_force_summary(&matches, 75);
        assert!(lines.iter().any(|l| l.contains("Found 2 device(s)")));
        assert!(lines.iter().any(|l| l.contains("046d:c539")));
        assert!(lines.iter().any(|l| l.contains("1038:1832")));
    }

    #[test]
    fn test_count_likely_battery_results() {
        let results = vec![
            (0x01u8, vec![0x01, 0x00], None),
            (0x02u8, vec![0x02, 50], Some(50)),
            (0x03u8, vec![0x03, 75], Some(75)),
        ];
        assert_eq!(count_likely_battery_results(&results), 2);
        assert_eq!(count_likely_battery_results(&[]), 0);
    }

    #[test]
    fn test_parse_likely_battery_index_outside_first_four_returns_none() {
        // Zero appearing past index 3 → not battery
        let data = [0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0];
        assert_eq!(parse_likely_battery(&data), None);
    }
}
