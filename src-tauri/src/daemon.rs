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
            let device = connect_hid(&self.term, &api)?;
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
            let value = json!({ "line1": "Disconnected", "line2": "FROM", "line3": "HWiNFO" });
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

        // Build display value
        let value = if self.config.is_summary {
            let sensors = fetch_summary_sensors(hwinfo, &self.config.gpu)?;
            if self.config.is_vertical {
                format_vertical_summary(&sensors, self.config.decimal)
            } else {
                format_horizontal_summary(&sensors, self.config.decimal)
            }
        } else {
            let ticks_per_second = 1000 / TICK_RATE as isize;
            if self.i.0 % (self.config.page_time * ticks_per_second) == 0 && self.i.0 != 0 {
                self.page_counter = (self.page_counter + 1) % self.config.pages;
                debug!("Switching to page {}", self.page_counter + 1);
            }
            let pages_sensors = self.pages_vec.get(self.page_counter)
                .ok_or_else(|| anyhow!("Page {} missing", self.page_counter))?;

            let mut labels = vec![""; CUSTOM_SENSORS];
            let mut units = vec![""; CUSTOM_SENSORS];
            let mut values = vec![String::new(); CUSTOM_SENSORS];

            run_sensors(
                pages_sensors,
                &mut labels,
                &mut units,
                &mut values,
                hwinfo,
                self.config.decimal,
                &mut self.mouse_battery_reader,
                &mut self.media_reader,
                self.hid_api.as_ref(),
            )?;

            format_custom_value(self.config.sensors_per_line, labels, values, units)
        };

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
