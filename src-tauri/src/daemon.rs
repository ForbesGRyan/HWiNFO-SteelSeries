use crate::connect::{connect_hid, connect_hwinfo, connect_steelseries};
use crate::consts::{DISPLAY_LINES, TICK_RATE, CUSTOM_SENSORS};
use crate::media::MediaReader;
use crate::mouse_battery::MouseBatteryReader;
use crate::render::{render_text_to_oled, OledBuffer};
use crate::settings::AppConfig;
use crate::state::{ActiveMode, SensorValue, Shared, SleepCommand};
use crate::steelseries::page_handler;
use crate::utils::{format_custom_value, run_sensors};
use anyhow::anyhow;
use console::Term;
use gamesense::client::GameSenseClient;
use hwinfo_steelseries_oled::Hwinfo;
use ini::Ini;
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use std::num::Wrapping;
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

#[derive(Debug)]
struct SummarySensors {
    cpu_temp: f64,
    cpu_usage: f64,
    gpu_temp: f64,
    gpu_usage: f64,
    mem_used: f64,
    mem_free: f64,
    mem_load: f64,
}

fn fetch_summary_sensors(hwinfo: &Hwinfo, gpu_name: &str) -> Result<SummarySensors, anyhow::Error> {
    let sensor_cpu_usage = hwinfo.find_first("Total CPU Usage")?;
    let sensor_cpu_temp = hwinfo.find_first("CPU (Tctl/Tdie)")?;
    let sensor_gpu_usage = hwinfo.find_first("GPU Core Load")?;

    let sensor_gpu_temp = if gpu_name.is_empty() {
        hwinfo.find_first("GPU Temperature")?
    } else {
        hwinfo
            .get(gpu_name, "GPU Temperature")
            .ok_or_else(|| anyhow!("GPU Temperature not found"))?
    };

    let sensor_mem_used = hwinfo.find_first("Physical Memory Used")?;
    let sensor_mem_free = hwinfo.find_first("Physical Memory Available")?;
    let sensor_mem_load = hwinfo.find_first("Physical Memory Load")?;

    Ok(SummarySensors {
        cpu_temp: sensor_cpu_temp.value,
        cpu_usage: sensor_cpu_usage.value,
        gpu_temp: sensor_gpu_temp.value,
        gpu_usage: sensor_gpu_usage.value,
        mem_used: sensor_mem_used.value / 1024.0,
        mem_free: sensor_mem_free.value / 1024.0,
        mem_load: sensor_mem_load.value,
    })
}

fn format_vertical_summary(s: &SummarySensors, decimal: bool) -> Value {
    let precision = if decimal { 1 } else { 0 };
    let spacing = if decimal { " " } else { "   " };
    let spacing2 = if decimal { " " } else { "    " };
    json!({
        "line1": "CPU   GPU   MEM",
        "line2": format!("{:.p$}°{}{:.p$}°{}{:.p$}G",
            s.cpu_temp, spacing, s.gpu_temp, spacing, s.mem_used, p = precision),
        "line3": format!("{:.p$}%{}{:.p$}%{}{:.p$}G",
            s.cpu_usage, spacing2, s.gpu_usage, spacing2, s.mem_free, p = precision),
    })
}

fn format_horizontal_summary(s: &SummarySensors, decimal: bool) -> Value {
    let precision = if decimal { 1 } else { 0 };
    json!({
        "line1": format!("CPU {:.p$}° {:.p$}%", s.cpu_temp, s.cpu_usage, p = precision),
        "line2": format!("GPU {:.p$}° {:.p$}%", s.gpu_temp, s.gpu_usage, p = precision),
        "line3": format!("MEM {:.p$}G {:.p$}%", s.mem_used, s.mem_load, p = precision),
    })
}

fn value_to_oled_buffer(value: &Value) -> OledBuffer {
    let mut text = String::new();
    if let Some(obj) = value.as_object() {
        for i in 1..=DISPLAY_LINES {
            let key = format!("line{}", i);
            if let Some(s) = obj.get(&key).and_then(|v| v.as_str()) {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(s);
            }
        }
    }
    render_text_to_oled(&text, 0)
}

