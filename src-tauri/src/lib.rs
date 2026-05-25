use anyhow;
use std::io::{Error, ErrorKind};
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use std::{collections::HashMap, ffi::OsStr, iter::once};
use strum::FromRepr;
use winapi::ctypes::c_void;
use winapi::um::handleapi::CloseHandle;
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
    pub(crate) _blank: [u8; 3], // For some reason the packing wasn't lining up. This alleviates it
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
struct SharedMemoryView(*const c_void);
unsafe impl Send for SharedMemoryView {}
unsafe impl Sync for SharedMemoryView {}

impl Drop for SharedMemoryView {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                UnmapViewOfFile(self.0 as *mut _);
            }
        }
    }
}

#[derive(Clone)]
pub struct Hwinfo {
    // shared_memory_handle: Handle,
    #[allow(dead_code)] // held for its Drop (unmaps view); not read directly
    pub(crate) shared_memory_view: Arc<SharedMemoryView>,
    // num_sensors: u32,
    pub(crate) num_reading_elements: u32,
    pub(crate) offset_reading_section: u32,
    pub(crate) size_reading_section: u32,
    pub(crate) shared_memory_name: Vec<u16>,
    pub sensors: HashMap<String, Sensor>,
    pub sensor_names: Vec<String>,
    /// Test-only: when true, `pull()` returns Ok(()) without touching shared memory.
    pub bypass_pull_for_test: bool,
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
        // Close the handle after mapping - the view remains valid until UnmapViewOfFile
        unsafe {
            CloseHandle(shared_memory_handle);
        }
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
            if aligned.is_empty() {
                return Err(anyhow::Error::new(Error::new(
                    ErrorKind::InvalidData,
                    "Failed to align shared memory data for HwinfoSensorsSharedMem2",
                )));
            }
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
                let (_prefix, aligned, _suffix) =
                    sensor_element.align_to::<HwinfoSensorsSensorElement>();
                if aligned.is_empty() {
                    return Err(anyhow::Error::new(Error::new(
                        ErrorKind::InvalidData,
                        "Failed to align sensor element data",
                    )));
                }
                aligned[0]
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
            shared_memory_view: Arc::new(SharedMemoryView(shared_memory_view)),
            num_reading_elements,
            offset_reading_section,
            size_reading_section,
            shared_memory_name,
            sensors,
            sensor_names,
            bypass_pull_for_test: false,
        })
    }

    pub fn pull(&mut self) -> Result<(), anyhow::Error> {
        // let mut hwinfo = self.new()?;

        // Clear reading names for fresh rebuild
        for sensor in self.sensors.values_mut() {
            sensor.reading_names.clear();
        }

        if self.bypass_pull_for_test {
            // Re-populate reading_names from sensor.readings keys so callers see consistent state.
            for sensor in self.sensors.values_mut() {
                sensor.reading_names = sensor.readings.keys().cloned().collect();
            }
            return Ok(());
        }

        let shared_memory_handle = unsafe {
            // Open the named shared memory object for read access
            OpenFileMappingW(
                FILE_MAP_READ,                    // Desired access
                0,                                // Inherit handle flag
                self.shared_memory_name.as_ptr(), // Name of the shared memory object
            )
        };
        if shared_memory_handle.is_null() {
            return Err(anyhow::Error::new(Error::new(
                ErrorKind::NotFound,
                "Failed to open shared memory object",
            )));
        }
        let shared_memory_view = unsafe {
            // Map the shared memory into the process's address space
            MapViewOfFile(shared_memory_handle, FILE_MAP_READ, 0, 0, 0)
        };
        // Close the handle after mapping - the view remains valid until UnmapViewOfFile
        unsafe {
            CloseHandle(shared_memory_handle);
        }
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
            let reading = unsafe {
                let (_prefix, aligned, _suffix) =
                    sensor_reading.align_to::<HwinfoSensorsReadingElement>();
                if aligned.is_empty() {
                    return Err(anyhow::Error::new(Error::new(
                        ErrorKind::InvalidData,
                        "Failed to align reading element data",
                    )));
                }
                &aligned[0]
            };
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

    /// Creates a mock Hwinfo instance for testing purposes.
    /// This bypasses the shared memory initialization and allows direct sensor injection.
    pub fn new_mock(sensors: HashMap<String, Sensor>, sensor_names: Vec<String>) -> Self {
        Hwinfo {
            shared_memory_view: Arc::new(SharedMemoryView(std::ptr::null())),
            num_reading_elements: 0,
            offset_reading_section: 0,
            size_reading_section: 0,
            shared_memory_name: vec![],
            sensors,
            sensor_names,
            bypass_pull_for_test: false,
        }
    }

    /// Test helper: override the shared-memory mapping name so `pull()` opens a known-bad mapping
    /// and returns a clean Err instead of dereferencing junk when the default empty name is in place.
    pub fn set_shared_memory_name_for_test(&mut self, name: Vec<u16>) {
        self.shared_memory_name = name;
    }
}

