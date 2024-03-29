use anyhow;
use std::io::{Error, ErrorKind};
use std::os::windows::ffi::OsStrExt;
use std::{collections::HashMap, ffi::OsStr, iter::once};
use strum::FromRepr;
use winapi::um::memoryapi::{MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, FILE_MAP_READ};

const HWINFO_SENSORS_MAP_FILE_NAME2: &str = "Global\\HWiNFO_SENS_SM2";
// const HWINFO_SENSORS_SM2_MUTEX: &str = "Global\\HWiNFO_SM2_MUTEX";
const HWINFO_SENSORS_STRING_LEN2: usize = 128;
const HWINFO_UNIT_STRING_LEN: usize = 16;

#[allow(dead_code)]
#[derive(FromRepr, Clone, Copy)]
pub enum SensorReadingType {
    SensorTypeNone = 0,
    SensorTypeTemp,
    SensorTypeVolt,
    SensorTypeFan,
    SensorTypeCurrent,
    SensorTypePower,
    SensorTypeClock,
    SensorTypeUsage,
    SensorTypeOther,
}

#[allow(dead_code)]
#[repr(C, packed(1))]
#[derive(Clone)]
pub struct HwinfoSensorsReadingElement {
    pub t_reading: SensorReadingType,
    _blank: [u8; 3], // For some reason the packing wasn't lining up. This alleviates it
    pub dw_sensor_index: u32,
    pub dw_reading_id: u32,
    pub sz_label_orig: [u8; HWINFO_SENSORS_STRING_LEN2],
    pub sz_label_user: [u8; HWINFO_SENSORS_STRING_LEN2],
    pub sz_unit: [u8; HWINFO_UNIT_STRING_LEN],
    pub value: f64,
    pub value_min: f64,
    pub value_max: f64,
    pub value_avg: f64,
    pub utf_label_user: [u8; HWINFO_SENSORS_STRING_LEN2],
    pub utf_unit: [u8; HWINFO_UNIT_STRING_LEN],
}
impl PartialEq for HwinfoSensorsReadingElement {
    fn eq(&self, other: &Self) -> bool {
        self.dw_reading_id == other.dw_reading_id
            && self.value == other.value
            && self.value_min == other.value_min
            && self.value_max == other.value_max
            && self.value_avg == other.value_avg
    }
}
impl Eq for HwinfoSensorsReadingElement {}

#[allow(dead_code)]
#[repr(C, align(1))]
#[derive(Hash, Clone, Copy)]
pub struct HwinfoSensorsSensorElement {
    pub dw_sensor_id: u32,
    pub dw_sensor_inst: u32,
    pub sz_sensor_name_orig: [u8; HWINFO_SENSORS_STRING_LEN2],
    pub sz_sensor_name_user: [u8; HWINFO_SENSORS_STRING_LEN2],
    pub utf_sensor_name_user: [u8; HWINFO_SENSORS_STRING_LEN2],
}
impl PartialEq for HwinfoSensorsSensorElement {
    fn eq(&self, other: &Self) -> bool {
        self.utf_sensor_name_user == other.utf_sensor_name_user
    }
}
impl Eq for HwinfoSensorsSensorElement {}

#[allow(dead_code)]
#[derive(Debug, Copy, Clone)]
struct HwinfoSensorsSharedMem2 {
    dw_signature: u32,
    dw_version: u32,
    dw_revision: u32,
    poll_time: i64,
    dw_offset_of_sensor_section: u32,
    dw_size_of_sensor_element: u32,
    dw_num_sensor_elements: u32,
    // descriptors for the Readings section
    dw_offset_of_reading_section: u32, // Offset of the Reading section from beginning of HWiNFO_SENSORS_SHARED_MEM2
    dw_size_of_reading_element: u32, // Size of each Reading element = sizeof( HWiNFO_SENSORS_READING_ELEMENT )
    dw_num_reading_elements: u32,    // Number of Reading elements
}

#[derive(Clone)]
pub struct Sensor {
    pub sensor: Box<HwinfoSensorsSensorElement>,
    pub reading: Box<HashMap<String, Box<HwinfoSensorsReadingElement>>>,
}
impl PartialEq for Sensor {
    fn eq(&self, other: &Self) -> bool {
        self.reading == other.reading
    }
}
impl Eq for Sensor {}

type _MasterReadings = Box<HashMap<String, Sensor>>;
#[derive(Clone)]
pub struct MasterReadings {
    pub sensors: Box<HashMap<String, Sensor>>,
}
impl PartialEq for MasterReadings {
    fn eq(&self, other: &Self) -> bool {
        self.sensors == other.sensors
    }
}
impl Eq for MasterReadings {}

