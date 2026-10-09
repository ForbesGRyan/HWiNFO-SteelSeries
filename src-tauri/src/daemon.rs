use crate::connect::{connect_hid, connect_steelseries};
use crate::consts::{DISPLAY_LINES, TICK_RATE};
use crate::media::MediaReader;
use crate::mouse_battery::MouseBatteryReader;
use crate::render::{render_text_to_oled, FontSize, OledBuffer};
use crate::settings::AppConfig;
use crate::state::{ActiveMode, SensorValue, Shared, SleepCommand};
use crate::steelseries::page_handler;
use crate::utils::{format_custom_lines, run_sensors};
use anyhow::anyhow;
use gamesense::client::GameSenseClient;
use hwinfo_steelseries_oled::{Hwinfo, Snapshot};
use ini::Ini;
use log::{debug, error, info, warn};
use serde_json::Value;
use std::num::Wrapping;
use std::thread;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Runtime};

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

fn fetch_summary_sensors(
    hwinfo: &Snapshot,
    gpu_name: &str,
) -> Result<SummarySensors, anyhow::Error> {
    let first = |reading: &str| {
        hwinfo
            .find_first(reading)
            .ok_or_else(|| anyhow!("{} not found", reading))
    };
    let gpu_temp = if gpu_name.is_empty() {
        first("GPU Temperature")?
    } else {
        hwinfo
            .get(gpu_name, "GPU Temperature")
            .ok_or_else(|| anyhow!("GPU Temperature not found"))?
    };
    Ok(SummarySensors {
        cpu_temp: first("CPU (Tctl/Tdie)")?,
        cpu_usage: first("Total CPU Usage")?,
        gpu_temp,
        gpu_usage: first("GPU Core Load")?,
        mem_used: first("Physical Memory Used")? / 1024.0,
        mem_free: first("Physical Memory Available")? / 1024.0,
        mem_load: first("Physical Memory Load")?,
    })
}

fn format_vertical_summary(s: &SummarySensors, decimal: bool) -> Vec<String> {
    let precision = if decimal { 1 } else { 0 };
    let spacing = if decimal { " " } else { "   " };
    let spacing2 = if decimal { " " } else { "    " };
    vec![
        "CPU   GPU   MEM".to_string(),
        format!(
            "{:.p$}°{}{:.p$}°{}{:.p$}G",
            s.cpu_temp,
            spacing,
            s.gpu_temp,
            spacing,
            s.mem_used,
            p = precision
        ),
        format!(
            "{:.p$}%{}{:.p$}%{}{:.p$}G",
            s.cpu_usage,
            spacing2,
            s.gpu_usage,
            spacing2,
            s.mem_free,
            p = precision
        ),
    ]
}

fn format_horizontal_summary(s: &SummarySensors, decimal: bool) -> Vec<String> {
    let precision = if decimal { 1 } else { 0 };
    vec![
        format!(
            "CPU {:.p$}° {:.p$}%",
            s.cpu_temp,
            s.cpu_usage,
            p = precision
        ),
        format!(
            "GPU {:.p$}° {:.p$}%",
            s.gpu_temp,
            s.gpu_usage,
            p = precision
        ),
        format!("MEM {:.p$}G {:.p$}%", s.mem_used, s.mem_load, p = precision),
    ]
}

/// GameSense frame payload: `{"line1": ..., "line2": ...}`.
fn lines_to_frame(lines: &[String]) -> Value {
    Value::Object(
        lines
            .iter()
            .enumerate()
            .map(|(i, line)| (format!("line{}", i + 1), Value::String(line.clone())))
            .collect(),
    )
}

fn lines_to_oled_buffer(lines: &[String], font_sizes: &[FontSize], size: (u32, u32)) -> OledBuffer {
    render_text_to_oled(&lines.join("\n"), 0, font_sizes, size.0, size.1)
}

fn lines_to_sensor_values(lines: &[String]) -> Vec<SensorValue> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| SensorValue {
            label: format!("Line {}", i + 1),
            value: line.clone(),
        })
        .collect()
}

/// Pure helper: create an OledBuffer with every pixel turned on ("white" screen).
fn white_buffer(width: u32, height: u32) -> OledBuffer {
    let mut buffer = OledBuffer::new(width, height);
    for x in 0..width {
        for y in 0..height {
            buffer.set_pixel(x, y, true);
        }
    }
    buffer
}