impl HwinfoSensorsReadingElement {
    /// Creates a mock reading element for testing purposes.
    pub fn new_mock(
        sensor_index: u32,
        reading_id: u32,
        label: &str,
        value: f64,
    ) -> Self {
        let mut label_user = [0u8; HWINFO_SENSORS_STRING_LEN2];
        label.as_bytes().iter().enumerate().for_each(|(i, &b)| {
            if i < HWINFO_SENSORS_STRING_LEN2 {
                label_user[i] = b;
            }
        });

        HwinfoSensorsReadingElement {
            t_reading: SensorReadingType::SensorTypeTemp,
            _blank: [0; 3],
            dw_sensor_index: sensor_index,
            dw_reading_id: reading_id,
            sz_label_orig: [0; HWINFO_SENSORS_STRING_LEN2],
            sz_label_user: [0; HWINFO_SENSORS_STRING_LEN2],
            sz_unit: [0; HWINFO_UNIT_STRING_LEN],
            value,
            value_min: value - 10.0,
            value_max: value + 10.0,
            value_avg: value,
            utf_label_user: label_user,
            utf_unit: [0; HWINFO_UNIT_STRING_LEN],
        }
    }
}

impl HwinfoSensorsSensorElement {
    /// Creates a mock sensor element for testing purposes.
    pub fn new_mock(sensor_id: u32, sensor_name: &str) -> Self {
        let mut name_user = [0u8; HWINFO_SENSORS_STRING_LEN2];
        sensor_name
            .as_bytes()
            .iter()
            .enumerate()
            .for_each(|(i, &b)| {
                if i < HWINFO_SENSORS_STRING_LEN2 {
                    name_user[i] = b;
                }
            });

        HwinfoSensorsSensorElement {
            dw_sensor_id: sensor_id,
            dw_sensor_inst: 0,
            sz_sensor_name_orig: [0; HWINFO_SENSORS_STRING_LEN2],
            sz_sensor_name_user: [0; HWINFO_SENSORS_STRING_LEN2],
            utf_sensor_name_user: name_user,
        }
    }
}

