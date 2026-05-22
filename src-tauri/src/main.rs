mod settings;
use settings::{settings_create_config, AppConfig};

mod consts;

mod connect;
use connect::connect_hwinfo;

mod console_utils;
use console_utils::{console_window, Console};

mod steelseries;

mod utils;

mod mouse_battery;

mod media;

mod render;

mod state;
use state::{Shared, SharedState, SleepCommand};

mod daemon;

mod gui;

use anyhow;
use console::Term;
use image::ImageReader;
use ini::Ini;
use log::{debug, error, info, warn};
use std::io::Cursor;
use std::sync::{Arc, Mutex};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Manager, WindowEvent,
};

fn handle_fatal_error(term: &Term, err: anyhow::Error) -> anyhow::Error {
    error!("Fatal error: {}", err);
    console_window(Console::SHOW);
    let _ = term.write_line("");
    let _ = term.write_line("=================================");
    let _ = term.write_line("ERROR: Application stopped");
    let _ = term.write_line("=================================");
    let _ = term.write_line(&format!("{}", err));
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
    let _ = std::io::stdin().read_line(&mut String::new());
    err
}

fn handle_discovery_flags(args: &[String]) -> bool {
    // Brute force discovery
    if let Some(pos) = args.iter().position(|arg| arg == "--discover-all-battery") {
        if args.len() < pos + 2 {
            eprintln!("Usage: {} --discover-all-battery <EXPECTED_BATTERY>", args[0]);
            std::process::exit(1);
        }
        let expected = match args[pos + 1].parse::<u8>() {
            Ok(v) if v <= 100 => v,
            _ => {
                eprintln!("Error: Expected battery must be 0-100");
                std::process::exit(1);
            }
        };
        let hid_api = match hidapi::HidApi::new() {
            Ok(api) => api,
            Err(e) => {
                error!("Failed to initialize HID API: {}", e);
                std::process::exit(1);
            }
        };
        match mouse_battery::discover_all_devices_for_battery(&hid_api, expected) {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                error!("Discovery failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    if let Some(pos) = args.iter().position(|arg| arg == "--discover-mouse-battery") {
        if args.len() < pos + 2 {
            eprintln!("Usage: {} --discover-mouse-battery <VID> [PID] [EXPECTED]", args[0]);
            std::process::exit(1);
        }
        let vid_str = &args[pos + 1];
        let vid = if vid_str.starts_with("0x") || vid_str.starts_with("0X") {
            u16::from_str_radix(&vid_str[2..], 16)
        } else {
            u16::from_str_radix(vid_str, 16)
        }
        .unwrap_or_else(|_| {
            eprintln!("Error: Invalid VID format");
            std::process::exit(1);
        });

        let pid = args.get(pos + 2).and_then(|s| {
            if s == "-" {
                None
            } else if s.starts_with("0x") || s.starts_with("0X") {
                u16::from_str_radix(&s[2..], 16).ok()
            } else {
                u16::from_str_radix(s, 16).ok()
            }
        });

        let expected = args.get(pos + 3).and_then(|s| s.parse::<u8>().ok()).filter(|v| *v <= 100);

        let hid_api = match hidapi::HidApi::new() {
            Ok(api) => api,
            Err(e) => {
                error!("Failed to init HID API: {}", e);
                std::process::exit(1);
            }
        };

        match mouse_battery::discover_battery_report_id(&hid_api, vid, pid, expected) {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                error!("Discovery failed: {}", e);
                std::process::exit(1);
            }
        }
    }

    false
}

fn load_or_create_config(term: &Term) -> Result<AppConfig, anyhow::Error> {
    info!("Loading configuration from conf.ini");
    let ini = match Ini::load_from_file("conf.ini") {
        Ok(conf) => {
            info!("Configuration loaded successfully");
            conf
        }
        Err(e) => {
            warn!("conf.ini not found: {}. Running setup wizard.", e);
            let mut hwinfo = connect_hwinfo(term)?;
            hwinfo.pull()?;
            settings_create_config(term, &hwinfo)?
        }
    };
    AppConfig::from_ini(&ini)
}

fn build_tray_icon_image() -> Option<Image<'static>> {
    const ICON_DATA: &[u8] = include_bytes!("../assets/hwinfo-steelseries-icon.ico");
    let reader = ImageReader::new(Cursor::new(ICON_DATA)).with_guessed_format().ok()?;
    let img = reader.decode().ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(Image::new_owned(rgba.into_raw(), w, h))
}

