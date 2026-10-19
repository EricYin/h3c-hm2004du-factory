//! H3C HM2004-DU factory partition layout.
//!
//! This module mirrors `model.rs` from the original `fiberhome-factory`
//! project, but retargets every offset to the real H3C layout. On the
//! HM2004-DU the "factory" partition is a UBI volume (`ubi-volume-factory`,
//! 2_359_296 bytes) whose raw nvmem cells are:
//!
//! ```text
//! 0x140000  magic 21 43 34 12  (0x12344321)
//! 0x140005  base-MAC placeholder 02:00:00:XX:00:00  (NOT a usable address)
//! 0x14102c  Device SN, 12-byte ASCII   (duplicate at 0x14105f)
//! 0x1c0400  PON calibration, 0x200 bytes (EN7572 / UX3326 BOSA)
//! 0x4c000   Wi-Fi EEPROM, 0x1000 bytes (MT7916 DBDC)
//! 0x80000   gzip of ctromfile.cfg — the real label MAC is AL-MAC="..." inside it
//! ```
//!
//! ## 下行 PON 口校准（来自 mtd13）
//!
//! 下行 PON 口的光模块校准不在 factory 分区里，而是在 **mtd13** 分区中。
//! mtd13 是一个 4 MiB 的 UBI 镜像（PEB = 0x20000，共 32 个 PEB），其中
//! `ubi-volume-factory` 之外还挂了一个名为 `ubifs1` 的 UBIFS 动态卷。逆向
//! 该卷后可以定位到一棵很小的目录树，它就是下行 PON 口 BOSA 的配置与校准表：
//!
//! ```text
//! /bosa/OBosaMode              u16   BOSA 工作模式
//! /bosa/BosaFlag               u16   BOSA 生效标志
//! /bosa/TempFlag               u16   温度补偿标志
//! /bosa/mmodified_UX3326.bin   0x120 UX3326 光模块校准表（核心数据）
//! ```
//!
//! `mmodified_UX3326.bin` 的载荷是 **288 (0x120) 字节**，结构为：
//!
//! ```text
//! 0x00  u32  版本/标记（实测小端 0xC86F0E02）
//! 0x04  u16  模式 A（实测 0x0014 / 0x0014）
//! 0x06  u16  模式 B（实测 0x0030 或 0x3903，随校准批次变化）
//! 0x08..0x6f  厂商标定参数（阈值、增益、偏置、温漂，定点/浮点混合）
//! 0x6f  u16[84]  接收灵敏度 LUT，单调递增（实测 0x013C .. 0x01C6）
//! 0x117 u8[7]   尾部保留 (0)
//! ```
//!
//! 该 blob 在 mtd13 内以 **多副本 + sqnum 递增** 的方式重复存放（UBIFS 日志
//! 行为，实测 10 份），最新的副本 sqnum 最大。因此解析时取 sqnum 最大的
//! 那一份，而不是物理上最靠后的那份。
//!
//! 本工具把整块 288 字节当作不透明 blob 保存/还原（`pon_downlink_calibration`），
//! 与上行 PON 校准的处理方式保持一致；同时导出三个 u16 标量供 UI 展示。
//!
//! The label MAC is NOT stored in an addressable field: stock firmware keeps
//! the reservearea slot blank and recovers it from `AL-MAC` inside the gzipped
//! ctromfile at 0x80000 (see `02_network::h3c_hm2004du_base_mac`). This tool
//! therefore reads/writes `AL-MAC` rather than the 0x140005 placeholder.
//!
//! MAC derivation (last byte only, from `02_network::h3c_mac_offset`):
//!   eth0 = label+1, pon0/wan = label+2, 5G = label+4, 2.4G = label+5.
//!
//! The `encode()` export here is a *layout-mapped demo image*: it places every
//! field at its true H3C offset into a single buffer, but it does NOT synthesize
//! UBI EC/VID headers and it writes the AL-MAC region as readable plain text
//! instead of re-gzipping ctromfile.cfg. It is for viewing/comparison only and
//! is NOT directly flashable to NAND.

