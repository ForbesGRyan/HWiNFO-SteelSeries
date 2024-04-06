use console::Term;
use gamesense::client::GameSenseClient;
use hwinfo_steelseries_oled::Hwinfo;

pub fn connect_hwinfo(term: &Term) -> Result<Hwinfo, anyhow::Error> {
    match Hwinfo::new() {
        Ok(hwinfo) => {
            term.write_line("Connected to HWiNFO")?;
            Ok(hwinfo)
        }
        Err(_err) => {
            // println!("Can't connect to HWiNFO. Trying again in 1 second.");
            for i in (1..=3).rev() {
                term.clear_line()?;
                term.write_line(
                    format!("Can't connect to HWiNFO. Trying again in {} second.", i).as_str(),
                )?;
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            connect_hwinfo(term)
        }
    }
}

pub fn connect_steelseries(term: &Term) -> Result<GameSenseClient, anyhow::Error> {
    match GameSenseClient::new("HWINFO", "HWiNFO_Stats", "Ryan", None) {
        Ok(c) => {
            term.write_line("Connected to SteelSeries GG")?;
            Ok(c)
        }
        Err(_e) => {
            for i in (1..=3).rev() {
                term.clear_line()?;
                term.write_line(
                    format!(
                        "Can't connect to SteelSeries GG. Trying again in {} second.",
                        i
                    )
                    .as_str(),
                )?;
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
            connect_steelseries(term)
        }
    }
}
