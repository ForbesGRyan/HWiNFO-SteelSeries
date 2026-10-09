//! HWiNFO shared-memory reader.
//!
//! HWiNFO publishes its sensors in the `Global\HWiNFO_SENS_SM2` file mapping:
//! a header, a table of sensor elements and a table of reading elements. The
//! structs are packed and little-endian, so fields are read from the mapped
//! bytes at fixed offsets rather than through `#[repr]` structs.

use anyhow::{anyhow, bail, Result};
use std::collections::HashMap;
use std::ffi::OsStr;
use std::iter::once;
use std::os::windows::ffi::OsStrExt;
use std::sync::Arc;
use winapi::ctypes::c_void;
use winapi::um::handleapi::CloseHandle;
use winapi::um::memoryapi::{
    MapViewOfFile, OpenFileMappingW, UnmapViewOfFile, VirtualQuery, FILE_MAP_READ,
};
use winapi::um::winnt::MEMORY_BASIC_INFORMATION;

const MAP_NAME: &str = "Global\\HWiNFO_SENS_SM2";

// Header: poll time, then (offset, element size, element count) per table.
const HDR_POLL_TIME: usize = 12;
const HDR_SENSORS: usize = 20;
const HDR_READINGS: usize = 32;
// Sensor element: user-visible name, ANSI and (newer HWiNFO) UTF-8.
const SENSOR_NAME: usize = 136;
const SENSOR_NAME_UTF8: usize = 264;
// Reading element.
const READING_SENSOR_INDEX: usize = 4;
const READING_LABEL: usize = 140;
const READING_VALUE: usize = 284;
const READING_LABEL_UTF8: usize = 316;
const STRING_LEN: usize = 128;

fn bytes_at<const N: usize>(bytes: &[u8], offset: usize) -> Result<[u8; N]> {
    bytes
        .get(offset..)
        .and_then(|rest| rest.get(..N))
        .and_then(|field| field.try_into().ok())
        .ok_or_else(|| anyhow!("HWiNFO shared memory truncated at offset {}", offset))
}

fn u32_at(bytes: &[u8], offset: usize) -> Result<usize> {
    Ok(u32::from_le_bytes(bytes_at(bytes, offset)?) as usize)
}

/// One element table inside the mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Section {
    offset: usize,
    size: usize,
    count: usize,
}

impl Section {
    /// Read a table descriptor and check the whole table lies inside `bytes`
    /// with elements of at least `min_size`, so later indexing cannot go out
    /// of bounds.
    fn parse(bytes: &[u8], offset: usize, min_size: usize) -> Result<Self> {
        let section = Section {
            offset: u32_at(bytes, offset)?,
            size: u32_at(bytes, offset + 4)?,
            count: u32_at(bytes, offset + 8)?,
        };
        let end = section
            .count
            .checked_mul(section.size)
            .and_then(|len| len.checked_add(section.offset));
        if section.size < min_size || end.is_none_or(|end| end > bytes.len()) {
            bail!(
                "HWiNFO shared memory layout invalid: {:?} in {} bytes",
                section,
                bytes.len()
            );
        }
        Ok(section)
    }

    fn elements<'a>(&self, bytes: &'a [u8]) -> impl Iterator<Item = &'a [u8]> {
        bytes[self.offset..self.offset + self.count * self.size].chunks_exact(self.size)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Header {
    poll_time: i64,
    sensors: Section,
    readings: Section,
    /// Fingerprint of every name the layout is built from.
    names: u64,
}

impl Header {
    fn parse(bytes: &[u8]) -> Result<Self> {
        let poll_time = i64::from_le_bytes(bytes_at(bytes, HDR_POLL_TIME)?);
        let sensors = Section::parse(bytes, HDR_SENSORS, SENSOR_NAME + STRING_LEN)?;
        let readings = Section::parse(bytes, HDR_READINGS, READING_VALUE + 8)?;
        Ok(Header {
            poll_time,
            sensors,
            readings,
            names: names_fingerprint(bytes, sensors, readings),
        })
    }
}

/// Hash of every field the layout is built from, so a pull can tell cheaply
/// whether HWiNFO renamed, reordered or swapped readings without changing
/// the table sizes.
fn names_fingerprint(bytes: &[u8], sensors: Section, readings: Section) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    for e in sensors.elements(bytes) {
        name_field(e, SENSOR_NAME_UTF8, SENSOR_NAME).hash(&mut hasher);
    }
    for e in readings.elements(bytes) {
        e[READING_SENSOR_INDEX..READING_SENSOR_INDEX + 4].hash(&mut hasher);
        name_field(e, READING_LABEL_UTF8, READING_LABEL).hash(&mut hasher);
    }
    hasher.finish()
}