#[derive(Clone)]
pub struct Hwinfo {
    num_reading_elements: u32,
    offset_reading_section: u32,
    size_reading_section: u32,
    shared_memory_name: Vec<u16>,
    pub master_sensor_names: Box<Vec<String>>,
    pub master_label_user: Box<Vec<String>>,
    // pub master_readings: Box<HashMap<String, HashMap<String, (String, [f64; 4])>>>,
    // pub new_master_reading: Box<HashMap<HwinfoSensorsSensorElement, HashMap<String, HwinfoSensorsReadingElement>>>
    pub master_readings: Box<MasterReadings>,
}
impl PartialEq for Hwinfo {
    fn eq(&self, other: &Self) -> bool {
        self.master_readings == other.master_readings
    }
}
impl Eq for Hwinfo {}

impl Hwinfo {
    pub fn new() -> Result<Hwinfo, anyhow::Error> {
        let hwinfo_memory_size = std::mem::size_of::<HwinfoSensorsSharedMem2>();
        // Convert the name to a wide string (UTF-16)
        let shared_memory_name = OsStr::new(HWINFO_SENSORS_MAP_FILE_NAME2)
            .encode_wide()
            .chain(once(0))
            .collect::<Vec<u16>>();
        let shared_memory_handle = unsafe {
            // Open the named shared memory object for read access
            OpenFileMappingW(
                FILE_MAP_READ,               // Desired access
                0,                           // Inherit handle flag
                shared_memory_name.as_ptr(), // Name of the shared memory object
            )
        };
        if shared_memory_handle.is_null() {
            // println!("Failed to open shared memory object");
            return Err(anyhow::Error::new(Error::new(
                ErrorKind::NotFound,
                "Failed to open shared memory object",
            )));
        }
        let shared_memory_view = unsafe {
            // Map the shared memory into the process's address space
            MapViewOfFile(shared_memory_handle, FILE_MAP_READ, 0, 0, 0)
        };
        if shared_memory_view.is_null() {
            // println!("Failed to map view of shared memory");
            return Err(anyhow::Error::new(Error::new(
                ErrorKind::NotFound,
                "Failed to map view of shared memory",
            )));
        }
        let start = shared_memory_view as *const u8;
        let shared_memory_content = unsafe {
            // Access the shared memory content (unsafe)
            std::slice::from_raw_parts(start, hwinfo_memory_size)
        };
        let hwinfo_memory = unsafe {
            shared_memory_content
                .align_to::<HwinfoSensorsSharedMem2>()
                .1[0]
        };
        let num_sensors = hwinfo_memory.dw_num_sensor_elements;
        let num_reading_elements = hwinfo_memory.dw_num_reading_elements;
        let offset_sensor_section = hwinfo_memory.dw_offset_of_sensor_section;
        let size_sensor_element = hwinfo_memory.dw_size_of_sensor_element;
        let offset_reading_section = hwinfo_memory.dw_offset_of_reading_section;
        let size_reading_section = hwinfo_memory.dw_size_of_reading_element;

        let mut master_sensor_names: Vec<String> = Vec::new();
        // #[allow(unused_mut)]
        let master_label_user: Vec<String> = Vec::new();
        // let mut master_readings: HashMap<String, HashMap<String, (String, [f64; 4])>> =
        //     HashMap::new();
        // let mut new_master_readings: HashMap<HwinfoSensorsSensorElement, HashMap<String, HwinfoSensorsReadingElement>> = HashMap::new();
        let mut master_readings = MasterReadings {
            sensors: Box::new(HashMap::new()),
        };

        // Getting Sensor Labels
        for dw_sensor in 0..num_sensors {
            let offset = offset_sensor_section + (dw_sensor * size_sensor_element);
            let ptr = unsafe { start.offset(offset as isize) };
            let sensor_element =
                unsafe { std::slice::from_raw_parts(ptr, size_sensor_element as usize) };
            let sensor = unsafe { &sensor_element.align_to::<HwinfoSensorsSensorElement>().1[0] };
            let utf_sensor_name_user = String::from_utf8(sensor.utf_sensor_name_user.to_vec())?
                .trim_matches(char::from(0))
                .to_string();
            master_sensor_names.push(utf_sensor_name_user.clone());
            // let blank_reading: HashMap<String, (String, [f64; 4])> = HashMap::new();
            // master_readings.insert(utf_sensor_name_user.clone(), blank_reading);

            // _master_readings.push(Sensor { sensor: *sensor, reading: HashMap::new() })
            master_readings.sensors.insert(
                utf_sensor_name_user.clone(),
                Sensor {
                    sensor: Box::new(*sensor),
                    reading: Box::new(HashMap::new()),
                },
            );

            // let new_blank: HashMap<String, HwinfoSensorsReadingElement> = HashMap::new();
            // new_master_readings.insert(*sensor, new_blank);
        }

        unsafe {
            // Unmap the shared memory view when done
            UnmapViewOfFile(shared_memory_view);
        }
        Ok(Hwinfo {
            num_reading_elements,
            offset_reading_section,
            size_reading_section,
            shared_memory_name,
            master_sensor_names: Box::new(master_sensor_names),
            master_label_user: Box::new(master_label_user),
            // master_readings: Box::new(master_readings),
            master_readings: Box::new(master_readings), // new_master_reading: Box::new(new_master_readings)
        })
    }

