mod settings;
use ini::Ini;
use settings::{settings_create_config, AppConfig};

mod consts;
use consts::*;

mod connect;
use connect::{connect_hwinfo, connect_steelseries};

mod console_utils;
use console_utils::{console_window, display_value_in_console, Console};

mod steelseries;
use steelseries::page_handler;

mod utils;
use utils::{format_custom_value, run_sensors};

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
use tray_icon::{Icon, TrayIconBuilder, menu::{Menu, MenuItem, MenuEvent, MenuId}, TrayIconEvent};
use winit::application::ApplicationHandler;
use winit::event::StartCause;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};

// User events sent to winit event loop
#[derive(Debug)]
enum UserEvent {
    TrayIconEvent(TrayIconEvent),
    MenuEvent(MenuEvent),
    UpdateDisplay,
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

// Application state
struct App {
    term: Term,
    steelseries: Option<GameSenseClient>,
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
    event_loop_proxy: Option<EventLoopProxy<UserEvent>>,
}

impl App {
    fn new(term: Term, event_loop_proxy: EventLoopProxy<UserEvent>) -> Self {
        Self {
            term,
            steelseries: None,
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
            event_loop_proxy: Some(event_loop_proxy),
        }
    }

    fn setup_tray_icon(&mut self) -> Result<(), anyhow::Error> {
        let event_loop_proxy = self.event_loop_proxy.as_ref().unwrap().clone();

        // Embed the icon at compile time
        const ICON_DATA: &[u8] = include_bytes!("../assets/hwinfo-steelseries-icon.ico");

        // Create menu
        let tray_menu = Menu::new();
        let exit_menu_item = MenuItem::new("Exit", true, None);
        tray_menu.append(&exit_menu_item)?;

        // Save exit menu ID
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
        // Connect to services
        self.steelseries = Some(connect_steelseries(&self.term)?);

        let mut hwinfo = connect_hwinfo(&self.term)?;
        hwinfo.pull().map_err(|e| {
            error!("Failed to pull initial HWiNFO data: {}", e);
            e
        })?;
        self.hwinfo = Some(hwinfo);

        // Load configuration
        info!("Loading configuration from conf.ini");
        let config_file = match Ini::load_from_file("conf.ini") {
            Ok(conf) => {
                info!("Configuration file loaded successfully");
                conf
            }
            Err(err) => {
                warn!("Configuration file not found: {}. Creating new config", err);
                settings_create_config(&self.term, self.hwinfo.as_ref().unwrap())?
            }
        };

        // Store config_file first
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

        // Setup pages
        info!("Setting up {} page(s)", self.pages);
        let steelseries = self.steelseries.as_mut().unwrap();

        for i in 1..=self.pages {
            let handler = page_handler(3, "line1", "line2", "line3", None);
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

            // For custom mode, store the sensor configuration
            if !self.is_summary {
                match self.config_file.as_ref().unwrap().section(Some(format!("PAGE{}.Sensors", i))) {
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

        steelseries.start_heartbeat();

        // Hide console window in release mode after successful startup
        #[cfg(not(debug_assertions))]
        {
            thread::sleep(Duration::from_millis(500));
            console_window(Console::HIDE);
        }

        Ok(())
    }

    fn update_display(&mut self) -> Result<(), anyhow::Error> {
        let hwinfo = match self.hwinfo.as_mut(){
            Some(hwinfo) => hwinfo,
            None => return Err(anyhow::anyhow!("HWiNFO not initialized")),
        };
        let steelseries = match self.steelseries.as_mut(){
            Some(steelseries) => steelseries,
            None => return Err(anyhow::anyhow!("SteelSeries not initialized")),
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
            if let Err(e) = steelseries.trigger_event_frame("ERROR", self.i.0, value) {
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
            if self.i.0 % self.page_time == 0 && self.i.0 != 0 {
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
            )?;

            format_custom_value(self.sensors_per_line, labels, values, units)
        };

        if self.display_in_console {
            display_value_in_console(&self.term, &value)?;
        }

        steelseries.trigger_event_frame(
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
                            console_window(Console::SHOW);
                        }
                    }
                    _ => {}
                }
            }
            UserEvent::MenuEvent(event) => {
                debug!("Received menu event: {:?}", event);
                if let Some(exit_id) = &self.exit_menu_id {
                    if event.id() == exit_id {
                        info!("Exit menu item clicked, shutting down");
                        if let Some(steelseries) = self.steelseries.as_mut() {
                            let _ = steelseries.stop_heartbeat();
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
        }
    }
}

fn main() -> Result<(), anyhow::Error> {
    // Initialize logger - set RUST_LOG environment variable to control log level
    // e.g., RUST_LOG=debug, RUST_LOG=info, RUST_LOG=warn, RUST_LOG=error
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

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
    event_loop
        .run_app(&mut app)
        .map_err(|e| {
            error!("Event loop error: {}", e);
            anyhow::anyhow!("Event loop error: {}", e)
        })?;

    info!("Application exited successfully");
    Ok(())
}


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