/// The raw string field: the UTF-8 copy when the element is large enough to
/// have one, else the ANSI copy, else empty.
fn name_field(element: &[u8], utf8: usize, ansi: usize) -> &[u8] {
    element
        .get(utf8..utf8 + STRING_LEN)
        .or_else(|| element.get(ansi..ansi + STRING_LEN))
        .unwrap_or_default()
}

/// NUL-terminated string field, see `name_field`.
fn name_at(element: &[u8], utf8: usize, ansi: usize) -> String {
    let field = name_field(element, utf8, ansi);
    let end = field.iter().position(|&b| b == 0).unwrap_or(field.len());
    String::from_utf8_lossy(&field[..end]).into_owned()
}

/// Names of every reading, parsed once and shared between snapshots.
#[derive(Default)]
struct Layout {
    /// `(sensor, reading)` per reading, in HWiNFO's order.
    names: Vec<(String, String)>,
    /// sensor -> reading -> position in `names`.
    index: HashMap<String, HashMap<String, usize>>,
}

impl Layout {
    fn new(names: Vec<(String, String)>) -> Self {
        let mut index: HashMap<String, HashMap<String, usize>> = HashMap::new();
        for (i, (sensor, reading)) in names.iter().enumerate() {
            index
                .entry(sensor.clone())
                .or_default()
                .insert(reading.clone(), i);
        }
        Layout { names, index }
    }

    fn parse(bytes: &[u8], header: &Header) -> Self {
        let sensors: Vec<String> = header
            .sensors
            .elements(bytes)
            .map(|e| name_at(e, SENSOR_NAME_UTF8, SENSOR_NAME))
            .collect();
        let names = header
            .readings
            .elements(bytes)
            .map(|e| {
                let field = &e[READING_SENSOR_INDEX..READING_SENSOR_INDEX + 4];
                let sensor_index = u32::from_le_bytes(field.try_into().unwrap()) as usize;
                // An out-of-range sensor index keeps the reading's slot (so
                // values stay aligned) under an empty sensor name.
                (
                    sensors.get(sensor_index).cloned().unwrap_or_default(),
                    name_at(e, READING_LABEL_UTF8, READING_LABEL),
                )
            })
            .collect();
        Layout::new(names)
    }
}

/// Sensor readings at one moment. Cloning is cheap: the names are shared and
/// only the values are copied.
#[derive(Clone, Default)]
pub struct Snapshot {
    layout: Arc<Layout>,
    values: Vec<f64>,
}

impl Snapshot {
    /// Build a snapshot from `(sensor, reading, value)` triples.
    pub fn from_readings(readings: &[(&str, &str, f64)]) -> Self {
        let names = readings
            .iter()
            .map(|(sensor, reading, _)| (sensor.to_string(), reading.to_string()))
            .collect();
        Snapshot {
            layout: Arc::new(Layout::new(names)),
            values: readings.iter().map(|r| r.2).collect(),
        }
    }

    pub fn get(&self, sensor: &str, reading: &str) -> Option<f64> {
        let i = *self.layout.index.get(sensor)?.get(reading)?;
        self.values.get(i).copied()
    }

    /// Value of the first reading with this label, in HWiNFO's own order.
    pub fn find_first(&self, reading: &str) -> Option<f64> {
        let i = self.layout.names.iter().position(|(_, r)| r == reading)?;
        self.values.get(i).copied()
    }

    /// Every `(sensor, reading, value)`, in HWiNFO's own order.
    pub fn readings(&self) -> impl Iterator<Item = (&str, &str, f64)> + '_ {
        self.layout
            .names
            .iter()
            .zip(&self.values)
            .map(|((sensor, reading), value)| (sensor.as_str(), reading.as_str(), *value))
    }
}

fn read_values(bytes: &[u8], readings: Section, out: &mut Vec<f64>) {
    out.clear();
    out.extend(
        readings
            .elements(bytes)
            .map(|e| f64::from_le_bytes(e[READING_VALUE..READING_VALUE + 8].try_into().unwrap())),
    );
}

/// Parse a full snapshot from the mapped bytes.
fn load(bytes: &[u8]) -> Result<(Header, Snapshot)> {
    let header = Header::parse(bytes)?;
    let mut snapshot = Snapshot {
        layout: Arc::new(Layout::parse(bytes, &header)),
        values: Vec::new(),
    };
    read_values(bytes, header.readings, &mut snapshot.values);
    Ok((header, snapshot))
}

