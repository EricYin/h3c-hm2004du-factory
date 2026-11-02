//! H3C HL202-DU（从机）reservearea 分区布局 + 独立 512 KiB 紧凑镜像。
//!
//! ## 为什么单开一个模块
//!
//! HL202-DU 是 FTTR 从机，它的 `reservearea` 挂在 **mtd12**（不是主机的 mtd14），
//! 大小同为 `0x240000`，magic 也一样是 `21 43 34 12`，但内容结构不同：
//!
//! ```text
//! 0x04c000  Wi-Fi EEPROM, 0x1000      (MT7916D，与主机同偏移同长度)
//! 0x080000  gzip(ctromfile.cfg)       (label MAC 在 AL-MAC="..." 内，同主机)
//! 0x100000  syslog                     (可丢弃)
//! 0x140000  GUD 字段索引表 (uint32 数组)
//! 0x141010  GUD 出厂字符串, 151 字节   (SN / SSID / 密码 / 生产日期)
//! 0x1c0400  PON BOB 校准, 0xE1 (225)  (EN7571，长度逐台实测)
//! ```
//!
//! 关键差异，逐条实测确认（两台主机 + 这台从机）：
//!
//! - **没有下行 PON 校准**。整分区搜 `UX3326` / `OBosaMode` / `BosaFlag` /
//!   `TempFlag` 全部 0 命中，所以从机不需要 mtd13，紧凑镜像里也没有这一块。
//! - **上行校准长度不同**：主机是 `0x200`（512 B，EN7572），从机是 `0xE1`
//!   （225 B，EN7571），且内部结构不同（从机的在 `+0x40` 处有 `RCF\0` 段标记）。
//! - **GUD 字符串语义不同**：主机那段是 `SN + "68CU" + 混淆串`，从机是定长字段
//!   顺序拼接（SN / SSID / 密码 / 型号 / 配置名 / 密钥 / PON 密码 / 标记 / 日期）。
//!
//! ## 512 KiB 紧凑布局（与主机**不共用**）
//!
//! 用户明确要求从机走独立布局，因此这里用**专属 magic `HL2-512K`**，偏移也独立
//! 规划，不与 `H3C-1MiB` 混用。解析时靠 magic 区分机型，两套互不干扰。
//!
//! 尺寸取 **512 KiB（0x80000）**，不是 1 MiB：从机需要搬运的全部数据只到
//! `0x20E1`（EEPROM 4 KiB + BOB 225 B），512 KiB 已有 8 倍余量，再大只是空洞。
//! 分配单元的常见取值（NAND 块 128 KiB / 4 倍）也能整除 512 KiB。
//!
//! ```text
//! 0x000000  8   magic "HL2-512K"
//! 0x000010  6   label MAC
//! 0x000020  12  Device SN（12 字符）
//! 0x000030  6   eth0  = label+1
//! 0x000040  6   pon0  = label+2
//! 0x000050  6   5G    = label+4
//! 0x000060  6   2.4G  = label+5
//! 0x001000  0x1000  Wi-Fi EEPROM (MT7916D)
//! 0x002000  0xE1    PON BOB 校准 (EN7571)
//! ---------- 0x0020E1 之后至 0x80000 全 0xFF ----------
//! ```
//!
//! 派生 MAC 的偏移与主机 `model.rs` 用同一组常量（末字节 +1/+2/+4/+5），因为
//! 两台机型共用同一套 H3C 出厂框架（ctromfile / AL-MAC / nas 命名一致）。
//! **但要如实说明证据强度**：这台从机的 mtd12 里只有 label MAC 一个明文值
//! （`AL-MAC="44:76:09:10:30:f8"`），romfile 的 `mw_mac_offset="7"/"4"` 属于
//! WAN 连接（PVC）字段、与该派生无关，syslog 区也没有接口 MAC 记录。也就是说
//! 从机的派生值**没有真机实证**，是沿用同厂商约定的推定值，界面上按此显示。
//!
//! GUD 里只保留 **SN 和 MAC**（用户选择），SSID / 密码 / 生产日期 / 设备密钥 /
//! PON 密码**不写入**紧凑镜像 —— 这些在原分区里仍在，不经过本工具搬运。

// 本模块里若干常量（分区大小、AL-MAC 区范围等）是给后来人看的「偏移档案」，
// 不一定都被代码引用；为了不让编译输出被 dead_code 警告刷屏，统一放开。
#![allow(dead_code)]

