mod settings;
use ini::Ini;
use settings::{settings_create_config, AppConfig};

mod consts;
use consts::*;

mod connect;
use connect::{connect_hid, connect_hwinfo, connect_steelseries};

mod console_utils;
use console_utils::{console_window, display_value_in_console, Console};

mod steelseries;
use steelseries::page_handler;

mod utils;
use utils::{format_custom_value, run_sensors};

mod mouse_battery;
use mouse_battery::MouseBatteryReader;

mod render;
use render::render_text_to_oled;

mod gui;

use anyhow;
use console::Term;
use gamesense::client::GameSenseClient;
use hwinfo_steelseries_oled::Hwinfo;
use image::ImageReader;
use log::{debug, error, info, warn};
use serde_json::{json, Value};
use std::io::Cursor;
use std::num::Wrapping;
use std::thread;
use std::time::{Duration, Instant};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuId, MenuItem},
    Icon, TrayIconBuilder, TrayIconEvent,
};
use winit::application::ApplicationHandler;
use winit::event::StartCause;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

// User events sent to winit event loop
#[derive(Debug)]
enum UserEvent {
    TrayIconEvent(TrayIconEvent),
    MenuEvent(MenuEvent),
    UpdateDisplay,
    OpenGui,
}

// Summary sensors data
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
            .ok_or_else(|| anyhow::anyhow!("GPU Temperature not found"))?
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

fn format_vertical_summary(sensors: &SummarySensors, decimal: bool) -> Value {
    let precision = if decimal { 1 } else { 0 };
    let spacing = if decimal { " " } else { "   " };
    let spacing2 = if decimal { " " } else { "    " };

    json!({
        "line1": "CPU   GPU   MEM",
        "line2": format!("{:.prec$}°{}{:.prec$}°{}{:.prec$}G",
            sensors.cpu_temp,
            spacing,
            sensors.gpu_temp,
            spacing,
            sensors.mem_used,
            prec = precision),
        "line3": format!("{:.prec$}%{}{:.prec$}%{}{:.prec$}G",
            sensors.cpu_usage,
            spacing2,
            sensors.gpu_usage,
            spacing2,
            sensors.mem_free,
            prec = precision),
    })
}

fn format_horizontal_summary(sensors: &SummarySensors, decimal: bool) -> Value {
    let precision = if decimal { 1 } else { 0 };

    json!({
        "line1": format!("CPU {:.prec$}° {:.prec$}%",
            sensors.cpu_temp,
            sensors.cpu_usage,
            prec = precision),
        "line2": format!("GPU {:.prec$}° {:.prec$}%",
            sensors.gpu_temp,
            sensors.gpu_usage,
            prec = precision),
        "line3": format!("MEM {:.prec$}G {:.prec$}%",
            sensors.mem_used,
            sensors.mem_load,
            prec = precision),
    })
}

fn check_hwinfo_connection(
    old: &Hwinfo,
    new: &Hwinfo,
    disconnect_count: &mut usize,
    limit: usize,
) -> bool {
    if old == new {
        *disconnect_count = (*disconnect_count + 1).min(limit);
    } else {
        *disconnect_count = 0;
    }
    *disconnect_count >= limit
}

fn handle_fatal_error(term: &Term, err: anyhow::Error) -> anyhow::Error {
    error!("Fatal error occurred: {}", err);

    // Show console window to ensure user can see the error
    console_window(Console::SHOW);

    // Display error message
    let _ = term.write_line("");
    let _ = term.write_line("=================================");
    let _ = term.write_line("ERROR: Application stopped");
    let _ = term.write_line("=================================");
    let _ = term.write_line(&format!("{}", err));

    // Show the cause chain if available
    let mut source = err.source();
    if source.is_some() {
        let _ = term.write_line("");
        let _ = term.write_line("Caused by:");
    }
    while let Some(cause) = source {
        let _ = term.write_line(&format!("  {}", cause));
        source = cause.source();
    }

    let _ = term.write_line("");
    let _ = term.write_line("Press Enter to exit...");

    // Wait for user input
    let _ = std::io::stdin().read_line(&mut String::new());

    err
}

// OledClient abstraction
enum OledClient {
    GameSense(GameSenseClient),
    Hid(hidapi::HidDevice),
}

impl OledClient {
    fn trigger_frame(&mut self, event: &str, i: isize, value: Value) -> Result<(), anyhow::Error> {
        match self {
            OledClient::GameSense(client) => {
                client.trigger_event_frame(event, i, value)?;
            }
            OledClient::Hid(device) => {
                // For HID, we need to render the JSON value to a bitmap
                let mut text = String::new();
                if let Some(obj) = value.as_object() {
                    for i in 1..=DISPLAY_LINES {
                        let line_key = format!("line{}", i);
                        if let Some(val) = obj.get(&line_key) {
                            if let Some(s) = val.as_str() {
                                if !text.is_empty() {
                                    text.push('\n');
                                }
                                text.push_str(s);
                            }
                        }
                    }
                }

                let buffer = render_text_to_oled(&text, 0);
                let screen_height: u8 = 64;
                let chunk_width: u8 = 64;

                for chunk_x in [0u8, 64u8] {
                    let chunk_bitmap = buffer.get_chunk(chunk_x, chunk_width);
                    let mut packet = vec![0x06u8, 0x93, chunk_x, 0, chunk_width, screen_height];
                    packet.extend_from_slice(&chunk_bitmap);
                    packet.resize(1024, 0);

                    // We ignore errors here for now or just log them to avoid crashing the loop
                    if let Err(e) = device.send_feature_report(&packet) {
                        error!("Failed to send HID frame: {}", e);
                    }
                }
            }
        }
        Ok(())
    }

