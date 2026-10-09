use chrono::{DateTime, Local};
use log::error;

use crate::consts::{CUSTOM_SENSORS, DISPLAY_LINES};
use crate::media::{MediaField, MediaReader};
use crate::mouse_battery::MouseBatteryReader;
use crate::settings::CustomSensor;
use crate::weather::{WeatherField, WeatherReader};
use hwinfo_steelseries_oled::Snapshot;

/// Format `now` with a user-supplied strftime string. chrono's `Display`
/// panics on an invalid specifier, so render into a buffer via `write!`
/// (which returns `Err` instead) and fall back to `fallback` on failure.
fn safe_strftime(now: &DateTime<Local>, fmt: &str, fallback: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    if write!(out, "{}", now.format(fmt)).is_err() {
        out.clear();
        let _ = write!(out, "{}", now.format(fallback));
    }
    out
}

/// One resolved sensor position, ready for layout.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Slot {
    pub label: String,
    pub value: String,
    pub unit: String,
    pub icon: String,
}

/// Resolve one page of configured sensors to display slots. Always returns
/// `CUSTOM_SENSORS` slots; unset or hidden sensors leave their slot's text
/// empty.
pub fn run_sensors(
    page: &[CustomSensor],
    decimal: bool,
    hwinfo: &Snapshot,
    mouse_battery_reader: &mut MouseBatteryReader,
    media_reader: &mut MediaReader,
    weather_reader: &WeatherReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<Vec<Slot>, anyhow::Error> {
    let mut slots = vec![Slot::default(); CUSTOM_SENSORS];

    for (configured, slot) in page.iter().zip(slots.iter_mut()) {
        slot.icon = configured.icon.clone();

        // Remove quotes if they exist (for backwards compatibility with old configs)
        let sensor_str = configured.sensor.trim_matches('"');
        if sensor_str.trim().is_empty() {
            continue;
        }
        let sensor: Vec<&str> = sensor_str.split(';').collect();

        // `None` hides the slot: label and unit stay empty too.
        let value: Option<String> = if sensor[0] == "BLANK" {
            Some(String::new())
        } else if sensor[0] == "CLOCK" || sensor[0] == "DATE" {
            let default_format = if sensor[0] == "CLOCK" {
                "%I:%M:%S %P"
            } else {
                "%Y-%m-%d"
            };
            let format = sensor
                .get(1)
                .copied()
                .filter(|s| !s.is_empty())
                .unwrap_or(default_format);
            Some(safe_strftime(&Local::now(), format, default_format))
        } else if sensor[0] == "MOUSE_BATTERY" {
            Some(mouse_battery_reader.get_battery_percentage(hid_api))
        } else if let Some(field) = MediaField::from_sensor_name(sensor[0]) {
            // Hidden when nothing is playing.
            media_reader.get_media_field(field)
        } else if let Some(field) = WeatherField::from_sensor_name(sensor[0]) {
            // Hidden when there is no data yet or the field is unset.
            weather_reader.get_field(field)
        } else if sensor.len() < 2 {
            error!("Malformed sensor entry (missing ';'): {}", sensor_str);
            None
        } else {
            let mut value = hwinfo.get(sensor[0], sensor[1]).ok_or_else(|| {
                error!("Sensor not found: {} / {}", sensor[0], sensor[1]);
                anyhow::anyhow!("Sensor not found:\n\t{}\n\t{}", sensor[0], sensor[1])
            })?;
            if matches!(configured.convert.as_str(), "MB/GB" | "kb/mb" | "KB/MB") {
                value /= 1024.0;
            }
            Some(if decimal {
                format!("{:.1}", value)
            } else {
                format!("{:02.0}", value)
            })
        };

        if let Some(value) = value {
            slot.label = configured.label.clone();
            slot.unit = configured.unit.clone();
            slot.value = value;
        }
    }
    Ok(slots)
}

/// Lay the slots out as `DISPLAY_LINES` text lines, `sensors_per_line` slots
/// per line. Positions beyond the available slots render empty.
pub fn format_custom_lines(sensors_per_line: u8, slots: &[Slot]) -> Vec<String> {
    let per_line = sensors_per_line as usize;
    // Tighter spacing when three or more sensors share a line.
    let separator = if per_line >= 3 { " " } else { "  " };
    let empty = Slot::default();

    (0..DISPLAY_LINES)
        .map(|line| {
            (0..per_line)
                .map(|i| {
                    let slot = slots.get(line * per_line + i).unwrap_or(&empty);
                    let label = slot.label.trim();
                    let unit = slot.unit.trim();
                    let icon = slot.icon.trim();

                    // Leading icon token (drawn only on the direct-USB bitmap
                    // path; a harmless control char in GameSense text).
                    let prefix = if icon.is_empty() {
                        String::new()
                    } else {
                        crate::render::icon_token(icon)
                    };
                    // A space between label and value only when there is a label.
                    if label.is_empty() {
                        format!("{}{}{}", prefix, slot.value, unit)
                    } else {
                        format!("{}{} {}{}", prefix, label, slot.value, unit)
                    }
                })
                .collect::<Vec<_>>()
                .join(separator)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn build_hwinfo(entries: &[(&str, &str, f64)]) -> Snapshot {
        Snapshot::from_readings(entries)
    }

    /// Build a page from `("sensor_0", "...")`-style pairs.
    fn make_page(pairs: &[(&str, &str)]) -> Vec<CustomSensor> {
        let mut page = vec![CustomSensor::default(); CUSTOM_SENSORS];
        for (key, value) in pairs {
            let (field, index) = key.rsplit_once('_').expect("key like sensor_0");
            let slot = &mut page[index.parse::<usize>().expect("numeric index")];
            match field {
                "sensor" => slot.sensor = value.to_string(),
                "label" => slot.label = value.to_string(),
                "unit" => slot.unit = value.to_string(),
                "convert" => slot.convert = value.to_string(),
                "icon" => slot.icon = value.to_string(),
                other => panic!("unknown field {}", other),
            }
        }
        page
    }

    fn slots(entries: &[(&str, &str, &str, &str)]) -> Vec<Slot> {
        entries
            .iter()
            .map(|(label, value, unit, icon)| Slot {
                label: label.to_string(),
                value: value.to_string(),
                unit: unit.to_string(),
                icon: icon.to_string(),
            })
            .collect()
    }

    fn weather_with_info() -> crate::weather::WeatherReader {
        let mut info = crate::weather::WeatherInfo {
            temp: Some("72".into()),
            condition_short: Some("P.Cloudy".into()),
            ..Default::default()
        };
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
        let props = make_page(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "Out"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "Out");
        assert_eq!(slots[0].unit, "°F");
        assert_eq!(slots[0].value, "72");
    }

    #[test]
    fn test_run_sensors_weather_forecast_day_field() {
        let props = make_page(&[
            ("sensor_0", "WEATHER_HI_D1"),
            ("label_0", "Tmrw"),
            ("unit_0", "°"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = weather_with_info();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "Tmrw");
        assert_eq!(slots[0].unit, "°");
        assert_eq!(slots[0].value, "75");
    }

    #[test]
    fn test_run_sensors_weather_hides_when_no_data() {
        let props = make_page(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "Out"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        // No data → sensor hides (empty label/unit/value).
        assert_eq!(slots[0].label, "");
        assert_eq!(slots[0].unit, "");
        assert_eq!(slots[0].value, "");
    }

    #[test]
    fn test_run_sensors_weather_unset_field_hides() {
        // Reader has *some* info but temp is None → sensor hides.
        let info = crate::weather::WeatherInfo::default(); // every field None
        let weather = crate::weather::WeatherReader::with_cached_info(info);

        let props = make_page(&[
            ("sensor_0", "WEATHER_TEMP"),
            ("label_0", "T"),
            ("unit_0", "°F"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "");
        assert_eq!(slots[0].value, "");
    }

    #[test]
    fn test_run_sensors_blank_skips_value() {
        let props = make_page(&[("sensor_0", "BLANK"), ("label_0", "spacer"), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "spacer");
        assert_eq!(slots[0].unit, "");
        assert_eq!(slots[0].value, "");
    }

    #[test]
    fn test_run_sensors_clock_formats_time() {
        let props = make_page(&[("sensor_0", "CLOCK"), ("label_0", "T"), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "T");
        // HH:MM:SS am/pm
        let re_like = slots[0].value.len() == 11
            && slots[0].value.chars().nth(2) == Some(':')
            && slots[0].value.chars().nth(5) == Some(':')
            && (slots[0].value.ends_with("am") || slots[0].value.ends_with("pm"));
        assert!(re_like, "unexpected CLOCK value: {}", slots[0].value);
    }

    #[test]
    fn test_run_sensors_clock_custom_format() {
        let props = make_page(&[("sensor_0", "CLOCK;%H:%M"), ("label_0", ""), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        // HH:MM, 24-hour, no seconds, no am/pm
        assert_eq!(slots[0].value.len(), 5);
        assert_eq!(slots[0].value.chars().nth(2), Some(':'));
        assert!(!slots[0].value.ends_with("am") && !slots[0].value.ends_with("pm"));
    }

    #[test]
    fn test_run_sensors_date_default_format() {
        let props = make_page(&[("sensor_0", "DATE"), ("label_0", "D"), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "D");
        // YYYY-MM-DD
        assert_eq!(slots[0].value.len(), 10);
        assert_eq!(slots[0].value.chars().nth(4), Some('-'));
        assert_eq!(slots[0].value.chars().nth(7), Some('-'));
    }

    #[test]
    fn test_run_sensors_date_custom_format() {
        let props = make_page(&[
            ("sensor_0", "DATE;%m/%d/%Y"),
            ("label_0", ""),
            ("unit_0", ""),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        // MM/DD/YYYY
        assert_eq!(slots[0].value.len(), 10);
        assert_eq!(slots[0].value.chars().nth(2), Some('/'));
        assert_eq!(slots[0].value.chars().nth(5), Some('/'));
    }

    #[test]
    fn test_run_sensors_mouse_battery_returns_na_without_hidapi() {
        let props = make_page(&[
            ("sensor_0", "MOUSE_BATTERY"),
            ("label_0", "MB"),
            ("unit_0", "%"),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "MB");
        assert_eq!(slots[0].unit, "%");
        assert_eq!(slots[0].value, "N/A");
    }

    #[test]
    fn test_run_sensors_media_returns_cached_value() {
        // MediaReader with is_playing=true and a populated title returns Some → slot populated.
        let props = make_page(&[
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

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "T:");
        assert_eq!(slots[0].unit, "!");
        assert_eq!(slots[0].value, "Song Name");
    }

    #[test]
    fn test_run_sensors_unknown_conversion_ignored() {
        // An unrecognised convert_ value leaves the value unchanged.
        let props = make_page(&[("sensor_0", "S;R"), ("convert_0", "unknown-unit")]);
        let hwinfo = build_hwinfo(&[("S", "R", 42.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].value, "42");
    }

    #[test]
    fn test_run_sensors_media_hides_when_nothing_playing() {
        // MediaReader::new() has no manager and is_playing=false → field is None → hide sensor.
        let props = make_page(&[
            ("sensor_0", "MEDIA_TITLE"),
            ("label_0", "Title:"),
            ("unit_0", ""),
        ]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "");
        assert_eq!(slots[0].unit, "");
        assert_eq!(slots[0].value, "");
    }

    #[test]
    fn test_run_sensors_regular_lookup_integer_format() {
        let props = make_page(&[
            ("sensor_0", "CPU [#0];Temperature"),
            ("label_0", "CPU"),
            ("unit_0", "°C"),
        ]);
        let hwinfo = build_hwinfo(&[("CPU [#0]", "Temperature", 42.7)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "CPU");
        assert_eq!(slots[0].unit, "°C");
        assert_eq!(slots[0].value, "43"); // {:02.0} rounds 42.7
    }

    #[test]
    fn test_run_sensors_decimal_format() {
        let props = make_page(&[("sensor_0", "CPU;Temp"), ("label_0", ""), ("unit_0", "")]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 42.75)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, true, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].value, "42.8"); // {:.1}
    }

    #[test]
    fn test_run_sensors_integer_pads_single_digit() {
        let props = make_page(&[("sensor_0", "S;R")]);
        let hwinfo = build_hwinfo(&[("S", "R", 7.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].value, "07"); // {:02.0}
    }

    #[test]
    fn test_run_sensors_mb_gb_conversion() {
        let props = make_page(&[("sensor_0", "MEM;Used"), ("convert_0", "MB/GB")]);
        let hwinfo = build_hwinfo(&[("MEM", "Used", 8192.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, true, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].value, "8.0"); // 8192 / 1024 = 8.0
    }

    #[test]
    fn test_run_sensors_kb_mb_conversion_both_cases() {
        for convert in &["kb/mb", "KB/MB"] {
            let props = make_page(&[("sensor_0", "S;R"), ("convert_0", *convert)]);
            let hwinfo = build_hwinfo(&[("S", "R", 2048.0)]);
            let mut mouse = MouseBatteryReader::new();
            let mut media = MediaReader::new();
            let weather = crate::weather::WeatherReader::disabled();

            let slots = run_sensors(
                &props, true, &hwinfo, &mut mouse, &mut media, &weather, None,
            )
            .unwrap();

            assert_eq!(slots[0].value, "2.0", "convert={}", convert);
        }
    }

    #[test]
    fn test_run_sensors_quoted_sensor_backwards_compat() {
        let props = make_page(&[("sensor_0", "\"CPU;Temp\"")]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 50.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].value, "50");
    }

    #[test]
    fn test_run_sensors_empty_sensor_string_skipped() {
        let props = make_page(&[("sensor_0", "   ")]);
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "");
        assert_eq!(slots[0].value, "");
    }

    #[test]
    fn test_run_sensors_no_sensor_key_skipped() {
        let props = make_page(&[]); // no sensor_* at all
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert!(slots.iter().all(|s| s.value.is_empty()));
    }

    #[test]
    fn test_run_sensors_malformed_no_semicolon_skipped() {
        // No ';' separator → logs error and continues; slot left empty.
        let props = make_page(&[("sensor_0", "NoSeparator")]);
        let hwinfo = build_hwinfo(&[("NoSeparator", "X", 1.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "");
        assert_eq!(slots[0].value, "");
    }

    #[test]
    fn test_run_sensors_missing_sensor_returns_err() {
        let props = make_page(&[("sensor_0", "GHOST;Reading")]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 1.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let err = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap_err();
        assert!(format!("{}", err).contains("Sensor not found"));
    }

    #[test]
    fn test_run_sensors_label_unit_default_when_missing() {
        let props = make_page(&[("sensor_0", "S;R")]); // no label_0/unit_0
        let hwinfo = build_hwinfo(&[("S", "R", 10.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].label, "");
        assert_eq!(slots[0].unit, "");
        assert_eq!(slots[0].value, "10");
    }

    #[test]
    fn test_run_sensors_multiple_sensors_at_different_indices() {
        let props = make_page(&[
            ("sensor_0", "CPU;Temp"),
            ("sensor_2", "GPU;Temp"),
            ("label_2", "G"),
            ("unit_2", "°C"),
        ]);
        let hwinfo = build_hwinfo(&[("CPU", "Temp", 40.0), ("GPU", "Temp", 60.0)]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let slots = run_sensors(
            &props, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(slots[0].value, "40");
        assert_eq!(slots[1].value, ""); // untouched
        assert_eq!(slots[2].label, "G");
        assert_eq!(slots[2].value, "60");
    }

    #[test]
    fn test_format_custom_lines_prepends_icon_token_when_set() {
        let lines = format_custom_lines(1, &slots(&[("CPU", "42", "", "cpu")]));
        assert!(lines[0].starts_with(&crate::render::icon_token("cpu")));
        assert!(lines[0].contains("CPU 42"));
    }

    #[test]
    fn test_format_custom_lines_no_token_when_icon_empty() {
        let lines = format_custom_lines(1, &slots(&[("CPU", "42", "", "")]));
        assert_eq!(lines[0], "CPU 42");
    }

    #[test]
    fn test_format_custom_lines_one_sensor_per_line() {
        let lines = format_custom_lines(
            1,
            &slots(&[
                ("L1", "1", "U1", ""),
                ("L2", "2", "U2", ""),
                ("L3", "3", "U3", ""),
                ("L4", "4", "U4", ""),
                ("L5", "5", "U5", ""),
            ]),
        );
        assert_eq!(
            lines,
            vec!["L1 1U1", "L2 2U2", "L3 3U3", "L4 4U4", "L5 5U5"]
        );
    }

    #[test]
    fn test_format_custom_lines_three_sensors_per_line() {
        let all: Vec<Slot> = (0..15)
            .map(|i| Slot {
                value: i.to_string(),
                ..Default::default()
            })
            .collect();
        let lines = format_custom_lines(3, &all);
        assert_eq!(
            lines,
            vec!["0 1 2", "3 4 5", "6 7 8", "9 10 11", "12 13 14"]
        );
    }

    #[test]
    fn test_format_custom_lines_always_returns_display_lines() {
        assert_eq!(format_custom_lines(1, &[]).len(), DISPLAY_LINES);
        assert_eq!(
            format_custom_lines(0, &slots(&[("A", "1", "", "")])).len(),
            DISPLAY_LINES
        );
    }

    #[test]
    fn test_format_custom_lines_survives_oversized_sensors_per_line() {
        // 4 per line x 5 lines = 20 positions, but only 15 slots exist.
        let all: Vec<Slot> = (0..15)
            .map(|i| Slot {
                value: i.to_string(),
                ..Default::default()
            })
            .collect();
        let lines = format_custom_lines(4, &all);
        assert_eq!(lines.len(), DISPLAY_LINES);
        assert_eq!(lines[0], "0 1 2 3");
        assert_eq!(lines[3], "12 13 14 ");
        assert_eq!(lines[4], "   ");
    }

    #[test]
    fn test_run_sensors_keeps_icon_and_pads_to_custom_sensors() {
        let page = vec![CustomSensor {
            sensor: "BLANK".into(),
            icon: "cpu".into(),
            ..Default::default()
        }];
        let hwinfo = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();

        let result = run_sensors(
            &page, false, &hwinfo, &mut mouse, &mut media, &weather, None,
        )
        .unwrap();

        assert_eq!(result.len(), CUSTOM_SENSORS);
        assert_eq!(result[0].icon, "cpu");
        assert_eq!(result[1], Slot::default());
    }
}
