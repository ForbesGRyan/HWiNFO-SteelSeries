use crate::consts::Style;
use console::Term;
use dialoguer::Input;
use hwinfo_steelseries_oled::Hwinfo;
use ini::Ini;

// Configuration struct for parsing existing config files
pub struct AppConfig<'a> {
    pub is_summary: bool,
    pub is_vertical: bool,
    pub gpu: &'a str,
    pub decimal: bool,
    pub pages: usize,
    pub page_time: isize,
    pub sensors_per_line: u8,
}

impl<'a> AppConfig<'a> {
    pub fn from_ini(config: &'a Ini) -> Result<Self, anyhow::Error> {
        let main = config
            .section(Some("Main"))
            .ok_or_else(|| anyhow::anyhow!("Main config section not found"))?;

        let style = main
            .get("style")
            .ok_or_else(|| anyhow::anyhow!("Style not found"))?
            .to_lowercase();

        let is_summary = matches!(style.as_str(), "vertical" | "horizontal");
        let is_vertical = style == "vertical";

        let gpu = if is_summary {
            main.get("gpu").unwrap_or("")
        } else {
            ""
        };

        let decimal = main
            .get("decimal")
            .and_then(|d| d.parse::<bool>().ok())
            .unwrap_or(false);

        let pages = main
            .get("pages")
            .and_then(|p| p.parse::<usize>().ok())
            .unwrap_or(1);

        let page_time = main
            .get("page_time")
            .and_then(|pt| pt.parse::<isize>().ok())
            .map(|num| if (0..=60).contains(&num) { num } else { 5 })
            .unwrap_or(5);

        let sensors_per_line = if !is_summary {
            main.get("sensors_per_line")
                .and_then(|spl| spl.parse::<u8>().ok())
                .unwrap_or(1)
        } else {
            1
        };

        Ok(Self {
            is_summary,
            is_vertical,
            gpu,
            decimal,
            pages,
            page_time,
            sensors_per_line,
        })
    }
}

fn configure_gpu_selection(
    term: &Term,
    hwinfo: &Hwinfo,
    conf: &mut Ini,
) -> Result<(), anyhow::Error> {
    let gpus = hwinfo.find("GPU Temperature")?;
    if gpus.len() <= 1 {
        return Ok(());
    }

    term.write_line("Which GPU:\n")?;
    for (i, gpu) in gpus.iter().enumerate() {
        let sensor_name = &hwinfo.sensor_names[gpu.dw_sensor_index as usize];
        term.write_line(&format!("{}: {}", i, sensor_name))?;
    }

    let gpu_selection: usize = Input::new()
        .with_prompt(format!("0..{}", gpus.len() - 1))
        .interact_text()?;

    let gpu_selected = &hwinfo.sensor_names[gpus[gpu_selection].dw_sensor_index as usize];
    conf.with_section(Some("Main")).set("gpu", gpu_selected);

    Ok(())
}

fn configure_custom_sensors(
    hwinfo: &Hwinfo,
    conf: &mut Ini,
    lines: u8,
    sensors_per_line: u8,
) -> Result<(), anyhow::Error> {
    for k in 0..(lines * sensors_per_line) {
        println!("\n{} / {}\n", k + 1, lines * sensors_per_line);

        // Display available sensors
        for (i, sensor) in hwinfo.sensor_names.iter().enumerate() {
            println!("{}) {}", i, sensor);
        }

        let category: usize = Input::new()
            .with_prompt("Category")
            .interact_text()
            .unwrap_or(0);

        if category >= hwinfo.sensor_names.len() {
            println!("Category out of range, please try again.");
            return Err(anyhow::anyhow!("Invalid category selection"));
        }

        let sensor_name = &hwinfo.sensor_names[category];
        let sensor = hwinfo.sensors.get(sensor_name).unwrap();

        // Display available readings for selected sensor
        println!("\n{}:", sensor_name);
        let temp_readings: Vec<String> = sensor
            .readings
            .iter()
            .enumerate()
            .map(|(i, reading)| {
                println!("\t{}) {}", i, reading.0);
                format!("{};{}", sensor_name, reading.0)
            })
            .collect();

        let sensor_selection: usize = Input::new().with_prompt("Sensor").interact_text()?;
        let sensor_selected = format!("\"{}\"", &temp_readings[sensor_selection]);
        let label: String = Input::new().with_prompt("Label").interact_text()?;
        let unit: String = Input::new().with_prompt("Unit").interact_text()?;

        conf.with_section(Some("PAGE1.Sensors"))
            .set(format!("sensor_{}", k), sensor_selected)
            .set(format!("label_{}", k), label)
            .set(format!("unit_{}", k), unit);
    }

    Ok(())
}

pub fn settings_create_config(term: &Term, hwinfo: &Hwinfo) -> Result<Ini, anyhow::Error> {
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
        1 => Style::Vertical,
        3 => Style::Custom,
        2 | _ => Style::Horizontal,
    };

    conf.with_section(Some("Main"))
        .set("style", style.to_string());

    if style != Style::Custom {
        configure_gpu_selection(term, hwinfo, &mut conf)?;
    } else {
        println!("\n3 lines will fit on the Arctis(or Nova) Pro screen, and 2 on the Apex Pro.");

        let lines: u8 = Input::new()
            .with_prompt("How many lines? (2-3)")
            .interact_text()
            .ok()
            .filter(|&l| l == 2 || l == 3)
            .unwrap_or(3);

        let sensors_per_line: u8 = Input::new()
            .with_prompt("How many sensors per line? (1-3)")
            .interact_text()?;

        if !(1..=3).contains(&sensors_per_line) {
            return settings_create_config(term, hwinfo);
        }

        conf.with_section(Some("Main"))
            .set("sensors_per_line", sensors_per_line.to_string());

        configure_custom_sensors(hwinfo, &mut conf, lines, sensors_per_line)?;
    }

    conf.write_to_file("conf.ini")?;
    term.write_line("config created.")?;

    Ok(conf)
}