    pub fn pull(&mut self) -> Result<(), anyhow::Error> {
        // let mut hwinfo = self.new()?;

        let shared_memory_handle = unsafe {
            // Open the named shared memory object for read access
            OpenFileMappingW(
                FILE_MAP_READ,                    // Desired access
                0,                                // Inherit handle flag
                self.shared_memory_name.as_ptr(), // Name of the shared memory object
            )
        };
        let shared_memory_view = unsafe {
            // Map the shared memory into the process's address space
            MapViewOfFile(shared_memory_handle, FILE_MAP_READ, 0, 0, 0)
        };
        if shared_memory_view.is_null() {
            println!("Failed to map view of shared memory");
            return Err(anyhow::Error::new(Error::new(
                ErrorKind::NotFound,
                "Failed to map view of shared memory",
            )));
        }
        let start = shared_memory_view as *const u8;
        // Getting Sensor Readings
        for dw_reading in 0..self.num_reading_elements {
            let offset = self.offset_reading_section + (dw_reading * self.size_reading_section);
            let ptr = unsafe { start.offset(offset as isize) };

            let sensor_reading =
                unsafe { std::slice::from_raw_parts(ptr, self.size_reading_section as usize) };
            // if sensor_reading.len() != 460 {
            //     panic!();
            // }
            let reading = unsafe { &sensor_reading.align_to::<HwinfoSensorsReadingElement>().1[0] };
            let label = String::from_utf8(reading.utf_label_user.to_vec())?
                .trim_matches(char::from(0))
                .to_string();
            // self.master_label_user.insert(0, label);

            // Because the packed struct is unaligned
            // let value: f64 = reading.value;
            // let value_min: f64 = reading.value_min;
            // let value_max: f64 = reading.value_max;
            // let value_avg: f64 = reading.value_avg;

            // let mut values_list: [f64; 4] = [0.0_f64; 4];
            // let unit = String::from_utf8(reading.utf_unit.to_vec())?
            //     .trim_matches(char::from(0))
            //     .to_string();
            // // values_list.push(unit);
            // values_list[0] = value;
            // values_list[1] = value_min;
            // values_list[2] = value_max;
            // values_list[3] = value_avg;

            let current_sensor_name = &self.master_sensor_names[reading.dw_sensor_index as usize];
            // if let Some(x) = self.master_readings.get_mut(current_sensor_name) {
            //     x.insert(label.clone(), (unit, values_list));
            // }

            if let Some(sensor) = self.master_readings.sensors.get_mut(current_sensor_name) {
                sensor.reading.insert(label, Box::new(reading.to_owned()));
            }
        }

        unsafe {
            // Unmap the shared memory view when done
            UnmapViewOfFile(shared_memory_view);
        }
        Ok(())
        // Ok(Hwinfo {
        //     num_reading_elements:   self.num_reading_elements,
        //     offset_reading_section: self.offset_reading_section,
        //     size_reading_section:   self.size_reading_section,
        //     shared_memory_name:     self.shared_memory_name.clone(),
        //     master_sensor_names:    self.master_sensor_names.clone(),
        //     master_label_user:      self.master_label_user.clone(),
        //     master_readings:        self.master_readings.clone()
        // })
    }

    pub fn get(&self, sensor_key: &str, reading_key: &str) -> Option<&HwinfoSensorsReadingElement> {
        match self.master_readings.sensors.get(sensor_key) {
            Some(sensor) => match sensor.reading.get(reading_key) {
                Some(reading) => Some(reading),
                None => None,
            },
            None => None,
        }
    }

    pub fn find_first(&self, key: &str) -> Result<&HwinfoSensorsReadingElement, anyhow::Error> {
        for (_i, sensor) in self.master_readings.sensors.iter() {
            for (j, _reading) in sensor.reading.iter() {
                if j == &key.to_string() {
                    return Ok(_reading);
                }
            }
        }
        Err(anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Not found",
        )))
    }

    pub fn find(&self, key: &str) -> Result<Vec<&HwinfoSensorsReadingElement>, anyhow::Error> {
        let mut results: Vec<&HwinfoSensorsReadingElement> = Vec::new();
        for (_i, sensor) in self.master_readings.sensors.iter() {
            for (j, _reading) in sensor.reading.iter() {
                if j == &key.to_string() {
                    results.push(_reading);
                    // return Ok(_reading);
                }
            }
        }

        if results.len() == 0 {
            Err(anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Not found",
            )))
        } else {
            Ok(results)
        }
    }
}

// impl Drop for Hwinfo {
//     fn drop(&mut self){
//         unsafe{ UnmapViewOfFile(self.shared_memory_name) };
//     }
// }