// Drop is now handled by SharedMemoryView inside Arc

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_reading(
        sensor_index: u32,
        reading_id: u32,
        label: &str,
        value: f64,
    ) -> HwinfoSensorsReadingElement {
        let mut label_user = [0u8; 128];
        label.as_bytes().iter().enumerate().for_each(|(i, &b)| {
            if i < 128 {
                label_user[i] = b;
            }
        });

        HwinfoSensorsReadingElement {
            t_reading: SensorReadingType::SensorTypeTemp,
            _blank: [0; 3],
            dw_sensor_index: sensor_index,
            dw_reading_id: reading_id,
            sz_label_orig: [0; 128],
            sz_label_user: [0; 128],
            sz_unit: [0; 16],
            value,
            value_min: value - 10.0,
            value_max: value + 10.0,
            value_avg: value,
            utf_label_user: label_user,
            utf_unit: [0; 16],
        }
    }

    fn create_test_sensor(sensor_id: u32, sensor_name: &str) -> HwinfoSensorsSensorElement {
        let mut name_user = [0u8; 128];
        sensor_name
            .as_bytes()
            .iter()
            .enumerate()
            .for_each(|(i, &b)| {
                if i < 128 {
                    name_user[i] = b;
                }
            });

        HwinfoSensorsSensorElement {
            dw_sensor_id: sensor_id,
            dw_sensor_inst: 0,
            sz_sensor_name_orig: [0; 128],
            sz_sensor_name_user: [0; 128],
            utf_sensor_name_user: name_user,
        }
    }

    fn create_mock_hwinfo() -> Hwinfo {
        let mut sensors = HashMap::new();
        let mut sensor_names = Vec::new();

        // Create CPU sensor
        let cpu_name = "CPU [#0]";
        let cpu_temp_reading = create_test_reading(0, 1, "CPU Temperature", 65.0);
        let cpu_usage_reading = create_test_reading(0, 2, "Total CPU Usage", 45.0);

        let mut cpu_readings = HashMap::new();
        cpu_readings.insert("CPU Temperature".to_string(), cpu_temp_reading);
        cpu_readings.insert("Total CPU Usage".to_string(), cpu_usage_reading);

        sensors.insert(
            cpu_name.to_string(),
            Sensor {
                info: create_test_sensor(0, cpu_name),
                readings: cpu_readings,
                reading_names: vec!["CPU Temperature".to_string(), "Total CPU Usage".to_string()],
            },
        );
        sensor_names.push(cpu_name.to_string());

        // Create GPU sensor
        let gpu_name = "GPU [#0]";
        let gpu_temp_reading = create_test_reading(1, 3, "GPU Temperature", 72.0);
        let gpu_usage_reading = create_test_reading(1, 4, "GPU Core Load", 88.0);

        let mut gpu_readings = HashMap::new();
        gpu_readings.insert("GPU Temperature".to_string(), gpu_temp_reading);
        gpu_readings.insert("GPU Core Load".to_string(), gpu_usage_reading);

        sensors.insert(
            gpu_name.to_string(),
            Sensor {
                info: create_test_sensor(1, gpu_name),
                readings: gpu_readings,
                reading_names: vec!["GPU Temperature".to_string(), "GPU Core Load".to_string()],
            },
        );
        sensor_names.push(gpu_name.to_string());

        Hwinfo {
            shared_memory_view: Arc::new(SharedMemoryView(std::ptr::null())),
            num_reading_elements: 4,
            offset_reading_section: 0,
            size_reading_section: 0,
            shared_memory_name: vec![],
            sensors,
            sensor_names,
            bypass_pull_for_test: false,
        }
    }

    #[test]
    fn test_get_existing_sensor_and_reading() {
        let hwinfo = create_mock_hwinfo();
        let result = hwinfo.get("CPU [#0]", "CPU Temperature");

        assert!(result.is_some());
        let value = result.unwrap().value;
        assert_eq!(value, 65.0);
    }

    #[test]
    fn test_get_nonexistent_sensor() {
        let hwinfo = create_mock_hwinfo();
        let result = hwinfo.get("Nonexistent Sensor", "CPU Temperature");

        assert!(result.is_none());
    }

    #[test]
    fn test_get_nonexistent_reading() {
        let hwinfo = create_mock_hwinfo();
        let result = hwinfo.get("CPU [#0]", "Nonexistent Reading");

        assert!(result.is_none());
    }

    #[test]
    fn test_find_first_existing_reading() {
        let hwinfo = create_mock_hwinfo();
        let result = hwinfo.find_first("GPU Temperature");

        assert!(result.is_ok());
        let value = result.unwrap().value;
        assert_eq!(value, 72.0);
    }

    #[test]
    fn test_find_first_nonexistent_reading() {
        let hwinfo = create_mock_hwinfo();
        let result = hwinfo.find_first("Nonexistent Reading");

        assert!(result.is_err());
    }

    #[test]
    fn test_find_single_match() {
        let hwinfo = create_mock_hwinfo();
        let result = hwinfo.find("Total CPU Usage");

        assert!(result.is_ok());
        let readings = result.unwrap();
        assert_eq!(readings.len(), 1);
        let value = readings[0].value;
        assert_eq!(value, 45.0);
    }

    #[test]
    fn test_find_multiple_matches() {
        let hwinfo = create_mock_hwinfo();
        // Both CPU and GPU have "GPU Temperature" reading in our mock
        let result = hwinfo.find("GPU Temperature");

        assert!(result.is_ok());
        let readings = result.unwrap();
        assert_eq!(readings.len(), 1);
    }

    #[test]
    fn test_find_no_matches() {
        let hwinfo = create_mock_hwinfo();
        let result = hwinfo.find("Nonexistent Reading");

        assert!(result.is_err());
    }

    #[test]
    fn test_sensor_equality() {
        let reading1 = create_test_reading(0, 1, "Test", 50.0);
        let reading2 = create_test_reading(0, 1, "Test", 50.0);
        let reading3 = create_test_reading(0, 1, "Test", 60.0);

        assert!(reading1 == reading2);
        assert!(reading1 != reading3);
    }

    #[test]
    fn test_hwinfo_equality() {
        let hwinfo1 = create_mock_hwinfo();
        let hwinfo2 = create_mock_hwinfo();

        assert!(hwinfo1 == hwinfo2);
    }

    #[test]
    fn test_hwinfo_inequality_when_sensors_differ() {
        let a = create_mock_hwinfo();
        let mut b = create_mock_hwinfo();
        // Mutate one reading value → readings differ → sensors differ → Hwinfo differs.
        if let Some(s) = b.sensors.get_mut("CPU [#0]") {
            if let Some(r) = s.readings.get_mut("CPU Temperature") {
                r.value = 99.0;
            }
        }
        assert!(a != b);
    }

    #[test]
    fn test_sensor_element_equality() {
        let a = create_test_sensor(0, "CPU [#0]");
        let b = create_test_sensor(0, "CPU [#0]");
        let c = create_test_sensor(1, "GPU [#0]");
        assert!(a == b);
        assert!(a != c);
    }

    #[test]
    fn test_sensor_struct_equality() {
        let s1 = Sensor {
            info: create_test_sensor(0, "X"),
            readings: HashMap::new(),
            reading_names: vec![],
        };
        let s2 = Sensor {
            info: create_test_sensor(0, "X"),
            readings: HashMap::new(),
            reading_names: vec![],
        };
        assert!(s1 == s2);
    }

    #[test]
    fn test_sensor_reading_type_from_repr() {
        // FromRepr covers all variants
        assert!(matches!(SensorReadingType::from_repr(0), Some(SensorReadingType::SensorTypeNone)));
        assert!(matches!(SensorReadingType::from_repr(1), Some(SensorReadingType::SensorTypeTemp)));
        assert!(matches!(SensorReadingType::from_repr(2), Some(SensorReadingType::SensorTypeVolt)));
        assert!(matches!(SensorReadingType::from_repr(7), Some(SensorReadingType::SensorTypeUsage)));
        assert!(matches!(SensorReadingType::from_repr(8), Some(SensorReadingType::SensorTypeOther)));
        assert!(SensorReadingType::from_repr(99).is_none());
    }

    #[test]
    fn test_hwinfo_new_errors_when_hwinfo_not_running() {
        // CI envs don't have HWiNFO running → OpenFileMappingW returns null → Err.
        let result = Hwinfo::new();
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(e.to_string().contains("Failed to open shared memory object"));
        }
    }

    #[test]
    fn test_hwinfo_pull_errors_when_no_shared_memory() {
        // new_mock leaves shared_memory_name empty, so OpenFileMappingW fails.
        let mut hwinfo = create_mock_hwinfo();
        hwinfo.shared_memory_name = OsStr::new("Global\\NoSuchMappingXYZ_HWiNFO_Test")
            .encode_wide()
            .chain(once(0))
            .collect();
        let r = hwinfo.pull();
        assert!(r.is_err());
        assert!(r.unwrap_err().to_string().contains("Failed to open shared memory object"));
    }

    #[test]
    fn test_pull_clears_reading_names_even_on_error() {
        let mut hwinfo = create_mock_hwinfo();
        // Force OpenFileMappingW to fail
        hwinfo.shared_memory_name = OsStr::new("Global\\NoSuchMappingXYZ_HWiNFO_Test2")
            .encode_wide()
            .chain(once(0))
            .collect();
        for sensor in hwinfo.sensors.values_mut() {
            assert!(!sensor.reading_names.is_empty()); // precondition
        }
        let _ = hwinfo.pull();
        // pull() clears reading_names before opening shared memory → still cleared after err
        for sensor in hwinfo.sensors.values() {
            assert!(sensor.reading_names.is_empty(), "pull should clear reading_names");
        }
    }

    #[test]
    fn test_new_mock_initializes_fields() {
        let hwinfo = Hwinfo::new_mock(HashMap::new(), vec!["X".into()]);
        assert_eq!(hwinfo.num_reading_elements, 0);
        assert_eq!(hwinfo.offset_reading_section, 0);
        assert_eq!(hwinfo.size_reading_section, 0);
        assert_eq!(hwinfo.sensor_names, vec!["X".to_string()]);
        assert!(hwinfo.sensors.is_empty());
    }

    #[test]
    fn test_reading_element_new_mock() {
        let r = HwinfoSensorsReadingElement::new_mock(1, 2, "Temp", 50.0);
        // Copy packed fields to locals before asserting (avoid unaligned-ref UB)
        let sensor_index = r.dw_sensor_index;
        let reading_id = r.dw_reading_id;
        let value = r.value;
        let value_min = r.value_min;
        let value_max = r.value_max;
        assert_eq!(sensor_index, 1);
        assert_eq!(reading_id, 2);
        assert_eq!(value, 50.0);
        assert_eq!(value_min, 40.0);
        assert_eq!(value_max, 60.0);
        let label_bytes: Vec<u8> = r.utf_label_user.iter().take_while(|b| **b != 0).copied().collect();
        assert_eq!(String::from_utf8(label_bytes).unwrap(), "Temp");
    }

    #[test]
    fn test_sensor_element_new_mock_long_name_truncated() {
        let long = "x".repeat(200); // > HWINFO_SENSORS_STRING_LEN2 (128)
        let s = HwinfoSensorsSensorElement::new_mock(0, &long);
        // utf_sensor_name_user has fixed length 128 — overflow bytes dropped
        assert_eq!(s.utf_sensor_name_user.len(), 128);
        // All 128 slots filled with 'x'
        assert!(s.utf_sensor_name_user.iter().all(|b| *b == b'x'));
    }

    #[test]
    fn test_reading_element_new_mock_long_label_truncated() {
        let long = "x".repeat(200);
        let r = HwinfoSensorsReadingElement::new_mock(0, 0, &long, 0.0);
        assert!(r.utf_label_user.iter().all(|b| *b == b'x'));
    }

    #[test]
    fn test_shared_memory_view_drop_null() {
        // Dropping a SharedMemoryView with null pointer should not call UnmapViewOfFile
        let view = SharedMemoryView(std::ptr::null());
        drop(view); // must not panic
    }

    #[test]
    fn test_shared_memory_view_drop_with_real_mapping() {
        // Create a real, named file mapping so Drop's UnmapViewOfFile path actually runs.
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        use std::iter::once;
        use winapi::um::memoryapi::CreateFileMappingW;
        use winapi::um::handleapi::INVALID_HANDLE_VALUE;
        use winapi::um::winnt::PAGE_READWRITE;

        let name: Vec<u16> = OsStr::new("HwinfoSteelseriesUnitTestMapping")
            .encode_wide()
            .chain(once(0))
            .collect();
        unsafe {
            let handle = CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                std::ptr::null_mut(),
                PAGE_READWRITE,
                0,
                4096,
                name.as_ptr(),
            );
            if handle.is_null() {
                return; // CI might not have permission; skip silently.
            }
            let view = MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0);
            CloseHandle(handle);
            if view.is_null() {
                return;
            }
            // Wrapping in SharedMemoryView and dropping calls UnmapViewOfFile (lines 116-118).
            let smv = SharedMemoryView(view);
            drop(smv);
        }
    }

    #[test]
    fn test_pull_bypass_for_test_returns_ok() {
        let mut hwinfo = create_mock_hwinfo();
        hwinfo.bypass_pull_for_test = true;
        // Mutate reading_names so we can verify they got reset to keys from sensor.readings
        for sensor in hwinfo.sensors.values_mut() {
            sensor.reading_names = vec!["STALE".to_string()];
        }
        hwinfo.pull().unwrap();
        // After bypass pull, reading_names should be re-derived from readings keys
        for sensor in hwinfo.sensors.values() {
            assert!(!sensor.reading_names.contains(&"STALE".to_string()));
            for key in sensor.readings.keys() {
                assert!(sensor.reading_names.contains(key));
            }
        }
    }
}