use crate::import;
use crate::model::{
    derive_mac, mac_to_string, OFF_ETH0, OFF_PON0, OFF_WIFI_24G, OFF_WIFI_5G,
};

/// HL202-DU `reservearea`（mtd12）分区大小。
pub const HL_FACTORY_SIZE: usize = 0x240000;

pub const HL_MAGIC_OFFSET: usize = 0x140000;
pub const HL_MAGIC: [u8; 4] = [0x21, 0x43, 0x34, 0x12];

/// GUD 出厂字符串区起点。
pub const HL_GUD_OFFSET: usize = 0x141010;
/// GUD 字符串区总长（索引表头部声明的 151 字节 + 1 字节 0xFF 结束符）。
pub const HL_GUD_SIZE: usize = 151;

/// SN 主槽：GUD 字符串区内 `+0x1c`，12 字符。
pub const HL_SN_OFFSET: usize = 0x14102c;
/// SN 副本：GUD 字符串区内 `+0x4f`，同样 12 字符。
pub const HL_SN_DUP_OFFSET: usize = 0x14105f;
pub const HL_SN_SIZE: usize = 0x0c;

pub const HL_ALMAC_OFFSET: usize = 0x80000;
pub const HL_ALMAC_SIZE: usize = 0x4000;

pub const HL_EEPROM_OFFSET: usize = 0x4c000;
pub const HL_EEPROM_SIZE: usize = 0x1000;

/// PON BOB（EN7571）校准区。原厂 `mtd bob get` 按 `offset 0x1C0400 / len 0xE1`
/// 读写这一块（反汇编确认），故长度取 225 而非整块对齐。
///
/// ## 型号依据（分区内查不到，来自外部证据）
///
/// 整盘搜 `EN7570` / `EN7571` / `EN7572` 明文 **全部 0 命中**；这块 225 字节里唯一
/// 的可打印串是 `+0x40` 的 `RCF\0`。所以型号不是从校准数据读出来的，依据是：
///
/// 1. 拆机实测 HL202-DU 的 CPU 是 `EN7529CT`（恩山首拆帖 + 与 FRR-2022X 同配置：
///    MT7916AN + MT7976DN + 256 MB + 2.5G PHY）。
/// 2. Econet 方案里 `EN7529` 一律搭配 **EN7571**（LDD/LA），用于 WiFi5/6
///    GPON/EPON 光猫；同架构的 FTTR 从机 FRR-2022X 即 `EN7529CT + EN7571N`。
/// 3. `EN7572` 是 `AN7581` 平台的 10G-PON 搭配 —— 那正是主机 HM2004-DU。
/// 4. `EN7570` 在公开的 ONU/光猫方案里查不到对应型号。
pub const HL_BOB_OFFSET: usize = 0x1c0400;
pub const HL_BOB_SIZE: usize = 0xe1;

/// 从机 PON 芯片型号，供界面与摘要显示。依据见 `HL_BOB_OFFSET` 的注释：
/// 校准数据里没有型号明文，这是按 EN7529 平台搭配推定出来的。
pub const HL_PON_CHIP: &str = "EN7571";

// ---- 从机专属 512 KiB 紧凑布局 ----

/// 紧凑镜像总长：512 KiB。有效数据只到 `0x20E1`，余量充裕。
pub const HL_COMPACT_SIZE: usize = 0x80000;
pub const HL_COMPACT_MAGIC: [u8; 8] = *b"HL2-512K";

/// 旧版 1 MiB 镜像的 magic。仅用于**读取兼容** —— 早先生成的文件尺寸是
/// `0x100000`、magic 是 `HL2-1MiB`，偏移与本版完全相同，因此仍可正常解析；
/// 但新生成一律是 512 KiB + `HL2-512K`。
pub const HL_COMPACT_MAGIC_LEGACY: [u8; 8] = *b"HL2-1MiB";