fn main() -> Result<(), anyhow::Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    handle_discovery_flags(&args);

    info!("Starting HWiNFO-SteelSeries");
    let term = Term::stdout();

    let config = match load_or_create_config(&term) {
        Ok(c) => c,
        Err(e) => return Err(handle_fatal_error(&term, e)),
    };

    let shared: Shared = Arc::new(Mutex::new(SharedState::new(config.clone())));

    let shared_for_setup = shared.clone();
    let config_for_setup = config.clone();

    tauri::Builder::default()
        .manage(shared.clone())
        .invoke_handler(tauri::generate_handler![
            gui::get_status,
            gui::get_config,
            gui::save_config,
            gui::get_live_preview,
            gui::list_sensors,
            gui::preview_config,
            gui::request_sleep,
            gui::request_wake,
            gui::request_white_screen,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();

            // Hide main window on launch
            if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }

            // Build tray
            let icon = build_tray_icon_image()
                .or_else(|| app.default_window_icon().cloned())
                .ok_or_else(|| anyhow::anyhow!("Failed to load tray icon"))?;

            let settings_item = MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
            let reload_item = MenuItem::with_id(app, "reload", "Reload Config", true, None::<&str>)?;
            let sleep_item = MenuItem::with_id(app, "sleep", "Sleep Display", true, None::<&str>)?;
            let white_item = MenuItem::with_id(app, "white", "White Screen", true, None::<&str>)?;
            let exit_item = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;

            let menu = Menu::with_items(app, &[
                &settings_item,
                &reload_item,
                &sleep_item,
                &white_item,
                &exit_item,
            ])?;

            let shared_for_menu = shared_for_setup.clone();
            TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .tooltip("HWiNFO-SteelSeries")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| {
                    debug!("Tray menu event: {:?}", event.id);
                    match event.id.as_ref() {
                        "settings" => {
                            if let Some(win) = app.get_webview_window("main") {
                                let _ = win.show();
                                let _ = win.unminimize();
                                let _ = win.set_focus();
                            }
                            if let Ok(mut g) = shared_for_menu.lock() {
                                g.sleep_requested = Some(SleepCommand::Wake);
                            }
                        }
                        "reload" => {
                            if let Ok(mut g) = shared_for_menu.lock() {
                                g.reload_requested = true;
                                g.sleep_requested = Some(SleepCommand::Wake);
                            }
                        }
                        "sleep" => {
                            if let Ok(mut g) = shared_for_menu.lock() {
                                g.sleep_requested = Some(SleepCommand::Sleep);
                            }
                        }
                        "white" => {
                            if let Ok(mut g) = shared_for_menu.lock() {
                                g.sleep_requested = Some(SleepCommand::White);
                            }
                        }
                        "exit" => {
                            app.exit(0);
                        }
                        _ => {}
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        let app = tray.app_handle();
                        if let Some(win) = app.get_webview_window("main") {
                            let _ = win.show();
                            let _ = win.unminimize();
                            let _ = win.set_focus();
                        }
                    }
                })
                .build(app)?;

            // Hide console in release after tray is up
            #[cfg(not(debug_assertions))]
            {
                std::thread::sleep(std::time::Duration::from_millis(500));
                console_window(Console::HIDE);
            }

            // Spawn daemon
            daemon::spawn(shared_for_setup.clone(), app_handle, config_for_setup.clone());

            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                // Intercept close — hide instead of destroy, keep daemon running
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .run(tauri::generate_context!())
        .map_err(|e| anyhow::anyhow!("Tauri runtime error: {}", e))?;

    info!("Application exited");
    Ok(())
}