fn value_to_sensor_values(value: &Value) -> Vec<SensorValue> {
    let mut out = Vec::new();
    if let Some(obj) = value.as_object() {
        for i in 1..=DISPLAY_LINES {
            let key = format!("line{}", i);
            if let Some(s) = obj.get(&key).and_then(|v| v.as_str()) {
                out.push(SensorValue {
                    label: format!("Line {}", i),
                    value: s.to_string(),
                });
            }
        }
    }
    out
}

fn buffer_to_rgba_grayscale(buf: &OledBuffer) -> Vec<u8> {
    let mut pixels = Vec::with_capacity(128 * 64);
    for y in 0..64u32 {
        for x in 0..128u32 {
            let col = x as usize;
            let byte_row = (y / 8) as usize;
            let bit = (y % 8) as u8;
            let idx = col * 8 + byte_row;
            let on = (buf.data[idx] & (1 << bit)) != 0;
            pixels.push(if on { 255 } else { 0 });
        }
    }
    pixels
}

enum OledClient {
    GameSense(GameSenseClient),
    Hid(hidapi::HidDevice),
}

impl OledClient {
    fn trigger_frame(&mut self, event: &str, i: isize, value: &Value, buffer: &OledBuffer) -> Result<(), anyhow::Error> {
        match self {
            OledClient::GameSense(client) => {
                client.trigger_event_frame(event, i, value.clone())?;
            }
            OledClient::Hid(device) => {
                let screen_height: u8 = 64;
                let chunk_width: u8 = 64;
                for chunk_x in [0u8, 64u8] {
                    let chunk_bitmap = buffer.get_chunk(chunk_x, chunk_width);
                    let mut packet = vec![0x06u8, 0x93, chunk_x, 0, chunk_width, screen_height];
                    packet.extend_from_slice(&chunk_bitmap);
                    packet.resize(1024, 0);
                    if let Err(e) = device.send_feature_report(&packet) {
                        error!("Failed to send HID frame: {}", e);
                        return Err(anyhow!("HID send failed: {}", e));
                    }
                }
            }
        }
        Ok(())
    }

    fn send_blank(&mut self) -> Result<(), anyhow::Error> {
        match self {
            OledClient::GameSense(client) => {
                let v = json!({ "line1": "", "line2": "", "line3": "" });
                client.trigger_event_frame("BLANK", 0, v)?;
            }
            OledClient::Hid(device) => {
                let buffer = OledBuffer::new();
                for chunk_x in [0u8, 64u8] {
                    let chunk_bitmap = buffer.get_chunk(chunk_x, 64);
                    let mut packet = vec![0x06u8, 0x93, chunk_x, 0, 64, 64];
                    packet.extend_from_slice(&chunk_bitmap);
                    packet.resize(1024, 0);
                    let _ = device.send_feature_report(&packet);
                }
            }
        }
        Ok(())
    }

    fn send_white(&mut self) -> Result<(), anyhow::Error> {
        match self {
            OledClient::GameSense(client) => {
                let v = json!({
                    "line1": "████████████████",
                    "line2": "████████████████",
                    "line3": "████████████████"
                });
                client.trigger_event_frame("WHITE", 0, v)?;
            }
            OledClient::Hid(device) => {
                let mut buffer = OledBuffer::new();
                for x in 0..128 { for y in 0..64 { buffer.set_pixel(x, y, true); } }
                for chunk_x in [0u8, 64u8] {
                    let chunk_bitmap = buffer.get_chunk(chunk_x, 64);
                    let mut packet = vec![0x06u8, 0x93, chunk_x, 0, 64, 64];
                    packet.extend_from_slice(&chunk_bitmap);
                    packet.resize(1024, 0);
                    let _ = device.send_feature_report(&packet);
                }
            }
        }
        Ok(())
    }

    pub fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
        if let OledClient::GameSense(client) = self {
            client.stop_heartbeat()?;
        }
        Ok(())
    }
}

fn check_disconnect(old: &Hwinfo, new: &Hwinfo, count: &mut usize, limit: usize) -> bool {
    if old == new {
        *count = (*count + 1).min(limit);
    } else {
        *count = 0;
    }
    *count >= limit
}

fn next_page_counter(i: isize, page_time: isize, pages: usize, current: usize) -> usize {
    if pages == 0 || page_time <= 0 {
        return current;
    }
    let ticks_per_second = 1000 / TICK_RATE as isize;
    let interval = page_time * ticks_per_second;
    if interval > 0 && i != 0 && i % interval == 0 {
        (current + 1) % pages
    } else {
        current
    }
}

