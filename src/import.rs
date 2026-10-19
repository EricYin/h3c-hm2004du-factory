//! Import an H3C factory UBI-volume image (e.g. a `reservearea` backup `.bin`).
//!
//! This is the H3C analog of the original `import.rs`. The original dug a
//! `fhdata` UBI volume or a JFFS2 partition out of a FiberHome dump; H3C gives
//! us the factory volume directly, so import is just "parse the raw bytes at
//! the known offsets". The only non-trivial part is recovering `AL-MAC` from the
//! gzip member at 0x80000 — that mirrors `02_network::h3c_hm2004du_base_mac`.

use crate::model::*;
use std::io::Read;

/// Parse a raw H3C factory volume and recover the editable fields.
/// Auto-detects the 1 MiB compact bundle (magic `H3C-1MiB`) written by
/// [`Factory::encode_1mib`] versus the real-layout demo image / true backup.
pub fn parse_factory_volume(data: &[u8]) -> Factory {
    if data.starts_with(&COMPACT_MAGIC) {
        return parse_compact_1mib(data);
    }
    let mut device_sn = read_device_sn(data);

    let almac = read_almac_bytes(data);

    // Some units ship the 0x14102c slot blank; ctromfile carries the same
    // serial, so fall back to it instead of leaving the field empty.
    if device_sn.is_empty() {
        if let Some(text) = &almac {
            if let Some(sn) = find_serial_number(text) {
                device_sn = sn;
            }
        }
    }

    let label_mac = almac
        .as_deref()
        .and_then(find_al_mac)
        .or_else(|| read_base_mac_placeholder(data))
        .unwrap_or([0u8; 6]);

    let calibration = read_region(data, CALIBRATION_OFFSET, CALIBRATION_SIZE);
    let wifi_eeprom = read_region(data, WIFI_EEPROM_OFFSET, WIFI_EEPROM_SIZE);

    // If this happens to be an mtd13 dump we can still lift the downlink
    // calibration out of it; a plain reservearea backup simply yields blank.
    let (pon_downlink_cal, pon_downlink_flags) = match parse_mtd13(data) {
        Some((cal, flags, _off)) => (cal, flags),
        None => read_downlink_from_layout(data),
    };

    Factory {
        device_sn,
        label_mac,
        calibration,
        wifi_eeprom,
        pon_downlink_cal,
        pon_downlink_flags,
    }
}

/// Parse the 1 MiB compact bundle written by `Factory::encode_1mib`.
fn parse_compact_1mib(data: &[u8]) -> Factory {
    let label_mac = read_mac6(data, C_LABEL_MAC_OFFSET).unwrap_or([0u8; 6]);
    let mut device_sn = String::new();
    if let Some(raw) = data.get(C_DEVICE_SN_OFFSET..C_DEVICE_SN_OFFSET + DEVICE_SN_SIZE) {
        device_sn = String::from_utf8_lossy(raw)
            .trim_matches(|c| c == '\0' || c == ' ' || c == '\u{fffd}')
            .to_string();
    }
    let calibration = read_region(data, C_CALIBRATION_OFFSET, CALIBRATION_SIZE);
    let wifi_eeprom = read_region(data, C_WIFI_EEPROM_OFFSET, WIFI_EEPROM_SIZE);
    let pon_downlink_cal = read_region(data, C_PON_DOWNLINK_CAL_OFFSET, PON_DOWNLINK_CAL_SIZE);
    let pon_downlink_flags = PonDownlinkFlags::from_bytes(
        data.get(C_PON_DOWNLINK_FLAGS_OFFSET..C_PON_DOWNLINK_FLAGS_OFFSET + PON_DOWNLINK_FLAGS_SIZE)
            .unwrap_or(&[]),
    );
    Factory {
        device_sn,
        label_mac,
        calibration,
        wifi_eeprom,
        pon_downlink_cal,
        pon_downlink_flags,
    }
}

/// Downlink data as written by our own real-layout export (`encode()`).
///
/// The 1 MiB compact bundle never reaches here — [`parse_factory_volume`]
/// dispatches it to [`parse_compact_1mib`] first, which reads the flags from
/// their own aligned offset (`C_PON_DOWNLINK_FLAGS_OFFSET`) rather than from
/// the bytes hugging the blob.
fn read_downlink_from_layout(data: &[u8]) -> (Vec<u8>, PonDownlinkFlags) {
    let base = PON_DOWNLINK_CAL_OFFSET;
    let cal = read_region(data, base, PON_DOWNLINK_CAL_SIZE);
    let flags = PonDownlinkFlags::from_bytes(
        data.get(base + PON_DOWNLINK_CAL_SIZE..base + PON_DOWNLINK_CAL_SIZE + PON_DOWNLINK_FLAGS_SIZE)
            .unwrap_or(&[]),
    );
    (cal, flags)
}

fn read_mac6(data: &[u8], off: usize) -> Option<[u8; 6]> {
    let m = data.get(off..off + 6)?;
    let mut a = [0u8; 6];
    a.copy_from_slice(m);
    Some(a)
}

