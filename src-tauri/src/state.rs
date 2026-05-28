use crate::render::OledBuffer;
use crate::settings::AppConfig;
use hwinfo_steelseries_oled::Hwinfo;
use serde::Serialize;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveMode {
    GameSense,
    DirectUsb,
    Disconnected,
}

#[derive(Debug, Clone, Serialize)]
pub struct SensorValue {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusPayload {
    pub hwinfo_connected: bool,
    pub gg_connected: bool,
    pub usb_connected: bool,
    pub active_mode: ActiveMode,
    pub last_error: Option<String>,
    pub sensor_values: Vec<SensorValue>,
}

pub struct SharedState {
    pub hwinfo_connected: bool,
    pub gg_connected: bool,
    pub usb_connected: bool,
    pub active_mode: ActiveMode,
    pub last_error: Option<String>,
    pub sensor_values: Vec<SensorValue>,
    pub oled_buffer: OledBuffer,
    pub config: AppConfig,
    pub reload_requested: bool,
    pub sleep_requested: Option<SleepCommand>,
    pub hwinfo_snapshot: Option<Hwinfo>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SleepCommand {
    Sleep,
    White,
    Wake,
}

impl SharedState {
    pub fn new(config: AppConfig) -> Self {
        Self {
            hwinfo_connected: false,
            gg_connected: false,
            usb_connected: false,
            active_mode: ActiveMode::Disconnected,
            last_error: None,
            sensor_values: Vec::new(),
            oled_buffer: OledBuffer::new(),
            config,
            reload_requested: false,
            sleep_requested: None,
            hwinfo_snapshot: None,
        }
    }

    pub fn status_payload(&self) -> StatusPayload {
        StatusPayload {
            hwinfo_connected: self.hwinfo_connected,
            gg_connected: self.gg_connected,
            usb_connected: self.usb_connected,
            active_mode: self.active_mode,
            last_error: self.last_error.clone(),
            sensor_values: self.sensor_values.clone(),
        }
    }
}

pub type Shared = Arc<Mutex<SharedState>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::{AppConfig, WeatherConfig};

    fn mock_config() -> AppConfig {
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
            custom_sensors: vec![],
            weather: WeatherConfig::default(),
        }
    }

    #[test]
    fn test_new_initializes_defaults() {
        let state = SharedState::new(mock_config());
        assert!(!state.hwinfo_connected);
        assert!(!state.gg_connected);
        assert!(!state.usb_connected);
        assert_eq!(state.active_mode, ActiveMode::Disconnected);
        assert!(state.last_error.is_none());
        assert!(state.sensor_values.is_empty());
        assert!(!state.reload_requested);
        assert!(state.sleep_requested.is_none());
        assert!(state.hwinfo_snapshot.is_none());
    }

    #[test]
    fn test_status_payload_clones_state() {
        let mut state = SharedState::new(mock_config());
        state.hwinfo_connected = true;
        state.gg_connected = true;
        state.usb_connected = false;
        state.active_mode = ActiveMode::GameSense;
        state.last_error = Some("boom".to_string());
        state.sensor_values.push(SensorValue {
            label: "CPU".into(),
            value: "42".into(),
        });

        let payload = state.status_payload();
        assert!(payload.hwinfo_connected);
        assert!(payload.gg_connected);
        assert!(!payload.usb_connected);
        assert_eq!(payload.active_mode, ActiveMode::GameSense);
        assert_eq!(payload.last_error.as_deref(), Some("boom"));
        assert_eq!(payload.sensor_values.len(), 1);
        assert_eq!(payload.sensor_values[0].label, "CPU");
        assert_eq!(payload.sensor_values[0].value, "42");
    }

    #[test]
    fn test_active_mode_eq() {
        assert_eq!(ActiveMode::GameSense, ActiveMode::GameSense);
        assert_ne!(ActiveMode::GameSense, ActiveMode::DirectUsb);
        assert_ne!(ActiveMode::Disconnected, ActiveMode::DirectUsb);
    }

    #[test]
    fn test_sleep_command_variants() {
        assert_eq!(SleepCommand::Sleep, SleepCommand::Sleep);
        assert_ne!(SleepCommand::Sleep, SleepCommand::Wake);
        assert_ne!(SleepCommand::Wake, SleepCommand::White);
    }

    #[test]
    fn test_status_payload_serializes() {
        let state = SharedState::new(mock_config());
        let payload = state.status_payload();
        let s = serde_json::to_string(&payload).unwrap();
        assert!(s.contains("\"hwinfo_connected\""));
        assert!(s.contains("\"active_mode\""));
        assert!(s.contains("disconnected"));
    }

    #[test]
    fn test_sensor_value_serializes() {
        let v = SensorValue {
            label: "L".into(),
            value: "V".into(),
        };
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, r#"{"label":"L","value":"V"}"#);
    }
}
