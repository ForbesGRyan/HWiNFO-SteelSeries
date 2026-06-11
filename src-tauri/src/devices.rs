use crate::render::OledBuffer;

/// HID packet protocol family for a direct-USB OLED device. Each variant
/// owns the full frame encoding for its device family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    /// Arctis Nova Pro (Wireless) base station: two 1024-byte feature
    /// reports, header [0x06, 0x93, chunk_x, 0, 64, height], column-major
    /// bitmap in 64-column chunks.
    NovaPro,
    /// Apex 5/7/Pro legacy keyboards: one 641-byte feature report,
    /// 0x61 followed by the 640-byte SSD1306 page-major bitmap.
    #[allow(dead_code)] // wired up in Task 5 (connect/daemon) and Task 6 (GUI)
    ApexLegacy,
}

/// One supported direct-USB device model.
#[derive(Debug)]
#[allow(dead_code)] // wired up in Task 5 (connect/daemon) and Task 6 (GUI)
pub struct SupportedDevice {
    /// Display name shown in the GUI device picker.
    pub name: &'static str,
    pub product_ids: &'static [u16],
    pub width: u32,
    pub height: u32,
    pub protocol: Protocol,
}

/// Registry of devices supported in direct-USB mode. VID is always 0x1038
/// (enforced by the discovery filter in connect.rs). To add a device that
/// speaks an existing protocol, add a row. A new packet format needs a new
/// Protocol variant and a build_packets arm.
///
/// PID→model mapping verified against apex-tux (apex-hardware/src/usb.rs).
/// Apex Gen3 (0x1640/0x1644/0x1646) intentionally absent — different
/// protocols, not yet implemented.
#[allow(dead_code)] // wired up in Task 5 (connect/daemon) and Task 6 (GUI)
pub static SUPPORTED_DEVICES: &[SupportedDevice] = &[
    SupportedDevice {
        name: "Arctis Nova Pro Wireless",
        product_ids: &[0x12E0],
        width: 128,
        height: 64,
        protocol: Protocol::NovaPro,
    },
    SupportedDevice {
        name: "Apex Pro",
        product_ids: &[0x1610],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex 7",
        product_ids: &[0x1612],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex Pro TKL",
        product_ids: &[0x1614],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex 7 TKL",
        product_ids: &[0x1618],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
    SupportedDevice {
        name: "Apex 5",
        product_ids: &[0x161C],
        width: 128,
        height: 40,
        protocol: Protocol::ApexLegacy,
    },
];

/// Look up the registry entry for a product ID.
#[allow(dead_code)] // wired up in Task 5 (connect/daemon) and Task 6 (GUI)
pub fn find_supported(product_id: u16) -> Option<&'static SupportedDevice> {
    SUPPORTED_DEVICES
        .iter()
        .find(|d| d.product_ids.contains(&product_id))
}

impl Protocol {
    /// Build the complete HID feature-report sequence for one frame.
    pub fn build_packets(&self, buf: &OledBuffer) -> Vec<Vec<u8>> {
        match self {
            Protocol::NovaPro => [0u8, 64u8]
                .iter()
                .map(|&chunk_x| {
                    let bitmap = buf.get_chunk(chunk_x, 64);
                    let mut packet = vec![0x06u8, 0x93, chunk_x, 0, 64, buf.height as u8];
                    packet.extend_from_slice(&bitmap);
                    packet.resize(1024, 0);
                    packet
                })
                .collect(),
            Protocol::ApexLegacy => {
                let mut packet = Vec::with_capacity(641);
                packet.push(0x61);
                packet.extend_from_slice(&buf.to_page_major());
                vec![packet]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_find_supported_nova_pro() {
        let d = find_supported(0x12E0).expect("Nova Pro in registry");
        assert_eq!(d.name, "Arctis Nova Pro Wireless");
        assert_eq!((d.width, d.height), (128, 64));
        assert_eq!(d.protocol, Protocol::NovaPro);
    }

    #[test]
    fn test_find_supported_apex_pro() {
        let d = find_supported(0x1610).expect("Apex Pro in registry");
        assert_eq!(d.name, "Apex Pro");
        assert_eq!((d.width, d.height), (128, 40));
        assert_eq!(d.protocol, Protocol::ApexLegacy);
    }

    #[test]
    fn test_find_supported_all_apex_legacy_pids() {
        for pid in [0x1610u16, 0x1612, 0x1614, 0x1618, 0x161C] {
            let d = find_supported(pid).unwrap_or_else(|| panic!("PID {pid:#06X} missing"));
            assert_eq!(d.protocol, Protocol::ApexLegacy);
            assert_eq!((d.width, d.height), (128, 40));
        }
    }

    #[test]
    fn test_find_supported_unknown_pid_is_none() {
        assert!(find_supported(0x9999).is_none());
        // Gen3 PIDs are deliberately unsupported for now
        assert!(find_supported(0x1640).is_none());
        assert!(find_supported(0x1644).is_none());
        assert!(find_supported(0x1646).is_none());
    }

    #[test]
    fn test_registry_invariants() {
        for d in SUPPORTED_DEVICES {
            // Buffer layout requires height % 8 == 0 (silent corruption otherwise).
            assert_eq!(d.height % 8, 0, "{}: height must be multiple of 8", d.name);
            assert!(
                !d.product_ids.is_empty(),
                "{}: needs at least one PID",
                d.name
            );
        }
        // No PID may appear in two entries.
        let mut all: Vec<u16> = SUPPORTED_DEVICES
            .iter()
            .flat_map(|d| d.product_ids.iter().copied())
            .collect();
        all.sort_unstable();
        let len_before = all.len();
        all.dedup();
        assert_eq!(
            len_before,
            all.len(),
            "duplicate PID across registry entries"
        );
    }

    #[test]
    fn test_nova_pro_packets_layout() {
        let mut buf = OledBuffer::new(128, 64);
        buf.set_pixel(0, 0, true); // first byte of chunk 0
        buf.set_pixel(64, 0, true); // first byte of chunk 1

        let packets = Protocol::NovaPro.build_packets(&buf);
        assert_eq!(packets.len(), 2);
        for (i, p) in packets.iter().enumerate() {
            assert_eq!(p.len(), 1024);
            assert_eq!(p[0], 0x06);
            assert_eq!(p[1], 0x93);
            assert_eq!(p[2], if i == 0 { 0 } else { 64 }); // chunk_x
            assert_eq!(p[3], 0);
            assert_eq!(p[4], 64); // chunk width
            assert_eq!(p[5], 64); // screen height
            assert_eq!(p[6], 0x01); // pixel at top-left of this chunk
        }
    }

    #[test]
    fn test_apex_legacy_packet_layout() {
        let mut buf = OledBuffer::new(128, 40);
        buf.set_pixel(0, 0, true); // page 0, col 0 → payload byte 0
        buf.set_pixel(127, 39, true); // page 4, col 127 → last payload byte

        let packets = Protocol::ApexLegacy.build_packets(&buf);
        assert_eq!(packets.len(), 1);
        let p = &packets[0];
        assert_eq!(p.len(), 641);
        assert_eq!(p[0], 0x61);
        assert_eq!(p[1], 0x01); // (0,0)
        assert_eq!(p[640], 0x80); // (127,39): bit 7 of page 4
    }
}
