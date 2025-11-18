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
