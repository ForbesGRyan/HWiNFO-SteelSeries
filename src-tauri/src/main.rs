#![allow(clippy::too_many_arguments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::ptr_arg)]
#![allow(clippy::upper_case_acronyms)]

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

mod weather;

mod render;

mod state;
use state::{Shared, SharedState, SleepCommand};

mod daemon;

mod gui;

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

/// Pure helper: format the fatal-error report (used by `handle_fatal_error`).
/// Returns the lines that should be written to the terminal, in order.
fn format_fatal_error_lines(err: &anyhow::Error) -> Vec<String> {
    let mut lines = vec![
        String::new(),
        "=================================".to_string(),
        "ERROR: Application stopped".to_string(),
        "=================================".to_string(),
        format!("{}", err),
    ];
    let mut source = err.source();
    if source.is_some() {
        lines.push(String::new());
        lines.push("Caused by:".to_string());
    }
    while let Some(cause) = source {
        lines.push(format!("  {}", cause));
        source = cause.source();
    }
    lines.push(String::new());
    lines.push("Press Enter to exit...".to_string());
    lines
}

fn handle_fatal_error(term: &Term, err: anyhow::Error) -> anyhow::Error {
    error!("Fatal error: {}", err);
    console_window(Console::SHOW);
    for line in format_fatal_error_lines(&err) {
        let _ = term.write_line(&line);
    }
    let _ = std::io::stdin().read_line(&mut String::new());
    err
}

/// Parse a possibly-hex u16 ("0x1234", "0X1234", or "1234"). Returns None on parse failure.
fn parse_hex_u16(s: &str) -> Option<u16> {
    if let Some(rest) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(rest, 16).ok()
    } else {
        u16::from_str_radix(s, 16).ok()
    }
}

/// Parse the optional PID argument: "-" → None, valid hex → Some, invalid → None.
fn parse_optional_pid(s: Option<&str>) -> Option<u16> {
    match s {
        Some("-") => None,
        Some(v) => parse_hex_u16(v),
        None => None,
    }
}

/// Parse the expected battery percentage (0..=100). Returns None if invalid/out-of-range.
fn parse_expected_battery(s: Option<&str>) -> Option<u8> {
    s.and_then(|v| v.parse::<u8>().ok()).filter(|v| *v <= 100)
}

/// Position of the named flag in the argv slice (if present).
fn find_flag_position(args: &[String], flag: &str) -> Option<usize> {
    args.iter().position(|a| a == flag)
}

/// Whether argv requests opening the settings window on launch (`--settings`).
fn wants_open_settings(args: &[String]) -> bool {
    find_flag_position(args, "--settings").is_some()
}

/// Validate that argv contains at least one more arg after the given flag position.
fn has_required_arg_after(args: &[String], flag_pos: usize) -> bool {
    args.len() >= flag_pos + 2
}

/// Validate "expected battery" value parsed from a string, 0..=100 only. Used by --discover-all-battery.
fn validate_expected_battery_strict(s: &str) -> Option<u8> {
    s.parse::<u8>().ok().filter(|v| *v <= 100)
}

/// Pure helper: execute "--discover-all-battery" given the argv and the flag's position.
/// On success returns the expected battery value used (for testing).
fn run_discover_all_battery(args: &[String], pos: usize) -> Result<u8, String> {
    if !has_required_arg_after(args, pos) {
        return Err(format!(
            "Usage: {} --discover-all-battery <EXPECTED_BATTERY>",
            args[0]
        ));
    }
    let expected = validate_expected_battery_strict(&args[pos + 1])
        .ok_or_else(|| "Expected battery must be 0-100".to_string())?;
    let hid_api =
        hidapi::HidApi::new().map_err(|e| format!("Failed to initialize HID API: {}", e))?;
    mouse_battery::discover_all_devices_for_battery(&hid_api, expected)
        .map_err(|e| e.to_string())?;
    Ok(expected)
}

/// Pure helper: execute "--discover-mouse-battery" given argv and flag position.
fn run_discover_mouse_battery(
    args: &[String],
    pos: usize,
) -> Result<(u16, Option<u16>, Option<u8>), String> {
    if !has_required_arg_after(args, pos) {
        return Err(format!(
            "Usage: {} --discover-mouse-battery <VID> [PID] [EXPECTED]",
            args[0]
        ));
    }
    let vid = parse_hex_u16(&args[pos + 1]).ok_or_else(|| "Invalid VID format".to_string())?;
    let pid = parse_optional_pid(args.get(pos + 2).map(String::as_str));
    let expected = parse_expected_battery(args.get(pos + 3).map(String::as_str));
    let hid_api = hidapi::HidApi::new().map_err(|e| format!("Failed to init HID API: {}", e))?;
    mouse_battery::discover_battery_report_id(&hid_api, vid, pid, expected)
        .map(|_| (vid, pid, expected))
        .map_err(|e| e.to_string())
}

