use console::Term;
use gamesense::client::GameSenseClient;
use hidapi::{HidApi, HidDevice};
use hwinfo_steelseries_oled::Hwinfo;
use log::{error, info, warn};

fn retry_connect<T, F>(term: &Term, service_name: &str, connect_fn: F) -> Result<T, anyhow::Error>
where
    F: Fn() -> Result<T, anyhow::Error>,
{
    match connect_fn() {
        Ok(result) => {
            info!("Successfully connected to {}", service_name);
            term.clear_line()?;
            term.write_line(&format!("Connected to {}", service_name))?;
            Ok(result)
        }
        Err(e) => {
            warn!(
                "Failed to connect to {}: {}. Retrying in 3 seconds...",
                service_name, e
            );
            for i in (1..=3).rev() {
                term.clear_line()?;
                term.write_line(&format!(
                    "Can't connect to {}. Trying again in {} second.",
                    service_name, i
                ))?;
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            retry_connect(term, service_name, connect_fn)
        }
    }
}

pub fn connect_hwinfo(term: &Term) -> Result<Hwinfo, anyhow::Error> {
    retry_connect(term, "HWiNFO", || Hwinfo::new())
}

pub fn connect_steelseries(term: &Term) -> Result<GameSenseClient, anyhow::Error> {
    retry_connect(term, "SteelSeries GG", || {
        GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", None)
    })
}

pub fn connect_hid(term: &Term, api: &HidApi) -> Result<HidDevice, anyhow::Error> {
    retry_connect(term, "SteelSeries OLED (HID)", || {
        let vendor_id = 0x1038u16;
        let product_id = 0x12E0u16;
        let interface_number = 0x04i32;
        let usage_page = 0xFFC0u16;

        let device_info = api
            .device_list()
            .find(|d| {
                d.vendor_id() == vendor_id
                    && d.product_id() == product_id
                    && d.interface_number() == interface_number
                    && d.usage_page() == usage_page
            })
            .ok_or_else(|| anyhow::anyhow!("OLED device not found"))?;

        device_info.open_device(api).map_err(|e| {
            error!("Failed to open HID device: {}", e);
            anyhow::anyhow!("Failed to open HID device: {}", e)
        })
    })
}