// Real ubi-volume-factory size.
pub const FACTORY_VOLUME_SIZE: usize = 0x240000; // 2_359_296
// Layout-mapped demo export size: highest used field rounded up to 4 KiB.
// The downlink PON calibration sits at 0x1c1000 and runs for 0x120 bytes,
// plus 6 bytes of BOSA flags right after it.
pub const DEMO_SIZE: usize = 0x1c3000; // 1_847_296

/// Our layout-mapped export must never claim to be larger than the real
/// volume, otherwise a stray field offset would silently produce an image no
/// flasher could accept. Checked at compile time.
const _: () = assert!(DEMO_SIZE < FACTORY_VOLUME_SIZE);

pub const MAGIC_OFFSET: usize = 0x140000;
pub const MAGIC: [u8; 4] = [0x21, 0x43, 0x34, 0x12];

pub const BASE_MAC_OFFSET: usize = 0x140005;
pub const BASE_MAC_SIZE: usize = 6;

pub const DEVICE_SN_OFFSET: usize = 0x14102c;
pub const DEVICE_SN_DUP_OFFSET: usize = 0x14105f;
pub const DEVICE_SN_SIZE: usize = 0x0c;

pub const ALMAC_REGION_OFFSET: usize = 0x80000;
pub const ALMAC_REGION_SIZE: usize = 0x4000;

pub const CALIBRATION_OFFSET: usize = 0x1c0400;
pub const CALIBRATION_SIZE: usize = 0x200;

pub const WIFI_EEPROM_OFFSET: usize = 0x4c000;
pub const WIFI_EEPROM_SIZE: usize = 0x1000;

// ---- 下行 PON 口校准（mtd13 / UBIFS `ubifs1` 卷）----
//
// mtd13 是一个独立的 4 MiB UBI 镜像，不属于 ubi-volume-factory，所以真实
// 分区里没有固定偏移。为了让演示/紧凑镜像也能携带这块数据，这里沿用
// 「布局映射演示」的思路：把它放在紧跟 factory 校准区之后的固定偏移，
// 并在 1 MiB 紧凑布局里分配一个文档化偏移。
//
// 原始数据在 mtd13 内部的定位由 `import::parse_mtd13` 完成（UBIFS 节点遍历
// + sqnum 取最新），这里的常量只描述我们自己的导出格式。

/// 演示镜像中下行 PON 校准的偏移（紧接上行 PON 校准区之后）。
pub const PON_DOWNLINK_CAL_OFFSET: usize = 0x1c1000;
/// UX3326 校准表长度（`mmodified_UX3326.bin` 的载荷）。
pub const PON_DOWNLINK_CAL_SIZE: usize = 0x120;

// ---- 1 MiB compact layout (self-describing, NOT flashable) ----
//
// The real H3C identity fields sit at 0x140000+, i.e. beyond 1 MiB, so a
// 1 MiB file cannot keep the true offsets. This layout packs every field
// into the first 1 MiB at fixed, documented offsets. Parsing auto-detects
// it via the ASCII magic.
pub const COMPACT_SIZE: usize = 0x100000; // 1_048_576
pub const COMPACT_MAGIC: [u8; 8] = *b"H3C-1MiB";

pub const C_LABEL_MAC_OFFSET: usize = 0x0010; // label / br-lan / LAN
pub const C_ETH0_MAC_OFFSET: usize = 0x0020; // label+1
pub const C_PON0_MAC_OFFSET: usize = 0x0030; // label+2 (pon0/wan)
pub const C_WIFI5G_MAC_OFFSET: usize = 0x0040; // label+4
pub const C_WIFI24_MAC_OFFSET: usize = 0x0050; // label+5
pub const C_DEVICE_SN_OFFSET: usize = 0x0060; // 12-byte ASCII
pub const C_WIFI_EEPROM_OFFSET: usize = 0x1000; // 0x1000, MT7916
pub const C_CALIBRATION_OFFSET: usize = 0x2000; // 0x200, EN7572 上行 PON
pub const C_PON_DOWNLINK_CAL_OFFSET: usize = 0x2200; // 0x120, UX3326 下行 PON
pub const C_PON_DOWNLINK_FLAGS_OFFSET: usize = 0x2400; // OBosaMode/BosaFlag/TempFlag

