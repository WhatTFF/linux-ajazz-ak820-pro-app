//! Interface discovery through `/sys/class/hidraw`.

use std::path::{Path, PathBuf};

use ak820_proto::{
    BULK_PAGE, CONFIG_PAGES, FALLBACK_BULK_REPORT, FALLBACK_CONFIG_REPORT, PIDS, VID,
};

/// One AK820 hidraw node and its capabilities.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IfaceInfo {
    /// `/dev/hidrawN`.
    pub path: PathBuf,
    /// Output report size in bytes, from the report descriptor.
    pub report_len: usize,
    /// First usage page in the descriptor (`0xFF67` = bulk, others = config).
    pub usage_page: u16,
}

/// The two interfaces the protocol needs.
#[derive(Clone, Debug, Default)]
pub struct Interfaces {
    /// Config interface (device info, clock).
    pub config: Option<IfaceInfo>,
    /// Bulk interface (animation upload, built-in index).
    pub bulk: Option<IfaceInfo>,
    /// Every AK820 node found, including ones that are neither role.
    pub nodes: Vec<IfaceInfo>,
}

/// Find the AK820 interfaces. Missing roles are `None`; an empty result means no device.
pub fn find() -> Interfaces {
    let mut out = Interfaces::default();
    for info in discover() {
        if info.usage_page == BULK_PAGE {
            out.bulk.get_or_insert(info.clone());
        } else if CONFIG_PAGES.contains(&info.usage_page) && out.config.is_none() {
            out.config = Some(info.clone());
        }
        out.nodes.push(info);
    }
    out
}

/// All AK820 nodes currently present, in `/sys` order.
pub fn discover() -> Vec<IfaceInfo> {
    discover_in(Path::new("/sys/class/hidraw"))
}

fn discover_in(root: &Path) -> Vec<IfaceInfo> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut names: Vec<PathBuf> = entries.filter_map(|e| e.ok().map(|e| e.path())).collect();
    names.sort();

    let mut found = Vec::new();
    for node in names {
        let Some((vid, pid)) = read_ids(&node) else {
            continue;
        };
        if vid != VID || !PIDS.contains(&pid) {
            continue;
        }
        let descriptor = std::fs::read(node.join("device/report_descriptor")).unwrap_or_default();
        let (page, out_len) = parse_descriptor(&descriptor);
        let Some(usage_page) = page else {
            continue;
        };
        let fallback = if usage_page == BULK_PAGE {
            FALLBACK_BULK_REPORT
        } else {
            FALLBACK_CONFIG_REPORT
        };
        let Some(name) = node.file_name() else {
            continue;
        };
        found.push(IfaceInfo {
            path: Path::new("/dev").join(name),
            report_len: if out_len == 0 { fallback } else { out_len },
            usage_page,
        });
    }
    found
}

/// Read `(vendor, product)` from `device/uevent`.
fn read_ids(node: &Path) -> Option<(u16, u16)> {
    let uevent = std::fs::read_to_string(node.join("device/uevent")).ok()?;
    let line = uevent.lines().find(|l| l.starts_with("HID_ID="))?;
    let mut parts = line.trim_start_matches("HID_ID=").split(':');
    let _bus = parts.next()?;
    let vid = u16::from_str_radix(parts.next()?, 16).ok()?;
    let pid = u16::from_str_radix(parts.next()?, 16).ok()?;
    Some((vid, pid))
}

/// Parse a HID report descriptor into `(first usage page, output report bytes)`.
///
/// Firmware revisions differ in report size (the config interface is 32 bytes on some,
/// 64 on others), so this must never be hard-coded.
pub fn parse_descriptor(desc: &[u8]) -> (Option<u16>, usize) {
    let mut page: Option<u16> = None;
    let mut size = 0usize;
    let mut count = 0usize;
    let mut out_bits = 0usize;
    let mut i = 0usize;

    while i < desc.len() {
        let b = desc[i];
        if b == 0xFE {
            // Long item: prefix, data size, tag, then data.
            if i + 2 >= desc.len() {
                break;
            }
            i += 3 + desc[i + 1] as usize;
            continue;
        }
        let n = [0usize, 1, 2, 4][(b & 0x03) as usize];
        if i + 1 + n > desc.len() {
            break;
        }
        let mut val = 0usize;
        for (k, byte) in desc[i + 1..i + 1 + n].iter().enumerate() {
            val |= (*byte as usize) << (8 * k);
        }
        match b & 0xFC {
            0x04 if page.is_none() => page = Some(val as u16),
            0x74 => size = val,
            0x94 => count = val,
            0x90 => out_bits += size * count,
            _ => {}
        }
        i += 1 + n;
    }

    (page, out_bits / 8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real descriptors from a PID 0x8009 board (fw 1.07).
    const CONFIG_DESC: [u8; 34] = [
        0x06, 0x68, 0xFF, 0x09, 0x61, 0xA1, 0x01, 0x09, 0x62, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x95,
        0x40, 0x75, 0x08, 0x81, 0x02, 0x09, 0x63, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x95, 0x40, 0x75,
        0x08, 0x91, 0x02, 0xC0,
    ];
    const BULK_DESC: [u8; 48] = [
        0x06, 0x67, 0xFF, 0x09, 0x61, 0xA1, 0x01, 0x09, 0x62, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x95,
        0x40, 0x75, 0x08, 0xB1, 0x02, 0x09, 0x63, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x96, 0x08, 0x10,
        0x75, 0x08, 0x91, 0x02, 0x09, 0x64, 0x15, 0x00, 0x26, 0xFF, 0x00, 0x95, 0x40, 0x75, 0x08,
        0x81, 0x02, 0xC0,
    ];

    #[test]
    fn config_interface_is_64_bytes() {
        assert_eq!(parse_descriptor(&CONFIG_DESC), (Some(0xFF68), 64));
    }

    #[test]
    fn bulk_interface_is_4104_bytes() {
        assert_eq!(parse_descriptor(&BULK_DESC), (Some(0xFF67), 4104));
    }

    #[test]
    fn empty_descriptor_is_harmless() {
        assert_eq!(parse_descriptor(&[]), (None, 0));
        assert_eq!(parse_descriptor(&[0x06]), (None, 0));
    }

    #[test]
    fn finds_device_on_this_machine() {
        // Not an assertion about hardware: just exercise the sysfs path if present.
        let interfaces = find();
        for info in [&interfaces.config, &interfaces.bulk].into_iter().flatten() {
            assert!(
                info.report_len > 8,
                "{} has report_len {}",
                info.path.display(),
                info.report_len
            );
            assert_eq!(info.path.parent(), Some(Path::new("/dev")));
        }
        if let Some(bulk) = &interfaces.bulk {
            assert_eq!(bulk.usage_page, BULK_PAGE);
        }
        if let Some(config) = &interfaces.config {
            assert!(CONFIG_PAGES.contains(&config.usage_page));
        }
    }
}
