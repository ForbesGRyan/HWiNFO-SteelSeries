use winapi::um::{
    memoryapi::{OpenFileMappingW, FILE_MAP_READ, MapViewOfFile, UnmapViewOfFile},
    winnt::HANDLE
};
use winapi::shared::minwindef::LPVOID;
use std::{ffi::OsStr, iter::once};
use std::os::windows::ffi::OsStrExt;

const HWINFO_SENSORS_MAP_FILE_NAME2: &str = "Global\\HWiNFO_SENS_SM2";
// const HWINFO_SENSORS_SM2_MUTEX: &str = "Global\\HWiNFO_SM2_MUTEX";
const HWINFO_SENSORS_STRING_LEN2: i32  = 128;
const HWINFO_UNIT_STRING_LEN: i32 = 16;
#[allow(dead_code)]
// #[repr(C, packed)]
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
#[derive(Debug, Clone)]
struct HwinfoSensorsSensorElement
{
    dw_sensor_id: u32,
    dw_sensor_inst: u32,
    // [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
    sz_sensor_name_orig: String,
    // [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
    sz_sensor_name_user: String
}
struct HwinfoSensorsReadingElement
{
    t_reading: SensorReadingType,
    dw_sensor_index: u32,
    dw_reading_id: u32,
    // https://stackoverflow.com/questions/73437843/rust-struct-field-str-slice-with-size
    // [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
    sz_label_orig: String,
    // [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_SENSORS_STRING_LEN2)]
    sz_label_user: String,
    // [MarshalAs(UnmanagedType.ByValTStr, SizeConst = HWiNFO_UNIT_STRING_LEN)]
    sz_unit: String,
    value: f64,
    value_min: f64,
    value_max: f64,
    value_avg: f64
}

fn main() -> Result<(), Box<dyn std::error::Error>>{
    let hwinfo_memory_size = std::mem::size_of::<HwinfoSensorsSharedMem2>();
    unsafe {
        // Convert the name to a wide string (UTF-16)
        let shared_memory_name = OsStr::new(HWINFO_SENSORS_MAP_FILE_NAME2)
            .encode_wide()
            .chain(once(0))
            .collect::<Vec<u16>>();
        // Open the named shared memory object for read access
        let shared_memory_handle: HANDLE = OpenFileMappingW(
            FILE_MAP_READ,   // Desired access
            0,               // Inherit handle flag
            shared_memory_name.as_ptr(),  // Name of the shared memory object
        );
        if shared_memory_handle.is_null() {
            println!("Failed to open shared memory object");
            return Ok(());
        }
        // Map the shared memory into the process's address space
        let shared_memory_view: LPVOID = MapViewOfFile(
            shared_memory_handle,
            FILE_MAP_READ,
            0,
            0,
            hwinfo_memory_size,
        );
        if shared_memory_view.is_null() {
            println!("Failed to map view of shared memory");
            return Ok(());
        }
        // Access the shared memory content (unsafe)
        let shared_memory_content = std::slice::from_raw_parts(shared_memory_view as *const u8, hwinfo_memory_size);
        let (_head, data, _tail) = shared_memory_content.align_to::<HwinfoSensorsSharedMem2>();
        let hwinfo_memory = data[0];
        let num_sensors = hwinfo_memory.dw_num_sensor_elements;
        let num_reading_elements = hwinfo_memory.dw_num_reading_elements;
        let offset_sensor_section = hwinfo_memory.dw_offset_of_sensor_section;
        let size_sensor_element = hwinfo_memory.dw_size_of_sensor_element;
        let offset_reading_section = hwinfo_memory.dw_offset_of_reading_section;
        let size_reading_section = hwinfo_memory.dw_size_of_reading_element;
        
        for dw_sensor in 0..num_sensors {
            let offset = offset_sensor_section + (dw_sensor * size_sensor_element);
            let accessor: LPVOID = MapViewOfFile(
                shared_memory_handle,
                FILE_MAP_READ, 
                0, 
                offset, 
                size_sensor_element as usize 
            );
            let sensor_element = std::slice::from_raw_parts(accessor as *const u8, size_sensor_element as usize);
            let (_, data, _) = sensor_element.align_to::<HwinfoSensorsSensorElement>();
            let sensor= data[0].clone();
            // println!("{}", sensor_element.sz_sensor_name_orig);
            // println!("{}", sensor_element.sz_sensor_name_user);
            println!();
        }
        // Unmap the shared memory view when done
        UnmapViewOfFile(shared_memory_view);
    }

    Ok(())
}
