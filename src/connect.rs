use console::Term;
use gamesense::client::GameSenseClient;
use hwinfo_steelseries_oled::Hwinfo;

fn retry_connect<T, F>(
    term: &Term,
    service_name: &str,
    connect_fn: F,
) -> Result<T, anyhow::Error>
where
    F: Fn() -> Result<T, anyhow::Error>,
{
    match connect_fn() {
        Ok(result) => {
            term.clear_line()?;
            term.write_line(&format!("Connected to {}", service_name))?;
            Ok(result)
        }
        Err(_) => {
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
