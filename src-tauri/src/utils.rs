use chrono::Local;
use log::{debug, error};
use serde_json::{json, Value};

use hwinfo_steelseries_oled::Hwinfo;
use crate::media::{MediaField, MediaReader};
use crate::mouse_battery::MouseBatteryReader;

use crate::consts::{CUSTOM_SENSORS, DISPLAY_LINES};

pub fn run_sensors<'a>(
    pages_sensors: &'a ini::Properties,
    labels: &mut Vec<&'a str>,
    units: &mut Vec<&'a str>,
    values: &mut Vec<String>,
    hwinfo: &Hwinfo,
    decimal: bool,
    mouse_battery_reader: &mut MouseBatteryReader,
    media_reader: &mut MediaReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<(), anyhow::Error> {
    for k in 0..CUSTOM_SENSORS {
        let sensor_str = match pages_sensors.get(format!("sensor_{}", k)) {
            Some(sensor) => sensor,
            None => continue,
        };

        // Remove quotes if they exist (for backwards compatibility with old configs)
        let sensor_str = sensor_str.trim_matches('"');

        // Skip empty sensor entries
        if sensor_str.trim().is_empty() {
            continue;
        }

        let sensor: Vec<&str> = sensor_str.split(";").collect();
        let label = match pages_sensors.get(format!("label_{}", k)) {
            Some(label) => label,
            None => "",
        };
        let unit = match pages_sensors.get(format!("unit_{}", k)) {
            Some(unit) => unit,
            None => "",
        };
        if sensor[0] == "BLANK" {
            labels[k] = label;
            units[k] = unit;
            continue;
        } else if sensor[0] == "CLOCK" {
            labels[k] = label;
            units[k] = unit;
            let now = Local::now();
            values[k] = now.format("%I:%M:%S %P").to_string();
            continue;
        } else if sensor[0] == "MOUSE_BATTERY" {
            labels[k] = label;
            units[k] = unit;
            values[k] = mouse_battery_reader.get_battery_percentage(hid_api);
            continue;
        } else if let Some(media_field) = MediaField::from_sensor_name(sensor[0]) {
            // Handle MEDIA_* sensors (MEDIA_TITLE, MEDIA_ARTIST, MEDIA_ALBUM, MEDIA_APP)
            match media_reader.get_media_field(media_field) {
                Some(value) => {
                    labels[k] = label;
                    units[k] = unit;
                    values[k] = value;
                }
                None => {
                    // Hide sensor when nothing is playing - use empty strings
                    labels[k] = "";
                    units[k] = "";
                    values[k] = String::new();
                }
            }
            continue;
        }
        if sensor.len() < 2 {
            error!("Malformed sensor entry (missing ';'): {}", sensor_str);
            continue;
        }
        let mut value = match hwinfo.get(sensor[0], sensor[1]) {
            Some(value) => {
                debug!("Successfully read sensor: {} / {}", sensor[0], sensor[1]);
                value
            }
            None => {
                error!("Sensor not found: {} / {}", sensor[0], sensor[1]);
                return Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Sensor not found:\n\t{}\n\t{}", sensor[0], sensor[1]),
                )));
            }
        }
        .value;
        match pages_sensors.get(format!("convert_{}", k)) {
            Some(convert) => match convert {
                "MB/GB" => value = value / 1024.0,
                "kb/mb" | "KB/MB" => value = value / 1024.0,
                _ => {}
            },
            None => {}
        };
        let value_string: String;
        if decimal {
            value_string = format!("{:.1}", &value);
        } else {
            value_string = format!("{:02.0}", &value);
        }
        labels[k] = label;
        units[k] = unit;
        values[k] = value_string;
    }
    Ok(())
}

pub fn format_custom_value(
    sensors_per_line: u8,
    labels: Vec<&str>,
    values: Vec<String>,
    units: Vec<&str>,
) -> Value {
    let mut value = json!({});

    for line_idx in 0..DISPLAY_LINES {
        let start_idx = line_idx * sensors_per_line as usize;
        let line_parts: Vec<String> = (0..sensors_per_line as usize)
            .map(|i| {
                let idx = start_idx + i;
                let label = labels[idx].trim();
                let unit = units[idx].trim();

                // Format with proper spacing: add space between label and value only if label exists
                if label.is_empty() {
                    format!("{}{}", values[idx], unit)
                } else {
                    format!("{} {}{}", label, values[idx], unit)
                }
            })
            .collect();

        // Determine spacing based on sensors per line - tighter spacing for multiple sensors
        let separator = if sensors_per_line >= 3 { " " } else { "  " };
        value[format!("line{}", line_idx + 1)] = json!(line_parts.join(separator));
    }

    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_custom_value_one_sensor_per_line() {
        let labels = vec!["L1", "L2", "L3", "L4", "L5"];
        let mut all_labels = vec![""; 15];
        for (i, l) in labels.iter().enumerate() {
            all_labels[i] = l;
        }
        let values = vec![
            "1".to_string(),
            "2".to_string(),
            "3".to_string(),
            "4".to_string(),
            "5".to_string(),
        ];
        let mut all_values = vec![String::new(); 15];
        for (i, v) in values.iter().enumerate() {
            all_values[i] = v.clone();
        }
        let units = vec!["U1", "U2", "U3", "U4", "U5"];
        let mut all_units = vec![""; 15];
        for (i, u) in units.iter().enumerate() {
            all_units[i] = u;
        }

        let result = format_custom_value(1, all_labels, all_values, all_units);

        assert_eq!(result["line1"], "L1 1U1");
        assert_eq!(result["line2"], "L2 2U2");
        assert_eq!(result["line3"], "L3 3U3");
        assert_eq!(result["line4"], "L4 4U4");
        assert_eq!(result["line5"], "L5 5U5");
    }

    #[test]
    fn test_format_custom_value_three_sensors_per_line() {
        let labels = vec![""; 15];
        let mut values = vec![String::new(); 15];
        let units = vec![""; 15];

        for i in 0..15 {
            values[i] = i.to_string();
        }

        let result = format_custom_value(3, labels, values, units);

        assert_eq!(result["line1"], "0 1 2");
        assert_eq!(result["line2"], "3 4 5");
        assert_eq!(result["line3"], "6 7 8");
        assert_eq!(result["line4"], "9 10 11");
        assert_eq!(result["line5"], "12 13 14");
    }
}