pub const HL_C_LABEL_MAC_OFFSET: usize = 0x0010;
pub const HL_C_SN_OFFSET: usize = 0x0020;
/// 派生 MAC 四连：偏移与主机一致（末字节 +1/+2/+4/+5）。
/// `0x30` 起是它们的位置；旧版镜像这里是 `0xFF`，读取时忽略、显示时现算。
pub const HL_C_ETH0_MAC_OFFSET: usize = 0x0030;
pub const HL_C_PON0_MAC_OFFSET: usize = 0x0040;
pub const HL_C_WIFI5G_MAC_OFFSET: usize = 0x0050;
pub const HL_C_WIFI24_MAC_OFFSET: usize = 0x0060;
pub const HL_C_EEPROM_OFFSET: usize = 0x1000;
pub const HL_C_BOB_OFFSET: usize = 0x2000;

/// 判断一段数据是不是从机的紧凑镜像（新版 512 KiB 或旧版 1 MiB）。
pub fn is_compact(data: &[u8]) -> bool {
    data.starts_with(&HL_COMPACT_MAGIC) || data.starts_with(&HL_COMPACT_MAGIC_LEGACY)
}

/// GUD 字符串区里除了 SN 之外还有 SSID / 密码等字段。本工具不解析它们（用户
/// 只要求保留 SN/MAC），但需要知道它们的确切边界，否则改 SN 时会误伤相邻字段。
///
/// 边界是**实测**的（不是照抄文档）：`ctromfile_GUD.cfg` 长 17 字符而非 16，
/// 所以自它之后的字段整体比常见资料里写的偏移多 1 字节。
pub mod gud {
    /// GUD 标识码，28 字节，如 `2021000FE201FFFFFFFF024FFF24`。
    pub const ID: (usize, usize) = (0x141010, 28);
    /// PON SN，12 字符。
    pub const SN: (usize, usize) = (0x14102c, 12);
    /// 保留 2 字节（实测恒为 `6E`）。
    pub const RESERVED: (usize, usize) = (0x141038, 2);
    /// SSID 2.4G，7 字符（如 `CU_Td9X`）。
    pub const SSID_24: (usize, usize) = (0x14103a, 7);
    /// WiFi 密码 2.4G，8 字符。
    pub const PW_24: (usize, usize) = (0x141041, 8);
    /// SSID 5G，10 字符。
    pub const SSID_5G: (usize, usize) = (0x141049, 10);
    /// WiFi 密码 5G，8 字符。
    pub const PW_5G: (usize, usize) = (0x141053, 8);
    /// 硬件版本，4 字符（`V1.0`）。
    pub const HW: (usize, usize) = (0x14105b, 4);
    /// SN 副本，12 字符。
    pub const SN_DUP: (usize, usize) = (0x14105f, 12);
    /// 来源配置名，17 字符（`ctromfile_GUD.cfg`）。
    pub const CFG_NAME: (usize, usize) = (0x14106b, 17);
    /// WiFi 密码副本，8 字符。
    pub const PW_DUP: (usize, usize) = (0x14107c, 8);
    /// 设备密钥，12 字符。
    pub const KEY: (usize, usize) = (0x141084, 12);
    /// PON 密码，10 字符。
    pub const PON_PW: (usize, usize) = (0x141090, 10);
    /// 标记 `GUD`，3 字符。
    pub const FLAG: (usize, usize) = (0x14109a, 3);
    /// 生产日期，10 字符（`2025-09-18`）。
    pub const DATE: (usize, usize) = (0x14109d, 10);
}

/// 从机可编辑字段集合。
#[derive(Clone)]
pub struct Hl202 {
    pub device_sn: String,
    pub label_mac: [u8; 6],
    /// PON BOB 校准 (EN7571)，`HL_BOB_SIZE` 字节。
    pub bob: Vec<u8>,
    /// Wi-Fi EEPROM，`HL_EEPROM_SIZE` 字节。
    pub wifi_eeprom: Vec<u8>,
}

impl Hl202 {
    pub fn new() -> Self {
        Hl202 {
            device_sn: crate::model::random_sn(),
            label_mac: crate::model::random_mac(),
            bob: vec![0xff; HL_BOB_SIZE],
            wifi_eeprom: vec![0xff; HL_EEPROM_SIZE],
        }
    }

    /// 四个派生接口 MAC：`(eth0, pon0, 5G, 2.4G)`，均由 label MAC 末字节相加得出。
    ///
    /// 与主机 `Factory::derive_macs()` 用同一组偏移常量，理由见模块文档顶部。
    pub fn derive_macs(&self) -> ([u8; 6], [u8; 6], [u8; 6], [u8; 6]) {
        (
            derive_mac(&self.label_mac, OFF_ETH0),
            derive_mac(&self.label_mac, OFF_PON0),
            derive_mac(&self.label_mac, OFF_WIFI_5G),
            derive_mac(&self.label_mac, OFF_WIFI_24G),
        )
    }