// MAC derivation offsets (applied to the last byte only).
pub const OFF_ETH0: u8 = 1;
pub const OFF_PON0: u8 = 2;
pub const OFF_WIFI_5G: u8 = 4;
pub const OFF_WIFI_24G: u8 = 5;

/// The three BOSA scalars that live next to the UX3326 table in mtd13's
/// `ubifs1` volume: `/bosa/OBosaMode`, `/bosa/BosaFlag`, `/bosa/TempFlag`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PonDownlinkFlags {
    pub obosa_mode: u16,
    pub bosa_flag: u16,
    pub temp_flag: u16,
}

impl PonDownlinkFlags {
    /// Six little-endian bytes, in the order the flags are stored.
    pub fn to_bytes(self) -> [u8; PON_DOWNLINK_FLAGS_SIZE] {
        let mut b = [0u8; PON_DOWNLINK_FLAGS_SIZE];
        b[0..2].copy_from_slice(&self.obosa_mode.to_le_bytes());
        b[2..4].copy_from_slice(&self.bosa_flag.to_le_bytes());
        b[4..6].copy_from_slice(&self.temp_flag.to_le_bytes());
        b
    }

    pub fn from_bytes(b: &[u8]) -> Self {
        let g = |i: usize| {
            b.get(i..i + 2)
                .map(|s| u16::from_le_bytes([s[0], s[1]]))
                .unwrap_or(0)
        };
        PonDownlinkFlags {
            obosa_mode: g(0),
            bosa_flag: g(2),
            temp_flag: g(4),
        }
    }
}

/// Size of the serialised [`PonDownlinkFlags`] blob (3 x u16 LE).
pub const PON_DOWNLINK_FLAGS_SIZE: usize = 6;

#[derive(Clone)]
pub struct Factory {
    pub device_sn: String,
    pub label_mac: [u8; 6],
    pub calibration: Vec<u8>, // 0x200, 上行 PON (EN7572)
    pub wifi_eeprom: Vec<u8>, // 0x1000
    /// 下行 PON 口校准表 (`mmodified_UX3326.bin`), 0x120 bytes.
    pub pon_downlink_cal: Vec<u8>,
    /// `/bosa/` 下的三个 u16 标量。
    pub pon_downlink_flags: PonDownlinkFlags,
}

impl Factory {
    pub fn new() -> Self {
        Factory {
            device_sn: random_sn(),
            label_mac: random_mac(),
            calibration: vec![0xff; CALIBRATION_SIZE],
            wifi_eeprom: vec![0xff; WIFI_EEPROM_SIZE],
            pon_downlink_cal: vec![0xff; PON_DOWNLINK_CAL_SIZE],
            pon_downlink_flags: PonDownlinkFlags::default(),
        }
    }

    /// eth0 (+1), pon0/wan (+2), 5G (+4), 2.4G (+5) — all on the last byte.
    pub fn derive_macs(&self) -> ([u8; 6], [u8; 6], [u8; 6], [u8; 6]) {
        (
            derive_mac(&self.label_mac, OFF_ETH0),
            derive_mac(&self.label_mac, OFF_PON0),
            derive_mac(&self.label_mac, OFF_WIFI_5G),
            derive_mac(&self.label_mac, OFF_WIFI_24G),
        )
    }