/// Update `snapshot` in place. Returns true when HWiNFO has polled since
/// `header` was read; the values are refreshed either way.
fn refresh(bytes: &[u8], header: &mut Header, snapshot: &mut Snapshot) -> Result<bool> {
    let new = Header::parse(bytes)?;
    if new.sensors != header.sensors || new.readings != header.readings || new.names != header.names
    {
        snapshot.layout = Arc::new(Layout::parse(bytes, &new));
    }
    read_values(bytes, new.readings, &mut snapshot.values);
    let fresh = new.poll_time != header.poll_time;
    *header = new;
    Ok(fresh)
}

/// A read-only view of HWiNFO's file mapping, unmapped on drop.
struct View {
    ptr: *mut c_void,
    len: usize,
}

impl View {
    fn open() -> Result<Self> {
        let name: Vec<u16> = OsStr::new(MAP_NAME).encode_wide().chain(once(0)).collect();
        let handle = unsafe { OpenFileMappingW(FILE_MAP_READ, 0, name.as_ptr()) };
        if handle.is_null() {
            bail!("Failed to open shared memory object");
        }
        let ptr = unsafe { MapViewOfFile(handle, FILE_MAP_READ, 0, 0, 0) };
        // The view stays valid after the handle is closed.
        unsafe { CloseHandle(handle) };
        if ptr.is_null() {
            bail!("Failed to map view of shared memory");
        }
        let mut view = View { ptr, len: 0 };
        let mut info: MEMORY_BASIC_INFORMATION = unsafe { std::mem::zeroed() };
        let written = unsafe {
            VirtualQuery(
                ptr,
                &mut info,
                std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
            )
        };
        if written == 0 {
            bail!("Failed to query shared memory size");
        }
        view.len = info.RegionSize;
        Ok(view)
    }

    fn bytes(&self) -> &[u8] {
        // SAFETY: `ptr` is a live mapping of `len` readable bytes until drop.
        // HWiNFO writes this memory concurrently, so the bytes can change
        // between reads. Every offset used for indexing is validated against
        // the slice length from values copied out once per pull, so a
        // concurrent rewrite can produce stale or torn values but not an
        // out-of-bounds read.
        unsafe { std::slice::from_raw_parts(self.ptr as *const u8, self.len) }
    }
}

impl Drop for View {
    fn drop(&mut self) {
        unsafe { UnmapViewOfFile(self.ptr) };
    }
}

/// Connection to a running HWiNFO's shared memory.
pub struct Hwinfo {
    view: View,
    header: Header,
    snapshot: Snapshot,
}

impl Hwinfo {
    pub fn new() -> Result<Self> {
        let view = View::open()?;
        let (header, snapshot) = load(view.bytes())?;
        Ok(Hwinfo {
            view,
            header,
            snapshot,
        })
    }

    /// Refresh the readings. Returns true when HWiNFO has polled since the
    /// previous pull.
    pub fn pull(&mut self) -> Result<bool> {
        refresh(self.view.bytes(), &mut self.header, &mut self.snapshot)
    }

