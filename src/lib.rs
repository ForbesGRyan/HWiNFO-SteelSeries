use anyhow;
use std::io::{Error, ErrorKind};
use std::os::windows::ffi::OsStrExt;
use std::{collections::HashMap, ffi::OsStr, iter::once};
use strum::FromRepr;
use winapi::ctypes::c_void;
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
    // Don't Touch
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
    // Don't Touch
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
    // Don't Touch
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
    pub info: HwinfoSensorsSensorElement,
    pub readings: HashMap<String, HwinfoSensorsReadingElement>,
    pub reading_names: Vec<String>,
}
impl PartialEq for Sensor {
    fn eq(&self, other: &Self) -> bool {
        self.readings == other.readings
    }
}
impl Eq for Sensor {}

#[derive(Clone)]
pub struct Hwinfo {
    // shared_memory_handle: Handle,
    shared_memory_view: *const c_void,
    // num_sensors: u32,
    num_reading_elements: u32,
    offset_reading_section: u32,
    size_reading_section: u32,
    shared_memory_name: Vec<u16>,
    pub sensors: HashMap<String, Sensor>,
    pub sensor_names: Vec<String>,
}
impl PartialEq for Hwinfo {
    fn eq(&self, other: &Self) -> bool {
        self.sensors == other.sensors
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
        let start: *const u8 = shared_memory_view as *const u8;
        let hwinfo_memory: HwinfoSensorsSharedMem2 = unsafe {
            let shared_memory_content = std::slice::from_raw_parts(start, hwinfo_memory_size);
            let (_prefix, aligned, _suffix) =
                shared_memory_content.align_to::<HwinfoSensorsSharedMem2>();
            // .1[0]
            aligned[0]
        };
        let num_sensors = hwinfo_memory.dw_num_sensor_elements;
        let num_reading_elements = hwinfo_memory.dw_num_reading_elements;
        let offset_sensor_section = hwinfo_memory.dw_offset_of_sensor_section;
        let size_sensor_element = hwinfo_memory.dw_size_of_sensor_element;
        let offset_reading_section = hwinfo_memory.dw_offset_of_reading_section;
        let size_reading_section = hwinfo_memory.dw_size_of_reading_element;

        let mut sensors: HashMap<String, Sensor> = HashMap::new();
        let mut sensor_names: Vec<String> = Vec::new(); // pre-allocate with num_sensors?

        // Getting Sensor Labels
        for dw_sensor in 0..num_sensors {
            let offset = offset_sensor_section + (dw_sensor * size_sensor_element);
            let sensor = unsafe {
                let ptr = start.offset(offset as isize);
                let sensor_element = std::slice::from_raw_parts(ptr, size_sensor_element as usize);
                let (_prefix, sensor, _suffix) =
                    &sensor_element.align_to::<HwinfoSensorsSensorElement>();
                sensor[0]
            };
            let sensor_name = String::from_utf8(sensor.utf_sensor_name_user.to_vec())?
                .trim_matches(char::from(0))
                .to_string();

            sensor_names.push(sensor_name.clone());

            sensors.insert(
                sensor_name,
                Sensor {
                    info: sensor,
                    readings: HashMap::new(),
                    reading_names: Vec::new(),
                },
            );
        }

        // unsafe {
        //     // Unmap the shared memory view when done
        //     UnmapViewOfFile(shared_memory_view);
        // }
        Ok(Hwinfo {
            shared_memory_view,
            num_reading_elements,
            offset_reading_section,
            size_reading_section,
            shared_memory_name,
            sensors,
            sensor_names,
        })
    }

    pub fn pull(&mut self) -> Result<(), anyhow::Error> {
        // let mut hwinfo = self.new()?;

        // Clear reading names for fresh rebuild
        for sensor in self.sensors.values_mut() {
            sensor.reading_names.clear();
        }

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

            let sensor_name = &self.sensor_names[reading.dw_sensor_index as usize];

            if let Some(sensor) = self.sensors.get_mut(sensor_name) {
                sensor.reading_names.push(label.clone());
                sensor.readings.insert(label, reading.clone());
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
        self.sensors
            .get(sensor_key)
            .and_then(|sensor| sensor.readings.get(reading_key))
    }

    pub fn find_first(&self, key: &str) -> Result<&HwinfoSensorsReadingElement, anyhow::Error> {
        for sensor in self.sensors.values() {
            if let Some(reading) = sensor.readings.get(key) {
                return Ok(reading);
            }
        }
        Err(anyhow::Error::new(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "Not found",
        )))
    }

    pub fn find(&self, key: &str) -> Result<Vec<&HwinfoSensorsReadingElement>, anyhow::Error> {
        let mut results: Vec<&HwinfoSensorsReadingElement> = Vec::new();
        for sensor in self.sensors.values() {
            if let Some(reading) = sensor.readings.get(key) {
                results.push(reading);
            }
        }

        if results.is_empty() {
            Err(anyhow::Error::new(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Not found",
            )))
        } else {
            Ok(results)
        }
    }
}

impl Drop for Hwinfo {
    fn drop(&mut self) {
        unsafe {
            UnmapViewOfFile(self.shared_memory_view as *mut _);
            // CloseHandle(self.shared_memory_handle);
        };
    }
}
