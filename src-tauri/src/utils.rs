use chrono::Local;
use log::{debug, error};
use serde_json::{json, Value};

use crate::media::{MediaField, MediaReader};
use crate::mouse_battery::MouseBatteryReader;
use crate::weather::WeatherReader;
use hwinfo_steelseries_oled::Hwinfo;

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
    weather_reader: &WeatherReader,
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
        let label = pages_sensors
            .get(format!("label_{}", k))
            .unwrap_or_default();
        let unit = pages_sensors.get(format!("unit_{}", k)).unwrap_or_default();
        if sensor[0] == "BLANK" {
            labels[k] = label;
            units[k] = unit;
            continue;
        } else if sensor[0] == "CLOCK" {
            labels[k] = label;
            units[k] = unit;
            let fmt = sensor
                .get(1)
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or("%I:%M:%S %P");
            let now = Local::now();
            values[k] = now.format(fmt).to_string();
            continue;
        } else if sensor[0] == "DATE" {
            labels[k] = label;
            units[k] = unit;
            let fmt = sensor
                .get(1)
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or("%Y-%m-%d");
            let now = Local::now();
            values[k] = now.format(fmt).to_string();
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
        } else if let Some(weather_field) =
            crate::weather::WeatherField::from_sensor_name(sensor[0])
        {
            match weather_reader.get_field(weather_field) {
                Some(value) => {
                    labels[k] = label;
                    units[k] = unit;
                    values[k] = value;
                }
                None => {
                    // No data yet, or field unset → hide sensor slot (matches MEDIA_* pattern).
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
        if let Some(convert) = pages_sensors.get(format!("convert_{}", k)) {
            match convert {
                "MB/GB" => value /= 1024.0,
                "kb/mb" | "KB/MB" => value /= 1024.0,
                _ => {}
            }
        };
        let value_string: String = if decimal {
            format!("{:.1}", &value)
        } else {
            format!("{:02.0}", &value)
        };
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
    use hwinfo_steelseries_oled::{
        HwinfoSensorsReadingElement, HwinfoSensorsSensorElement, Sensor,
    };
    use std::collections::HashMap;

    fn build_hwinfo(entries: &[(&str, &str, f64)]) -> Hwinfo {
        let mut sensors: HashMap<String, Sensor> = HashMap::new();
        let mut sensor_names: Vec<String> = Vec::new();
        for (sensor_key, reading_key, value) in entries {
            let entry = sensors.entry(sensor_key.to_string()).or_insert_with(|| {
                sensor_names.push(sensor_key.to_string());
                Sensor {
                    info: HwinfoSensorsSensorElement::new_mock(0, sensor_key),
                    readings: HashMap::new(),
                    reading_names: Vec::new(),
                }
            });
            entry.readings.insert(
                reading_key.to_string(),
                HwinfoSensorsReadingElement::new_mock(0, 0, reading_key, *value),
            );
            entry.reading_names.push(reading_key.to_string());
        }
        Hwinfo::new_mock(sensors, sensor_names)
    }

    fn make_props(pairs: &[(&str, &str)]) -> ini::Properties {
        let mut p = ini::Properties::new();
        for (k, v) in pairs {
            p.insert(*k, *v);
        }
        p
    }

    fn empty_buffers<'a>() -> (Vec<&'a str>, Vec<&'a str>, Vec<String>) {
        (
            vec![""; CUSTOM_SENSORS],
            vec![""; CUSTOM_SENSORS],
            vec![String::new(); CUSTOM_SENSORS],
        )
    }

    fn weather_with_info() -> crate::weather::WeatherReader {
        let mut info = crate::weather::WeatherInfo::default();
        info.temp = Some("72".into());
        info.condition_short = Some("P.Cloudy".into());
        info.days[0] = Some(crate::weather::DayForecast {
            hi: Some("75".into()),
            lo: Some("60".into()),
            condition: Some("Sunny".into()),
            condition_short: Some("Sunny".into()),
            precip_chance: Some("10".into()),
        });
        crate::weather::WeatherReader::with_cached_info(info)
    }

    #[test]
    fn test_run_sensors_weather_temp_returns_cached() {
        let props = make_props(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "Out"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "Out");
        assert_eq!(units[0], "°F");
        assert_eq!(values[0], "72");
    }

    #[test]
    fn test_run_sensors_weather_forecast_day_field() {
        let props = make_props(&[
            ("sensor_0", "WEATHER_HI_D1"),
            ("label_0", "Tmrw"),
            ("unit_0", "°"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "Tmrw");
        assert_eq!(units[0], "°");
        assert_eq!(values[0], "75");
    }

    #[test]
    fn test_run_sensors_weather_hides_when_no_data() {
        let props = make_props(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "Out"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        // No data → sensor hides (empty label/unit/value).
        assert_eq!(labels[0], "");
        assert_eq!(units[0], "");
        assert_eq!(values[0], "");
    }

    #[test]
    fn test_run_sensors_weather_unset_field_hides() {
        // Reader has *some* info but temp is None → sensor hides.
        let info = crate::weather::WeatherInfo::default(); // every field None
        let weather = crate::weather::WeatherReader::with_cached_info(info);

        let props = make_props(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "T"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "");
        assert_eq!(values[0], "");
    }

    #[test]
    fn test_run_sensors_blank_skips_value() {
        let props = make_props(&[("sensor_0", "BLANK"), ("label_0", "spacer"), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "spacer");
        assert_eq!(units[0], "");
        assert_eq!(values[0], "");
    }

    #[test]
    fn test_run_sensors_clock_formats_time() {
        let props = make_props(&[("sensor_0", "CLOCK"), ("label_0", "T"), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "T");
        // HH:MM:SS am/pm
        let re_like = values[0].len() == 11
            && values[0].chars().nth(2) == Some(':')
            && values[0].chars().nth(5) == Some(':')
            && (values[0].ends_with("am") || values[0].ends_with("pm"));
        assert!(re_like, "unexpected CLOCK value: {}", values[0]);
    }

    #[test]
    fn test_run_sensors_clock_custom_format() {
        let props = make_props(&[("sensor_0", "CLOCK;%H:%M"), ("label_0", ""), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        // HH:MM, 24-hour, no seconds, no am/pm
        assert_eq!(values[0].len(), 5);
        assert_eq!(values[0].chars().nth(2), Some(':'));
        assert!(!values[0].ends_with("am") && !values[0].ends_with("pm"));
    }

    #[test]
    fn test_run_sensors_date_default_format() {
        let props = make_props(&[("sensor_0", "DATE"), ("label_0", "D"), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "D");
        // YYYY-MM-DD
        assert_eq!(values[0].len(), 10);
        assert_eq!(values[0].chars().nth(4), Some('-'));
        assert_eq!(values[0].chars().nth(7), Some('-'));
    }

    #[test]
    fn test_run_sensors_date_custom_format() {
        let props = make_props(&[
            ("sensor_0", "DATE;%m/%d/%Y"),
            ("label_0", ""),
            ("unit_0", ""),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        // MM/DD/YYYY
        assert_eq!(values[0].len(), 10);
        assert_eq!(values[0].chars().nth(2), Some('/'));
        assert_eq!(values[0].chars().nth(5), Some('/'));
    }

    #[test]
    fn test_run_sensors_mouse_battery_returns_na_without_hidapi() {
        let props = make_props(&[
            ("sensor_0", "MOUSE_BATTERY"),
            ("label_0", "MB"),
            ("unit_0", "%"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "MB");
        assert_eq!(units[0], "%");
        assert_eq!(values[0], "N/A");
    }

    #[test]
    fn test_run_sensors_media_returns_cached_value() {
        // MediaReader with is_playing=true and a populated title returns Some → labels/units/values set.
        let props = make_props(&[
            ("sensor_0", "MEDIA_TITLE"),
            ("label_0", "T:"),
            ("unit_0", "!"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let info = crate::media::MediaInfo {
            title: "Song Name".to_string(),
            artist: "Artist".to_string(),
            album: "Album".to_string(),
            app_name: "Spotify".to_string(),
            is_playing: true,
        };
        let mut media = MediaReader::with_cached_info(info);
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "T:");
        assert_eq!(units[0], "!");
        assert_eq!(values[0], "Song Name");
    }

    #[test]
    fn test_run_sensors_unknown_conversion_ignored() {
        // Unknown convert_ value hits the _ arm — leaves value unchanged.
        let props = make_props(&[("sensor_0", "S;R"), ("convert_0", "unknown-unit")]);
        let hwinfo = build_hwinfo(&[("S", "R", 42.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(values[0], "42");
    }

    #[test]
    fn test_run_sensors_media_hides_when_nothing_playing() {
        // MediaReader::new() has no manager and is_playing=false → field is None → hide sensor.
        let props = make_props(&[
            ("sensor_0", "MEDIA_TITLE"),
            ("label_0", "Title:"),
            ("unit_0", ""),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "");
        assert_eq!(units[0], "");
        assert_eq!(values[0], "");
    }

    #[test]
    fn test_run_sensors_regular_lookup_integer_format() {
        let props = make_props(&[
            ("sensor_0", "CPU [#0];Temperature"),
            ("label_0", "CPU"),
            ("unit_0", "°C"),
        ]);
        let hwinfo = build_hwinfo(&[("CPU [#0]", "Temperature", 42.7)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "CPU");
        assert_eq!(units[0], "°C");
        assert_eq!(values[0], "43"); // {:02.0} rounds 42.7
    }

    #[test]
    fn test_run_sensors_decimal_format() {
        let props = make_props(&[("sensor_0", "CPU;Temp"), ("label_0", ""), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 42.75)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            true,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(values[0], "42.8"); // {:.1}
    }

    #[test]
    fn test_run_sensors_integer_pads_single_digit() {
        let props = make_props(&[("sensor_0", "S;R")]);
        let hwinfo = build_hwinfo(&[("S", "R", 7.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(values[0], "07"); // {:02.0}
    }

    #[test]
    fn test_run_sensors_mb_gb_conversion() {
        let props = make_props(&[("sensor_0", "MEM;Used"), ("convert_0", "MB/GB")]);
        let hwinfo = build_hwinfo(&[("MEM", "Used", 8192.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            true,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(values[0], "8.0"); // 8192 / 1024 = 8.0
    }

    #[test]
    fn test_run_sensors_kb_mb_conversion_both_cases() {
        for convert in &["kb/mb", "KB/MB"] {
            let props = make_props(&[("sensor_0", "S;R"), ("convert_0", *convert)]);
            let hwinfo = build_hwinfo(&[("S", "R", 2048.0)]);
            let mut mouse = MouseBatteryReader::new();
            let mut media = MediaReader::new();
            let weather = crate::weather::WeatherReader::disabled();
            let (mut labels, mut units, mut values) = empty_buffers();

            run_sensors(
                &props,
                &mut labels,
                &mut units,
                &mut values,
                &hwinfo,
                true,
                &mut mouse,
                &mut media,
                &weather,
                None,
            )
            .unwrap();

            assert_eq!(values[0], "2.0", "convert={}", convert);
        }
    }

    #[test]
    fn test_run_sensors_quoted_sensor_backwards_compat() {
        let props = make_props(&[("sensor_0", "\"CPU;Temp\"")]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 50.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(values[0], "50");
    }

    #[test]
    fn test_run_sensors_empty_sensor_string_skipped() {
        let props = make_props(&[("sensor_0", "   ")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "");
        assert_eq!(values[0], "");
    }

    #[test]
    fn test_run_sensors_no_sensor_key_skipped() {
        let props = make_props(&[]); // no sensor_* at all
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert!(values.iter().all(|v| v.is_empty()));
    }

    #[test]
    fn test_run_sensors_malformed_no_semicolon_skipped() {
        // No ';' separator → logs error and continues; buffers untouched.
        let props = make_props(&[("sensor_0", "NoSeparator")]);
        let hwinfo = build_hwinfo(&[("NoSeparator", "X", 1.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "");
        assert_eq!(values[0], "");
    }

    #[test]
    fn test_run_sensors_missing_sensor_returns_err() {
        let props = make_props(&[("sensor_0", "GHOST;Reading")]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 1.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        let err = run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap_err();
        assert!(format!("{}", err).contains("Sensor not found"));
    }

    #[test]
    fn test_run_sensors_label_unit_default_when_missing() {
        let props = make_props(&[("sensor_0", "S;R")]); // no label_0/unit_0
        let hwinfo = build_hwinfo(&[("S", "R", 10.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(labels[0], "");
        assert_eq!(units[0], "");
        assert_eq!(values[0], "10");
    }

    #[test]
    fn test_run_sensors_multiple_sensors_at_different_indices() {
        let props = make_props(&[
            ("sensor_0", "CPU;Temp"),
            ("sensor_2", "GPU;Temp"),
            ("label_2", "G"),
            ("unit_2", "°C"),
        ]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 40.0), ("GPU", "Temp", 60.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        let (mut labels, mut units, mut values) = empty_buffers();

        run_sensors(
            &props,
            &mut labels,
            &mut units,
            &mut values,
            &hwinfo,
            false,
            &mut mouse,
            &mut media,
            &weather,
            None,
        )
        .unwrap();

        assert_eq!(values[0], "40");
        assert_eq!(values[1], ""); // untouched
        assert_eq!(labels[2], "G");
        assert_eq!(values[2], "60");
    }

    #[test]
    fn test_format_custom_value_one_sensor_per_line() {
        let labels = ["L1", "L2", "L3", "L4", "L5"];
        let mut all_labels = vec![""; 15];
        for (i, l) in labels.iter().enumerate() {
            all_labels[i] = l;
        }
        let values = [
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
        let units = ["U1", "U2", "U3", "U4", "U5"];
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

        for (i, v) in values.iter_mut().enumerate().take(15) {
            *v = i.to_string();
        }

        let result = format_custom_value(3, labels, values, units);

        assert_eq!(result["line1"], "0 1 2");
        assert_eq!(result["line2"], "3 4 5");
        assert_eq!(result["line3"], "6 7 8");
        assert_eq!(result["line4"], "9 10 11");
        assert_eq!(result["line5"], "12 13 14");
    }
}