    pub fn snapshot(&self) -> &Snapshot {
        &self.snapshot
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEADER_SIZE: usize = 44;

    /// Element sizes and name offsets: (sensor size, reading size, sensor name
    /// offset, reading label offset).
    type Shape = (usize, usize, usize, usize);
    const CURRENT: Shape = (392, 460, SENSOR_NAME_UTF8, READING_LABEL_UTF8);
    /// Older HWiNFO builds without the UTF-8 string copies.
    const LEGACY: Shape = (264, 316, SENSOR_NAME, READING_LABEL);

    fn put(buf: &mut [u8], offset: usize, bytes: &[u8]) {
        buf[offset..offset + bytes.len()].copy_from_slice(bytes);
    }

    /// Build a fake HWiNFO mapping. Readings are `(sensor index, label, value)`.
    fn memory_with(
        shape: Shape,
        poll_time: i64,
        sensors: &[&str],
        readings: &[(u32, &str, f64)],
    ) -> Vec<u8> {
        let (sensor_size, reading_size, sensor_name, reading_label) = shape;
        let sensor_off = HEADER_SIZE;
        let reading_off = sensor_off + sensors.len() * sensor_size;
        let mut buf = vec![0u8; reading_off + readings.len() * reading_size];
        put(&mut buf, HDR_POLL_TIME, &poll_time.to_le_bytes());
        let fields = [
            (HDR_SENSORS, sensor_off),
            (HDR_SENSORS + 4, sensor_size),
            (HDR_SENSORS + 8, sensors.len()),
            (HDR_READINGS, reading_off),
            (HDR_READINGS + 4, reading_size),
            (HDR_READINGS + 8, readings.len()),
        ];
        for (offset, value) in fields {
            put(&mut buf, offset, &(value as u32).to_le_bytes());
        }
        for (i, name) in sensors.iter().enumerate() {
            put(
                &mut buf,
                sensor_off + i * sensor_size + sensor_name,
                name.as_bytes(),
            );
        }
        for (i, (sensor, label, value)) in readings.iter().enumerate() {
            let element = reading_off + i * reading_size;
            put(
                &mut buf,
                element + READING_SENSOR_INDEX,
                &sensor.to_le_bytes(),
            );
            put(&mut buf, element + reading_label, label.as_bytes());
            put(&mut buf, element + READING_VALUE, &value.to_le_bytes());
        }
        buf
    }

    fn memory(poll_time: i64, sensors: &[&str], readings: &[(u32, &str, f64)]) -> Vec<u8> {
        memory_with(CURRENT, poll_time, sensors, readings)
    }

    #[test]
    fn load_reads_names_and_values() {
        let buf = memory(
            100,
            &["CPU [#0]", "GPU [#0]"],
            &[(0, "Total CPU Usage", 45.5), (1, "GPU Temperature", 72.0)],
        );
        let (header, snap) = load(&buf).unwrap();
        assert_eq!(header.poll_time, 100);
        assert_eq!(snap.get("CPU [#0]", "Total CPU Usage"), Some(45.5));
        assert_eq!(snap.get("GPU [#0]", "GPU Temperature"), Some(72.0));
        assert_eq!(snap.get("GPU [#0]", "Total CPU Usage"), None);
        assert_eq!(snap.get("Nope", "GPU Temperature"), None);
    }

    #[test]
    fn load_reads_non_ascii_names() {
        let buf = memory(1, &["Carte mère"], &[(0, "Température °C", 40.0)]);
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.get("Carte mère", "Température °C"), Some(40.0));
    }

