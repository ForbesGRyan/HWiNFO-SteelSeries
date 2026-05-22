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