    /// 从机专属 512 KiB 紧凑镜像。magic 为 `HL2-512K`，与主机的 `H3C-1MiB` 区分。
    pub fn encode_compact(&self) -> Vec<u8> {
        let mut img = vec![0xffu8; HL_COMPACT_SIZE];
        img[..HL_COMPACT_MAGIC.len()].copy_from_slice(&HL_COMPACT_MAGIC);

        let (e0, p0, w5, w24) = self.derive_macs();
        for (off, mac) in [
            (HL_C_LABEL_MAC_OFFSET, &self.label_mac),
            (HL_C_ETH0_MAC_OFFSET, &e0),
            (HL_C_PON0_MAC_OFFSET, &p0),
            (HL_C_WIFI5G_MAC_OFFSET, &w5),
            (HL_C_WIFI24_MAC_OFFSET, &w24),
        ] {
            img[off..off + 6].copy_from_slice(mac);
        }

        let sn = self.device_sn.as_bytes();
        let n = sn.len().min(HL_SN_SIZE);
        if n > 0 {
            img[HL_C_SN_OFFSET..HL_C_SN_OFFSET + n].copy_from_slice(&sn[..n]);
        }

        let en = self.wifi_eeprom.len().min(HL_EEPROM_SIZE);
        img[HL_C_EEPROM_OFFSET..HL_C_EEPROM_OFFSET + en]
            .copy_from_slice(&self.wifi_eeprom[..en]);

        let bn = self.bob.len().min(HL_BOB_SIZE);
        img[HL_C_BOB_OFFSET..HL_C_BOB_OFFSET + bn].copy_from_slice(&self.bob[..bn]);

        img
    }

    pub fn summary(&self) -> String {
        let (e0, p0, w5, w24) = self.derive_macs();
        format!(
            "机型      : HL202-DU（从机，reservearea = mtd12）\nDevice SN : {}\nLabel MAC : {}\neth0  (+1): {}\npon0  (+2): {}\n5G    (+4): {}\n2.4G  (+5): {}\nPON BOB   : {} / {} bytes (EN7571)\nWi-Fi EEPROM: {} / {} bytes (MT7916D)\n\n本机型没有下行 PON 校准分区（mtd12 内无 /bosa/ 数据）。",
            self.device_sn,
            mac_to_string(&self.label_mac),
            mac_to_string(&e0),
            mac_to_string(&p0),
            mac_to_string(&w5),
            mac_to_string(&w24),
            self.count_loaded(&self.bob),
            self.bob.len(),
            self.count_loaded(&self.wifi_eeprom),
            self.wifi_eeprom.len(),
        )
    }

    fn count_loaded(&self, v: &[u8]) -> usize {
        v.iter().filter(|&&b| b != 0xff).count()
    }
}

impl Default for Hl202 {
    fn default() -> Self {
        Self::new()
    }
}

/// 判断一个 2.25 MB dump 是不是 HL202-DU 的 mtd12。
///
/// 不能只看 magic —— 主机 mtd14 也是 `21 43 34 12` @ `0x140000`、SN 也在
/// `0x14102c`。可靠的区分点是 GUD 字符串区里"配置名"这个字段：
///
/// - 主机（HM2004-DU）：`ctromfile_GUI.cfg`
/// - 从机（HL202-DU）：`ctromfile_GUD.cfg`
///
/// 再用一份辅助判据兜底：主机上行校准区在 `0x1c0414` 有明文 `ECONET`，
/// 从机没有。
pub fn looks_like_hl202_volume(data: &[u8]) -> bool {
    if data.len() < HL_MAGIC_OFFSET + 4 {
        return false;
    }
    if data[HL_MAGIC_OFFSET..HL_MAGIC_OFFSET + 4] != HL_MAGIC {
        return false;
    }
    let gud_end = (HL_GUD_OFFSET + 0x100).min(data.len());
    let gud = &data[HL_GUD_OFFSET.min(data.len())..gud_end];
    if import::find_subseq(gud, b"ctromfile_GUD.cfg").is_some() {
        return true;
    }
    if import::find_subseq(gud, b"ctromfile_GUI.cfg").is_some() {
        return false;
    }
    // 兜底：没有配置名时，用 ECONET 标记判断。
    data.get(HL_BOB_OFFSET + 0x14..HL_BOB_OFFSET + 0x1a)
        .map(|s| s != b"ECONET")
        .unwrap_or(false)
}