    #[test]
    fn load_reads_legacy_layout_without_utf8_fields() {
        let buf = memory_with(LEGACY, 1, &["CPU"], &[(0, "Core Temp", 55.0)]);
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.get("CPU", "Core Temp"), Some(55.0));
    }

    #[test]
    fn find_first_uses_hwinfo_order() {
        let buf = memory(
            1,
            &["GPU [#0]", "GPU [#1]"],
            &[
                (1, "Fan", 1.0),
                (0, "GPU Temperature", 60.0),
                (1, "GPU Temperature", 70.0),
            ],
        );
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.find_first("GPU Temperature"), Some(60.0));
        assert_eq!(snap.find_first("Missing"), None);
    }

    #[test]
    fn readings_iterates_in_order() {
        let buf = memory(1, &["A", "B"], &[(1, "y", 2.0), (0, "x", 1.0)]);
        let (_, snap) = load(&buf).unwrap();
        let all: Vec<_> = snap.readings().collect();
        assert_eq!(all, vec![("B", "y", 2.0), ("A", "x", 1.0)]);
    }

    #[test]
    fn refresh_is_fresh_only_when_poll_time_changes_but_always_updates_values() {
        let (mut header, mut snap) = load(&memory(100, &["S"], &[(0, "R", 1.0)])).unwrap();

        // Same poll time, new value: not fresh, value still updated.
        let fresh = refresh(
            &memory(100, &["S"], &[(0, "R", 2.0)]),
            &mut header,
            &mut snap,
        )
        .unwrap();
        assert!(!fresh);
        assert_eq!(snap.get("S", "R"), Some(2.0));

        let fresh = refresh(
            &memory(101, &["S"], &[(0, "R", 3.0)]),
            &mut header,
            &mut snap,
        )
        .unwrap();
        assert!(fresh);
        assert_eq!(snap.get("S", "R"), Some(3.0));
    }

    #[test]
    fn refresh_rebuilds_layout_when_reading_added() {
        let (mut header, mut snap) = load(&memory(1, &["S"], &[(0, "R", 1.0)])).unwrap();
        let grown = memory(2, &["S", "T"], &[(0, "R", 1.0), (1, "New", 9.0)]);
        refresh(&grown, &mut header, &mut snap).unwrap();
        assert_eq!(snap.get("T", "New"), Some(9.0));
        assert_eq!(snap.readings().count(), 2);
    }

    #[test]
    fn refresh_rebuilds_layout_when_readings_swap_without_count_change() {
        let (mut header, mut snap) =
            load(&memory(1, &["S"], &[(0, "A", 1.0), (0, "B", 2.0)])).unwrap();
        // Same sizes and counts, but the two readings traded places.
        let swapped = memory(2, &["S"], &[(0, "B", 2.0), (0, "A", 1.0)]);
        refresh(&swapped, &mut header, &mut snap).unwrap();
        assert_eq!(snap.get("S", "A"), Some(1.0));
        assert_eq!(snap.get("S", "B"), Some(2.0));
    }

    #[test]
    fn refresh_rebuilds_layout_when_reading_renamed() {
        let (mut header, mut snap) = load(&memory(1, &["S"], &[(0, "Old", 1.0)])).unwrap();
        refresh(
            &memory(2, &["S"], &[(0, "New", 1.0)]),
            &mut header,
            &mut snap,
        )
        .unwrap();
        assert_eq!(snap.get("S", "New"), Some(1.0));
        assert_eq!(snap.get("S", "Old"), None);
    }

    #[test]
    fn refresh_rebuilds_layout_when_sensor_renamed_or_reading_moves_sensor() {
        let (mut header, mut snap) = load(&memory(1, &["S", "T"], &[(0, "R", 1.0)])).unwrap();
        refresh(
            &memory(2, &["S2", "T"], &[(0, "R", 1.0)]),
            &mut header,
            &mut snap,
        )
        .unwrap();
        assert_eq!(snap.get("S2", "R"), Some(1.0));
        refresh(
            &memory(3, &["S2", "T"], &[(1, "R", 1.0)]),
            &mut header,
            &mut snap,
        )
        .unwrap();
        assert_eq!(snap.get("T", "R"), Some(1.0));
        assert_eq!(snap.get("S2", "R"), None);
    }

    #[test]
    fn refresh_keeps_layout_when_only_values_change() {
        let (mut header, mut snap) = load(&memory(1, &["S"], &[(0, "R", 1.0)])).unwrap();
        let before = Arc::as_ptr(&snap.layout);
        refresh(&memory(2, &["S"], &[(0, "R", 9.0)]), &mut header, &mut snap).unwrap();
        assert_eq!(
            Arc::as_ptr(&snap.layout),
            before,
            "layout must not be rebuilt"
        );
        assert_eq!(snap.get("S", "R"), Some(9.0));
    }

    #[test]
    fn snapshot_clone_keeps_old_values_after_refresh() {
        let (mut header, mut snap) = load(&memory(1, &["S"], &[(0, "R", 1.0)])).unwrap();
        let before = snap.clone();
        refresh(&memory(2, &["S"], &[(0, "R", 5.0)]), &mut header, &mut snap).unwrap();
        assert_eq!(before.get("S", "R"), Some(1.0));
        assert_eq!(snap.get("S", "R"), Some(5.0));
    }

    #[test]
    fn load_rejects_truncated_memory() {
        let mut buf = memory(1, &["S"], &[(0, "R", 1.0)]);
        buf.truncate(buf.len() - 10);
        assert!(load(&buf).is_err());
        assert!(load(&buf[..20]).is_err());
        assert!(load(&[]).is_err());
    }

    #[test]
    fn load_rejects_garbage_counts() {
        let mut buf = memory(1, &["S"], &[(0, "R", 1.0)]);
        put(&mut buf, HDR_READINGS + 8, &u32::MAX.to_le_bytes());
        assert!(load(&buf).is_err());

        // Zero-sized elements must not be accepted either.
        let mut buf = memory(1, &["S"], &[(0, "R", 1.0)]);
        put(&mut buf, HDR_SENSORS + 4, &0u32.to_le_bytes());
        assert!(load(&buf).is_err());
    }

    #[test]
    fn reading_with_out_of_range_sensor_index_does_not_panic() {
        let buf = memory(1, &["S"], &[(7, "Orphan", 3.0), (0, "R", 1.0)]);
        let (_, snap) = load(&buf).unwrap();
        assert_eq!(snap.get("", "Orphan"), Some(3.0));
        assert_eq!(snap.get("S", "R"), Some(1.0));
    }

    #[test]
    fn empty_tables_load_as_empty_snapshot() {
        let (_, snap) = load(&memory(1, &[], &[])).unwrap();
        assert_eq!(snap.readings().count(), 0);
        assert_eq!(snap.find_first("anything"), None);
    }

    #[test]
    fn from_readings_builds_lookup() {
        let snap = Snapshot::from_readings(&[("CPU", "Temp", 65.0), ("GPU", "Temp", 72.0)]);
        assert_eq!(snap.get("CPU", "Temp"), Some(65.0));
        assert_eq!(snap.get("GPU", "Temp"), Some(72.0));
        assert_eq!(snap.find_first("Temp"), Some(65.0));
        assert_eq!(Snapshot::default().get("CPU", "Temp"), None);
    }
}