/// The AL-MAC region contents: the *inflated* gzip ctromfile for a real backup,
/// or the raw region when it is already plain text (our own demo export).
fn read_almac_bytes(data: &[u8]) -> Option<Vec<u8>> {
    let region = data.get(ALMAC_REGION_OFFSET..)?;
    if let Some(start) = find_subseq(region, &[0x1f, 0x8b, 0x08]) {
        let mut out = Vec::new();
        // GzDecoder stops at the end of the gzip member; the 0xff padding that
        // follows is never read.
        if flate2::read::GzDecoder::new(&region[start..])
            .read_to_end(&mut out)
            .is_ok()
        {
            return Some(out);
        }
    }
    Some(region.to_vec())
}

fn read_region(data: &[u8], off: usize, size: usize) -> Vec<u8> {
    match data.get(off..off + size) {
        Some(slice) => slice.to_vec(),
        None => vec![0xff; size],
    }
}

fn read_device_sn(data: &[u8]) -> String {
    let Some(raw) = data.get(DEVICE_SN_OFFSET..DEVICE_SN_OFFSET + DEVICE_SN_SIZE) else {
        return String::new();
    };
    String::from_utf8_lossy(raw)
        .trim_matches(|c| c == '\0' || c == ' ' || c == '\u{fffd}')
        .to_string()
}

/// Last resort: the base-MAC placeholder at 0x140005.
fn read_base_mac_placeholder(data: &[u8]) -> Option<[u8; 6]> {
    let m = data.get(BASE_MAC_OFFSET..BASE_MAC_OFFSET + BASE_MAC_SIZE)?;
    if m.iter().any(|&b| b != 0xff && b != 0x00) {
        let mut a = [0u8; 6];
        a.copy_from_slice(m);
        Some(a)
    } else {
        None
    }
}

pub fn find_subseq(hay: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || hay.len() < needle.len() {
        return None;
    }
    hay.windows(needle.len()).position(|w| w == needle)
}

/// Scan `buf` for the first `AL-MAC="xx:xx:xx:xx:xx:xx"` and parse it.
pub fn find_al_mac(buf: &[u8]) -> Option<[u8; 6]> {
    let tag = b"AL-MAC=\"";
    let mut from = 0;
    while from + tag.len() + 17 <= buf.len() {
        match find_subseq(&buf[from..], tag) {
            Some(rel) => {
                let at = from + rel + tag.len();
                if let Some(m) = parse_mac(&String::from_utf8_lossy(&buf[at..at + 17])) {
                    return Some(m);
                }
                from = at; // skip past this occurrence and keep scanning
            }
            None => break,
        }
    }
    None
}

/// Scan `buf` for `SerialNumber="..."` (used to cross-check / recover the SN).
pub fn find_serial_number(buf: &[u8]) -> Option<String> {
    let tag = b"SerialNumber=\"";
    let rel = find_subseq(buf, tag)?;
    let at = rel + tag.len();
    let end = buf.get(at..)?.iter().position(|&b| b == b'"')?;
    if end == 0 {
        return None;
    }
    Some(String::from_utf8_lossy(&buf[at..at + end]).to_string())
}

// ===================== mtd13: 下行 PON 口校准 =====================
//
// mtd13 是一个 4 MiB 的 UBI 镜像，里面挂了一个名为 `ubifs1` 的 UBIFS 动态卷。
// 下行 PON 口的光模块校准就在这棵卷里：
//
//   /bosa/OBosaMode              u16
//   /bosa/BosaFlag               u16
//   /bosa/TempFlag               u16
//   /bosa/mmodified_UX3326.bin   0x120 字节 —— 我们要的那块
//
// UBIFS 的文件数据存在 LEAF 节点里，节点布局（全部小端）为：
//
//   +0x00 u32  magic = 0x06101831
//   +0x04 u32  crc（厂商实现，标准 crc32 校验不通，跳过）
//   +0x08 u64  sqnum —— 越大越新
//   +0x10 u32  len   —— 整个节点长度
//   +0x14 u8   node_type, +0x15 u8 group_type, +0x16 u16 pad
//   +0x18 u32  ino   —— 所属 inode
//   +0x28 u32  nodelen —— 载荷长度
//   +0x30 u8[nodelen] 载荷
//
// 注意 sqnum 的位置与标准 UBIFS 不同（厂商魔改），这里按实测偏移读取。
//
// **必须按 node_type 过滤**：该镜像里还有大量 CMT（type 7，提交日志）节点，
// 它们的载荷恰好会被当成 ino/nodelen 读，从而伪造出几百个 ino=0x45 的假记录。

const UBIFS_NODE_MAGIC: u32 = 0x0610_1831;
const UBIFS_NODE_HDR: usize = 0x30; // 48
/// UBIFS_LEAF_NODE：文件数据节点，载荷即文件内容。
const UBIFS_NODE_LEAF: u8 = 1;

/// mtd13 的标称大小（4 MiB）。
pub const MTD13_SIZE: usize = 0x40_0000;

