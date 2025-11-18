use chrono::Local;
use serde_json::{json, Value};

use crate::Hwinfo;

use crate::consts::{CUSTOM_SENSORS, DISPLAY_LINES};

pub fn run_sensors<'a>(
    pages_sensors: &'a ini::Properties,
    labels: &mut Vec<&'a str>,
    units: &mut Vec<&'a str>,
    values: &mut Vec<String>,
    hwinfo: &Hwinfo,
    decimal: bool,
) -> Result<(), anyhow::Error> {
    for k in 0..CUSTOM_SENSORS {
        let sensor = match pages_sensors.get(format!("sensor_{}", k)) {
            Some(sensor) => sensor,
            None => continue,
        }
        .split(";")
        .collect::<Vec<&str>>();
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
            values[k] = now.format("%I:%M%P").to_string();
            continue;
        }
        let mut value = match hwinfo.get(sensor[0], sensor[1]) {
            Some(value) => value,
            None => {
                return Err(anyhow::Error::new(std::io::Error::new(
                    std::io::ErrorKind::NotFound,
                    format!("Sensor not found:\n\t{}\n\t{}", sensor[0], sensor[1]),
                )))
            }
        }
        .value;
        match pages_sensors.get(format!("convert_{}", k)) {
            Some(convert) => match convert {
                "MB/GB" => value = value / 1024.0,
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
                format!("{} {}{}", labels[idx], values[idx], units[idx])
            })
            .collect();

        value[format!("line{}", line_idx + 1)] = json!(line_parts.join(" "));
    }

    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_custom_value_one_sensor_per_line() {
        let labels = vec!["CPU", "GPU", "MEM"];
        let values = vec!["65".to_string(), "72".to_string(), "16".to_string()];
        let units = vec!["°", "°", "G"];

        let result = format_custom_value(1, labels, values, units);

        assert_eq!(result["line1"], "CPU 65°");
        assert_eq!(result["line2"], "GPU 72°");
        assert_eq!(result["line3"], "MEM 16G");
    }

    #[test]
    fn test_format_custom_value_two_sensors_per_line() {
        let labels = vec!["C", "65", "G", "72", "M", "16"];
        let values = vec!["".to_string(), "%".to_string(), "".to_string(), "°".to_string(), "".to_string(), "G".to_string()];
        let units = vec!["", "", "", "", "", ""];

        let result = format_custom_value(2, labels, values, units);

        assert_eq!(result["line1"], "C  65 %");
        assert_eq!(result["line2"], "G  72 °");
        assert_eq!(result["line3"], "M  16 G");
    }

    #[test]
    fn test_format_custom_value_three_sensors_per_line() {
        let labels = vec!["A", "B", "C", "D", "E", "F", "G", "H", "I"];
        let values = vec!["1".to_string(), "2".to_string(), "3".to_string(),
                          "4".to_string(), "5".to_string(), "6".to_string(),
                          "7".to_string(), "8".to_string(), "9".to_string()];
        let units = vec!["", "", "", "", "", "", "", "", ""];

        let result = format_custom_value(3, labels, values, units);

        assert_eq!(result["line1"], "A 1 B 2 C 3");
        assert_eq!(result["line2"], "D 4 E 5 F 6");
        assert_eq!(result["line3"], "G 7 H 8 I 9");
    }

    #[test]
    fn test_format_custom_value_empty_labels() {
        let labels = vec!["", "", ""];
        let values = vec!["100".to_string(), "200".to_string(), "300".to_string()];
        let units = vec!["W", "W", "W"];

        let result = format_custom_value(1, labels, values, units);

        assert_eq!(result["line1"], " 100W");
        assert_eq!(result["line2"], " 200W");
        assert_eq!(result["line3"], " 300W");
    }

    #[test]
    fn test_format_custom_value_mixed_content() {
        let labels = vec!["FPS", "⛏", "💻"];
        let values = vec!["144".to_string(), "45".to_string(), "60".to_string()];
        let units = vec!["", "°", "°"];

        let result = format_custom_value(1, labels, values, units);

        assert_eq!(result["line1"], "FPS 144");
        assert_eq!(result["line2"], "⛏ 45°");
        assert_eq!(result["line3"], "💻 60°");
    }
}