    /// Send a blank frame to clear the OLED screen (sleep mode)
    fn send_blank_frame(&mut self) -> Result<(), anyhow::Error> {
        match self {
            OledClient::GameSense(client) => {
                // Send empty strings for all lines
                let blank_value = json!({
                    "line1": "",
                    "line2": "",
                    "line3": ""
                });
                client.trigger_event_frame("BLANK", 0, blank_value)?;
            }
            OledClient::Hid(device) => {
                // Create a blank buffer (all zeros = all pixels off)
                let buffer = render::OledBuffer::new();
                let screen_height: u8 = 64;
                let chunk_width: u8 = 64;

                // Send blank buffer in two chunks
                for chunk_x in [0u8, 64u8] {
                    let chunk_bitmap = buffer.get_chunk(chunk_x, chunk_width);
                    let mut packet = vec![0x06u8, 0x93, chunk_x, 0, chunk_width, screen_height];
                    packet.extend_from_slice(&chunk_bitmap);
                    packet.resize(1024, 0);

                    if let Err(e) = device.send_feature_report(&packet) {
                        error!("Failed to send blank HID frame: {}", e);
                        return Err(anyhow::anyhow!("Failed to send blank frame: {}", e));
                    }
                }
                info!("Sent blank frame to clear OLED screen");
            }
        }
        Ok(())
    }

    /// Send a white frame to fill the OLED screen (all pixels on)
    fn send_white_frame(&mut self) -> Result<(), anyhow::Error> {
        match self {
            OledClient::GameSense(client) => {
                // Send full white blocks for all lines
                let white_value = json!({
                    "line1": "████████████████",
                    "line2": "████████████████",
                    "line3": "████████████████"
                });
                client.trigger_event_frame("WHITE", 0, white_value)?;
            }
            OledClient::Hid(device) => {
                // Create a white buffer (all 0xFF = all pixels on)
                let mut buffer = render::OledBuffer::new();
                // Set all pixels to white
                for x in 0..128 {
                    for y in 0..64 {
                        buffer.set_pixel(x, y, true);
                    }
                }

                let screen_height: u8 = 64;
                let chunk_width: u8 = 64;

                // Send white buffer in two chunks
                for chunk_x in [0u8, 64u8] {
                    let chunk_bitmap = buffer.get_chunk(chunk_x, chunk_width);
                    let mut packet = vec![0x06u8, 0x93, chunk_x, 0, chunk_width, screen_height];
                    packet.extend_from_slice(&chunk_bitmap);
                    packet.resize(1024, 0);

                    if let Err(e) = device.send_feature_report(&packet) {
                        error!("Failed to send white HID frame: {}", e);
                        return Err(anyhow::anyhow!("Failed to send white frame: {}", e));
                    }
                }
                info!("Sent white frame to fill OLED screen");
            }
        }
        Ok(())
    }

    fn stop_heartbeat(&mut self) -> Result<(), anyhow::Error> {
        if let OledClient::GameSense(client) = self {
            client.stop_heartbeat()?;
        }
        Ok(())
    }
}

// Application state
struct App {
    term: Term,
    oled: Option<OledClient>,
    hid_api: Option<hidapi::HidApi>,
    hwinfo: Option<Hwinfo>,
    config_file: Option<Ini>,
    pages_vec: Vec<ini::Properties>,

    // Config values (stored directly to avoid lifetime issues)
    is_summary: bool,
    is_vertical: bool,
    gpu: String,
    decimal: bool,
    pages: usize,
    page_time: isize,
    sensors_per_line: u8,

    // Runtime state
    i: Wrapping<isize>,
    disconnect_count: usize,
    page_counter: usize,
    was_disconnected: bool,
    last_update: Instant,
    display_in_console: bool,

    // Tray icon
    _tray_icon: Option<tray_icon::TrayIcon>,
    exit_menu_id: Option<MenuId>,
    settings_menu_id: Option<MenuId>,
    reload_menu_id: Option<MenuId>,
    sleep_menu_id: Option<MenuId>,
    white_screen_menu_id: Option<MenuId>,
    event_loop_proxy: Option<EventLoopProxy<UserEvent>>,

    // Keep direct_usb flag
    direct_usb: bool,

    // Mouse battery tracking
    mouse_battery_reader: MouseBatteryReader,

    // Config reload
    config_last_modified: Option<std::time::SystemTime>,

    // Sleep mode state
    is_sleeping: bool,
    is_white_screen: bool,
}

