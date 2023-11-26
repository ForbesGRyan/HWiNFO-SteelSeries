use winapi::um::{
    memoryapi::{OpenFileMappingW, FILE_MAP_READ, MapViewOfFile, UnmapViewOfFile},
    // winnt::HANDLE
};
// use winapi::shared::minwindef::LPVOID;
use std::{ffi::OsStr, iter::once, collections::HashMap};
use std::os::windows::ffi::OsStrExt;
use std::io::{Error, ErrorKind};
use strum::FromRepr;

const HWINFO_SENSORS_MAP_FILE_NAME2: &str = "Global\\HWiNFO_SENS_SM2";
// const HWINFO_SENSORS_SM2_MUTEX: &str = "Global\\HWiNFO_SM2_MUTEX";
const HWINFO_SENSORS_STRING_LEN2: usize = 128;
const HWINFO_UNIT_STRING_LEN: usize = 16;

#[allow(dead_code)]
#[derive(FromRepr)]
enum SensorReadingType
{
    SensorTypeNone = 0,
    SensorTypeTemp,
    SensorTypeVolt,
    SensorTypeFan,
    SensorTypeCurrent,
    SensorTypePower,
    SensorTypeClock,
    SensorTypeUsage,
    SensorTypeOther
}
#[allow(dead_code)]
#[repr(C, packed(1))]
struct HwinfoSensorsReadingElement
{
    t_reading: SensorReadingType,
    _blank: [u8;3], // For some reason the packing wasn't lining up. This alleviates it
    dw_sensor_index: u32,
    dw_reading_id: u32,
    sz_label_orig: [u8; HWINFO_SENSORS_STRING_LEN2],
    sz_label_user: [u8; HWINFO_SENSORS_STRING_LEN2],
    sz_unit: [u8; HWINFO_UNIT_STRING_LEN],
    // starts at 281
    value: f64,
    value_min: f64,
    value_max: f64,
    value_avg: f64,
    utf_label_user: [u8; HWINFO_SENSORS_STRING_LEN2],
    utf_unit: [u8; HWINFO_UNIT_STRING_LEN]
}
#[allow(dead_code)]
#[repr(C, align(1))]
struct HwinfoSensorsSensorElement
{
    dw_sensor_id: u32,
    dw_sensor_inst: u32,
    sz_sensor_name_orig: [u8; HWINFO_SENSORS_STRING_LEN2],
    sz_sensor_name_user: [u8; HWINFO_SENSORS_STRING_LEN2],
    utf_sensor_name_user: [u8; HWINFO_SENSORS_STRING_LEN2]
}

#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
struct HwinfoSensorsSharedMem2
{
    dw_signature: u32,
    dw_version: u32,
    dw_revision: u32,
    poll_time: i64,
    dw_offset_of_sensor_section: u32,
    dw_size_of_sensor_element: u32,
    dw_num_sensor_elements: u32,
    // descriptors for the Readings section
    dw_offset_of_reading_section: u32, // Offset of the Reading section from beginning of HWiNFO_SENSORS_SHARED_MEM2
    dw_size_of_reading_element: u32,   // Size of each Reading element = sizeof( HWiNFO_SENSORS_READING_ELEMENT )
    dw_num_reading_elements: u32      // Number of Reading elements
}

pub struct Hwinfo {
    num_reading_elements: u32,
    offset_reading_section: u32,
    size_reading_section: u32,
    shared_memory_name: Vec<u16>,
    pub master_sensor_names: Box<Vec<String>>,
    pub master_label_user: Box<Vec<String>>,
    pub master_readings: Box<HashMap<String,  HashMap<String, Vec<String>>>>
}