/// Where frames go. Two implementations: SteelSeries GG (text lines) and
/// direct USB HID (bitmap).
trait DisplayDriver: Send {
    fn trigger_frame(
        &mut self,
        event: &str,
        i: isize,
        lines: &[String],
        buffer: &OledBuffer,
    ) -> Result<(), anyhow::Error>;
    fn send_blank(&mut self) -> Result<(), anyhow::Error>;
    fn send_white(&mut self) -> Result<(), anyhow::Error>;
    fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
        Ok(())
    }
}

struct GameSenseDisplay(GameSenseClient);

impl DisplayDriver for GameSenseDisplay {
    fn trigger_frame(
        &mut self,
        event: &str,
        i: isize,
        lines: &[String],
        _buffer: &OledBuffer,
    ) -> Result<(), anyhow::Error> {
        self.0
            .trigger_event_frame(event, i, lines_to_frame(lines))?;
        Ok(())
    }

    fn send_blank(&mut self) -> Result<(), anyhow::Error> {
        let blank = vec![String::new(); 3];
        self.0
            .trigger_event_frame("BLANK", 0, lines_to_frame(&blank))?;
        Ok(())
    }

    fn send_white(&mut self) -> Result<(), anyhow::Error> {
        let white = vec!["████████████████".to_string(); 3];
        self.0
            .trigger_event_frame("WHITE", 0, lines_to_frame(&white))?;
        Ok(())
    }

    fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
        self.0.stop_heartbeat()?;
        Ok(())
    }
}

struct HidDisplay {
    device: hidapi::HidDevice,
    model: &'static crate::devices::SupportedDevice,
}

impl HidDisplay {
    fn send(&self, buffer: &OledBuffer) -> Result<(), anyhow::Error> {
        for packet in self.model.protocol.build_packets(buffer) {
            self.device.send_feature_report(&packet).map_err(|e| {
                error!("Failed to send HID frame: {}", e);
                anyhow!("HID send failed: {}", e)
            })?;
        }
        Ok(())
    }
}

impl DisplayDriver for HidDisplay {
    fn trigger_frame(
        &mut self,
        _event: &str,
        _i: isize,
        _lines: &[String],
        buffer: &OledBuffer,
    ) -> Result<(), anyhow::Error> {
        self.send(buffer)
    }

    // Blank and white are best-effort: a failed send must not tear down the
    // connection when the user asks the screen to sleep.
    fn send_blank(&mut self) -> Result<(), anyhow::Error> {
        let _ = self.send(&OledBuffer::new(self.model.width, self.model.height));
        Ok(())
    }

    fn send_white(&mut self) -> Result<(), anyhow::Error> {
        let _ = self.send(&white_buffer(self.model.width, self.model.height));
        Ok(())
    }
}

/// Ticks without new HWiNFO data before the display shows "Disconnected".
const STALE_LIMIT: usize = 5;