    /// Build the layout-mapped demo image (not flashable).
    pub fn encode(&self) -> Vec<u8> {
        let mut img: Vec<u8> = vec![0xff; DEMO_SIZE];

        img[MAGIC_OFFSET..MAGIC_OFFSET + 4].copy_from_slice(&MAGIC);
        img[BASE_MAC_OFFSET..BASE_MAC_OFFSET + 6].copy_from_slice(&self.label_mac);

        let sn = self.device_sn.as_bytes();
        let n = sn.len().min(DEVICE_SN_SIZE);
        if n > 0 {
            img[DEVICE_SN_OFFSET..DEVICE_SN_OFFSET + n].copy_from_slice(&sn[..n]);
            img[DEVICE_SN_DUP_OFFSET..DEVICE_SN_DUP_OFFSET + n].copy_from_slice(&sn[..n]);
        }

        // AL-MAC region: plain-text ctromfile block for easy viewing/comparison.
        let block = format!(
            "<ctromfile>\n  <mapcfg AL-MAC=\"{}\" />\n  <sysinfo SerialNumber=\"{}\" />\n</ctromfile>\n",
            mac_to_string(&self.label_mac),
            self.device_sn
        );
        let b = block.as_bytes();
        let bn = b.len().min(ALMAC_REGION_SIZE);
        img[ALMAC_REGION_OFFSET..ALMAC_REGION_OFFSET + bn].copy_from_slice(&b[..bn]);

        let cn = self.calibration.len().min(CALIBRATION_SIZE);
        if cn > 0 {
            img[CALIBRATION_OFFSET..CALIBRATION_OFFSET + cn].copy_from_slice(&self.calibration[..cn]);
        }
        let en = self.wifi_eeprom.len().min(WIFI_EEPROM_SIZE);
        if en > 0 {
            img[WIFI_EEPROM_OFFSET..WIFI_EEPROM_OFFSET + en].copy_from_slice(&self.wifi_eeprom[..en]);
        }
        self.write_downlink(
            &mut img,
            PON_DOWNLINK_CAL_OFFSET,
            PON_DOWNLINK_CAL_OFFSET + PON_DOWNLINK_CAL_SIZE,
        );

        img
    }

    /// Write the downlink PON blob + BOSA flags at `base`. Shared by the
    /// layout-mapped demo image and the 1 MiB compact image so both stay in
    /// sync.
    fn write_downlink(&self, img: &mut [u8], base: usize, flags_off: usize) {
        let dn = self.pon_downlink_cal.len().min(PON_DOWNLINK_CAL_SIZE);
        if dn > 0 && base + dn <= img.len() {
            img[base..base + dn].copy_from_slice(&self.pon_downlink_cal[..dn]);
        }
        let fb = self.pon_downlink_flags.to_bytes();
        if flags_off + fb.len() <= img.len() {
            img[flags_off..flags_off + fb.len()].copy_from_slice(&fb);
        }
    }

    /// 1 MiB compact image: every field packed at fixed documented offsets so
    /// the whole bundle fits into 1 MiB. Carries the label MAC plus all four
    /// derived addresses (eth0=+1, pon0=+2, 5G=+4, 2.4G=+5), Device SN,
    /// Wi-Fi EEPROM and PON calibration. NOT flashable.
    pub fn encode_1mib(&self) -> Vec<u8> {
        let mut img: Vec<u8> = vec![0xff; COMPACT_SIZE];
        img[0..COMPACT_MAGIC.len()].copy_from_slice(&COMPACT_MAGIC);

        let (e0, p0, w5, w24) = self.derive_macs();
        for (off, mac) in [
            (C_LABEL_MAC_OFFSET, &self.label_mac),
            (C_ETH0_MAC_OFFSET, &e0),
            (C_PON0_MAC_OFFSET, &p0),
            (C_WIFI5G_MAC_OFFSET, &w5),
            (C_WIFI24_MAC_OFFSET, &w24),
        ] {
            img[off..off + 6].copy_from_slice(mac);
        }

        let sn = self.device_sn.as_bytes();
        let n = sn.len().min(DEVICE_SN_SIZE);
        if n > 0 {
            img[C_DEVICE_SN_OFFSET..C_DEVICE_SN_OFFSET + n].copy_from_slice(&sn[..n]);
        }

        let en = self.wifi_eeprom.len().min(WIFI_EEPROM_SIZE);
        if en > 0 {
            img[C_WIFI_EEPROM_OFFSET..C_WIFI_EEPROM_OFFSET + en]
                .copy_from_slice(&self.wifi_eeprom[..en]);
        }
        let cn = self.calibration.len().min(CALIBRATION_SIZE);
        if cn > 0 {
            img[C_CALIBRATION_OFFSET..C_CALIBRATION_OFFSET + cn]
                .copy_from_slice(&self.calibration[..cn]);
        }
        self.write_downlink(
            &mut img,
            C_PON_DOWNLINK_CAL_OFFSET,
            C_PON_DOWNLINK_FLAGS_OFFSET,
        );

        img
    }