impl App {
    fn new(term: Term, event_loop_proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            term,
            oled: None,
            hid_api: None,
            hwinfo: None,
            config_file: None,
            pages_vec: Vec::new(),
            is_summary: true,
            is_vertical: true,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            i: Wrapping(0),
            disconnect_count: 0,
            page_counter: 0,
            was_disconnected: false,
            last_update: Instant::now(),
            display_in_console: cfg!(debug_assertions),
            _tray_icon: None,
            exit_menu_id: None,
            settings_menu_id: None,
            reload_menu_id: None,
            sleep_menu_id: None,
            white_screen_menu_id: None,
            event_loop_proxy: Some(event_loop_proxy),
            direct_usb: false,
            mouse_battery_reader: MouseBatteryReader::new(),
            config_last_modified: None,
            is_sleeping: false,
            is_white_screen: false,
        }
    }

    fn setup_tray_icon(&mut self) -> Result<(), anyhow::Error> {
        let event_loop_proxy = self.event_loop_proxy.as_ref().unwrap().clone();

        // Embed the icon at compile time
        const ICON_DATA: &[u8] = include_bytes!("../assets/hwinfo-steelseries-icon.ico");

        // Create menu
        let tray_menu = Menu::new();
        let settings_menu_item = MenuItem::new("Settings...", true, None);
        let reload_menu_item = MenuItem::new("Reload Config", true, None);
        let sleep_menu_item = MenuItem::new("Sleep Display", true, None);
        let white_screen_menu_item = MenuItem::new("White Screen", true, None);
        let exit_menu_item = MenuItem::new("Exit", true, None);
        tray_menu.append(&settings_menu_item)?;
        tray_menu.append(&reload_menu_item)?;
        tray_menu.append(&sleep_menu_item)?;
        tray_menu.append(&white_screen_menu_item)?;
        tray_menu.append(&exit_menu_item)?;

        // Save menu IDs
        self.settings_menu_id = Some(settings_menu_item.id().clone());
        self.reload_menu_id = Some(reload_menu_item.id().clone());
        self.sleep_menu_id = Some(sleep_menu_item.id().clone());
        self.white_screen_menu_id = Some(white_screen_menu_item.id().clone());
        self.exit_menu_id = Some(exit_menu_item.id().clone());

        // Decode the embedded ICO file
        let icon_result = ImageReader::new(Cursor::new(ICON_DATA))
            .with_guessed_format()
            .map_err(|e| format!("Format error: {}", e))
            .and_then(|reader| reader.decode().map_err(|e| format!("Decode error: {}", e)));

        let icon: Option<Icon> = match icon_result {
            Ok(img) => {
                let rgba = img.to_rgba8();
                let (width, height) = rgba.dimensions();
                match Icon::from_rgba(rgba.into_raw(), width, height) {
                    Ok(icon) => {
                        info!(
                            "Successfully loaded embedded tray icon ({}x{})",
                            width, height
                        );
                        Some(icon)
                    }
                    Err(e) => {
                        warn!("Failed to create icon from RGBA data: {}", e);
                        None
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Failed to decode embedded icon (continuing without icon): {}",
                    e
                );
                None
            }
        };

        // Build tray icon WITH menu attached
        let tray_icon = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("HWiNFO-SteelSeries")
            .with_icon(icon.unwrap())
            .build()
            .map_err(|e| {
                error!("Failed to build tray icon: {}", e);
                e
            })?;

        self._tray_icon = Some(tray_icon);
        info!("Tray icon created successfully");

        // Set up event handlers to forward events to winit event loop
        let proxy_tray = event_loop_proxy.clone();
        TrayIconEvent::set_event_handler(Some(move |event| {
            debug!("Tray event handler called: {:?}", event);
            let _ = proxy_tray.send_event(UserEvent::TrayIconEvent(event));
        }));

        let proxy_menu = event_loop_proxy;
        MenuEvent::set_event_handler(Some(move |event| {
            debug!("Menu event handler called: {:?}", event);
            let _ = proxy_menu.send_event(UserEvent::MenuEvent(event));
        }));

        Ok(())
    }

    fn initialize(&mut self) -> Result<(), anyhow::Error> {
        // Clear existing custom sensor pages
        self.pages_vec.clear();

        // Load configuration first to know which connection to use
        info!("Loading configuration from conf.ini");
        let mut config_loaded = false;
        let config_file = match Ini::load_from_file("conf.ini") {
            Ok(conf) => {
                info!("Configuration file loaded successfully");
                config_loaded = true;
                conf
            }
            Err(err) => {
                warn!(
                    "Configuration file not found: {}. Creating dummy for setup",
                    err
                );
                Ini::new()
            }
        };

        // Need Hwinfo for setup if config not found
        let mut hwinfo = connect_hwinfo(&self.term)?;
        hwinfo.pull().map_err(|e| {
            error!("Failed to pull initial HWiNFO data: {}", e);
            e
        })?;
        self.hwinfo = Some(hwinfo);

        let config_file = if !config_loaded {
            settings_create_config(&self.term, self.hwinfo.as_ref().unwrap())?
        } else {
            config_file
        };

        // Store config_file
        self.config_file = Some(config_file);

        let config = AppConfig::from_ini(self.config_file.as_ref().unwrap()).map_err(|e| {
            error!("Failed to parse configuration: {}", e);
            e
        })?;
        info!("Configuration parsed successfully");

        // Store config values
        self.is_summary = config.is_summary;
        self.is_vertical = config.is_vertical;
        self.gpu = config.gpu.to_string();
        self.decimal = config.decimal;
        self.pages = config.pages;
        self.page_time = config.page_time;
        self.sensors_per_line = config.sensors_per_line;
        self.direct_usb = config.direct_usb;

        // Store modification time
        if let Ok(metadata) = std::fs::metadata("conf.ini") {
            self.config_last_modified = metadata.modified().ok();
        }

        // For custom mode, store the sensor configuration regardless of connection type
        if !self.is_summary {
            info!("Setting up {} custom sensor page(s)", self.pages);
            for i in 1..=self.pages {
                match self
                    .config_file
                    .as_ref()
                    .unwrap()
                    .section(Some(format!("PAGE{}.Sensors", i)))
                {
                    Some(page) => {
                        self.pages_vec.push(page.clone());
                    }
                    None => {
                        warn!("PAGE{}.Sensors section not found in config", i);
                        continue;
                    }
                };
            }
        }

        // Connect to OLED service
        if self.direct_usb {
            info!("Initializing Direct USB (HID) connection");
            let api = hidapi::HidApi::new().map_err(|e| {
                error!("Failed to initialize HID API: {}", e);
                anyhow::anyhow!("Failed to initialize HID API: {}", e)
            })?;
            let device = connect_hid(&self.term, &api)?;
            self.oled = Some(OledClient::Hid(device));
            self.hid_api = Some(api);
        } else {
            info!("Initializing SteelSeries GameSense connection");
            let mut steelseries = connect_steelseries(&self.term)?;

            // Setup pages for GameSense
            info!("Binding GameSense events for {} page(s)", self.pages);
            for i in 1..=self.pages {
                let mut line_keys = Vec::new();
                for j in 1..=DISPLAY_LINES {
                    line_keys.push(format!("line{}", j));
                }
                let line_keys_refs: Vec<&str> = line_keys.iter().map(|s| s.as_str()).collect();

                let handler = page_handler(3, &line_keys_refs, None);
                steelseries
                    .bind_event(
                        format!("PAGE{}", i).as_str(),
                        None,
                        None,
                        None,
                        None,
                        vec![handler],
                    )
                    .map_err(|e| {
                        error!("Failed to bind event for PAGE{}: {}", i, e);
                        e
                    })?;
                info!("Successfully bound event for PAGE{}", i);
            }
            steelseries.start_heartbeat();
            self.oled = Some(OledClient::GameSense(steelseries));
        }

        // Hide console window in release mode after successful startup
        #[cfg(not(debug_assertions))]
        {
            thread::sleep(Duration::from_millis(500));
            console_window(Console::HIDE);
        }

        Ok(())
    }

    fn update_display(&mut self) -> Result<(), anyhow::Error> {
        // Skip updates if in sleep mode or white screen mode
        if self.is_sleeping || self.is_white_screen {
            return Ok(());
        }

        // Check for config updates
        if let Ok(metadata) = std::fs::metadata("conf.ini") {
            if let Ok(modified) = metadata.modified() {
                if let Some(last_modified) = self.config_last_modified {
                    if modified > last_modified {
                        info!("Configuration file changed, reloading...");
                        self.is_sleeping = false; // Wake up on config reload
                        self.is_white_screen = false;
                        // Reload the whole app state or just variables?
                        // Simplified: Reload everything by re-calling initialize
                        if let Err(e) = self.initialize() {
                            error!("Failed to reload configuration: {}", e);
                            // Continue with old config
                        }
                    }
                }
            }
        }

        let hwinfo = match self.hwinfo.as_mut() {
            Some(hwinfo) => hwinfo,
            None => return Err(anyhow::anyhow!("HWiNFO not initialized")),
        };
        let oled = match self.oled.as_mut() {
            Some(oled) => oled,
            None => return Err(anyhow::anyhow!("OLED not initialized")),
        };

        let old = hwinfo.clone();
        hwinfo.pull()?;

        let disconnected = check_hwinfo_connection(&old, hwinfo, &mut self.disconnect_count, 5);
        drop(old);

        // Hide console when reconnected
        if self.was_disconnected && !disconnected {
            #[cfg(not(debug_assertions))]
            console_window(Console::HIDE);
        }
        self.was_disconnected = disconnected;

        if disconnected {
            warn!("Disconnected from HWiNFO (no data updates for 5 cycles)");
            console_window(Console::SHOW);
            self.term.clear_line()?;
            self.term.write_line("Disconnected from HWiNFO")?;
            let value = json!({
                "line1": "Disconnected",
                "line2": "FROM",
                "line3": "HWiNFO"
            });
            if let Err(e) = oled.trigger_frame("ERROR", self.i.0, value) {
                error!("Failed to trigger error frame: {}", e);
            }
            self.i += 1;
            self.last_update = Instant::now();
            return Ok(());
        }

        let value = if self.is_summary {
            match fetch_summary_sensors(hwinfo, &self.gpu) {
                Ok(sensors) => {
                    if self.is_vertical {
                        format_vertical_summary(&sensors, self.decimal)
                    } else {
                        format_horizontal_summary(&sensors, self.decimal)
                    }
                }
                Err(e) => {
                    error!("Failed to fetch summary sensors: {}", e);
                    return Err(e);
                }
            }
        } else {
            // Custom Sensors - Logic to alternate between pages
            let ticks_per_second = 1000 / TICK_RATE as isize;
            if self.i.0 % (self.page_time * ticks_per_second) == 0 && self.i.0 != 0 {
                self.page_counter = (self.page_counter + 1) % self.pages;
                debug!("Switching to page {}", self.page_counter + 1);
            }
            let pages_sensors = &self.pages_vec[self.page_counter];

            let mut labels = vec![""; CUSTOM_SENSORS];
            let mut units = vec![""; CUSTOM_SENSORS];
            let mut values = vec![String::new(); CUSTOM_SENSORS];

            run_sensors(
                pages_sensors,
                &mut labels,
                &mut units,
                &mut values,
                hwinfo,
                self.decimal,
                &mut self.mouse_battery_reader,
                self.hid_api.as_ref().unwrap(),
            )?;

            format_custom_value(self.sensors_per_line, labels, values, units)
        };

        if self.display_in_console {
            display_value_in_console(&self.term, &value)?;
        }

        oled.trigger_frame(
            format!("PAGE{}", self.page_counter + 1).as_str(),
            self.i.0,
            value,
        )?;

        self.i += 1;
        Ok(())
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, _event_loop: &ActiveEventLoop) {}

    fn window_event(
        &mut self,
        _event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        _event: winit::event::WindowEvent,
    ) {
        // We don't have any windows, so this is a no-op
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        if cause == StartCause::Init {
            info!("Event loop initialized, setting up tray icon");

            // Create tray icon
            if let Err(e) = self.setup_tray_icon() {
                error!("Failed to setup tray icon: {}", e);
                event_loop.exit();
                return;
            }

            // Initialize the application
            if let Err(e) = self.initialize() {
                error!("Failed to initialize application: {}", e);
                event_loop.exit();
                return;
            }

            info!("Application initialized successfully");
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::TrayIconEvent(event) => {
                debug!("Received tray icon event: {:?}", event);
                match event {
                    TrayIconEvent::Click {
                        button,
                        button_state,
                        ..
                    } => {
                        // Only handle button release (up) to avoid duplicate events
                        // Windows sends both button down and button up events
                        if button_state == tray_icon::MouseButtonState::Up {
                            debug!("Tray icon clicked with button: {:?}", button);
                        }
                    }
                    TrayIconEvent::DoubleClick { button, .. } => {
                        debug!("Tray icon double-clicked with button: {:?}", button);
                        if button == tray_icon::MouseButton::Left {
                            self.is_sleeping = false; // Wake up when opening settings via double-click
                            self.is_white_screen = false;
                            let _ = self
                                .event_loop_proxy
                                .as_ref()
                                .unwrap()
                                .send_event(UserEvent::OpenGui);
                        }
                    }
                    _ => {}
                }
            }
            UserEvent::MenuEvent(event) => {
                debug!("Received menu event: {:?}", event);
                if let Some(settings_id) = &self.settings_menu_id {
                    if event.id() == settings_id {
                        self.is_sleeping = false; // Wake up when opening settings
                        self.is_white_screen = false;
                        let _ = self
                            .event_loop_proxy
                            .as_ref()
                            .unwrap()
                            .send_event(UserEvent::OpenGui);
                    }
                }
                if let Some(reload_id) = &self.reload_menu_id {
                    if event.id() == reload_id {
                        info!("Reload Config menu item clicked, re-initializing...");
                        self.is_sleeping = false; // Wake up on reload
                        self.is_white_screen = false;
                        if let Err(e) = self.initialize() {
                            error!("Failed to reload configuration: {}", e);
                        }
                    }
                }
                if let Some(sleep_id) = &self.sleep_menu_id {
                    if event.id() == sleep_id {
                        info!("Sleep Display menu item clicked, entering sleep mode...");
                        self.is_sleeping = true;
                        self.is_white_screen = false;
                        if let Some(oled) = self.oled.as_mut() {
                            if let Err(e) = oled.send_blank_frame() {
                                error!("Failed to send blank frame: {}", e);
                            }
                        } else {
                            warn!("OLED not initialized, cannot send blank frame");
                        }
                    }
                }
                if let Some(white_screen_id) = &self.white_screen_menu_id {
                    if event.id() == white_screen_id {
                        info!("White Screen menu item clicked, entering white screen mode...");
                        self.is_white_screen = true;
                        self.is_sleeping = false;
                        if let Some(oled) = self.oled.as_mut() {
                            if let Err(e) = oled.send_white_frame() {
                                error!("Failed to send white frame: {}", e);
                            }
                        } else {
                            warn!("OLED not initialized, cannot send white frame");
                        }
                    }
                }
                if let Some(exit_id) = &self.exit_menu_id {
                    if event.id() == exit_id {
                        info!("Exit menu item clicked, shutting down");
                        if let Some(oled) = self.oled.as_mut() {
                            let _ = oled.stop_heartbeat();
                        }
                        event_loop.exit();
                    }
                }
            }
            UserEvent::UpdateDisplay => {
                // Only update if enough time has passed
                if self.last_update.elapsed() >= Duration::from_millis(TICK_RATE) {
                    self.last_update = Instant::now();

                    if let Err(e) = self.update_display() {
                        error!("Failed to update display: {}", e);
                        let _ = handle_fatal_error(&self.term, e);
                        event_loop.exit();
                    }
                }
            }
            UserEvent::OpenGui => {
                info!("Opening settings GUI (separate process)");
                match std::env::current_exe() {
                    Ok(current_exe) => {
                        if let Err(e) = std::process::Command::new(current_exe)
                            .arg("--settings")
                            .spawn()
                        {
                            error!("Failed to spawn settings process: {}", e);
                        }
                    }
                    Err(e) => error!("Failed to get current executable path: {}", e),
                }
            }
        }
    }
}

