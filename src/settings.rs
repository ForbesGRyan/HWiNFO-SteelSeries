use crate::consts::STYLE;
use console::Term;
use dialoguer::Input;
use hwinfo_steelseries_oled::Hwinfo;
use ini::Ini;

pub fn create_config(term: &Term, hwinfo: &Hwinfo) -> Result<Ini, anyhow::Error> {
    term.write_line("Config not found.")?;
    let mut conf = Ini::new();
    term.write_line(
        "Summary Vertical:
    1) CPU  GPU  MEM\n
       55°  45°  8.65G\n
       10%  0.0% 32.0G",
    )?;
    term.write_line(
        "Summary Horizontal:
    2) CPU  45°  10.0%\n
       GPU  35°  0.0%\n
       MEM  10G  33.3%",
    )?;
    term.write_line("3) Pick your own sensors")?;
    let input: u8 = Input::new()
        .with_prompt("Choose style\n(1,2,3)")
        .interact_text()?;
    let style = match input {
        1 => STYLE::VERTICAL,
        3 => STYLE::CUSTOM,
        2 | _ => STYLE::HORIZONTAL,
    };
    conf.with_section(Some("Main"))
        .set("style", style.to_string());

    if style != STYLE::CUSTOM {
        let gpus = hwinfo.find("GPU Temperature")?;
        let len_gpus = gpus.len();
        if len_gpus > 1 {
            term.write_line("Which GPU:\n")?;
            for (i, gpu) in gpus.iter().enumerate() {
                let sensor = &hwinfo.master_sensor_names[gpu.dw_sensor_index as usize];
                let setup = format!("{}: {}", i, sensor);
                term.write_line(&setup)?;
            }
            let gpu_selection: usize = Input::new()
                .with_prompt(format!("0..{}", len_gpus - 1))
                .interact_text()?;

            let gpu_selected =
                &hwinfo.master_sensor_names[gpus[gpu_selection].dw_sensor_index as usize];
            conf.with_section(Some("Main")).set("gpu", gpu_selected);
        }
    } else {
        println!("\n3 lines will fit on the Arctis(or Nova) Pro screen, and 2 on the Apex Pro.");
        let lines: u8 = match Input::new()
            .with_prompt("How many lines? (2-3)")
            .interact_text()
        {
            Ok(lines) => match lines {
                2 | 3 => lines,
                _ => 3,
            },
            Err(_) => 3,
        };
        let sensors_per_line: u8 = Input::new()
            .with_prompt("How many sensors per line? (1-3)")
            .interact_text()?;
        match sensors_per_line {
            1 | 2 | 3 => {
                conf.with_section(Some("Main"))
                    .set("sensors_per_line", sensors_per_line.to_string());
            }
            _ => return create_config(term, hwinfo),
        }

        for k in 0..(lines * sensors_per_line) {
            println!("\n{} / {}\n", k + 1, (lines * sensors_per_line));
            for (i, sensor) in hwinfo.master_sensor_names.iter().enumerate() {
                println!("{}) {}", i, sensor);
            }
            let length = hwinfo.master_sensor_names.len();
            let category: usize = match Input::new().with_prompt("Category").interact_text() {
                Ok(category) => {
                    if category >= length {
                        println!("Category out of range, please try again.");
                        return create_config(term, hwinfo);
                    } else {
                        category
                    }
                }
                Err(_) => 0,
            };
            let sensor_name = &hwinfo.master_sensor_names[category];
            let sensor = hwinfo.master_readings.sensors.get(sensor_name).unwrap();
            println!("\n{}:", sensor_name);
            let mut temp_readings = Vec::new();
            for (i, reading) in sensor.reading.iter().enumerate() {
                println!("\t{}) {}", i, reading.0);
                let sensor_key = format!("{};{}", sensor_name, reading.0);
                temp_readings.push(sensor_key.to_owned());
            }
            let sensor_selection: usize = Input::new().with_prompt("Sensor").interact_text()?;
            let sensor_selected = format!("\"{}\"", &temp_readings[sensor_selection]);
            let label: String = Input::new().with_prompt("Label").interact_text()?;
            let unit: String = Input::new().with_prompt("Unit").interact_text()?;
            let sensor_key = format!("sensor_{}", k);
            let label_key = format!("label_{}", k);
            let unit_key = format!("unit_{}", k);
            conf.with_section(Some("PAGE1.Sensors"))
                .set(sensor_key, sensor_selected);
            conf.with_section(Some("PAGE1.Sensors"))
                .set(label_key, label);
            conf.with_section(Some("PAGE1.Sensors")).set(unit_key, unit);
        }
    }
    conf.write_to_file("conf.ini")?;

    term.write_line("config created.")?;
    Ok(conf)
}