    pub fn summary(&self) -> String {
        let (e0, p0, w5, w24) = self.derive_macs();
        let f = self.pon_downlink_flags;
        format!(
            "Device SN : {}\nLabel MAC : {}\neth0  (+1): {}\npon0  (+2): {}\n5G    (+4): {}\n2.4G  (+5): {}\ncalibration: {} bytes\neeprom    : {} bytes\n下行PON   : {} bytes ({})\n  OBosaMode: {}\n  BosaFlag : {}\n  TempFlag : {}",
            self.device_sn,
            mac_to_string(&self.label_mac),
            mac_to_string(&e0),
            mac_to_string(&p0),
            mac_to_string(&w5),
            mac_to_string(&w24),
            self.calibration.len(),
            self.wifi_eeprom.len(),
            self.pon_downlink_cal.len(),
            detect_downlink_chip(&self.pon_downlink_cal).unwrap_or_else(|| "未识别".into()),
            f.obosa_mode,
            f.bosa_flag,
            f.temp_flag,
        )
    }
}

/// Identify the downlink optical module from the UX3326 calibration table.
///
/// The table is stored under the name `mmodified_UX3326.bin` inside mtd13's
/// `ubifs1` volume, but that name is not part of the blob itself. Detection
/// therefore relies on the one structural invariant that survives across
/// calibration revisions: an 84-entry receive-sensitivity lookup table sitting
/// at +0x70 of the 288-byte blob.
///
/// Note the entries are **big-endian** — the curve reads 220..318 (0x00dc..
/// 0x013e) with `struct.unpack_from(">84H", ...)` and is garbage when read
/// little-endian. All ten revisions preserved in this mtd13 agree on the
/// offset, the entry count and the byte order.
///
/// `None` means the blob is blank (or too short) and carries no such table.
pub fn detect_downlink_chip(cal: &[u8]) -> Option<String> {
    if cal.len() < PON_DOWNLINK_CAL_SIZE || cal.iter().all(|&b| b == 0xff) {
        return None;
    }
    find_sensitivity_lut(cal).map(|_| "UX3326".to_string())
}

/// Byte offset of the 84-entry big-endian receive-sensitivity LUT inside the
/// UX3326 table. Verified identical across every ino 0x45 revision found in
/// the sample mtd13 (0x2210a8 .. 0x225800).
const PON_LUT_ENTRIES: usize = 84;

/// Search for the sensitivity LUT; returns its offset when found.
const LUT_SEARCH_FROM: usize = 0x60;

fn find_sensitivity_lut(cal: &[u8]) -> Option<usize> {
    let end = PON_DOWNLINK_CAL_SIZE - 7;
    for start in (LUT_SEARCH_FROM..end).step_by(2) {
        let lut = cal.get(start..start + PON_LUT_ENTRIES * 2)?;
        if lut_is_sensitivity_lut(lut) {
            return Some(start);
        }
    }
    None
}

/// A real table: monotonically non-decreasing, starting above zero, never
/// 0xffff (the erased-blob filler), and actually climbing at the tail.
///
/// The extra predicates matter because a constant fill (e.g. a buffer of 0x5a)
/// trivially satisfies "non-decreasing" and would otherwise be misidentified
/// as a real optical module.
fn lut_is_sensitivity_lut(lut: &[u8]) -> bool {
    if lut.len() < PON_LUT_ENTRIES * 2 {
        return false;
    }
    let vals: Vec<u16> = lut
        .chunks_exact(2)
        .map(|p| u16::from_be_bytes([p[0], p[1]]))
        .collect();
    if vals[0] == 0 || vals.iter().any(|&v| v == 0xffff) {
        return false;
    }
    if !vals.windows(2).all(|w| w[0] <= w[1]) {
        return false;
    }
    // Strictly rising tail: the curve must actually climb, not sit flat.
    vals.windows(2).skip(PON_LUT_ENTRIES - 9).any(|w| w[0] < w[1])
}