fn handle_discovery_flags(args: &[String]) -> bool {
    if let Some(pos) = find_flag_position(args, "--discover-all-battery") {
        match run_discover_all_battery(args, pos) {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
    }

    if let Some(pos) = find_flag_position(args, "--discover-mouse-battery") {
        match run_discover_mouse_battery(args, pos) {
            Ok(_) => std::process::exit(0),
            Err(e) => {
                eprintln!("{}", e);
                std::process::exit(1);
            }
        }
    }

    false
}

fn load_or_create_config_at(path: &str, term: &Term) -> Result<AppConfig, anyhow::Error> {
    info!("Loading configuration from {}", path);
    let ini = match Ini::load_from_file(path) {
        Ok(conf) => {
            info!("Configuration loaded successfully");
            conf
        }
        Err(e) => {
            warn!("{} not found: {}. Running setup wizard.", path, e);
            let mut hwinfo = connect_hwinfo(term)?;
            hwinfo.pull()?;
            settings_create_config(term, &hwinfo)?
        }
    };
    AppConfig::from_ini(&ini)
}

fn load_or_create_config(term: &Term) -> Result<AppConfig, anyhow::Error> {
    load_or_create_config_at("conf.ini", term)
}

/// Helper used by both menu and tray click handlers: surface the main window.
fn show_main_window<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> bool {
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.show();
        let _ = win.unminimize();
        let _ = win.set_focus();
        true
    } else {
        false
    }
}

/// Pure helper: apply a tray-menu event id to shared state.
/// Returns true if the caller should also show the settings window.
fn apply_tray_menu_event(event_id: &str, shared: &Shared) -> bool {
    match event_id {
        "settings" => {
            if let Ok(mut g) = shared.lock() {
                g.sleep_requested = Some(SleepCommand::Wake);
            }
            true
        }
        "reload" => {
            if let Ok(mut g) = shared.lock() {
                g.reload_requested = true;
                g.sleep_requested = Some(SleepCommand::Wake);
            }
            false
        }
        "sleep" => {
            if let Ok(mut g) = shared.lock() {
                g.sleep_requested = Some(SleepCommand::Sleep);
            }
            false
        }
        "white" => {
            if let Ok(mut g) = shared.lock() {
                g.sleep_requested = Some(SleepCommand::White);
            }
            false
        }
        _ => false,
    }
}

fn build_tray_icon_image() -> Option<Image<'static>> {
    const ICON_DATA: &[u8] = include_bytes!("../assets/hwinfo-steelseries-icon.ico");
    let reader = ImageReader::new(Cursor::new(ICON_DATA))
        .with_guessed_format()
        .ok()?;
    let img = reader.decode().ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    Some(Image::new_owned(rgba.into_raw(), w, h))
}