#[allow(unused_variables)]
fn init_hwinfo() -> Result<Hwinfo, Box<dyn std::error::Error>>{
    let hwinfo_memory_size = std::mem::size_of::<HwinfoSensorsSharedMem2>();
    // Convert the name to a wide string (UTF-16)
    let shared_memory_name = OsStr::new(HWINFO_SENSORS_MAP_FILE_NAME2)
        .encode_wide()
        .chain(once(0))
        .collect::<Vec<u16>>();
    let shared_memory_handle = unsafe { // Open the named shared memory object for read access
        OpenFileMappingW(
            FILE_MAP_READ,   // Desired access
            0,               // Inherit handle flag
            shared_memory_name.as_ptr(),  // Name of the shared memory object
        )
    };
    if shared_memory_handle.is_null() {
        // println!("Failed to open shared memory object");
        return Err(Box::new(Error::new(ErrorKind::NotFound,"Failed to open shared memory object")));
    }
    let shared_memory_view = unsafe{ // Map the shared memory into the process's address space
        MapViewOfFile(
            shared_memory_handle,
            FILE_MAP_READ,
            0,
            0,
            0,
        )
    };
    if shared_memory_view.is_null() {
        // println!("Failed to map view of shared memory");
        return Err(Box::new(Error::new(ErrorKind::NotFound,"Failed to map view of shared memory")));
    }
    let start = shared_memory_view as *const u8;
    let shared_memory_content = unsafe{
        // Access the shared memory content (unsafe)
        std::slice::from_raw_parts(start, hwinfo_memory_size)
    };
    let hwinfo_memory= unsafe{shared_memory_content.align_to::<HwinfoSensorsSharedMem2>().1[0]};
    let num_sensors = hwinfo_memory.dw_num_sensor_elements;
    let num_reading_elements = hwinfo_memory.dw_num_reading_elements;
    let offset_sensor_section = hwinfo_memory.dw_offset_of_sensor_section;
    let size_sensor_element = hwinfo_memory.dw_size_of_sensor_element;
    let offset_reading_section = hwinfo_memory.dw_offset_of_reading_section;
    let size_reading_section = hwinfo_memory.dw_size_of_reading_element;

    let mut master_sensor_names: Vec<String> = Vec::new();
    #[allow(unused_mut)]
    let mut master_label_user: Vec<String> = Vec::new();
    let mut master_readings: HashMap<String,  HashMap<String, Vec<String>>> = HashMap::new();
    // let mut master_readings_array: Vec<(String, Vec<(String, Vec<String>)>)>;
        
    let size_u32 = std::mem::size_of::<u32>();
    let size_utf16_string = std::mem::size_of::<[u16;HWINFO_SENSORS_STRING_LEN2]>();
    let size_utf8_string = std::mem::size_of::<[u8;HWINFO_SENSORS_STRING_LEN2]>();
    let size_unit_string= std::mem::size_of::<[u8;HWINFO_UNIT_STRING_LEN]>();
    let size_f64 = std::mem::size_of::<f64>();
    let size_sensor_reading_type = std::mem::size_of::<SensorReadingType>();

    // Getting Sensor Labels
    for dw_sensor in 0..num_sensors {
        let offset = offset_sensor_section + (dw_sensor * size_sensor_element);
        let ptr = unsafe {start.offset(offset as isize)};
        let sensor_element = unsafe{std::slice::from_raw_parts(ptr, size_sensor_element as usize)};

        let sensor = unsafe {&sensor_element.align_to::<HwinfoSensorsSensorElement>().1[0]};

        let utf_sensor_name_user = String::from_utf8(sensor.utf_sensor_name_user.to_vec())?;
        master_sensor_names.push(utf_sensor_name_user.clone());

        let blank_reading: HashMap<String, Vec<String>> = HashMap::new();
        master_readings.insert(utf_sensor_name_user.clone(), blank_reading);
    }
    
    unsafe{ // Unmap the shared memory view when done
        UnmapViewOfFile(shared_memory_view);
    }
    Ok(Hwinfo {
        num_reading_elements,
        offset_reading_section,
        size_reading_section,
        shared_memory_name,
        master_sensor_names: Box::new(master_sensor_names),
        master_label_user: Box::new(master_label_user),
        master_readings: Box::new(master_readings)
    })
}

pub fn update_hwinfo() -> Result<Hwinfo, Box<dyn std::error::Error>> {
    let mut info = init_hwinfo()?;

    let shared_memory_handle = unsafe { // Open the named shared memory object for read access
        OpenFileMappingW(
            FILE_MAP_READ,   // Desired access
            0,               // Inherit handle flag
            info.shared_memory_name.as_ptr(),  // Name of the shared memory object
        )
    };
    let shared_memory_view = unsafe{ // Map the shared memory into the process's address space
        MapViewOfFile(
            shared_memory_handle,
            FILE_MAP_READ,
            0,
            0,
            0,
        )
    };
    if shared_memory_view.is_null() {
        println!("Failed to map view of shared memory");
        return Err(Box::new(Error::new(ErrorKind::NotFound,"Failed to map view of shared memory")));
    }
    let start = shared_memory_view as *const u8;
    // Getting Sensor Readings
    for dw_reading in 0..info.num_reading_elements {
        let offset = info.offset_reading_section + (dw_reading * info.size_reading_section);
        let ptr = unsafe {start.offset(offset as isize)};
        
        let sensor_reading = unsafe {std::slice::from_raw_parts(ptr, info.size_reading_section as usize)};
        if sensor_reading.len() != 460 {
            panic!();
        }
        let reading = unsafe {&sensor_reading.align_to::<HwinfoSensorsReadingElement>().1[0]};
        let label = String::from_utf8(reading.utf_label_user.to_vec())?;
        info.master_label_user.insert(0, label);

        // Because the packed struct is unaligned
        let value: f64 = reading.value;
        let value_min: f64 = reading.value_min;
        let value_max: f64 = reading.value_max;
        let value_avg: f64 = reading.value_avg;

        let mut values_list: Vec<String> = Vec::new();
        let unit = String::from_utf8(reading.utf_unit.to_vec())?;
        values_list.push(unit);
        values_list.push(f64::to_string(&value));
        values_list.push(f64::to_string(&value_min));
        values_list.push(f64::to_string(&value_max));
        values_list.push(f64::to_string(&value_avg));

        let dw_sensor_index = reading.dw_sensor_index;
        let current_sensor_name = &info.master_sensor_names[dw_sensor_index as usize];

        if let Some(x) = info.master_readings.get_mut(current_sensor_name){
            x.insert(
                String::from_utf8(reading.sz_label_user.to_vec())?,
                values_list
            );
        }
    }

    unsafe{ // Unmap the shared memory view when done
        UnmapViewOfFile(shared_memory_view);
    }

    Ok(Hwinfo {
        num_reading_elements: info.num_reading_elements,
        offset_reading_section: info.offset_reading_section,
        size_reading_section: info.size_reading_section,
        shared_memory_name: info.shared_memory_name,
        master_sensor_names: info.master_sensor_names,
        master_label_user: info.master_label_user,
        master_readings: info.master_readings
    })
}