/// Parse a MAC from `aa:bb:cc:dd:ee:ff`, `aa-bb-...` or 12 raw hex digits.
/// Rejects all-zero, all-ff and multicast addresses.
pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let s = s.trim();
    let parts: Vec<&str> = if s.contains(':') {
        s.split(':').collect()
    } else if s.contains('-') {
        s.split('-').collect()
    } else if s.len() == 12 {
        (0..12).step_by(2).map(|i| &s[i..i + 2]).collect()
    } else {
        return None;
    };
    if parts.len() != 6 {
        return None;
    }
    let mut m = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        m[i] = u8::from_str_radix(p, 16).ok()?;
    }
    if m == [0u8; 6] || m == [0xff; 6] {
        return None;
    }
    if m[0] & 0x01 != 0 {
        return None; // multicast
    }
    Some(m)
}

/// `aa:BB:cc:DD:ee:FF` (uppercase, colon separated).
pub fn mac_to_string(m: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )
}

/// H3C adds the offset to the last byte only (see `h3c_mac_offset`).
pub fn derive_mac(base: &[u8; 6], off: u8) -> [u8; 6] {
    let mut m = *base;
    m[5] = m[5].wrapping_add(off);
    m
}

/// Identify the PON frontend chip from the calibration blob, so the hardware
/// shown in the UI comes from the imported data instead of being hardcoded.
///
/// Two formats are recognised:
///   * FiberHome-style `APONCAL` header: numeric chip id at 0x0c
///     (1 = GN28L95, 2 = UX3363), the same mapping used upstream.
///   * H3C reservearea: the chip name sits in the calibration block as padded
///     ASCII — measured on a real HM2004-DU the 0x1c0400 region holds
///     "ECONET" at +0x14 and "EN7572" at +0x28.
///
/// `None` means the blob is blank or carries no recognisable part number.
pub fn detect_pon_chip(cal: &[u8]) -> Option<String> {
    if cal.is_empty() || cal.iter().all(|&b| b == 0xff) {
        return None;
    }
    if cal.starts_with(b"APONCAL\0") && cal.len() >= 0x10 {
        let id = u32::from_le_bytes([cal[0x0c], cal[0x0d], cal[0x0e], cal[0x0f]]);
        return Some(match id {
            1 => "GN28L95".to_string(),
            2 => "UX3363".to_string(),
            _ => format!("未知芯片 #{id}"),
        });
    }
    extract_chip_token(cal)
}

/// Vendor strings carry no part number; skip them when looking for the chip.
const VENDOR_TOKENS: &[&str] = &["ECONET"];

fn take_token(run: &mut Vec<u8>) -> Option<String> {
    let s = String::from_utf8_lossy(run).trim().to_string();
    run.clear();
    if s.len() < 4 {
        return None;
    }
    let alpha = s.chars().any(|c| c.is_ascii_alphabetic());
    let digit = s.chars().any(|c| c.is_ascii_digit());
    if alpha && digit && !VENDOR_TOKENS.iter().any(|v| s.eq_ignore_ascii_case(v)) {
        Some(s)
    } else {
        None
    }
}

/// First printable ASCII run that looks like a part number (letters + digits),
/// so unknown hardware is reported rather than mislabelled.
fn extract_chip_token(cal: &[u8]) -> Option<String> {
    let mut run: Vec<u8> = Vec::new();
    for &b in cal.iter().chain(std::iter::once(&b'\n')) {
        if (0x20..0x7f).contains(&b) {
            run.push(b);
        } else if let Some(t) = take_token(&mut run) {
            return Some(t);
        }
    }
    None
}

/// Locally-administered, unicast random MAC.
pub fn random_mac() -> [u8; 6] {
    let mut m = [0u8; 6];
    // getrandom 0.3 exposes `fill`, not `getrandom`.
    let _ = getrandom::fill(&mut m);
    // Set the locally administered bit and clear the multicast bit.
    m[0] = (m[0] & 0xfc) | 0x02;
    m
}

/// `H3CT` + 8 random uppercase hex = 12 chars, exactly matching DEVICE_SN_SIZE
/// and the real device serial format (`H3CT0005EBF0`).
pub fn random_sn() -> String {
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    format!("H3CT{:08X}", u32::from_be_bytes(b))
}