/// 解析 HL202-DU 的 mtd12，取出 SN / label MAC / BOB / EEPROM。
///
/// 与主机一样，label MAC 不在固定字段里，而是藏在 0x80000 的 gzip ctromfile
/// 的 `AL-MAC="..."` 中；若那里没有，再退回 `0x140005` 的占位槽。
pub fn parse_hl202_volume(data: &[u8]) -> Hl202 {
    let almac = import::read_almac_bytes(data);

    let mut device_sn = read_sn(data);
    if device_sn.is_empty() {
        if let Some(text) = &almac {
            if let Some(sn) = import::find_serial_number(text) {
                device_sn = sn;
            }
        }
    }

    let label_mac = almac
        .as_deref()
        .and_then(import::find_al_mac)
        .or_else(|| import::read_base_mac_placeholder(data))
        .unwrap_or([0u8; 6]);

    Hl202 {
        device_sn,
        label_mac,
        bob: read_region(data, HL_BOB_OFFSET, HL_BOB_SIZE),
        wifi_eeprom: read_region(data, HL_EEPROM_OFFSET, HL_EEPROM_SIZE),
    }
}

/// 解析从机专属紧凑镜像（magic `HL2-512K`，也接受旧版 `HL2-1MiB`）。
///
/// 两者布局偏移一致，只是总长不同，故同一套读取逻辑即可。
pub fn parse_compact(data: &[u8]) -> Hl202 {
    let label_mac = data
        .get(HL_C_LABEL_MAC_OFFSET..HL_C_LABEL_MAC_OFFSET + 6)
        .map(|s| {
            let mut m = [0u8; 6];
            m.copy_from_slice(s);
            m
        })
        .unwrap_or([0u8; 6]);

    let device_sn = data
        .get(HL_C_SN_OFFSET..HL_C_SN_OFFSET + HL_SN_SIZE)
        .map(|s| {
            String::from_utf8_lossy(s)
                .trim_matches(|c| c == '\0' || c == ' ' || c == '\u{fffd}')
                .to_string()
        })
        .unwrap_or_default();

    Hl202 {
        device_sn,
        label_mac,
        bob: read_region(data, HL_C_BOB_OFFSET, HL_BOB_SIZE),
        wifi_eeprom: read_region(data, HL_C_EEPROM_OFFSET, HL_EEPROM_SIZE),
    }
}

fn read_sn(data: &[u8]) -> String {
    let Some(raw) = data.get(HL_SN_OFFSET..HL_SN_OFFSET + HL_SN_SIZE) else {
        return String::new();
    };
    String::from_utf8_lossy(raw)
        .trim_matches(|c| c == '\0' || c == ' ' || c == '\u{fffd}')
        .to_string()
}

fn read_region(data: &[u8], off: usize, size: usize) -> Vec<u8> {
    match data.get(off..off + size) {
        Some(s) => s.to_vec(),
        None => vec![0xff; size],
    }
}

/// 供 GUI 显示：把 GUD 字符串区里的 SN 之外字段也读出来，纯展示用，不参与导出。
pub fn read_gud_strings(data: &[u8]) -> Vec<(&'static str, String)> {
    let f = |(off, len): (usize, usize)| -> String {
        data.get(off..off + len)
            .map(|s| {
                String::from_utf8_lossy(s)
                    .trim_matches(|c| c == '\0' || c == ' ' || c == '\u{fffd}')
                    .to_string()
            })
            .unwrap_or_default()
    };
    vec![
        ("GUD 标识码", f(gud::ID)),
        ("SN", f(gud::SN)),
        ("SSID 2.4G", f(gud::SSID_24)),
        ("WiFi 密码 2.4G", f(gud::PW_24)),
        ("SSID 5G", f(gud::SSID_5G)),
        ("WiFi 密码 5G", f(gud::PW_5G)),
        ("硬件版本", f(gud::HW)),
        ("配置名", f(gud::CFG_NAME)),
        ("设备密钥", f(gud::KEY)),
        ("PON 密码", f(gud::PON_PW)),
        ("生产日期", f(gud::DATE)),
    ]
}