fn main() -> Result<(), anyhow::Error> {
    // Initialize logger
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();

    // Check for brute-force battery discovery mode
    if let Some(pos) = args.iter().position(|arg| arg == "--discover-all-battery") {
        if args.len() < pos + 2 {
            eprintln!("Usage: {} --discover-all-battery <EXPECTED_BATTERY>", args[0]);
            eprintln!("");
            eprintln!("This will search ALL HID devices and ALL report IDs for the expected battery value.");
            eprintln!("");
            eprintln!("Example:");
            eprintln!("  {} --discover-all-battery 53", args[0]);
            eprintln!("  (Searches all devices for battery value of 53%)");
            std::process::exit(1);
        }

        let expected_str = &args[pos + 1];
        let expected_battery = match expected_str.parse::<u8>() {
            Ok(val) if val <= 100 => val,
            _ => {
                eprintln!("Error: Expected battery must be 0-100");
                std::process::exit(1);
            }
        };

        info!("Brute Force Battery Discovery Mode");
        info!("Expected battery value: {}%", expected_battery);
        info!("");

        // Initialize HID API
        let hid_api = match hidapi::HidApi::new() {
            Ok(api) => api,
            Err(e) => {
                error!("Failed to initialize HID API: {}", e);
                std::process::exit(1);
            }
        };

        // Run brute-force discovery
        match mouse_battery::discover_all_devices_for_battery(&hid_api, expected_battery) {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                error!("Discovery failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    // Check for discovery mode
    if let Some(pos) = args.iter().position(|arg| arg == "--discover-mouse-battery") {
        if args.len() < pos + 2 {
            eprintln!("Usage: {} --discover-mouse-battery <VID> [PID] [EXPECTED_BATTERY]", args[0]);
            eprintln!("");
            eprintln!("Examples:");
            eprintln!("  Search specific device:");
            eprintln!("    {} --discover-mouse-battery 0x046d 0xc539", args[0]);
            eprintln!("");
            eprintln!("  Search all devices with VID:");
            eprintln!("    {} --discover-mouse-battery 0e7e", args[0]);
            eprintln!("");
            eprintln!("  Search with expected battery value (highlights matching reports):");
            eprintln!("    {} --discover-mouse-battery 0e7e 151e 53", args[0]);
            eprintln!("    {} --discover-mouse-battery 0e7e - 53  (dash for any PID)", args[0]);
            std::process::exit(1);
        }

        let vid_str = &args[pos + 1];
        let pid_str = args.get(pos + 2);
        let expected_str = args.get(pos + 3);

        // Parse VID (support both 0x046d and 046d formats)
        let vid = if vid_str.starts_with("0x") || vid_str.starts_with("0X") {
            u16::from_str_radix(&vid_str[2..], 16)
        } else {
            u16::from_str_radix(vid_str, 16)
        };

        // Parse PID if provided (support "-" for wildcard)
        let pid = if let Some(pid_str) = pid_str {
            if pid_str == "-" {
                None
            } else {
                let parsed = if pid_str.starts_with("0x") || pid_str.starts_with("0X") {
                    u16::from_str_radix(&pid_str[2..], 16)
                } else {
                    u16::from_str_radix(pid_str, 16)
                };
                Some(parsed)
            }
        } else {
            None
        };

        // Parse expected battery value if provided
        let expected_battery = if let Some(expected_str) = expected_str {
            match expected_str.parse::<u8>() {
                Ok(val) if val <= 100 => Some(val),
                _ => {
                    eprintln!("Error: Expected battery must be 0-100");
                    std::process::exit(1);
                }
            }
        } else {
            None
        };

        match (vid, pid) {
            (Ok(vendor_id), None) => {
                info!("Battery Report ID Discovery Mode");
                info!("Target: All devices with VID={:04x}", vendor_id);
                if let Some(expected) = expected_battery {
                    info!("Expected battery: {}%", expected);
                }
                info!("");

                // Initialize HID API
                let hid_api = match hidapi::HidApi::new() {
                    Ok(api) => api,
                    Err(e) => {
                        error!("Failed to initialize HID API: {}", e);
                        std::process::exit(1);
                    }
                };

                // Run discovery
                match mouse_battery::discover_battery_report_id(&hid_api, vendor_id, None, expected_battery) {
                    Ok(results) => {
                        if !results.is_empty() {
                            println!("\n========================================");
                            println!("DISCOVERY RESULTS - ALL REPORTS");
                            println!("========================================");
                            println!("Device VID: {:04x}", vendor_id);
                            println!("\nAll reports that returned data:");
                            if let Some(expected) = expected_battery {
                                println!("(*** = contains expected value {}%)", expected);
                            }
                            println!();

                            for (report_id, data, likely_battery) in &results {
                                // Check if contains expected value
                                let contains_expected = if let Some(expected) = expected_battery {
                                    data.iter().any(|&b| b == expected)
                                } else {
                                    false
                                };

                                if contains_expected {
                                    println!("*** Report ID: 0x{:02x} *** CONTAINS {}% ***", report_id, expected_battery.unwrap());
                                } else {
                                    println!("Report ID: 0x{:02x}", report_id);
                                }

                                println!("  Raw hex: {:02x?}", data);

                                // Show decimal values for first 16 bytes
                                let decimals: Vec<String> = data.iter().take(16).map(|b| format!("{}", b)).collect();
                                println!("  Decimal: [{}{}]",
                                    decimals.join(", "),
                                    if data.len() > 16 { ", ..." } else { "" }
                                );

                                if let Some(battery) = likely_battery {
                                    println!("  Auto-detected: {}% (may be incorrect!)", battery);
                                }
                                println!();
                            }

                            println!("========================================");
                            println!("Manual Analysis:");
                            if let Some(expected) = expected_battery {
                                println!("1. Look for reports marked with *** (contain {}%)", expected);
                                println!("2. Note the report ID and byte position");
                            } else {
                                println!("1. Look through the decimal values above");
                                println!("2. Find which report contains your actual battery level");
                                println!("3. Note the report ID and byte position");
                            }
                            println!("4. Add to MOUSE_PROFILES with that report ID");
                            println!("========================================\n");
                        }
                        std::process::exit(0);
                    }
                    Err(e) => {
                        error!("Discovery failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            (Ok(vendor_id), Some(Ok(product_id))) => {
                info!("Battery Report ID Discovery Mode");
                info!("Target device: VID={:04x} PID={:04x}", vendor_id, product_id);
                if let Some(expected) = expected_battery {
                    info!("Expected battery: {}%", expected);
                }
                info!("");

                // Initialize HID API
                let hid_api = match hidapi::HidApi::new() {
                    Ok(api) => api,
                    Err(e) => {
                        error!("Failed to initialize HID API: {}", e);
                        std::process::exit(1);
                    }
                };

                // Run discovery
                match mouse_battery::discover_battery_report_id(&hid_api, vendor_id, Some(product_id), expected_battery) {
                    Ok(results) => {
                        if !results.is_empty() {
                            println!("\n========================================");
                            println!("DISCOVERY RESULTS - ALL REPORTS");
                            println!("========================================");
                            println!("Device: {:04x}:{:04x}", vendor_id, product_id);
                            println!("\nAll reports that returned data:");
                            if let Some(expected) = expected_battery {
                                println!("(*** = contains expected value {}%)", expected);
                            }
                            println!();

                            for (report_id, data, likely_battery) in &results {
                                // Check if contains expected value
                                let contains_expected = if let Some(expected) = expected_battery {
                                    data.iter().any(|&b| b == expected)
                                } else {
                                    false
                                };

                                if contains_expected {
                                    println!("*** Report ID: 0x{:02x} *** CONTAINS {}% ***", report_id, expected_battery.unwrap());
                                } else {
                                    println!("Report ID: 0x{:02x}", report_id);
                                }

                                println!("  Raw hex: {:02x?}", data);

                                // Show decimal values for first 16 bytes
                                let decimals: Vec<String> = data.iter().take(16).map(|b| format!("{}", b)).collect();
                                println!("  Decimal: [{}{}]",
                                    decimals.join(", "),
                                    if data.len() > 16 { ", ..." } else { "" }
                                );

                                if let Some(battery) = likely_battery {
                                    println!("  Auto-detected: {}% (may be incorrect!)", battery);
                                }
                                println!();
                            }

                            println!("========================================");
                            println!("Manual Analysis:");
                            if let Some(expected) = expected_battery {
                                println!("1. Look for reports marked with *** (contain {}%)", expected);
                                println!("2. Note the report ID and byte position");
                            } else {
                                println!("1. Look through the decimal values above");
                                println!("2. Find which report contains your actual battery level");
                                println!("3. Note the report ID and byte position");
                            }
                            println!("4. Add to MOUSE_PROFILES with that report ID");
                            println!("========================================\n");
                        }
                        std::process::exit(0);
                    }
                    Err(e) => {
                        error!("Discovery failed: {}", e);
                        std::process::exit(1);
                    }
                }
            }
            (Err(_), _) => {
                eprintln!("Error: Invalid VID format. Use hex format (e.g., 046d or 0x046d)");
                std::process::exit(1);
            }
            (_, Some(Err(_))) => {
                eprintln!("Error: Invalid PID format. Use hex format (e.g., 046d or 0x046d) or '-' for any");
                std::process::exit(1);
            }
        }
    }

    if args.iter().any(|arg| arg == "--settings") {
        info!("Starting HWiNFO-SteelSeries Settings GUI");
        let term = Term::stdout();
        let hwinfo = match connect_hwinfo(&term) {
            Ok(mut hw) => {
                let _ = hw.pull();
                Some(hw)
            }
            Err(_) => None,
        };
        if let Ok(cwd) = std::env::current_dir() {
            info!("Current directory (CWD): {:?}", cwd);
            let absolute_path = cwd.join("conf.ini");
            info!("Trying to load config from: {:?}", absolute_path);
        }
        let ini = match Ini::load_from_file("conf.ini") {
            Ok(ini) => {
                info!("Successfully loaded conf.ini for settings");
                ini
            }
            Err(e) => {
                warn!(
                    "conf.ini not found or failed to load: {}. Using empty ini.",
                    e
                );
                Ini::new()
            }
        };
        let _config = AppConfig::from_ini(&ini).unwrap_or(AppConfig {
            is_summary: true,
            is_vertical: true,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            custom_sensors: vec![vec![]],
        });

        return Ok(crate::gui::run_settings(ini, hwinfo));
    }

    info!("Starting HWiNFO-SteelSeries application");

    // Create the winit event loop
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| {
            error!("Failed to create event loop: {}", e);
            anyhow::anyhow!("Failed to create event loop: {}", e)
        })?;

    // Set control flow to Wait (low CPU usage, event-driven)
    event_loop.set_control_flow(ControlFlow::Wait);

    // Spawn timer thread to trigger display updates
    let proxy = event_loop.create_proxy();
    thread::spawn(move || {
        info!("Update timer thread started");
        loop {
            thread::sleep(Duration::from_millis(TICK_RATE));
            if proxy.send_event(UserEvent::UpdateDisplay).is_err() {
                info!("Event loop closed, exiting timer thread");
                break;
            }
        }
    });

    // Create application state with event loop proxy
    let proxy_for_app = event_loop.create_proxy();
    let mut app = App::new(Term::stdout(), proxy_for_app);

    // Run the event loop
    info!("Starting winit event loop");
    event_loop.run_app(&mut app).map_err(|e| {
        error!("Event loop error: {}", e);
        anyhow::anyhow!("Event loop error: {}", e)
    })?;

    info!("Application exited successfully");
    Ok(())
}

// OledClient and other logic continues...

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_sensors() -> SummarySensors {
        SummarySensors {
            cpu_temp: 65.5,
            cpu_usage: 45.2,
            gpu_temp: 72.8,
            gpu_usage: 88.9,
            mem_used: 16.3,
            mem_free: 15.7,
            mem_load: 50.9,
        }
    }

    #[test]
    fn test_format_vertical_summary_with_decimal() {
        let sensors = create_test_sensors();
        let result = format_vertical_summary(&sensors, true);

        assert_eq!(result["line1"], "CPU   GPU   MEM");
        assert_eq!(result["line2"], "65.5° 72.8° 16.3G");
        assert_eq!(result["line3"], "45.2% 88.9% 15.7G");
    }

    #[test]
    fn test_format_vertical_summary_without_decimal() {
        let sensors = create_test_sensors();
        let result = format_vertical_summary(&sensors, false);

        assert_eq!(result["line1"], "CPU   GPU   MEM");
        assert_eq!(result["line2"], "66°   73°   16G");
        assert_eq!(result["line3"], "45%    89%    16G");
    }

    #[test]
    fn test_format_horizontal_summary_with_decimal() {
        let sensors = create_test_sensors();
        let result = format_horizontal_summary(&sensors, true);

        assert_eq!(result["line1"], "CPU 65.5° 45.2%");
        assert_eq!(result["line2"], "GPU 72.8° 88.9%");
        assert_eq!(result["line3"], "MEM 16.3G 50.9%");
    }

    #[test]
    fn test_format_horizontal_summary_without_decimal() {
        let sensors = create_test_sensors();
        let result = format_horizontal_summary(&sensors, false);

        assert_eq!(result["line1"], "CPU 66° 45%");
        assert_eq!(result["line2"], "GPU 73° 89%");
        assert_eq!(result["line3"], "MEM 16G 51%");
    }

    // Note: check_hwinfo_connection is tested indirectly through the equality
    // implementation tested in lib.rs. The logic is simple: if hwinfo structs
    // are equal (unchanged readings), increment disconnect_count, else reset to 0.
    // Since we can't easily create mock Hwinfo in main.rs tests due to private
    // fields, we rely on lib.rs tests for Hwinfo equality verification.
}