fn disconnected_value() -> Value {
    json!({ "line1": "Disconnected", "line2": "FROM", "line3": "HWiNFO" })
}

fn build_display_value(
    config: &AppConfig,
    hwinfo: &Hwinfo,
    pages_vec: &[ini::Properties],
    page_counter: usize,
    mouse: &mut MouseBatteryReader,
    media: &mut MediaReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<Value, anyhow::Error> {
    if config.is_summary {
        let sensors = fetch_summary_sensors(hwinfo, &config.gpu)?;
        if config.is_vertical {
            Ok(format_vertical_summary(&sensors, config.decimal))
        } else {
            Ok(format_horizontal_summary(&sensors, config.decimal))
        }
    } else {
        let pages_sensors = pages_vec
            .get(page_counter)
            .ok_or_else(|| anyhow!("Page {} missing", page_counter))?;

        let mut labels = vec![""; CUSTOM_SENSORS];
        let mut units = vec![""; CUSTOM_SENSORS];
        let mut values = vec![String::new(); CUSTOM_SENSORS];

        run_sensors(
            pages_sensors,
            &mut labels,
            &mut units,
            &mut values,
            hwinfo,
            config.decimal,
            mouse,
            media,
            hid_api,
        )?;

        Ok(format_custom_value(config.sensors_per_line, labels, values, units))
    }
}

struct Daemon {
    state: Shared,
    app: AppHandle,
    term: Term,

    hwinfo: Option<Hwinfo>,
    oled: Option<OledClient>,
    hid_api: Option<hidapi::HidApi>,

    pages_vec: Vec<ini::Properties>,
    config: AppConfig,

    i: Wrapping<isize>,
    disconnect_count: usize,
    page_counter: usize,

    mouse_battery_reader: MouseBatteryReader,
    media_reader: MediaReader,

    is_sleeping: bool,
    is_white_screen: bool,
}

impl Daemon {
    fn new(state: Shared, app: AppHandle, config: AppConfig) -> Self {
        Self {
            state,
            app,
            term: Term::stdout(),
            hwinfo: None,
            oled: None,
            hid_api: None,
            pages_vec: Vec::new(),
            config,
            i: Wrapping(0),
            disconnect_count: 0,
            page_counter: 0,
            mouse_battery_reader: MouseBatteryReader::new(),
            media_reader: MediaReader::new(),
            is_sleeping: false,
            is_white_screen: false,
        }
    }

    fn write_state<F: FnOnce(&mut crate::state::SharedState)>(&self, f: F) {
        if let Ok(mut guard) = self.state.lock() {
            f(&mut *guard);
        }
    }

    fn push_status(&self) {
        let payload = match self.state.lock() {
            Ok(g) => g.status_payload(),
            Err(_) => return,
        };
        let _ = self.app.emit("status", payload);
    }

    fn push_frame(&self, buf: &OledBuffer) {
        let pixels = buffer_to_rgba_grayscale(buf);
        let _ = self.app.emit("frame", pixels);
    }

    fn connect_all(&mut self) -> Result<(), anyhow::Error> {
        self.write_state(|s| {
            s.last_error = Some("Connecting to HWiNFO...".to_string());
        });
        self.push_status();

        let mut hwinfo = connect_hwinfo(&self.term)?;
        hwinfo.pull()?;
        self.hwinfo = Some(hwinfo);

        self.write_state(|s| {
            s.hwinfo_connected = true;
            s.last_error = None;
        });
        self.push_status();

        if let Err(e) = self.media_reader.initialize() {
            warn!("MediaReader init failed: {}. Media sensors unavailable.", e);
        }

        self.pages_vec.clear();
        if !self.config.is_summary {
            let ini = Ini::load_from_file("conf.ini").unwrap_or_else(|_| Ini::new());
            for i in 1..=self.config.pages {
                if let Some(page) = ini.section(Some(format!("PAGE{}.Sensors", i))) {
                    self.pages_vec.push(page.clone());
                }
            }
        }

        if self.config.direct_usb {
            self.write_state(|s| {
                s.active_mode = ActiveMode::DirectUsb;
                s.last_error = Some("Connecting to SteelSeries HID...".to_string());
            });
            self.push_status();

            let api = hidapi::HidApi::new()
                .map_err(|e| anyhow!("HID API init failed: {}", e))?;
            let device = connect_hid(&self.term, &api, &self.config.direct_usb_serial)?;
            self.oled = Some(OledClient::Hid(device));
            self.hid_api = Some(api);

            self.write_state(|s| {
                s.usb_connected = true;
                s.gg_connected = false;
                s.last_error = None;
            });
        } else {
            self.write_state(|s| {
                s.active_mode = ActiveMode::GameSense;
                s.last_error = Some("Connecting to SteelSeries GG...".to_string());
            });
            self.push_status();

            let mut gg = connect_steelseries(&self.term)?;
            for i in 1..=self.config.pages {
                let line_keys: Vec<String> = (1..=DISPLAY_LINES).map(|j| format!("line{}", j)).collect();
                let line_refs: Vec<&str> = line_keys.iter().map(|s| s.as_str()).collect();
                let handler = page_handler(3, &line_refs, None);
                gg.bind_event(&format!("PAGE{}", i), None, None, None, None, vec![handler])?;
            }
            gg.start_heartbeat();
            self.oled = Some(OledClient::GameSense(gg));

            self.write_state(|s| {
                s.gg_connected = true;
                s.usb_connected = false;
                s.last_error = None;
            });
        }
        self.push_status();
        Ok(())
    }

    fn reload(&mut self) -> Result<(), anyhow::Error> {
        info!("Daemon: reloading config");
        let ini = Ini::load_from_file("conf.ini").map_err(|e| anyhow!("Load conf.ini failed: {}", e))?;
        let new_config = AppConfig::from_ini(&ini)?;
        self.config = new_config.clone();

        // Tear down existing OLED connection
        if let Some(mut oled) = self.oled.take() {
            let _ = oled.stop_heartbeat();
        }
        self.hid_api = None;

        self.write_state(|s| {
            s.config = new_config;
            s.gg_connected = false;
            s.usb_connected = false;
            s.active_mode = ActiveMode::Disconnected;
        });

        self.connect_all()?;
        Ok(())
    }

    fn handle_sleep_command(&mut self, cmd: SleepCommand) {
        match cmd {
            SleepCommand::Sleep => {
                self.is_sleeping = true;
                self.is_white_screen = false;
                if let Some(o) = self.oled.as_mut() {
                    if let Err(e) = o.send_blank() {
                        error!("Send blank failed: {}", e);
                    }
                }
            }
            SleepCommand::White => {
                self.is_sleeping = false;
                self.is_white_screen = true;
                if let Some(o) = self.oled.as_mut() {
                    if let Err(e) = o.send_white() {
                        error!("Send white failed: {}", e);
                    }
                }
            }
            SleepCommand::Wake => {
                self.is_sleeping = false;
                self.is_white_screen = false;
            }
        }
    }

    fn tick(&mut self) -> Result<(), anyhow::Error> {
        // Drain control flags
        let (reload, sleep_cmd) = {
            let mut g = self.state.lock().map_err(|e| anyhow!("State poisoned: {}", e))?;
            let r = g.reload_requested;
            let s = g.sleep_requested.take();
            g.reload_requested = false;
            (r, s)
        };

        if reload {
            if let Err(e) = self.reload() {
                error!("Reload failed: {}", e);
                self.write_state(|s| { s.last_error = Some(format!("Reload failed: {}", e)); });
                self.push_status();
            }
        }

        if let Some(cmd) = sleep_cmd {
            self.handle_sleep_command(cmd);
        }

        if self.is_sleeping || self.is_white_screen {
            return Ok(());
        }

        let hwinfo = self.hwinfo.as_mut().ok_or_else(|| anyhow!("hwinfo missing"))?;
        let oled = self.oled.as_mut().ok_or_else(|| anyhow!("oled missing"))?;

        let old = hwinfo.clone();
        hwinfo.pull()?;
        let disconnected = check_disconnect(&old, hwinfo, &mut self.disconnect_count, 5);
        drop(old);

        let snapshot = hwinfo.clone();
        if let Ok(mut g) = self.state.lock() {
            g.hwinfo_snapshot = Some(snapshot);
        }

        if disconnected {
            warn!("HWiNFO disconnected");
            let value = disconnected_value();
            let buffer = value_to_oled_buffer(&value);
            let _ = oled.trigger_frame("ERROR", self.i.0, &value, &buffer);

            self.write_state(|s| {
                s.hwinfo_connected = false;
                s.oled_buffer = OledBuffer { data: buffer.data };
                s.sensor_values = value_to_sensor_values(&value);
                s.last_error = Some("HWiNFO disconnected".to_string());
            });
            self.push_status();
            self.push_frame(&buffer);
            self.i += 1;
            return Ok(());
        }

        // Advance page counter (custom mode only)
        if !self.config.is_summary {
            let next = next_page_counter(
                self.i.0,
                self.config.page_time,
                self.config.pages,
                self.page_counter,
            );
            if next != self.page_counter {
                self.page_counter = next;
                debug!("Switching to page {}", self.page_counter + 1);
            }
        }

        let value = build_display_value(
            &self.config,
            hwinfo,
            &self.pages_vec,
            self.page_counter,
            &mut self.mouse_battery_reader,
            &mut self.media_reader,
            self.hid_api.as_ref(),
        )?;

        let buffer = value_to_oled_buffer(&value);
        let event_name = format!("PAGE{}", self.page_counter + 1);
        oled.trigger_frame(&event_name, self.i.0, &value, &buffer)?;

        self.write_state(|s| {
            s.hwinfo_connected = true;
            s.oled_buffer = OledBuffer { data: buffer.data };
            s.sensor_values = value_to_sensor_values(&value);
            s.last_error = None;
        });
        self.push_status();
        self.push_frame(&buffer);
        self.i += 1;
        Ok(())
    }

    fn run(&mut self) {
        loop {
            if let Err(e) = self.connect_all() {
                error!("Daemon connect failed: {}", e);
                self.write_state(|s| { s.last_error = Some(format!("Connect failed: {}", e)); });
                self.push_status();
                thread::sleep(Duration::from_secs(3));
                continue;
            }
            info!("Daemon connected, entering tick loop");

            loop {
                let start = std::time::Instant::now();
                if let Err(e) = self.tick() {
                    error!("Daemon tick error: {}", e);
                    self.write_state(|s| { s.last_error = Some(format!("Tick error: {}", e)); });
                    self.push_status();
                    break; // exit inner loop, reconnect
                }
                let elapsed = start.elapsed();
                let target = Duration::from_millis(TICK_RATE);
                if elapsed < target {
                    thread::sleep(target - elapsed);
                }
            }

            // Reconnect: drop everything
            if let Some(mut oled) = self.oled.take() {
                let _ = oled.stop_heartbeat();
            }
            self.hwinfo = None;
            self.hid_api = None;
            self.write_state(|s| {
                s.hwinfo_connected = false;
                s.gg_connected = false;
                s.usb_connected = false;
                s.active_mode = ActiveMode::Disconnected;
            });
            self.push_status();
            thread::sleep(Duration::from_secs(2));
        }
    }
}

pub fn spawn(state: Shared, app: AppHandle, config: AppConfig) {
    thread::spawn(move || {
        let mut d = Daemon::new(state, app, config);
        d.run();
    });
}

// Ensure Arc usage compiles regardless of Mutex location elsewhere
#[allow(dead_code)]
fn _arc_assert(_: Arc<()>) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::AppConfig;
    use hwinfo_steelseries_oled::{
        HwinfoSensorsReadingElement, HwinfoSensorsSensorElement, Sensor,
    };
    use std::collections::HashMap;

    fn sample_summary() -> SummarySensors {
        SummarySensors {
            cpu_temp: 42.0,
            cpu_usage: 50.0,
            gpu_temp: 60.0,
            gpu_usage: 70.0,
            mem_used: 8.0,
            mem_free: 8.0,
            mem_load: 50.0,
        }
    }

    fn build_hwinfo(entries: &[(&str, &str, f64)]) -> Hwinfo {
        let mut sensors: HashMap<String, Sensor> = HashMap::new();
        let mut sensor_names: Vec<String> = Vec::new();
        for (sk, rk, val) in entries {
            let entry = sensors.entry(sk.to_string()).or_insert_with(|| {
                sensor_names.push(sk.to_string());
                Sensor {
                    info: HwinfoSensorsSensorElement::new_mock(0, sk),
                    readings: HashMap::new(),
                    reading_names: Vec::new(),
                }
            });
            entry.readings.insert(
                rk.to_string(),
                HwinfoSensorsReadingElement::new_mock(0, 0, rk, *val),
            );
            entry.reading_names.push(rk.to_string());
        }
        Hwinfo::new_mock(sensors, sensor_names)
    }

    fn full_summary_hwinfo() -> Hwinfo {
        build_hwinfo(&[
            ("CPU", "Total CPU Usage", 50.0),
            ("CPU", "CPU (Tctl/Tdie)", 42.0),
            ("GPU", "GPU Core Load", 70.0),
            ("GPU", "GPU Temperature", 60.0),
            ("MEM", "Physical Memory Used", 8192.0),
            ("MEM", "Physical Memory Available", 8192.0),
            ("MEM", "Physical Memory Load", 50.0),
        ])
    }

    fn base_config() -> AppConfig {
        AppConfig {
            is_summary: true,
            is_vertical: true,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            direct_usb_serial: String::new(),
            custom_sensors: Vec::new(),
        }
    }

    #[test]
    fn test_next_page_counter_advances_at_interval() {
        // TICK_RATE = 500 → ticks_per_second = 2 → interval = page_time * 2
        // page_time=5 → interval=10
        assert_eq!(next_page_counter(10, 5, 3, 0), 1);
        assert_eq!(next_page_counter(20, 5, 3, 1), 2);
        assert_eq!(next_page_counter(30, 5, 3, 2), 0); // wraps
    }

    #[test]
    fn test_next_page_counter_keeps_when_not_at_boundary() {
        assert_eq!(next_page_counter(5, 5, 3, 0), 0);
        assert_eq!(next_page_counter(1, 5, 3, 1), 1);
    }

    #[test]
    fn test_next_page_counter_zero_tick_does_not_advance() {
        assert_eq!(next_page_counter(0, 5, 3, 0), 0);
    }

    #[test]
    fn test_next_page_counter_handles_zero_pages_or_page_time() {
        assert_eq!(next_page_counter(10, 5, 0, 0), 0);
        assert_eq!(next_page_counter(10, 0, 3, 1), 1);
        assert_eq!(next_page_counter(10, -1, 3, 1), 1);
    }

    #[test]
    fn test_disconnected_value_has_three_lines() {
        let v = disconnected_value();
        assert_eq!(v["line1"], "Disconnected");
        assert_eq!(v["line2"], "FROM");
        assert_eq!(v["line3"], "HWiNFO");
    }

    #[test]
    fn test_fetch_summary_sensors_uses_find_first_when_gpu_empty() {
        let hw = full_summary_hwinfo();
        let s = fetch_summary_sensors(&hw, "").unwrap();
        assert_eq!(s.cpu_temp, 42.0);
        assert_eq!(s.gpu_temp, 60.0);
        assert_eq!(s.mem_used, 8.0); // 8192/1024
    }

    #[test]
    fn test_fetch_summary_sensors_uses_named_gpu() {
        let hw = full_summary_hwinfo();
        let s = fetch_summary_sensors(&hw, "GPU").unwrap();
        assert_eq!(s.gpu_temp, 60.0);
    }

    #[test]
    fn test_fetch_summary_sensors_named_gpu_missing_errors() {
        let hw = full_summary_hwinfo();
        let err = fetch_summary_sensors(&hw, "NoSuchGpu").unwrap_err();
        assert!(format!("{}", err).contains("GPU Temperature not found"));
    }

    #[test]
    fn test_fetch_summary_sensors_missing_cpu_temp_errors() {
        let hw = build_hwinfo(&[("CPU", "Total CPU Usage", 50.0)]);
        assert!(fetch_summary_sensors(&hw, "").is_err());
    }

    #[test]
    fn test_format_vertical_summary_no_decimal() {
        let v = format_vertical_summary(&sample_summary(), false);
        assert_eq!(v["line1"], "CPU   GPU   MEM");
        assert_eq!(v["line2"], "42°   60°   8G");
        assert_eq!(v["line3"], "50%    70%    8G");
    }

    #[test]
    fn test_format_vertical_summary_decimal() {
        let v = format_vertical_summary(&sample_summary(), true);
        assert_eq!(v["line2"], "42.0° 60.0° 8.0G");
        assert_eq!(v["line3"], "50.0% 70.0% 8.0G");
    }

    #[test]
    fn test_format_horizontal_summary_no_decimal() {
        let v = format_horizontal_summary(&sample_summary(), false);
        assert_eq!(v["line1"], "CPU 42° 50%");
        assert_eq!(v["line2"], "GPU 60° 70%");
        assert_eq!(v["line3"], "MEM 8G 50%");
    }

    #[test]
    fn test_format_horizontal_summary_decimal() {
        let v = format_horizontal_summary(&sample_summary(), true);
        assert_eq!(v["line1"], "CPU 42.0° 50.0%");
    }

    #[test]
    fn test_build_display_value_summary_vertical() {
        let cfg = base_config();
        let hw = full_summary_hwinfo();
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();

        let v = build_display_value(&cfg, &hw, &[], 0, &mut mouse, &mut media, None).unwrap();
        assert_eq!(v["line1"], "CPU   GPU   MEM");
    }

    #[test]
    fn test_build_display_value_summary_horizontal() {
        let mut cfg = base_config();
        cfg.is_vertical = false;
        let hw = full_summary_hwinfo();
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();

        let v = build_display_value(&cfg, &hw, &[], 0, &mut mouse, &mut media, None).unwrap();
        assert_eq!(v["line1"], "CPU 42° 50%");
    }

    #[test]
    fn test_build_display_value_custom_missing_page_errors() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        let hw = build_hwinfo(&[]);
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();

        let err = build_display_value(&cfg, &hw, &[], 0, &mut mouse, &mut media, None).unwrap_err();
        assert!(format!("{}", err).contains("Page 0 missing"));
    }

    #[test]
    fn test_build_display_value_custom_renders_blank_sensor() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.sensors_per_line = 1;
        let hw = build_hwinfo(&[]);
        let mut props = ini::Properties::new();
        props.insert("sensor_0", "BLANK");
        props.insert("label_0", "X");

        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();

        let v = build_display_value(&cfg, &hw, &[props], 0, &mut mouse, &mut media, None).unwrap();
        assert!(v["line1"].as_str().unwrap().contains("X"));
    }

    #[test]
    fn test_value_to_oled_buffer_populates_text() {
        let v = json!({ "line1": "Hi", "line2": "There", "line3": "" });
        let buf = value_to_oled_buffer(&v);
        // Some pixels should be lit
        assert!(buf.data.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_value_to_sensor_values_collects_lines() {
        let v = json!({ "line1": "A", "line2": "B", "line3": "C" });
        let svs = value_to_sensor_values(&v);
        assert_eq!(svs.len(), 3);
        assert_eq!(svs[0].label, "Line 1");
        assert_eq!(svs[0].value, "A");
        assert_eq!(svs[2].value, "C");
    }

    #[test]
    fn test_value_to_sensor_values_skips_missing_lines() {
        let v = json!({ "line1": "Only" });
        let svs = value_to_sensor_values(&v);
        assert_eq!(svs.len(), 1);
        assert_eq!(svs[0].value, "Only");
    }

    #[test]
    fn test_check_disconnect_increments_on_match() {
        let hw = build_hwinfo(&[("S", "R", 1.0)]);
        let mut count = 0;
        assert!(!check_disconnect(&hw, &hw, &mut count, 3));
        assert_eq!(count, 1);
        assert!(!check_disconnect(&hw, &hw, &mut count, 3));
        assert_eq!(count, 2);
        assert!(check_disconnect(&hw, &hw, &mut count, 3));
        assert_eq!(count, 3);
    }

    #[test]
    fn test_check_disconnect_resets_on_change() {
        let a = build_hwinfo(&[("S", "R", 1.0)]);
        let b = build_hwinfo(&[("S", "R", 2.0)]);
        let mut count = 2;
        assert!(!check_disconnect(&a, &b, &mut count, 3));
        assert_eq!(count, 0);
    }

    #[test]
    fn test_check_disconnect_caps_at_limit() {
        let hw = build_hwinfo(&[("S", "R", 1.0)]);
        let mut count = 5;
        check_disconnect(&hw, &hw, &mut count, 3);
        assert_eq!(count, 3); // capped to limit
    }

    #[test]
    fn test_buffer_to_rgba_grayscale_size_and_mapping() {
        let mut buf = OledBuffer::new();
        buf.set_pixel(0, 0, true);
        buf.set_pixel(127, 63, true);
        let px = buffer_to_rgba_grayscale(&buf);
        assert_eq!(px.len(), 128 * 64);
        assert_eq!(px[0], 255); // (0,0)
        assert_eq!(px[63 * 128 + 127], 255); // (127,63)
        assert_eq!(px[1], 0); // (1,0) unlit
    }
}
