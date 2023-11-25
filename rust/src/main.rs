use rust::update_hwinfo;
use std::path::Path;

struct SteelSeries {
    Address: String,
    EncryptedAddress: String
}

fn get_sse_address() -> Result<String, Box<dyn std::error::Error>>{
    let path = Path::new("C:/ProgramData/SteelSeries/SteelSeries Engine 3/coreProps.json");
    
    println!("{:?}",path);
    Ok(String::new())
}
fn main() -> Result<(), Box<dyn std::error::Error>>{
    let sse_address = get_sse_address()?;
    // let info = update_hwinfo().unwrap();
    // for sensor in info.master_sensor_names.into_iter() {
    //     println!("{}", sensor)
    // }
    Ok(())
}