/// Count consecutive pulls without new HWiNFO data; true once `limit` is reached.
fn is_stale(fresh: bool, count: &mut usize, limit: usize) -> bool {
    *count = if fresh { 0 } else { (*count + 1).min(limit) };
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

fn build_display_lines(
    config: &AppConfig,
    hwinfo: &Snapshot,
    page_counter: usize,
    mouse: &mut MouseBatteryReader,
    media: &mut MediaReader,
    weather: &crate::weather::WeatherReader,
    hid_api: Option<&hidapi::HidApi>,
) -> Result<Vec<String>, anyhow::Error> {
    if config.is_summary {
        let sensors = fetch_summary_sensors(hwinfo, &config.gpu)?;
        return Ok(if config.is_vertical {
            format_vertical_summary(&sensors, config.decimal)
        } else {
            format_horizontal_summary(&sensors, config.decimal)
        });
    }
    let page = config
        .custom_sensors
        .get(page_counter)
        .ok_or_else(|| anyhow!("Page {} missing", page_counter))?;
    let slots = run_sensors(page, config.decimal, hwinfo, mouse, media, weather, hid_api)?;
    Ok(format_custom_lines(config.sensors_per_line, &slots))
}

struct Daemon<R: Runtime = tauri::Wry> {
    state: Shared,
    app: AppHandle<R>,

    hwinfo: Option<Hwinfo>,
    oled: Option<Box<dyn DisplayDriver>>,
    hid_api: Option<hidapi::HidApi>,

    config: AppConfig,

    i: Wrapping<isize>,
    disconnect_count: usize,
    page_counter: usize,

    /// Active OLED dimensions: registry entry's size in direct-USB mode,
    /// 128×64 for GameSense.
    display_size: (u32, u32),

    mouse_battery_reader: MouseBatteryReader,
    media_reader: MediaReader,
    weather_reader: crate::weather::WeatherReader,

    /// True while the display is asleep or showing the white screen; ticks
    /// draw nothing until a Wake command.
    paused: bool,
}

impl<R: Runtime> Daemon<R> {
    fn new(state: Shared, app: AppHandle<R>, config: AppConfig) -> Self {
        let weather_reader = crate::weather::WeatherReader::spawn(&config.weather);
        Self {
            state,
            app,
            hwinfo: None,
            oled: None,
            hid_api: None,
            config,
            i: Wrapping(0),
            disconnect_count: 0,
            page_counter: 0,
            display_size: (128, 64),
            mouse_battery_reader: MouseBatteryReader::new(),
            media_reader: MediaReader::new(),
            weather_reader,
            paused: false,
        }
    }

    fn write_state<F: FnOnce(&mut crate::state::SharedState)>(&self, f: F) {
        if let Ok(mut guard) = self.state.lock() {
            f(&mut guard);
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
        let _ = self.app.emit("frame", crate::gui::buffer_to_frame(buf));
    }

    /// Show a "Connecting to ..." message in the settings window.
    fn announce(&self, mode: Option<ActiveMode>, message: &str) {
        self.write_state(|s| {
            if let Some(mode) = mode {
                s.active_mode = mode;
            }
            s.last_error = Some(message.to_string());
        });
        self.push_status();
    }

    /// Log a failure and surface it in the settings window.
    fn record_failure(&self, what: &str, err: &anyhow::Error) {
        error!("Daemon: {}: {}", what, err);
        self.write_state(|s| {
            s.last_error = Some(format!("{}: {}", what, err));
        });
        self.push_status();
    }

    fn connect_all(&mut self) -> Result<(), anyhow::Error> {
        self.announce(None, "Connecting to HWiNFO...");
        self.hwinfo = Some(Hwinfo::new().map_err(|e| {
            anyhow!(
                "HWiNFO not reachable ({}). Is it running with Shared Memory Support enabled?",
                e
            )
        })?);
        self.write_state(|s| {
            s.hwinfo_connected = true;
            s.last_error = None;
        });
        self.push_status();

        if let Err(e) = self.media_reader.initialize() {
            warn!("MediaReader init failed: {}. Media sensors unavailable.", e);
        }

        if self.config.direct_usb {
            self.announce(
                Some(ActiveMode::DirectUsb),
                "Connecting to SteelSeries HID...",
            );

            let api = hidapi::HidApi::new().map_err(|e| anyhow!("HID API init failed: {}", e))?;
            let (device, supported) = connect_hid(&api, &self.config.direct_usb_serial)?;
            info!(
                "Direct USB device: {} ({}x{})",
                supported.name, supported.width, supported.height
            );
            self.set_display_size((supported.width, supported.height));
            self.oled = Some(Box::new(HidDisplay {
                device,
                model: supported,
            }));
            self.hid_api = Some(api);
        } else {
            self.set_display_size((128, 64));
            self.announce(
                Some(ActiveMode::GameSense),
                "Connecting to SteelSeries GG...",
            );

            let mut gg = connect_steelseries()?;
            let line_keys: Vec<String> =
                (1..=DISPLAY_LINES).map(|j| format!("line{}", j)).collect();
            let line_refs: Vec<&str> = line_keys.iter().map(|s| s.as_str()).collect();
            for i in 1..=self.config.pages {
                let handler = page_handler(3, &line_refs, None);
                gg.bind_event(&format!("PAGE{}", i), None, None, None, None, vec![handler])?;
            }
            gg.start_heartbeat();
            self.oled = Some(Box::new(GameSenseDisplay(gg)));
        }

        let direct_usb = self.config.direct_usb;
        self.write_state(|s| {
            s.usb_connected = direct_usb;
            s.gg_connected = !direct_usb;
            s.last_error = None;
        });
        self.push_status();
        Ok(())
    }

    fn set_display_size(&mut self, size: (u32, u32)) {
        self.display_size = size;
        self.write_state(|s| {
            s.display_size = size;
            s.oled_buffer = OledBuffer::new(size.0, size.1);
        });
    }

    /// Re-read conf.ini into the daemon and shared state, restarting the
    /// weather reader so [Weather] changes take effect.
    fn load_config(&mut self) -> Result<(), anyhow::Error> {
        let ini =
            Ini::load_from_file("conf.ini").map_err(|e| anyhow!("Load conf.ini failed: {}", e))?;
        self.config = AppConfig::from_ini(&ini)?;

        self.weather_reader.stop();
        self.weather_reader = crate::weather::WeatherReader::spawn(&self.config.weather);

        let config = self.config.clone();
        self.write_state(|s| s.config = config);
        Ok(())
    }

    /// Whether a config reload was requested; clears the request.
    fn take_reload_request(&self) -> bool {
        self.state
            .lock()
            .map(|mut g| std::mem::take(&mut g.reload_requested))
            .unwrap_or(false)
    }

    fn reload(&mut self) -> Result<(), anyhow::Error> {
        info!("Daemon: reloading config");
        self.load_config()?;

        // Tear down the existing OLED connection, then reconnect with the new config.
        if let Some(mut oled) = self.oled.take() {
            let _ = oled.stop_heartbeat();
        }
        self.hid_api = None;
        self.write_state(|s| {
            s.gg_connected = false;
            s.usb_connected = false;
            s.active_mode = ActiveMode::Disconnected;
        });

        self.connect_all()
    }

    fn handle_sleep_command(&mut self, cmd: SleepCommand) {
        self.paused = cmd != SleepCommand::Wake;
        let Some(oled) = self.oled.as_mut() else {
            return;
        };
        let result = match cmd {
            SleepCommand::Sleep => oled.send_blank(),
            SleepCommand::White => oled.send_white(),
            SleepCommand::Wake => Ok(()),
        };
        if let Err(e) = result {
            error!("{:?} command failed: {}", cmd, e);
        }
    }

    fn tick(&mut self) -> Result<(), anyhow::Error> {
        // Drain control flags
        let (reload, sleep_cmd) = {
            let mut g = self
                .state
                .lock()
                .map_err(|e| anyhow!("State poisoned: {}", e))?;
            let r = g.reload_requested;
            let s = g.sleep_requested.take();
            g.reload_requested = false;
            (r, s)
        };

        if reload {
            if let Err(e) = self.reload() {
                self.record_failure("Reload failed", &e);
            }
        }

        if let Some(cmd) = sleep_cmd {
            self.handle_sleep_command(cmd);
        }

        if self.paused {
            return Ok(());
        }

        let mut hwinfo = self
            .hwinfo
            .take()
            .ok_or_else(|| anyhow!("hwinfo missing"))?;
        let fresh = hwinfo.pull()?;
        let result = self.render(hwinfo.snapshot(), fresh);
        // A stale view never recovers on its own: after HWiNFO restarts, the
        // old mapping stays frozen. Remap, keeping the old view if HWiNFO is
        // still gone.
        self.hwinfo = Some(if self.disconnect_count >= STALE_LIMIT {
            Hwinfo::new().unwrap_or(hwinfo)
        } else {
            hwinfo
        });
        result
    }

    /// Draw one frame from `hwinfo`. `fresh` is false when HWiNFO has not
    /// polled since the previous tick.
    fn render(&mut self, hwinfo: &Snapshot, fresh: bool) -> Result<(), anyhow::Error> {
        let oled = self.oled.as_mut().ok_or_else(|| anyhow!("oled missing"))?;
        let disconnected = is_stale(fresh, &mut self.disconnect_count, STALE_LIMIT);

        if let Ok(mut g) = self.state.lock() {
            g.hwinfo_snapshot = Some(hwinfo.clone());
        }

        if disconnected {
            warn!("HWiNFO disconnected");
            // Only the PAGE events are bound to a screen handler in GG mode, so
            // the message goes out on the current page's event.
            let event_name = format!("PAGE{}", self.page_counter + 1);
            let lines = ["Disconnected", "FROM", "HWiNFO"].map(String::from);
            let buffer = lines_to_oled_buffer(&lines, &self.config.font_sizes, self.display_size);
            let _ = oled.trigger_frame(&event_name, self.i.0, &lines, &buffer);

            self.write_state(|s| {
                s.hwinfo_connected = false;
                s.oled_buffer = buffer.clone();
                s.sensor_values = lines_to_sensor_values(&lines);
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

        let lines = build_display_lines(
            &self.config,
            hwinfo,
            self.page_counter,
            &mut self.mouse_battery_reader,
            &mut self.media_reader,
            &self.weather_reader,
            self.hid_api.as_ref(),
        )?;

        let buffer = lines_to_oled_buffer(&lines, &self.config.font_sizes, self.display_size);
        let event_name = format!("PAGE{}", self.page_counter + 1);
        oled.trigger_frame(&event_name, self.i.0, &lines, &buffer)?;

        // Publish live dynamic-sensor snapshots so the settings preview can render
        // MEDIA_*/WEATHER_* with the same data the OLED shows (the GUI preview has
        // no access to the daemon's live readers otherwise).
        let media_snapshot = self.media_reader.snapshot();
        let weather_snapshot = self.weather_reader.current_info();

        self.write_state(|s| {
            s.hwinfo_connected = true;
            s.oled_buffer = buffer.clone();
            s.sensor_values = lines_to_sensor_values(&lines);
            s.last_error = None;
            s.media_info = media_snapshot;
            s.weather_info = weather_snapshot;
        });
        self.push_status();
        self.push_frame(&buffer);
        self.i += 1;
        Ok(())
    }

    /// Tear down connections and reset state for reconnect.
    fn disconnect_and_cleanup(&mut self) {
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
    }

    fn run(&mut self) {
        loop {
            if self.take_reload_request() {
                if let Err(e) = self.load_config() {
                    self.record_failure("Connect failed", &e);
                }
            }
            if let Err(e) = self.connect_all() {
                self.record_failure("Connect failed", &e);
                thread::sleep(Duration::from_secs(3));
                continue;
            }
            info!("Daemon connected, entering tick loop");

            loop {
                let start = std::time::Instant::now();
                if let Err(e) = self.tick() {
                    self.record_failure("Tick error", &e);
                    break; // exit inner loop, reconnect
                }
                let elapsed = start.elapsed();
                let target = Duration::from_millis(TICK_RATE);
                if elapsed < target {
                    thread::sleep(target - elapsed);
                }
            }

            self.disconnect_and_cleanup();
            thread::sleep(Duration::from_secs(2));
        }
    }
}

pub fn spawn<R: Runtime>(state: Shared, app: AppHandle<R>, config: AppConfig) {
    thread::spawn(move || {
        let mut d = Daemon::new(state, app, config);
        d.run();
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AppConfig, CustomSensor, WeatherConfig};

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

    fn build_hwinfo(entries: &[(&str, &str, f64)]) -> Snapshot {
        Snapshot::from_readings(entries)
    }

    fn full_summary_hwinfo() -> Snapshot {
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
            weather: WeatherConfig::default(),
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
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
        assert_eq!(v[0], "CPU   GPU   MEM");
        assert_eq!(v[1], "42°   60°   8G");
        assert_eq!(v[2], "50%    70%    8G");
    }

    #[test]
    fn test_format_vertical_summary_decimal() {
        let v = format_vertical_summary(&sample_summary(), true);
        assert_eq!(v[1], "42.0° 60.0° 8.0G");
        assert_eq!(v[2], "50.0% 70.0% 8.0G");
    }

    #[test]
    fn test_format_horizontal_summary_no_decimal() {
        let v = format_horizontal_summary(&sample_summary(), false);
        assert_eq!(v[0], "CPU 42° 50%");
        assert_eq!(v[1], "GPU 60° 70%");
        assert_eq!(v[2], "MEM 8G 50%");
    }

    #[test]
    fn test_format_horizontal_summary_decimal() {
        let v = format_horizontal_summary(&sample_summary(), true);
        assert_eq!(v[0], "CPU 42.0° 50.0%");
    }

    fn lines_for(cfg: &AppConfig, hw: &Snapshot) -> Result<Vec<String>, anyhow::Error> {
        let mut mouse = MouseBatteryReader::new();
        let mut media = MediaReader::new();
        let weather = crate::weather::WeatherReader::disabled();
        build_display_lines(cfg, hw, 0, &mut mouse, &mut media, &weather, None)
    }

    #[test]
    fn test_build_display_lines_summary_vertical() {
        let lines = lines_for(&base_config(), &full_summary_hwinfo()).unwrap();
        assert_eq!(lines[0], "CPU   GPU   MEM");
        assert_eq!(lines.len(), 3);
    }

    #[test]
    fn test_build_display_lines_summary_horizontal() {
        let mut cfg = base_config();
        cfg.is_vertical = false;
        let lines = lines_for(&cfg, &full_summary_hwinfo()).unwrap();
        assert_eq!(lines[0], "CPU 42° 50%");
    }

    #[test]
    fn test_build_display_lines_custom_missing_page_errors() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        let err = lines_for(&cfg, &build_hwinfo(&[])).unwrap_err();
        assert!(format!("{}", err).contains("Page 0 missing"));
    }

    #[test]
    fn test_build_display_lines_custom_renders_configured_sensor() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![CustomSensor {
            sensor: "CPU;Temp".into(),
            label: "X".into(),
            unit: "°".into(),
            ..Default::default()
        }]];
        let lines = lines_for(&cfg, &build_hwinfo(&[("CPU", "Temp", 42.0)])).unwrap();
        assert_eq!(lines[0], "X 42°");
        assert_eq!(lines.len(), crate::consts::DISPLAY_LINES);
    }

    #[test]
    fn test_build_display_lines_custom_propagates_missing_sensor() {
        let mut cfg = base_config();
        cfg.is_summary = false;
        cfg.custom_sensors = vec![vec![CustomSensor {
            sensor: "Nope;Missing".into(),
            ..Default::default()
        }]];
        assert!(lines_for(&cfg, &build_hwinfo(&[])).is_err());
    }

    #[test]
    fn test_lines_to_frame_numbers_keys_from_one() {
        let frame = lines_to_frame(&["A".to_string(), String::new(), "C".to_string()]);
        assert_eq!(frame["line1"], "A");
        assert_eq!(frame["line2"], "");
        assert_eq!(frame["line3"], "C");
        assert!(frame.get("line4").is_none());
    }

    #[test]
    fn test_lines_to_oled_buffer_keeps_blank_line_positions() {
        let with_gap = lines_to_oled_buffer(&[String::new(), "CPU".to_string()], &[], (128, 64));
        let without_gap = lines_to_oled_buffer(&["CPU".to_string()], &[], (128, 64));
        assert!(with_gap.data.iter().any(|&b| b != 0));
        // A leading blank line pushes the text down: the buffers differ.
        assert_ne!(with_gap, without_gap);
    }

    #[test]
    fn test_lines_to_sensor_values_labels_each_line() {
        let values = lines_to_sensor_values(&["A".to_string(), "B".to_string()]);
        assert_eq!(values.len(), 2);
        assert_eq!(values[1].label, "Line 2");
        assert_eq!(values[1].value, "B");
    }

    // ==================== Daemon impl tests via Tauri mock_app ====================
    use crate::state::SharedState;
    use std::sync::{Arc, Mutex};
    use tauri::test::{mock_app, MockRuntime};
    use tauri::Manager;

    fn mock_app_handle() -> tauri::AppHandle<MockRuntime> {
        mock_app().app_handle().clone()
    }

    // Daemon is generic over Runtime via AppHandle. The actual struct uses tauri::AppHandle
    // (default runtime = wry::Wry). We can only construct Daemon::new with AppHandle<Wry>,
    // which mock_app() does NOT provide. So we test the pure helpers + structural pieces only.

    fn fresh_shared() -> Shared {
        Arc::new(Mutex::new(SharedState::new(base_config())))
    }

    fn daemon_for_tests() -> Daemon<MockRuntime> {
        Daemon::<MockRuntime>::new(fresh_shared(), mock_app_handle(), base_config())
    }

    #[test]
    fn test_daemon_new_initializes_fields() {
        let d = daemon_for_tests();
        assert!(d.hwinfo.is_none());
        assert!(d.oled.is_none());
        assert!(d.hid_api.is_none());
        assert_eq!(d.disconnect_count, 0);
        assert_eq!(d.page_counter, 0);
        assert!(!d.paused);
    }

    #[test]
    fn test_daemon_write_state_mutates_through_lock() {
        let d = daemon_for_tests();
        d.write_state(|s| {
            s.last_error = Some("test-err".to_string());
        });
        let g = d.state.lock().unwrap();
        assert_eq!(g.last_error.as_deref(), Some("test-err"));
    }

    #[test]
    fn test_daemon_push_status_runs() {
        // push_status emits an event — with MockRuntime this is a no-op but exercises the code path.
        let d = daemon_for_tests();
        d.push_status();
    }

    #[test]
    fn test_daemon_push_frame_runs() {
        let d = daemon_for_tests();
        d.push_frame(&OledBuffer::new(128, 64));
    }

    #[test]
    fn test_daemon_tick_missing_hwinfo_errors() {
        let mut d = daemon_for_tests();
        let r = d.tick();
        assert!(r.is_err());
        assert!(format!("{}", r.unwrap_err()).contains("hwinfo missing"));
    }

    #[test]
    fn test_handle_sleep_command_sets_paused() {
        let mut d = daemon_for_tests();
        d.handle_sleep_command(SleepCommand::Sleep);
        assert!(d.paused);
        d.handle_sleep_command(SleepCommand::Wake);
        assert!(!d.paused);
        d.handle_sleep_command(SleepCommand::White);
        assert!(d.paused);
    }

    #[test]
    fn test_tick_drains_sleep_request_and_skips_while_paused() {
        let mut d = daemon_for_tests();
        d.state.lock().unwrap().sleep_requested = Some(SleepCommand::Sleep);
        // No hwinfo installed: tick would fail if it did not return early.
        assert!(d.tick().is_ok());
        assert!(d.paused);
        assert!(d.state.lock().unwrap().sleep_requested.is_none());
    }

    #[test]
    fn test_tick_wake_resumes() {
        let mut d = daemon_for_tests();
        d.paused = true;
        d.state.lock().unwrap().sleep_requested = Some(SleepCommand::Wake);
        // Resumed, so tick goes on and fails on the missing hwinfo.
        assert!(d.tick().is_err());
        assert!(!d.paused);
    }

    #[test]
    fn test_tick_clears_reload_request_even_when_reload_fails() {
        let mut d = daemon_for_tests();
        d.paused = true; // return right after the reload attempt
        d.state.lock().unwrap().reload_requested = true;
        assert!(d.tick().is_ok());
        assert!(!d.state.lock().unwrap().reload_requested);
    }

    #[test]
    fn test_record_failure_writes_status_text() {
        let d = daemon_for_tests();
        d.record_failure("Connect failed", &anyhow!("boom"));
        assert_eq!(
            d.state.lock().unwrap().last_error.as_deref(),
            Some("Connect failed: boom")
        );
    }

    #[test]
    fn test_announce_sets_mode_and_message() {
        let d = daemon_for_tests();
        d.announce(
            Some(ActiveMode::DirectUsb),
            "Connecting to SteelSeries HID...",
        );
        let g = d.state.lock().unwrap();
        assert_eq!(g.active_mode, ActiveMode::DirectUsb);
        assert_eq!(
            g.last_error.as_deref(),
            Some("Connecting to SteelSeries HID...")
        );
    }

    #[test]
    fn test_take_reload_request_returns_and_clears_flag() {
        let d = daemon_for_tests();
        assert!(!d.take_reload_request());
        d.state.lock().unwrap().reload_requested = true;
        assert!(d.take_reload_request());
        assert!(!d.state.lock().unwrap().reload_requested);
        assert!(!d.take_reload_request());
    }

    /// Mock display driver that records calls without doing I/O.
    struct MockDriver {
        trigger_calls: usize,
        blank_calls: usize,
        white_calls: usize,
        stop_calls: usize,
        next_trigger_err: bool,
    }
    impl MockDriver {
        fn new() -> Self {
            Self {
                trigger_calls: 0,
                blank_calls: 0,
                white_calls: 0,
                stop_calls: 0,
                next_trigger_err: false,
            }
        }
    }
    impl DisplayDriver for MockDriver {
        fn trigger_frame(
            &mut self,
            _event: &str,
            _i: isize,
            _lines: &[String],
            _buf: &OledBuffer,
        ) -> Result<(), anyhow::Error> {
            self.trigger_calls += 1;
            if self.next_trigger_err {
                self.next_trigger_err = false;
                Err(anyhow!("mock trigger err"))
            } else {
                Ok(())
            }
        }
        fn send_blank(&mut self) -> Result<(), anyhow::Error> {
            self.blank_calls += 1;
            Ok(())
        }
        fn send_white(&mut self) -> Result<(), anyhow::Error> {
            self.white_calls += 1;
            Ok(())
        }
        fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
            self.stop_calls += 1;
            Ok(())
        }
    }

    fn install_mock_driver(d: &mut Daemon<MockRuntime>) {
        d.oled = Some(Box::new(MockDriver::new()));
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct Calls {
        frames: AtomicUsize,
        blank: AtomicUsize,
        white: AtomicUsize,
        stop: AtomicUsize,
        events: Mutex<Vec<String>>,
    }

    struct CountingDriver(Arc<Calls>);

    impl DisplayDriver for CountingDriver {
        fn trigger_frame(
            &mut self,
            event: &str,
            _i: isize,
            _lines: &[String],
            _buffer: &OledBuffer,
        ) -> Result<(), anyhow::Error> {
            self.0.events.lock().unwrap().push(event.to_string());
            self.0.frames.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn send_blank(&mut self) -> Result<(), anyhow::Error> {
            self.0.blank.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn send_white(&mut self) -> Result<(), anyhow::Error> {
            self.0.white.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
        fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
            self.0.stop.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }
    }

    #[test]
    fn test_daemon_drives_installed_display_driver() {
        let calls = Arc::new(Calls::default());
        let mut d = daemon_for_tests();
        d.oled = Some(Box::new(CountingDriver(calls.clone())));

        d.render(&full_summary_hwinfo(), true).unwrap();
        d.handle_sleep_command(SleepCommand::Sleep);
        d.handle_sleep_command(SleepCommand::White);
        d.disconnect_and_cleanup();

        assert_eq!(calls.frames.load(Ordering::Relaxed), 1);
        assert_eq!(calls.blank.load(Ordering::Relaxed), 1);
        assert_eq!(calls.white.load(Ordering::Relaxed), 1);
        assert_eq!(calls.stop.load(Ordering::Relaxed), 1);
        assert!(d.oled.is_none());
    }

    #[test]
    fn test_is_stale_counts_up_to_limit_and_resets_on_fresh() {
        let mut count = 0;
        assert!(!is_stale(false, &mut count, 3));
        assert!(!is_stale(false, &mut count, 3));
        assert!(is_stale(false, &mut count, 3));
        // Capped: stays at the limit, does not overflow.
        assert!(is_stale(false, &mut count, 3));
        assert_eq!(count, 3);
        assert!(!is_stale(true, &mut count, 3));
        assert_eq!(count, 0);
    }

    #[test]
    fn test_render_summary_updates_state_and_counter() {
        let mut d = daemon_for_tests();
        install_mock_driver(&mut d);
        let i_before = d.i.0;

        d.render(&full_summary_hwinfo(), true).unwrap();

        assert_eq!(d.i.0, i_before + 1);
        let g = d.state.lock().unwrap();
        assert!(g.hwinfo_connected);
        assert!(!g.sensor_values.is_empty());
        assert!(g.hwinfo_snapshot.is_some());
        assert!(g.last_error.is_none());
    }

    #[test]
    fn test_render_without_driver_errors() {
        let mut d = daemon_for_tests();
        let err = d.render(&full_summary_hwinfo(), true).unwrap_err();
        assert!(format!("{}", err).contains("oled missing"));
    }

    #[test]
    fn test_render_custom_mode_advances_pages() {
        let mut d = daemon_for_tests();
        d.config.is_summary = false;
        d.config.pages = 2;
        d.config.page_time = 1; // TICK_RATE=500 → 2 ticks per second → switch every 2 ticks
        d.config.custom_sensors = vec![Vec::new(), Vec::new()];
        install_mock_driver(&mut d);
        let hw = build_hwinfo(&[("S", "R", 1.0)]);

        for _ in 0..3 {
            d.render(&hw, true).unwrap();
        }
        assert_eq!(d.page_counter, 1);
    }

    #[test]
    fn test_render_shows_disconnected_after_stale_limit() {
        let mut d = daemon_for_tests();
        install_mock_driver(&mut d);
        let hw = full_summary_hwinfo();

        for _ in 0..STALE_LIMIT - 1 {
            d.render(&hw, false).unwrap();
        }
        assert!(d.state.lock().unwrap().hwinfo_connected);

        d.render(&hw, false).unwrap();
        let g = d.state.lock().unwrap();
        assert!(!g.hwinfo_connected);
        assert_eq!(g.last_error.as_deref(), Some("HWiNFO disconnected"));
    }

    #[test]
    fn test_render_sends_disconnected_frame_on_bound_page_event() {
        let calls = Arc::new(Calls::default());
        let mut d = daemon_for_tests();
        d.oled = Some(Box::new(CountingDriver(calls.clone())));
        let hw = full_summary_hwinfo();

        for _ in 0..STALE_LIMIT {
            d.render(&hw, false).unwrap();
        }

        assert!(!d.state.lock().unwrap().hwinfo_connected);
        let events = calls.events.lock().unwrap();
        assert_eq!(events.len(), STALE_LIMIT);
        assert_eq!(events.last().map(String::as_str), Some("PAGE1"));
    }

    #[test]
    fn test_render_recovers_after_stale() {
        let mut d = daemon_for_tests();
        install_mock_driver(&mut d);
        let hw = full_summary_hwinfo();
        for _ in 0..STALE_LIMIT {
            d.render(&hw, false).unwrap();
        }
        assert!(!d.state.lock().unwrap().hwinfo_connected);

        d.render(&hw, true).unwrap();
        let g = d.state.lock().unwrap();
        assert!(g.hwinfo_connected);
        assert!(g.last_error.is_none());
    }

    #[test]
    fn test_disconnect_and_cleanup_clears_everything() {
        let mut d = daemon_for_tests();
        install_mock_driver(&mut d);
        d.state.lock().unwrap().hwinfo_connected = true;
        d.state.lock().unwrap().gg_connected = true;
        d.disconnect_and_cleanup();
        // A Hwinfo cannot be constructed without a live HWiNFO mapping, so the
        // test cannot install one first.
        assert!(d.hwinfo.is_none());
        assert!(d.oled.is_none());
        assert!(d.hid_api.is_none());
        let g = d.state.lock().unwrap();
        assert!(!g.hwinfo_connected);
        assert!(!g.gg_connected);
        assert!(!g.usb_connected);
        assert_eq!(g.active_mode, ActiveMode::Disconnected);
    }

    #[test]
    fn test_white_buffer_all_pixels_on() {
        let buf = white_buffer(128, 64);
        // Every byte should be 0xFF
        assert!(buf.data.iter().all(|b| *b == 0xFF));
    }
}