/// `/bosa/mmodified_UX3326.bin` 的 inode（由 DENT 目录项 `mmodified_UX3326.bin` 确认）。
const INO_UX3326_BIN: u32 = 0x45;
/// `/bosa/OBosaMode`。
const INO_OBOSA_MODE: u32 = 0x42;
/// `/bosa/BosaFlag`。
const INO_BOSA_FLAG: u32 = 0x43;
/// `/bosa/TempFlag`。
const INO_TEMP_FLAG: u32 = 0x44;

/// One UBIFS data node lifted out of the mtd13 image.
struct UbiNode {
    off: usize,
    sqnum: u64,
    ino: u32,
    payload: Vec<u8>,
}

/// Scan a buffer for UBIFS data nodes and return the newest copy of each inode.
///
/// The image stores many generations of the same file (UBIFS keeps a log), so
/// callers must pick by `sqnum`, not by physical position: the copy with the
/// highest sequence number is the one the filesystem would read back.
fn scan_ubifs_leaves(data: &[u8]) -> Vec<UbiNode> {
    let mut best: Vec<UbiNode> = Vec::new();
    let mut i = 0usize;
    while i + UBIFS_NODE_HDR <= data.len() {
        let magic = u32::from_le_bytes([data[i], data[i + 1], data[i + 2], data[i + 3]]);
        if magic != UBIFS_NODE_MAGIC {
            i += 1;
            continue;
        }
        let sqnum = u64::from_le_bytes(data[i + 0x08..i + 0x10].try_into().unwrap());
        let len = u32::from_le_bytes(data[i + 0x10..i + 0x14].try_into().unwrap()) as usize;
        let node_type = data[i + 0x14];
        let ino = u32::from_le_bytes(data[i + 0x18..i + 0x1c].try_into().unwrap());
        let nodelen = u32::from_le_bytes(data[i + 0x28..i + 0x2c].try_into().unwrap()) as usize;

        // Sanity: a plausible node is at least a header, fits in the image, and
        // its payload fits inside it. Reject and rescan byte-by-byte otherwise,
        // since a stray magic word inside payload data must not derail us.
        let ok = len >= UBIFS_NODE_HDR
            && i + len <= data.len()
            && nodelen <= len - UBIFS_NODE_HDR
            && nodelen <= MTD13_SIZE;
        if !ok || node_type != UBIFS_NODE_LEAF {
            i += 1;
            continue;
        }

        let payload = data[i + UBIFS_NODE_HDR..i + UBIFS_NODE_HDR + nodelen].to_vec();
        match best.iter_mut().find(|n| n.ino == ino) {
            Some(slot) if slot.sqnum >= sqnum => {}
            Some(slot) => *slot = UbiNode { off: i, sqnum, ino, payload },
            None => best.push(UbiNode { off: i, sqnum, ino, payload }),
        }

        // Nodes are 8-byte aligned; skip to the next one.
        i += (len + 7) & !7;
    }
    best
}

/// Latest payload stored for `ino`, if the image carries it.
fn latest_payload(nodes: &[UbiNode], ino: u32) -> Option<&[u8]> {
    nodes.iter().find(|n| n.ino == ino).map(|n| n.payload.as_slice())
}

/// True when `data` looks like an mtd13 UBI dump rather than a factory volume.
///
/// The factory volume has no UBI EC header, and mtd13 always starts with one.
pub fn looks_like_mtd13(data: &[u8]) -> bool {
    data.len() >= MTD13_SIZE && data.starts_with(b"UBI#")
}

/// Extract the downlink PON calibration from an mtd13 dump.
///
/// Returns `(mmodified_UX3326.bin 的 288 字节载荷, OBosaMode/BosaFlag/TempFlag)`,
/// or `None` when the image is not an mtd13 dump or carries no `/bosa/` table.
/// Extract the downlink PON calibration from an mtd13 image.
///
/// Returns `(calibration blob, BOSA flags, source offset)`. The offset is the
/// byte position of the winning UBIFS record, which the GUI reports so the user
/// can tell *which* copy of the many revisions in the journal was used.
pub fn parse_mtd13(data: &[u8]) -> Option<(Vec<u8>, PonDownlinkFlags, usize)> {
    if !looks_like_mtd13(data) {
        return None;
    }
    let nodes = scan_ubifs_leaves(data);
    let node = nodes.iter().find(|n| n.ino == INO_UX3326_BIN)?;
    if node.payload.len() < PON_DOWNLINK_CAL_SIZE {
        return None;
    }
    let cal = node.payload[..PON_DOWNLINK_CAL_SIZE].to_vec();
    let flags = PonDownlinkFlags {
        obosa_mode: read_u16(latest_payload(&nodes, INO_OBOSA_MODE)),
        bosa_flag: read_u16(latest_payload(&nodes, INO_BOSA_FLAG)),
        temp_flag: read_u16(latest_payload(&nodes, INO_TEMP_FLAG)),
    };
    Some((cal, flags, node.off))
}

fn read_u16(p: Option<&[u8]>) -> u16 {
    match p {
        Some(b) if b.len() >= 2 => u16::from_le_bytes([b[0], b[1]]),
        _ => 0,
    }
}
