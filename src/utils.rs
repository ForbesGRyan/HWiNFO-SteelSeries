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
    if sensors_per_line == 1 {
        for i in 0..DISPLAY_LINES {
            value[format!("line{}", i + 1)] =
                json!(format!("{} {}{}", labels[i], values[i], units[i]));
        }
    } else if sensors_per_line == 2 {
        value["line1"] = json!(format!(
            "{} {}{} {} {}{}",
            labels[0], values[0], units[0], labels[1], values[1], units[1]
        ));
        value["line2"] = json!(format!(
            "{} {}{} {} {}{}",
            labels[2], values[2], units[2], labels[3], values[3], units[3]
        ));
        value["line3"] = json!(format!(
            "{} {}{} {} {}{}",
            labels[4], values[4], units[4], labels[5], values[5], units[5]
        ));
    } else if sensors_per_line == 3 {
        value["line1"] = json!(format!(
            "{} {}{} {}{}{} {}{}{}",
            labels[0],
            values[0],
            units[0],
            labels[1],
            values[1],
            units[1],
            labels[2],
            values[2],
            units[2]
        ));
        value["line2"] = json!(format!(
            "{} {}{} {}{}{} {}{}{}",
            labels[3],
            values[3],
            units[3],
            labels[4],
            values[4],
            units[4],
            labels[5],
            values[5],
            units[5]
        ));
        value["line3"] = json!(format!(
            "{} {}{} {}{}{} {}{}{}",
            labels[6],
            values[6],
            units[6],
            labels[7],
            values[7],
            units[7],
            labels[8],
            values[8],
            units[8]
        ));
    }
    value
}