fn main() -> Result<(), anyhow::Error> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let args: Vec<String> = std::env::args().collect();
    handle_discovery_flags(&args);
    let open_settings = wants_open_settings(&args);

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
            gui::list_hid_devices,
            gui::preview_config,
            gui::request_sleep,
            gui::request_wake,
            gui::request_white_screen,
        ])
        .setup(move |app| {
            let app_handle = app.handle().clone();

            // Show settings on launch if requested via `--settings`, otherwise
            // start hidden in the tray.
            if open_settings {
                show_main_window(&app_handle);
            } else if let Some(win) = app.get_webview_window("main") {
                let _ = win.hide();
            }

            // Build tray
            let icon = build_tray_icon_image()
                .or_else(|| app.default_window_icon().cloned())
                .ok_or_else(|| anyhow::anyhow!("Failed to load tray icon"))?;

            // Use the same icon for the main window titlebar/taskbar.
            if let (Some(win), Some(win_icon)) =
                (app.get_webview_window("main"), build_tray_icon_image())
            {
                let _ = win.set_icon(win_icon);
            }

            let settings_item =
                MenuItem::with_id(app, "settings", "Settings...", true, None::<&str>)?;
            let reload_item =
                MenuItem::with_id(app, "reload", "Reload Config", true, None::<&str>)?;
            let sleep_item = MenuItem::with_id(app, "sleep", "Sleep Display", true, None::<&str>)?;
            let white_item = MenuItem::with_id(app, "white", "White Screen", true, None::<&str>)?;
            let exit_item = MenuItem::with_id(app, "exit", "Exit", true, None::<&str>)?;

            let menu = Menu::with_items(
                app,
                &[
                    &settings_item,
                    &reload_item,
                    &sleep_item,
                    &white_item,
                    &exit_item,
                ],
            )?;

            let shared_for_menu = shared_for_setup.clone();
            TrayIconBuilder::with_id("main-tray")
                .icon(icon)
                .tooltip("HWiNFO-SteelSeries")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| {
                    debug!("Tray menu event: {:?}", event.id);
                    if event.id.as_ref() == "exit" {
                        app.exit(0);
                        return;
                    }
                    if apply_tray_menu_event(event.id.as_ref(), &shared_for_menu) {
                        let _ = show_main_window(&app.app_handle().clone());
                    }
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        let _ = show_main_window(tray.app_handle());
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
            daemon::spawn(
                shared_for_setup.clone(),
                app_handle,
                config_for_setup.clone(),
            );

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_u16_lowercase_prefix() {
        assert_eq!(parse_hex_u16("0x1234"), Some(0x1234));
        assert_eq!(parse_hex_u16("0xffff"), Some(0xffff));
    }

    #[test]
    fn test_parse_hex_u16_uppercase_prefix() {
        assert_eq!(parse_hex_u16("0X1234"), Some(0x1234));
    }

    #[test]
    fn test_parse_hex_u16_no_prefix() {
        assert_eq!(parse_hex_u16("1234"), Some(0x1234));
        assert_eq!(parse_hex_u16("ff"), Some(0xff));
    }

    #[test]
    fn test_parse_hex_u16_invalid() {
        assert!(parse_hex_u16("xyz").is_none());
        assert!(parse_hex_u16("0xZZZZ").is_none());
        assert!(parse_hex_u16("").is_none());
    }

    #[test]
    fn test_parse_hex_u16_overflow() {
        assert!(parse_hex_u16("0x10000").is_none()); // > u16::MAX
    }

    #[test]
    fn test_parse_optional_pid_dash() {
        assert_eq!(parse_optional_pid(Some("-")), None);
    }

    #[test]
    fn test_parse_optional_pid_hex() {
        assert_eq!(parse_optional_pid(Some("0xc539")), Some(0xc539));
        assert_eq!(parse_optional_pid(Some("c539")), Some(0xc539));
    }

    #[test]
    fn test_parse_optional_pid_none() {
        assert_eq!(parse_optional_pid(None), None);
    }

    #[test]
    fn test_parse_optional_pid_invalid_returns_none() {
        assert_eq!(parse_optional_pid(Some("notahex")), None);
    }

    #[test]
    fn test_parse_expected_battery_valid() {
        assert_eq!(parse_expected_battery(Some("50")), Some(50));
        assert_eq!(parse_expected_battery(Some("0")), Some(0));
        assert_eq!(parse_expected_battery(Some("100")), Some(100));
    }

    #[test]
    fn test_parse_expected_battery_out_of_range() {
        assert_eq!(parse_expected_battery(Some("101")), None);
        assert_eq!(parse_expected_battery(Some("255")), None);
    }

    #[test]
    fn test_parse_expected_battery_invalid_or_none() {
        assert_eq!(parse_expected_battery(Some("abc")), None);
        assert_eq!(parse_expected_battery(None), None);
        assert_eq!(parse_expected_battery(Some("")), None);
    }

    #[test]
    fn test_find_flag_position_found_and_not_found() {
        let args = vec!["prog".to_string(), "--a".to_string(), "--b".to_string()];
        assert_eq!(find_flag_position(&args, "--a"), Some(1));
        assert_eq!(find_flag_position(&args, "--b"), Some(2));
        assert_eq!(find_flag_position(&args, "--c"), None);
    }

    #[test]
    fn test_has_required_arg_after() {
        let args = vec!["prog".into(), "--flag".into(), "val".into()];
        assert!(has_required_arg_after(&args, 1));
        // Pos 1 + 2 = 3 > len(3) so missing
        let short = vec!["prog".into(), "--flag".into()];
        assert!(!has_required_arg_after(&short, 1));
    }

    #[test]
    fn test_validate_expected_battery_strict() {
        assert_eq!(validate_expected_battery_strict("50"), Some(50));
        assert_eq!(validate_expected_battery_strict("100"), Some(100));
        assert_eq!(validate_expected_battery_strict("101"), None);
        assert_eq!(validate_expected_battery_strict("abc"), None);
    }

    #[test]
    fn test_run_discover_all_battery_missing_arg() {
        let args = vec!["prog".into(), "--discover-all-battery".into()];
        let r = run_discover_all_battery(&args, 1);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Usage:"));
    }

    #[test]
    fn test_run_discover_all_battery_bad_value() {
        let args = vec!["prog".into(), "--discover-all-battery".into(), "999".into()];
        let r = run_discover_all_battery(&args, 1);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("0-100"));
    }

    #[test]
    fn test_run_discover_all_battery_invokes_discovery() {
        // 99% is unlikely to appear in any feature report on test box → discovery returns Ok with no matches.
        if hidapi::HidApi::new().is_err() {
            return;
        }
        let args = vec!["prog".into(), "--discover-all-battery".into(), "99".into()];
        let r = run_discover_all_battery(&args, 1);
        assert!(r.is_ok());
    }

    #[test]
    fn test_run_discover_mouse_battery_missing_arg() {
        let args = vec!["prog".into(), "--discover-mouse-battery".into()];
        let r = run_discover_mouse_battery(&args, 1);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Usage:"));
    }

    #[test]
    fn test_run_discover_mouse_battery_bad_vid() {
        let args = vec![
            "prog".into(),
            "--discover-mouse-battery".into(),
            "not-a-hex".into(),
        ];
        let r = run_discover_mouse_battery(&args, 1);
        assert!(r.is_err());
        assert!(r.unwrap_err().contains("Invalid VID"));
    }

    #[test]
    fn test_run_discover_mouse_battery_unknown_vid_propagates() {
        if hidapi::HidApi::new().is_err() {
            return;
        }
        let args = vec![
            "prog".into(),
            "--discover-mouse-battery".into(),
            "0xdead".into(),
            "0xbeef".into(),
            "50".into(),
        ];
        let r = run_discover_mouse_battery(&args, 1);
        // discover_battery_report_id returns Err when no devices match
        assert!(r.is_err());
    }

    #[test]
    fn test_wants_open_settings() {
        assert!(wants_open_settings(&["prog".into(), "--settings".into()]));
        assert!(wants_open_settings(&[
            "prog".into(),
            "--other".into(),
            "--settings".into()
        ]));
        assert!(!wants_open_settings(&["prog".into()]));
        assert!(!wants_open_settings(&["prog".into(), "--setting".into()]));
    }

    #[test]
    fn test_handle_discovery_flags_no_flags_returns_false() {
        let args = vec!["prog".into(), "normal-run".into()];
        assert!(!handle_discovery_flags(&args));
    }

    #[test]
    fn test_build_tray_icon_image_loads_embedded_icon() {
        let img = build_tray_icon_image();
        assert!(img.is_some(), "Embedded icon must decode");
    }

    #[test]
    fn test_format_fatal_error_lines_no_cause() {
        let err = anyhow::anyhow!("boom");
        let lines = format_fatal_error_lines(&err);
        assert!(lines
            .iter()
            .any(|l| l.contains("ERROR: Application stopped")));
        assert!(lines.iter().any(|l| l == "boom"));
        // No "Caused by:" since no source chain
        assert!(!lines.iter().any(|l| l == "Caused by:"));
        // Must end with the prompt
        assert_eq!(lines.last().unwrap(), "Press Enter to exit...");
    }

    use crate::settings::WeatherConfig;
    use crate::state::SharedState;
    use std::sync::{Arc, Mutex};

    fn fresh_shared_for_test() -> Shared {
        Arc::new(Mutex::new(SharedState::new(AppConfig {
            is_summary: true,
            is_vertical: true,
            gpu: String::new(),
            decimal: false,
            pages: 1,
            page_time: 5,
            sensors_per_line: 1,
            direct_usb: false,
            direct_usb_serial: String::new(),
            custom_sensors: vec![],
            weather: WeatherConfig::default(),
            font_sizes: [crate::render::FontSize::Medium; crate::consts::DISPLAY_LINES],
        })))
    }

    #[test]
    fn test_show_main_window_returns_false_when_window_missing() {
        let app = tauri::test::mock_app();
        // mock_app has no "main" webview window
        let h = tauri::Manager::app_handle(&app).clone();
        assert!(!show_main_window(&h));
    }

    #[test]
    fn test_apply_tray_menu_event_settings_returns_show_window() {
        let shared = fresh_shared_for_test();
        let show = apply_tray_menu_event("settings", &shared);
        assert!(show);
        assert_eq!(
            shared.lock().unwrap().sleep_requested,
            Some(SleepCommand::Wake)
        );
    }

    #[test]
    fn test_apply_tray_menu_event_reload() {
        let shared = fresh_shared_for_test();
        let show = apply_tray_menu_event("reload", &shared);
        assert!(!show);
        let g = shared.lock().unwrap();
        assert!(g.reload_requested);
        assert_eq!(g.sleep_requested, Some(SleepCommand::Wake));
    }

    #[test]
    fn test_apply_tray_menu_event_sleep() {
        let shared = fresh_shared_for_test();
        let show = apply_tray_menu_event("sleep", &shared);
        assert!(!show);
        assert_eq!(
            shared.lock().unwrap().sleep_requested,
            Some(SleepCommand::Sleep)
        );
    }

    #[test]
    fn test_apply_tray_menu_event_white() {
        let shared = fresh_shared_for_test();
        let show = apply_tray_menu_event("white", &shared);
        assert!(!show);
        assert_eq!(
            shared.lock().unwrap().sleep_requested,
            Some(SleepCommand::White)
        );
    }

    #[test]
    fn test_apply_tray_menu_event_unknown_is_noop() {
        let shared = fresh_shared_for_test();
        let show = apply_tray_menu_event("unknown", &shared);
        assert!(!show);
        assert!(shared.lock().unwrap().sleep_requested.is_none());
    }

    #[test]
    fn test_handle_fatal_error_returns_same_error() {
        // In `cargo test` stdin is typically closed/non-interactive, so read_line returns immediately.
        // If your platform blocks here, set CARGO_TEST_HANDLE_FATAL=skip to bypass.
        if std::env::var("CARGO_TEST_HANDLE_FATAL").as_deref() == Ok("skip") {
            return;
        }
        let term = console::Term::stdout();
        let err = anyhow::anyhow!("test fatal");
        let returned = handle_fatal_error(&term, err);
        assert!(returned.to_string().contains("test fatal"));
    }

    #[test]
    fn test_format_fatal_error_lines_with_cause_chain() {
        use std::io;
        let io_err = io::Error::other("inner cause");
        let err: anyhow::Error = anyhow::Error::new(io_err).context("outer wrap");
        let lines = format_fatal_error_lines(&err);
        assert!(lines.iter().any(|l| l == "Caused by:"));
        assert!(lines.iter().any(|l| l.contains("inner cause")));
    }

    #[test]
    fn test_load_or_create_config_at_loads_existing_ini() {
        let tmp =
            std::env::temp_dir().join(format!("hwinfo_ss_main_loadcfg_{}.ini", std::process::id()));
        let mut ini = Ini::new();
        ini.with_section(Some("Main"))
            .set("style", "Horizontal")
            .set("gpu", "GPU [#0]");
        ini.write_to_file(&tmp).unwrap();

        let term = console::Term::stdout();
        let cfg = load_or_create_config_at(tmp.to_str().unwrap(), &term).unwrap();
        assert!(cfg.is_summary);
        assert!(!cfg.is_vertical);
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn test_load_or_create_config_reads_existing_ini() {
        // Write a minimal valid conf.ini to a temp path, then ensure AppConfig::from_ini round-trips
        // via the same code path load_or_create_config uses.
        let tmp =
            std::env::temp_dir().join(format!("hwinfo_ss_main_load_{}.ini", std::process::id()));
        let mut ini = Ini::new();
        ini.with_section(Some("Main"))
            .set("style", "Vertical")
            .set("gpu", "GPU [#0]")
            .set("decimal", "false")
            .set("pages", "1");
        ini.write_to_file(&tmp).unwrap();

        // load_or_create_config uses fixed "conf.ini" path so we exercise its parser via AppConfig::from_ini directly,
        // verifying the same fallback chain (load → from_ini) behaves correctly on success.
        let loaded = Ini::load_from_file(&tmp).unwrap();
        let cfg = AppConfig::from_ini(&loaded).unwrap();
        assert!(cfg.is_summary);
        assert!(cfg.is_vertical);

        let _ = std::fs::remove_file(&tmp);
    }
}
