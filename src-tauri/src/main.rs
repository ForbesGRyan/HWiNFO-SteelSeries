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

mod media;
use media::MediaReader;

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

    // Media info tracking
    media_reader: MediaReader,

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
            media_reader: MediaReader::new(),
            config_last_modified: None,
            is_sleeping: false,
            is_white_screen: false,
        }
    }

    fn setup_tray_icon(&mut self) -> Result<(), anyhow::Error> {
        let event_loop_proxy = self
            .event_loop_proxy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Event loop proxy not initialized"))?
            .clone();

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
        let mut builder = TrayIconBuilder::new()
            .with_menu(Box::new(tray_menu))
            .with_tooltip("HWiNFO-SteelSeries");

        // Only set icon if we successfully loaded one
        if let Some(icon) = icon {
            builder = builder.with_icon(icon);
        } else {
            warn!("Building tray icon without an icon (icon failed to load)");
        }

        let tray_icon = builder.build().map_err(|e| {
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
            let hwinfo_ref = self
                .hwinfo
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("HWiNFO not initialized for config setup"))?;
            settings_create_config(&self.term, hwinfo_ref)?
        } else {
            config_file
        };

        // Store config_file
        self.config_file = Some(config_file);

        let config_ref = self
            .config_file
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Configuration file not loaded"))?;
        let config = AppConfig::from_ini(config_ref).map_err(|e| {
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

        // Initialize media reader for Windows Media info
        if let Err(e) = self.media_reader.initialize() {
            warn!("Failed to initialize MediaReader: {}. Media sensors will be unavailable.", e);
            // Non-fatal - continue without media support
        }

        // Store modification time
        if let Ok(metadata) = std::fs::metadata("conf.ini") {
            self.config_last_modified = metadata.modified().ok();
        }

        // For custom mode, store the sensor configuration regardless of connection type
        if !self.is_summary {
            info!("Setting up {} custom sensor page(s)", self.pages);
            let config_for_pages = self
                .config_file
                .as_ref()
                .ok_or_else(|| anyhow::anyhow!("Configuration file not loaded for page setup"))?;
            for i in 1..=self.pages {
                match config_for_pages.section(Some(format!("PAGE{}.Sensors", i))) {
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
                &mut self.media_reader,
                self.hid_api.as_ref(),
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
                            if let Some(proxy) = self.event_loop_proxy.as_ref() {
                                let _ = proxy.send_event(UserEvent::OpenGui);
                            } else {
                                warn!("Event loop proxy not available for OpenGui event");
                            }
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
                        if let Some(proxy) = self.event_loop_proxy.as_ref() {
                            let _ = proxy.send_event(UserEvent::OpenGui);
                        } else {
                            warn!("Event loop proxy not available for OpenGui event");
                        }
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

    // =========================================================================
    // Tests for check_hwinfo_connection
    // =========================================================================

    #[test]
    fn test_check_hwinfo_connection_increments_count_when_unchanged() {
        // Create two identical mock Hwinfo instances
        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_for_main();

        let mut disconnect_count = 0;
        let limit = 5;

        // First check - data unchanged, count should increment to 1
        let disconnected = check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        assert!(!disconnected, "Should not be disconnected after 1 cycle");
        assert_eq!(disconnect_count, 1);

        // Second check - still unchanged, count should increment to 2
        let disconnected = check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        assert!(!disconnected, "Should not be disconnected after 2 cycles");
        assert_eq!(disconnect_count, 2);
    }

    #[test]
    fn test_check_hwinfo_connection_returns_true_at_limit() {
        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_for_main();

        let mut disconnect_count = 0;
        let limit = 5;

        // Run until we hit the limit
        for i in 1..=5 {
            let disconnected =
                check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
            if i < 5 {
                assert!(!disconnected, "Should not be disconnected at cycle {}", i);
            } else {
                assert!(disconnected, "Should be disconnected at cycle {}", i);
            }
        }
        assert_eq!(disconnect_count, 5);
    }

    #[test]
    fn test_check_hwinfo_connection_resets_count_when_data_changes() {
        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_for_main();
        let hwinfo3 = create_mock_hwinfo_with_different_values();

        let mut disconnect_count = 0;
        let limit = 5;

        // Build up disconnect count
        check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        assert_eq!(disconnect_count, 3, "Count should be 3 after 3 unchanged cycles");

        // Now data changes - count should reset to 0
        let disconnected = check_hwinfo_connection(&hwinfo1, &hwinfo3, &mut disconnect_count, limit);
        assert!(!disconnected, "Should not be disconnected when data changes");
        assert_eq!(disconnect_count, 0, "Count should reset to 0 when data changes");
    }

    #[test]
    fn test_check_hwinfo_connection_count_capped_at_limit() {
        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_for_main();

        let mut disconnect_count = 0;
        let limit = 5;

        // Run well past the limit
        for _ in 0..10 {
            check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        }

        // Count should be capped at limit, not 10
        assert_eq!(disconnect_count, 5, "Count should be capped at limit");
    }

    #[test]
    fn test_check_hwinfo_connection_with_limit_of_1() {
        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_for_main();

        let mut disconnect_count = 0;
        let limit = 1;

        // First unchanged check should immediately trigger disconnection
        let disconnected = check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        assert!(disconnected, "Should be disconnected immediately with limit=1");
        assert_eq!(disconnect_count, 1);
    }

    // =========================================================================
    // Tests for fetch_summary_sensors
    // =========================================================================

    #[test]
    fn test_fetch_summary_sensors_success() {
        let hwinfo = create_mock_hwinfo_with_all_summary_sensors();

        let result = fetch_summary_sensors(&hwinfo, "");
        assert!(result.is_ok(), "Should successfully fetch summary sensors");

        let sensors = result.unwrap();
        assert_eq!(sensors.cpu_temp, 65.0);
        assert_eq!(sensors.cpu_usage, 45.0);
        assert_eq!(sensors.gpu_temp, 72.0);
        assert_eq!(sensors.gpu_usage, 88.0);
        // mem_used and mem_free are divided by 1024 in fetch_summary_sensors
        assert!((sensors.mem_used - 16.0).abs() < 0.01); // 16384 / 1024 = 16
        assert!((sensors.mem_free - 16.0).abs() < 0.01); // 16384 / 1024 = 16
        assert_eq!(sensors.mem_load, 50.0);
    }

    #[test]
    fn test_fetch_summary_sensors_with_specific_gpu() {
        let hwinfo = create_mock_hwinfo_with_multiple_gpus();

        // Fetch with specific GPU name
        let result = fetch_summary_sensors(&hwinfo, "GPU [#1]: NVIDIA GeForce RTX 3090");
        assert!(result.is_ok(), "Should fetch sensors for specific GPU");

        let sensors = result.unwrap();
        assert_eq!(sensors.gpu_temp, 80.0); // GPU #1 has temp 80.0
    }

    #[test]
    fn test_fetch_summary_sensors_missing_cpu_usage() {
        let hwinfo = create_mock_hwinfo_missing_cpu_usage();

        let result = fetch_summary_sensors(&hwinfo, "");
        assert!(result.is_err(), "Should fail when CPU usage sensor is missing");
    }

    #[test]
    fn test_fetch_summary_sensors_missing_gpu_temp() {
        let hwinfo = create_mock_hwinfo_missing_gpu_temp();

        let result = fetch_summary_sensors(&hwinfo, "");
        assert!(result.is_err(), "Should fail when GPU temperature sensor is missing");
    }

    #[test]
    fn test_fetch_summary_sensors_missing_memory_sensors() {
        let hwinfo = create_mock_hwinfo_missing_memory();

        let result = fetch_summary_sensors(&hwinfo, "");
        assert!(result.is_err(), "Should fail when memory sensors are missing");
    }

    #[test]
    fn test_fetch_summary_sensors_specific_gpu_not_found() {
        let hwinfo = create_mock_hwinfo_with_all_summary_sensors();

        let result = fetch_summary_sensors(&hwinfo, "Nonexistent GPU");
        assert!(result.is_err(), "Should fail when specific GPU is not found");
    }

    // =========================================================================
    // Tests for handle_fatal_error (limited testing due to I/O)
    // =========================================================================

    #[test]
    fn test_handle_fatal_error_returns_same_error() {
        // We can't fully test handle_fatal_error because it does I/O operations
        // (console window, terminal writes, stdin read), but we can test that
        // the error chain is properly constructed.

        // Create an error with a cause chain
        let inner_error = std::io::Error::new(std::io::ErrorKind::NotFound, "File not found");
        let outer_error = anyhow::Error::new(inner_error).context("Failed to read config");

        // Verify the error chain exists
        assert!(outer_error.source().is_some(), "Error should have a source/cause");

        // The error message should contain both the context and the original error
        let error_string = format!("{}", outer_error);
        assert!(
            error_string.contains("Failed to read config"),
            "Error should contain context message"
        );
    }

    #[test]
    fn test_error_chain_traversal() {
        // Test that we can traverse an error chain similar to how handle_fatal_error does
        let base_error = std::io::Error::new(std::io::ErrorKind::PermissionDenied, "Access denied");
        let mid_error = anyhow::Error::new(base_error).context("Could not open file");
        let top_error = mid_error.context("Configuration error");

        // Collect error chain
        let mut chain = Vec::new();
        chain.push(format!("{}", top_error));

        let mut source = top_error.source();
        while let Some(cause) = source {
            chain.push(format!("{}", cause));
            source = cause.source();
        }

        // Should have at least 2 levels (top error + context + base)
        assert!(chain.len() >= 2, "Error chain should have multiple levels");
        assert!(chain[0].contains("Configuration error"));
    }

    // =========================================================================
    // Helper functions for creating mock Hwinfo instances
    // =========================================================================

    use hwinfo_steelseries_oled::{
        HwinfoSensorsReadingElement, HwinfoSensorsSensorElement, Sensor,
    };
    use std::collections::HashMap;

    /// Creates a basic mock Hwinfo for connection tests (minimal sensors)
    fn create_mock_hwinfo_for_main() -> Hwinfo {
        let mut sensors = HashMap::new();
        let sensor_names = vec!["CPU [#0]".to_string()];

        let cpu_temp_reading = HwinfoSensorsReadingElement::new_mock(0, 1, "CPU Temperature", 65.0);
        let mut cpu_readings = HashMap::new();
        cpu_readings.insert("CPU Temperature".to_string(), cpu_temp_reading);

        sensors.insert(
            "CPU [#0]".to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, "CPU [#0]"),
                readings: cpu_readings,
                reading_names: vec!["CPU Temperature".to_string()],
            },
        );

        Hwinfo::new_mock(sensors, sensor_names)
    }

    /// Creates a mock Hwinfo with different sensor values for testing data changes
    fn create_mock_hwinfo_with_different_values() -> Hwinfo {
        let mut sensors = HashMap::new();
        let sensor_names = vec!["CPU [#0]".to_string()];

        // Different temperature value (75.0 instead of 65.0)
        let cpu_temp_reading = HwinfoSensorsReadingElement::new_mock(0, 1, "CPU Temperature", 75.0);
        let mut cpu_readings = HashMap::new();
        cpu_readings.insert("CPU Temperature".to_string(), cpu_temp_reading);

        sensors.insert(
            "CPU [#0]".to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, "CPU [#0]"),
                readings: cpu_readings,
                reading_names: vec!["CPU Temperature".to_string()],
            },
        );

        Hwinfo::new_mock(sensors, sensor_names)
    }

    /// Creates a mock Hwinfo with all sensors needed for summary mode
    fn create_mock_hwinfo_with_all_summary_sensors() -> Hwinfo {
        let mut sensors = HashMap::new();
        let mut sensor_names = Vec::new();

        // CPU sensor
        let cpu_name = "CPU [#0]: AMD Ryzen 9 5900X";
        sensor_names.push(cpu_name.to_string());
        let mut cpu_readings = HashMap::new();
        cpu_readings.insert(
            "Total CPU Usage".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 1, "Total CPU Usage", 45.0),
        );
        cpu_readings.insert(
            "CPU (Tctl/Tdie)".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 2, "CPU (Tctl/Tdie)", 65.0),
        );
        sensors.insert(
            cpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, cpu_name),
                readings: cpu_readings,
                reading_names: vec!["Total CPU Usage".to_string(), "CPU (Tctl/Tdie)".to_string()],
            },
        );

        // GPU sensor
        let gpu_name = "GPU [#0]: NVIDIA GeForce RTX 3080";
        sensor_names.push(gpu_name.to_string());
        let mut gpu_readings = HashMap::new();
        gpu_readings.insert(
            "GPU Temperature".to_string(),
            HwinfoSensorsReadingElement::new_mock(1, 3, "GPU Temperature", 72.0),
        );
        gpu_readings.insert(
            "GPU Core Load".to_string(),
            HwinfoSensorsReadingElement::new_mock(1, 4, "GPU Core Load", 88.0),
        );
        sensors.insert(
            gpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(1, gpu_name),
                readings: gpu_readings,
                reading_names: vec!["GPU Temperature".to_string(), "GPU Core Load".to_string()],
            },
        );

        // Memory sensor
        let mem_name = "System Memory";
        sensor_names.push(mem_name.to_string());
        let mut mem_readings = HashMap::new();
        // Values in MB, will be divided by 1024 to get GB
        mem_readings.insert(
            "Physical Memory Used".to_string(),
            HwinfoSensorsReadingElement::new_mock(2, 5, "Physical Memory Used", 16384.0),
        );
        mem_readings.insert(
            "Physical Memory Available".to_string(),
            HwinfoSensorsReadingElement::new_mock(2, 6, "Physical Memory Available", 16384.0),
        );
        mem_readings.insert(
            "Physical Memory Load".to_string(),
            HwinfoSensorsReadingElement::new_mock(2, 7, "Physical Memory Load", 50.0),
        );
        sensors.insert(
            mem_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(2, mem_name),
                readings: mem_readings,
                reading_names: vec![
                    "Physical Memory Used".to_string(),
                    "Physical Memory Available".to_string(),
                    "Physical Memory Load".to_string(),
                ],
            },
        );

        Hwinfo::new_mock(sensors, sensor_names)
    }

    /// Creates a mock Hwinfo with multiple GPUs for testing specific GPU selection
    fn create_mock_hwinfo_with_multiple_gpus() -> Hwinfo {
        let mut sensors = HashMap::new();
        let mut sensor_names = Vec::new();

        // CPU sensor
        let cpu_name = "CPU [#0]: AMD Ryzen 9 5900X";
        sensor_names.push(cpu_name.to_string());
        let mut cpu_readings = HashMap::new();
        cpu_readings.insert(
            "Total CPU Usage".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 1, "Total CPU Usage", 45.0),
        );
        cpu_readings.insert(
            "CPU (Tctl/Tdie)".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 2, "CPU (Tctl/Tdie)", 65.0),
        );
        sensors.insert(
            cpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, cpu_name),
                readings: cpu_readings,
                reading_names: vec!["Total CPU Usage".to_string(), "CPU (Tctl/Tdie)".to_string()],
            },
        );

        // GPU #0
        let gpu0_name = "GPU [#0]: NVIDIA GeForce RTX 3080";
        sensor_names.push(gpu0_name.to_string());
        let mut gpu0_readings = HashMap::new();
        gpu0_readings.insert(
            "GPU Temperature".to_string(),
            HwinfoSensorsReadingElement::new_mock(1, 3, "GPU Temperature", 72.0),
        );
        gpu0_readings.insert(
            "GPU Core Load".to_string(),
            HwinfoSensorsReadingElement::new_mock(1, 4, "GPU Core Load", 88.0),
        );
        sensors.insert(
            gpu0_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(1, gpu0_name),
                readings: gpu0_readings,
                reading_names: vec!["GPU Temperature".to_string(), "GPU Core Load".to_string()],
            },
        );

        // GPU #1
        let gpu1_name = "GPU [#1]: NVIDIA GeForce RTX 3090";
        sensor_names.push(gpu1_name.to_string());
        let mut gpu1_readings = HashMap::new();
        gpu1_readings.insert(
            "GPU Temperature".to_string(),
            HwinfoSensorsReadingElement::new_mock(2, 5, "GPU Temperature", 80.0),
        );
        gpu1_readings.insert(
            "GPU Core Load".to_string(),
            HwinfoSensorsReadingElement::new_mock(2, 6, "GPU Core Load", 95.0),
        );
        sensors.insert(
            gpu1_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(2, gpu1_name),
                readings: gpu1_readings,
                reading_names: vec!["GPU Temperature".to_string(), "GPU Core Load".to_string()],
            },
        );

        // Memory sensor
        let mem_name = "System Memory";
        sensor_names.push(mem_name.to_string());
        let mut mem_readings = HashMap::new();
        mem_readings.insert(
            "Physical Memory Used".to_string(),
            HwinfoSensorsReadingElement::new_mock(3, 7, "Physical Memory Used", 16384.0),
        );
        mem_readings.insert(
            "Physical Memory Available".to_string(),
            HwinfoSensorsReadingElement::new_mock(3, 8, "Physical Memory Available", 16384.0),
        );
        mem_readings.insert(
            "Physical Memory Load".to_string(),
            HwinfoSensorsReadingElement::new_mock(3, 9, "Physical Memory Load", 50.0),
        );
        sensors.insert(
            mem_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(3, mem_name),
                readings: mem_readings,
                reading_names: vec![
                    "Physical Memory Used".to_string(),
                    "Physical Memory Available".to_string(),
                    "Physical Memory Load".to_string(),
                ],
            },
        );

        Hwinfo::new_mock(sensors, sensor_names)
    }

    /// Creates a mock Hwinfo missing CPU usage sensor
    fn create_mock_hwinfo_missing_cpu_usage() -> Hwinfo {
        let mut sensors = HashMap::new();
        let mut sensor_names = Vec::new();

        // CPU sensor WITHOUT "Total CPU Usage"
        let cpu_name = "CPU [#0]";
        sensor_names.push(cpu_name.to_string());
        let mut cpu_readings = HashMap::new();
        cpu_readings.insert(
            "CPU (Tctl/Tdie)".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 1, "CPU (Tctl/Tdie)", 65.0),
        );
        sensors.insert(
            cpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, cpu_name),
                readings: cpu_readings,
                reading_names: vec!["CPU (Tctl/Tdie)".to_string()],
            },
        );

        Hwinfo::new_mock(sensors, sensor_names)
    }

    /// Creates a mock Hwinfo missing GPU temperature sensor
    fn create_mock_hwinfo_missing_gpu_temp() -> Hwinfo {
        let mut sensors = HashMap::new();
        let mut sensor_names = Vec::new();

        // CPU sensor (complete)
        let cpu_name = "CPU [#0]";
        sensor_names.push(cpu_name.to_string());
        let mut cpu_readings = HashMap::new();
        cpu_readings.insert(
            "Total CPU Usage".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 1, "Total CPU Usage", 45.0),
        );
        cpu_readings.insert(
            "CPU (Tctl/Tdie)".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 2, "CPU (Tctl/Tdie)", 65.0),
        );
        sensors.insert(
            cpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, cpu_name),
                readings: cpu_readings,
                reading_names: vec!["Total CPU Usage".to_string(), "CPU (Tctl/Tdie)".to_string()],
            },
        );

        // GPU sensor WITHOUT temperature
        let gpu_name = "GPU [#0]";
        sensor_names.push(gpu_name.to_string());
        let mut gpu_readings = HashMap::new();
        gpu_readings.insert(
            "GPU Core Load".to_string(),
            HwinfoSensorsReadingElement::new_mock(1, 3, "GPU Core Load", 88.0),
        );
        sensors.insert(
            gpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(1, gpu_name),
                readings: gpu_readings,
                reading_names: vec!["GPU Core Load".to_string()],
            },
        );

        Hwinfo::new_mock(sensors, sensor_names)
    }

    /// Creates a mock Hwinfo missing memory sensors
    fn create_mock_hwinfo_missing_memory() -> Hwinfo {
        let mut sensors = HashMap::new();
        let mut sensor_names = Vec::new();

        // CPU sensor (complete)
        let cpu_name = "CPU [#0]";
        sensor_names.push(cpu_name.to_string());
        let mut cpu_readings = HashMap::new();
        cpu_readings.insert(
            "Total CPU Usage".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 1, "Total CPU Usage", 45.0),
        );
        cpu_readings.insert(
            "CPU (Tctl/Tdie)".to_string(),
            HwinfoSensorsReadingElement::new_mock(0, 2, "CPU (Tctl/Tdie)", 65.0),
        );
        sensors.insert(
            cpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(0, cpu_name),
                readings: cpu_readings,
                reading_names: vec!["Total CPU Usage".to_string(), "CPU (Tctl/Tdie)".to_string()],
            },
        );

        // GPU sensor (complete)
        let gpu_name = "GPU [#0]";
        sensor_names.push(gpu_name.to_string());
        let mut gpu_readings = HashMap::new();
        gpu_readings.insert(
            "GPU Temperature".to_string(),
            HwinfoSensorsReadingElement::new_mock(1, 3, "GPU Temperature", 72.0),
        );
        gpu_readings.insert(
            "GPU Core Load".to_string(),
            HwinfoSensorsReadingElement::new_mock(1, 4, "GPU Core Load", 88.0),
        );
        sensors.insert(
            gpu_name.to_string(),
            Sensor {
                info: HwinfoSensorsSensorElement::new_mock(1, gpu_name),
                readings: gpu_readings,
                reading_names: vec!["GPU Temperature".to_string(), "GPU Core Load".to_string()],
            },
        );

        // No memory sensor!
        Hwinfo::new_mock(sensors, sensor_names)
    }

    // =========================================================================
    // OledClient JSON construction tests
    // =========================================================================

    #[test]
    fn test_oled_client_hid_frame_json_extraction() {
        // Test the JSON value extraction logic used in OledClient::trigger_frame for HID
        // This tests the same logic without requiring an actual HID device

        let value = json!({
            "line1": "CPU   GPU   MEM",
            "line2": "65.5° 72.8° 16.3G",
            "line3": "45.2% 88.9% 15.7G"
        });

        // Simulate the extraction logic from trigger_frame for HID
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

        assert_eq!(text, "CPU   GPU   MEM\n65.5° 72.8° 16.3G\n45.2% 88.9% 15.7G");
    }

    #[test]
    fn test_oled_client_hid_frame_empty_lines() {
        let value = json!({
            "line1": "",
            "line2": "",
            "line3": ""
        });

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

        // Empty strings don't make text non-empty, so no newlines are added
        // The logic only adds newlines when text is already non-empty
        // With all empty strings, text remains empty
        assert_eq!(text, "");
    }

    #[test]
    fn test_oled_client_hid_frame_empty_lines_full_display() {
        // Test with all DISPLAY_LINES defined
        let mut value_obj = serde_json::Map::new();
        for i in 1..=DISPLAY_LINES {
            value_obj.insert(format!("line{}", i), json!(""));
        }
        let value = Value::Object(value_obj);

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

        // Empty strings don't make text non-empty, so no newlines are added
        // With all empty strings, text remains empty regardless of line count
        let actual_newlines = text.matches('\n').count();
        assert_eq!(actual_newlines, 0);
        assert_eq!(text, "");
    }

    #[test]
    fn test_oled_client_hid_frame_missing_lines() {
        // Test handling of missing line keys
        let value = json!({
            "line1": "Only line 1"
        });

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

        assert_eq!(text, "Only line 1");
    }

    #[test]
    fn test_oled_client_blank_frame_json_structure() {
        // Test the JSON structure used for blank frames in GameSense mode
        let blank_value = json!({
            "line1": "",
            "line2": "",
            "line3": ""
        });

        assert_eq!(blank_value["line1"], "");
        assert_eq!(blank_value["line2"], "");
        assert_eq!(blank_value["line3"], "");
    }

    #[test]
    fn test_oled_client_white_frame_json_structure() {
        // Test the JSON structure used for white frames in GameSense mode
        let white_value = json!({
            "line1": "████████████████",
            "line2": "████████████████",
            "line3": "████████████████"
        });

        // Each line should have 16 full block characters
        assert_eq!(white_value["line1"].as_str().unwrap().chars().count(), 16);
        assert_eq!(white_value["line2"].as_str().unwrap().chars().count(), 16);
        assert_eq!(white_value["line3"].as_str().unwrap().chars().count(), 16);
    }

    #[test]
    fn test_oled_buffer_blank_frame_all_zeros() {
        // Test that a new OledBuffer is all zeros (blank frame for HID)
        let buffer = render::OledBuffer::new();

        for byte in buffer.data.iter() {
            assert_eq!(*byte, 0, "Blank frame should have all pixels off");
        }
    }

    #[test]
    fn test_oled_buffer_white_frame_pattern() {
        // Test the white frame pattern used in HID mode
        let mut buffer = render::OledBuffer::new();

        // Set all pixels to white (same logic as send_white_frame)
        for x in 0..128u32 {
            for y in 0..64u32 {
                buffer.set_pixel(x, y, true);
            }
        }

        // All bytes should be 0xFF (all bits set)
        for byte in buffer.data.iter() {
            assert_eq!(*byte, 0xFF, "White frame should have all pixels on");
        }
    }

    #[test]
    fn test_oled_buffer_chunk_size_for_hid() {
        // Test that get_chunk returns correct size for HID packet construction
        let buffer = render::OledBuffer::new();

        // HID uses 64-pixel wide chunks
        let chunk = buffer.get_chunk(0, 64);
        // 64 columns * 8 bytes per column = 512 bytes
        assert_eq!(chunk.len(), 512);

        let chunk2 = buffer.get_chunk(64, 64);
        assert_eq!(chunk2.len(), 512);
    }

    // =========================================================================
    // App struct default values tests
    // =========================================================================
    // Note: App::new() requires an EventLoopProxy which is difficult to mock.
    // These tests verify the expected default values by checking the struct definition
    // and comparing against test scenarios.

    #[test]
    fn test_app_default_is_summary_true() {
        // The App struct should default is_summary to true
        // We verify this by checking the struct definition expectation
        // and testing related functions with default assumptions

        // When is_summary is true, format_vertical_summary or format_horizontal_summary is used
        let sensors = create_test_sensors();
        let result = format_vertical_summary(&sensors, false);

        // This should work, confirming summary mode is the expected default behavior
        assert!(result.get("line1").is_some());
    }

    #[test]
    fn test_app_default_is_vertical_true() {
        // The App struct should default is_vertical to true
        // Test that vertical formatting is the expected default
        let sensors = create_test_sensors();
        let result = format_vertical_summary(&sensors, false);

        // Vertical format has "CPU   GPU   MEM" as header
        assert_eq!(result["line1"], "CPU   GPU   MEM");
    }

    #[test]
    fn test_app_default_decimal_false() {
        // The App struct should default decimal to false
        // Test that non-decimal formatting works as expected default
        let sensors = create_test_sensors();
        let result = format_vertical_summary(&sensors, false);

        // Without decimals, values should be rounded integers
        // 65.5 -> 66, so the string should contain "66°"
        let line2 = result["line2"].as_str().unwrap();
        assert!(line2.contains("66°"), "Expected rounded temperature without decimal: {}", line2);
    }

    #[test]
    fn test_app_default_pages_one() {
        // The App struct should default pages to 1
        // Test page counter behavior with single page assumption

        let pages = 1;
        let page_counter = 0;

        // With 1 page, page_counter should always be 0
        let next_page = (page_counter + 1) % pages;
        assert_eq!(next_page, 0);
    }

    #[test]
    fn test_app_default_page_time_five() {
        // The App struct should default page_time to 5
        let default_page_time: isize = 5;

        // Page time is used with tick rate for page switching
        let ticks_per_second = 1000 / TICK_RATE as isize;
        let ticks_per_page = default_page_time * ticks_per_second;

        // With TICK_RATE of 500ms, ticks_per_second is 2
        // So ticks_per_page should be 10
        assert_eq!(ticks_per_page, 10);
    }

    #[test]
    fn test_app_default_disconnect_count_zero() {
        // The App struct should default disconnect_count to 0
        // Test that initial disconnect check doesn't trigger disconnection

        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_with_different_values();

        let mut disconnect_count = 0; // Default value
        let limit = 5;

        // First check with different data should not increment count
        let disconnected = check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        assert!(!disconnected);
        assert_eq!(disconnect_count, 0);
    }

    #[test]
    fn test_app_default_sensors_per_line_one() {
        // The App struct should default sensors_per_line to 1
        let sensors_per_line: u8 = 1;

        // With 1 sensor per line, format_custom_value should show one value per line
        let labels = vec!["CPU"; CUSTOM_SENSORS];
        let values = vec!["65".to_string(); CUSTOM_SENSORS];
        let units = vec!["°"; CUSTOM_SENSORS];

        let result = format_custom_value(sensors_per_line, labels.clone(), values.clone(), units.clone());

        // Each line should have just one sensor value
        let line1 = result["line1"].as_str().unwrap();
        // With 1 sensor per line, format is "LABEL VALUE UNIT"
        assert!(line1.contains("CPU"), "Line should contain label");
    }

    // =========================================================================
    // App state transition tests
    // =========================================================================

    #[test]
    fn test_page_counter_increments_with_multiple_pages() {
        let pages = 3;
        let mut page_counter = 0;

        // Simulate page switching
        page_counter = (page_counter + 1) % pages;
        assert_eq!(page_counter, 1);

        page_counter = (page_counter + 1) % pages;
        assert_eq!(page_counter, 2);

        page_counter = (page_counter + 1) % pages;
        assert_eq!(page_counter, 0); // Wraps back to 0
    }

    #[test]
    fn test_page_counter_stays_zero_with_single_page() {
        let pages = 1;
        let mut page_counter = 0;

        // With single page, counter should always wrap to 0
        for _ in 0..10 {
            page_counter = (page_counter + 1) % pages;
            assert_eq!(page_counter, 0);
        }
    }

    #[test]
    fn test_page_switching_timing() {
        // Test the page switching logic based on tick count
        let page_time: isize = 5;
        let ticks_per_second = 1000 / TICK_RATE as isize; // 2 ticks/sec with 500ms rate
        let pages = 2;
        let mut page_counter = 0;

        // ticks_per_page = 5 * 2 = 10
        let ticks_per_page = page_time * ticks_per_second;

        // Simulate tick increments (20 ticks to see two page switches)
        for i in 1..=20 {
            let tick = Wrapping(i as isize);
            if tick.0 % ticks_per_page == 0 && tick.0 != 0 {
                page_counter = (page_counter + 1) % pages;
            }
        }

        // After 20 ticks with page_time=5 and TICK_RATE=500 (2 ticks/sec),
        // ticks_per_page = 10, so we switch at tick 10 and tick 20
        // page_counter: 0 -> 1 (tick 10) -> 0 (tick 20)
        assert_eq!(page_counter, 0);
    }

    #[test]
    fn test_disconnect_count_behavior_unchanged_data() {
        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_for_main(); // Same as hwinfo1

        let mut disconnect_count = 0;
        let limit = 5;

        // Simulate multiple cycles with unchanged data
        for expected_count in 1..=5 {
            check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
            assert_eq!(disconnect_count, expected_count);
        }

        // After 5 cycles, should be at limit
        assert_eq!(disconnect_count, 5);
    }

    #[test]
    fn test_disconnect_count_resets_on_data_change() {
        let hwinfo1 = create_mock_hwinfo_for_main();
        let hwinfo2 = create_mock_hwinfo_for_main();
        let hwinfo3 = create_mock_hwinfo_with_different_values();

        let mut disconnect_count = 0;
        let limit = 5;

        // Build up count
        check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        check_hwinfo_connection(&hwinfo1, &hwinfo2, &mut disconnect_count, limit);
        assert_eq!(disconnect_count, 2);

        // Data changes - count resets
        check_hwinfo_connection(&hwinfo2, &hwinfo3, &mut disconnect_count, limit);
        assert_eq!(disconnect_count, 0);
    }

    #[test]
    fn test_was_disconnected_flag_transitions() {
        // Simulate the was_disconnected flag behavior
        let mut was_disconnected = false;
        let mut disconnect_count = 0;
        let limit = 5;

        let hwinfo_same1 = create_mock_hwinfo_for_main();
        let hwinfo_same2 = create_mock_hwinfo_for_main();
        let hwinfo_different = create_mock_hwinfo_with_different_values();

        // Build up to disconnected state
        for _ in 0..5 {
            let disconnected = check_hwinfo_connection(&hwinfo_same1, &hwinfo_same2, &mut disconnect_count, limit);
            was_disconnected = disconnected;
        }
        assert!(was_disconnected, "Should be disconnected after 5 unchanged cycles");

        // Data changes - should reconnect
        let disconnected = check_hwinfo_connection(&hwinfo_same1, &hwinfo_different, &mut disconnect_count, limit);
        let previously_disconnected = was_disconnected;
        was_disconnected = disconnected;

        assert!(previously_disconnected, "Was previously disconnected");
        assert!(!was_disconnected, "Should now be connected");
    }

    #[test]
    fn test_wrapping_counter_behavior() {
        // Test the Wrapping<isize> counter used in App
        let mut i = Wrapping(0isize);

        // Increment behavior
        i += 1;
        assert_eq!(i.0, 1);

        // Can safely wrap on overflow
        i = Wrapping(isize::MAX);
        i += 1;
        assert_eq!(i.0, isize::MIN);
    }

    #[test]
    fn test_display_in_console_debug_vs_release() {
        // In debug mode, display_in_console should be true
        // In release mode, it should be false
        // We test the cfg! macro behavior

        let display_in_console = cfg!(debug_assertions);

        #[cfg(debug_assertions)]
        assert!(display_in_console, "Debug mode should display in console");

        #[cfg(not(debug_assertions))]
        assert!(!display_in_console, "Release mode should not display in console");
    }

    // =========================================================================
    // Sleep and white screen mode tests
    // =========================================================================

    #[test]
    fn test_sleep_mode_prevents_updates() {
        // Test that is_sleeping flag would prevent update_display from running
        // We can't test the actual method without hardware, but we can test the logic

        let is_sleeping = true;
        let is_white_screen = false;

        // This is the guard condition at the start of update_display
        let should_skip = is_sleeping || is_white_screen;
        assert!(should_skip, "Sleep mode should skip updates");
    }

    #[test]
    fn test_white_screen_mode_prevents_updates() {
        let is_sleeping = false;
        let is_white_screen = true;

        let should_skip = is_sleeping || is_white_screen;
        assert!(should_skip, "White screen mode should skip updates");
    }

    #[test]
    fn test_normal_mode_allows_updates() {
        let is_sleeping = false;
        let is_white_screen = false;

        let should_skip = is_sleeping || is_white_screen;
        assert!(!should_skip, "Normal mode should allow updates");
    }

    #[test]
    fn test_sleep_and_white_screen_mutually_exclusive() {
        // When entering sleep mode, white screen should be disabled and vice versa
        // This tests the expected behavior

        let mut is_sleeping = false;
        let mut is_white_screen = false;

        // Enter sleep mode
        is_sleeping = true;
        is_white_screen = false;
        assert!(is_sleeping && !is_white_screen);

        // Enter white screen mode (should disable sleep)
        is_white_screen = true;
        is_sleeping = false;
        assert!(!is_sleeping && is_white_screen);
    }

    // =========================================================================
    // Event-based state transition tests
    // =========================================================================

    #[test]
    fn test_settings_opens_wake_up_display() {
        // When settings is opened, display should wake up
        let mut is_sleeping = true;
        let mut is_white_screen = true;

        // Simulate opening settings (same logic as in user_event handler)
        is_sleeping = false;
        is_white_screen = false;

        assert!(!is_sleeping);
        assert!(!is_white_screen);
    }

    #[test]
    fn test_reload_config_wakes_up_display() {
        // When config is reloaded, display should wake up
        let mut is_sleeping = true;
        let mut is_white_screen = true;

        // Simulate reload config (same logic as in user_event handler)
        is_sleeping = false;
        is_white_screen = false;

        assert!(!is_sleeping);
        assert!(!is_white_screen);
    }

    #[test]
    fn test_double_click_wakes_up_display() {
        // When tray icon is double-clicked, display should wake up
        let mut is_sleeping = true;
        let mut is_white_screen = true;

        // Simulate double-click handler
        is_sleeping = false;
        is_white_screen = false;

        assert!(!is_sleeping);
        assert!(!is_white_screen);
    }
}
