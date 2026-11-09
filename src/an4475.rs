//! AN4475（10G PON ONT）出厂数据：mtd9 → 2 MiB 紧凑容器。
//!
//! ## mtd9 里到底是什么
//!
//! 真机 mtd9 不是裸分区，而是**一整个 UBI 设备**：
//!
//! ```text
//! PEB 128 KiB × 200 = 25 MiB
//!   ├─ PEB 0/1        布局卷（两份卷表副本）
//!   └─ 卷 0 `fhdata`  178 个 LEB ≈ 22.25 MiB，上面是 UBIFS
//! ```
//!
//! UBIFS 根目录下 21 个出厂文件（另有 15 个 `.factory_conf.uci-XXXXXX`
//! 是 UCI 写入时的临时文件，属中间态，本工具直接跳过）：
//!
//! ```text
//! factory_conf        SN / 12 套 MAC / GPON SN / 密码 / SSID …（449 行 UCI）
//! sysinfo_conf        软件版本 / 硬件版本 / 区域码 / 运营商
//! pre_usrconfig_conf  预置用户配置模板
//! pre_attrconfig_conf 预置属性模板
//! tr.conf             远程管理参数
//! voice_digitmap_conf 语音拨号表
//! fi_json.config      工厂 JSON 配置
//! table / xg_table / xgs_table / 10ge_sym_table / 10ge_asym_table / olt_table
//!                     6 张 2 KiB BOSA 校准表（`fiberhomebdbob` 头）
//! olt_bosa_info / olt_bosa_rssi / olt_bosa_apd_WTD01 / olt_bosa_apd_YS01
//!                     BOSA 型号与 APD/RSSI 定标（本机 BOSA = WTD01）
//! LanOpticTxCalib     LAN 光模块 TX 温补系数（Slope/Offset 共 8 项）
//! MT7993_EEPROM.bin   Wi-Fi 校准 + 两位 MAC，1 322 KiB（81% 为 0xFF）
//! wlan / factorymodeflag  空文件（旗标）
//! ```
//!
//! ## 为什么是「容器」而不是固定偏移
//!
//! H3C 两台机型的出厂数据是定长字段，1 MiB 里按固定偏移摆好即可。AN4475
//! 不同：单机唯一数据全部藏在**文本 UCI + 定长表**里，总量 1.40 MB，其中
//! `MT7993_EEPROM.bin` 一个文件就 1.32 MiB —— 直接摆进 1 MiB 会溢出。
//!
//! 因此这里用自描述的容器：**头部固定偏移放身份字段**（消费方不解析 TOC 也能
//! 直接读到 SN/MAC），**EEPROM 与 11 张 PON 校准表各占固定槽、原样不压缩**
//! （都能被 hex 工具直接读），只有 9 个配置文件进 zlib 载荷。整包实际占用
//! 1.66 MiB，2 MiB 里还剩约 340 KiB 空闲。
//!
//! ```text
//! 0x000000  8   magic "AN4-2MiB"（旧镜像为 "AN4-1MiB"，同样可读）
//! 0x000008  4   版本 = 1
//! 0x00000C  4   文件数
//! 0x000010  4   TOC 偏移 = 0x1000
//! 0x000014  4   载荷偏移 = 0x2000
//! 0x000018  4   载荷长度
//! 0x000020  24  设备序列号（ASCII，17 字符 + 0xFF 填充）
//! 0x000038  8   GPON SN（二进制，含 "FHTT" 厂商码）
//! 0x000040  6   基准 MAC（base_mac = brmac = ponmac）
//! 0x000046  6   LAN 基址 MAC（eth1mac）
//! 0x00004C  3   OUI（取基准 MAC 前三字节）
//! 0x00004F  2   厂商码 "FH"
//! 0x000051  10  GPON 密码（ASCII）
//! 0x000060  54  派生 MAC 9 项：internet/tr069/voip/priprotocol + eth1..eth5
//! 0x001000      TOC，每项 48 字节（名 32 + off/raw/stored 各 4 + 压缩标志 1）
//! 0x002000      其余 9 个配置文件（zlib 或原样，顺序压缩摆放）
//! 0x040000      MT7993_EEPROM.bin 原样存放（1 353 728 B，未压缩）
//! 0x18A800      EEPROM 结束
//! 0x1A0000      11 张 PON 校准表的直读区（每张 4 KiB 槽，原样不压缩）
//!               槽序见 AN_CAL_TABLES：0x1A0000 + 槽号 × 0x1000
//! 0x1AB000      直读区结束
//! ---------- 其余至 0x200000 全 0xFF ----------
//! ```
//!
//! ## MAC 派生规律（真机实证）
//!
//! `factory_conf` 里 12 个 MAC 全部明文，实测关系：
//!
//! | 字段 | 与基准 MAC 的关系 | 真机值 |
//! | --- | --- | --- |
//! | base_mac / brmac / ponmac | — | D4:9F:29:04:E7:50 |
//! | internetmac | +1 | D4:9F:29:04:E7:51 |
//! | tr069mac | +2 | D4:9F:29:04:E7:52 |
//! | voipmac | +3 | D4:9F:29:04:E7:53 |
//! | priprotocolmac | +8 | D4:9F:29:04:E7:58 |
//! | eth1..eth5 | LAN 基址 +0…+4 | D4:A0:29:04:E7:71…75 |
//!
//! 注意 LAN 段**不是**在基准 MAC 上做加法：真机上两者相差 `0x01_00_00_21`，
//! 是另一段预留地址，所以必须有独立的 `eth1mac` 基址，不能只存一个 MAC。

#![allow(dead_code)]

use std::collections::HashMap;
use std::io::{Read, Write};

/// UBI 物理擦除块大小（真机 mtd9 为 128 KiB）。
pub const AN_UBI_PEB: usize = 0x20000;
const AN_UBI_VID_OFF: usize = 0x800;
const AN_UBI_DATA_OFF: usize = 0x1000;
const AN_UBIFS_MAGIC: [u8; 4] = [0x31, 0x18, 0x10, 0x06];

/// 紧凑镜像总长：2 MiB。
///
/// AN4475 的出厂分区按 2 MiB 规划（原 1 MiB 版装得下，但余量只有 ~800 KiB，
/// 且部分批次的分区表就按 2 MiB 给）；镜像尾部一律填 `0xFF`。
pub const AN_COMPACT_SIZE: usize = 0x200000;
/// 当前 magic，末四位标出容量。
pub const AN_COMPACT_MAGIC: [u8; 8] = *b"AN4-2MiB";
/// 旧版 1 MiB 容器 magic：仍可读入，再导出时统一升级为 2 MiB。
pub const AN_COMPACT_MAGIC_V1: [u8; 8] = *b"AN4-1MiB";
pub const AN_COMPACT_VERSION: u32 = 1;

pub const AN_TOC_OFF: usize = 0x1000;
pub const AN_PAYLOAD_OFF: usize = 0x2000;
pub const AN_TOC_ENTRY: usize = 48;
pub const AN_NAME_MAX: usize = 32;
/// TOC 区能放下的最大文件数（0x1000 / 48 = 85）。
pub const AN_MAX_FILES: usize = (AN_PAYLOAD_OFF - AN_TOC_OFF) / AN_TOC_ENTRY;

/// 设备序列号长度。真机 `F3200D49F2904E750` = 17 字符。
pub const AN_SN_SIZE: usize = 17;
/// 头部给 SN 留的槽位（17 + 7 字节 0xFF 余量）。
pub const AN_SN_CAP: usize = 24;
pub const AN_GPON_SN_SIZE: usize = 8;
pub const AN_GPON_PW_SIZE: usize = 10;

pub const AN_H_SN: usize = 0x20;
pub const AN_H_GPON_SN: usize = 0x38;
pub const AN_H_BASE_MAC: usize = 0x40;
pub const AN_H_ETH_MAC: usize = 0x46;
pub const AN_H_OUI: usize = 0x4c;
pub const AN_H_VENDOR: usize = 0x4f;
pub const AN_H_GPON_PW: usize = 0x51;
pub const AN_H_MACS: usize = 0x60;

/// 0x60 + 9×6 = 0x96，Wi-Fi 两位 MAC 紧接其后（到 0x1000 前余量充足）。
pub const AN_H_WIFI_2G4: usize = 0x96;
pub const AN_H_WIFI_5G: usize = 0x9c;

/// 主配置文件名。身份字段全在这一个文件的 UCI 文本里。
pub const AN_MAIN_CONF: &str = "factory_conf";
/// Wi-Fi 校准文件名（1.32 MiB，容器里必须压缩存放）。
pub const AN_EEPROM: &str = "MT7993_EEPROM.bin";

/// 2 MiB 容器里 Wi-Fi EEPROM 的**固定偏移**，且不压缩。
///
/// EEPROM 原始 1 353 728 B，zlib 后约 199 KB —— 压缩只省 1.15 MB，而 2 MiB
/// 容器本来就有富余。固定摆在这里、原样存储，拿到镜像的人可以直接用 hex
/// 工具按 `0x40000` 打开看校准区，不必先解压；工具导出/写回也走同一份字节。
pub const AN_EEPROM_FIXED_OFF: usize = 0x40000;

/// PON 校准表的**直读区**起点：`0x1A0000`（1.625 MiB，128 KiB 对齐）。
///
/// 与 EEPROM 同样处理 —— 原样存放、不压缩，拿 hex 工具按槽偏移就能看。
/// 放在 EEPROM 之后（EEPROM 占 `0x40000–0x18A800`），前面留 86 KiB 缓冲，
/// 后面到 `0x200000` 还剩 340 KiB。
pub const AN_CAL_FIXED_OFF: usize = 0x1A0000;
/// 每张校准表占一个 4 KiB 槽（实测最大 2048 B，留一倍余量）。
pub const AN_CAL_SLOT: usize = 0x1000;

/// 直读区的槽位顺序（下标即槽号，偏移 = `0x1A0000 + 槽号 × 0x1000`）。
///
/// 顺序按用途分组排，方便记：10G 收发 → 通用/OLT 表 → OLT BOSA → 光口 TX。
/// 名字必须与 `fhdata` 卷里的一致，否则会落回压缩区。
pub const AN_CAL_TABLES: [&str; 11] = [
    "10ge_asym_table",
    "10ge_sym_table",
    "xg_table",
    "xgs_table",
    "table",
    "olt_table",
    "olt_bosa_apd_WTD01",
    "olt_bosa_apd_YS01",
    "olt_bosa_info",
    "olt_bosa_rssi",
    "LanOpticTxCalib",
];

/// 校准表名 → 直读区偏移；不是固定槽里的表就返回 `None`（走压缩区）。
pub fn cal_offset(name: &str) -> Option<usize> {
    AN_CAL_TABLES
        .iter()
        .position(|n| *n == name)
        .map(|i| AN_CAL_FIXED_OFF + i * AN_CAL_SLOT)
}

// Wi-Fi EEPROM 头很短：device id + 版本 + 两个 MAC，0x10..0x190 全 0，
// 0x190 起才是射频校准表。出厂值仍是芯片厂默认（2.4G/5G 分别
// 00:0C:43:26:60:10 / :11，00:0C:43 是 MediaTek 的 OUI），产线没写这两位。
pub const AN_EEPROM_DEVICE_ID: usize = 0x00; // u16 LE，MT7993 = 0x7993
pub const AN_EEPROM_VERSION: usize = 0x02; // u16 LE，实测 2
pub const AN_EEPROM_MAC_2G4: usize = 0x04;
pub const AN_EEPROM_MAC_5G: usize = 0x0a;

/// `factory_conf` 里对应的两个键。EEPROM 与应用层各存一份，必须一起改。
pub const AN_APMAC_2G4: &str = "APMAC_1";
pub const AN_APMAC_5G: &str = "APMAC_2";

/// 本工具约定的 Wi-Fi MAC 偏移（真机未写，无出厂实证，界面可改）。
pub const AN_WIFI_OFFSET_2G4: u8 = 6;
pub const AN_WIFI_OFFSET_5G: u8 = 7;

/// MT7993 的 device id。
///
/// 注意：开源 mt76 的 `mt7996` 驱动里这个值登记为 `MT7990_DEVICE_ID`（0x7993），
/// `mt7996_check_eeprom()` 认它，所以 MT7993 在 Linux 下被当成 MT7990 驱动。
pub const AN_MT7993_ID: u16 = 0x7993;

// ---------------------------------------------------------------------------
// mt76（开源驱动）视角的 EEPROM：定长 7 680 B
// ---------------------------------------------------------------------------

/// mt76 认的 EEPROM 长度 = `MT_EE_MAX + 1`。
///
/// 内核 `mt7996/eeprom.h`：`__MT_EE_MAX = 0x1dff`；
/// `mt7996/mt7996.h`：`MT7996_EEPROM_SIZE 7680`。驱动无论走 efuse 还是
/// 外部 EEPROM，都只按这个长度读；`/lib/firmware/mediatek/mt7996/mt7990_eeprom*.bin`
/// 也必须是 7 680 B（不够长度直接判 `Invalid default bin size`）。
///
/// 厂商 `MT7993_EEPROM.bin` 是 1 353 728 B —— **同一份数据的超集**：
/// 前 0x1000 与 mt76 模板基本逐字节相同（小 EE 头），0x1000–0x1DFF 起格式分叉，
/// 0x1E00 之后 1.30 MB 是 mt_wifi 私有校准区，mt76 根本不读。
pub const MT76_EE_SIZE: usize = 0x1E00;

/// 厂商 EEPROM 与 mt76 视图「说同一种话」的前缀长度：0x000–0xFFF。
///
/// 这一段两边字段布局一致（芯片 ID / 版本 / 两个 MAC / WIFI_CONF / NIC_CONF /
/// 各通道偏置），实测只差 10 个字节（都是补偿值），所以可以直接照搬。
pub const MT76_HEAD_SIZE: usize = 0x1000;

/// mt76 的 EEPROM 字段偏移（照抄内核 `mt7996/eeprom.h`，只列本工具用到的）。
pub const MT_EE_CHIP_ID: usize = 0x000;
pub const MT_EE_VERSION: usize = 0x002;
pub const MT_EE_MAC_ADDR: usize = 0x004;
pub const MT_EE_MAC_ADDR2: usize = 0x00a;
pub const MT_EE_WIFI_CONF: usize = 0x190;
pub const MT_EE_MAC_ADDR3: usize = 0x2c0;
pub const MT_EE_TX0_POWER_2G: usize = 0x1300;
pub const MT_EE_RATE_DELTA_2G: usize = 0x1400;

/// 官方默认 bin：`MT7993_MT7976_EEPROM_BE3600_iPAiLNA.bin`
/// （等同 `mediatek/mt7996/mt7990_eeprom_2i5i.bin`，内建 PA/LNA 变体）。
///
/// 选它的依据是实机 `WIFI_CONF+6/+7 = 0x08 0x08`，`MT_EE_WIFI_PA_LNA_CONFIG`
/// 两位都是 0 = `MT7996_FEM_INT`；换成 ePA 那份（0x8B）会被
/// `mt7996_eeprom_variant_valid()` 判为变体不符，直接回落到默认 bin。
pub const MT76_DEFAULT_IPA: &[u8] = include_bytes!("../assets/mt7990_eeprom_2i5i.bin");
/// 外置 PA/LNA 变体（`mt7990_eeprom_2e5e.bin`），仅供对照。
pub const MT76_DEFAULT_EPA: &[u8] = include_bytes!("../assets/mt7990_eeprom_2e5e.bin");

/// PA/LNA 变体：取 `WIFI_CONF+6` 的低 2 位（`MT_EE_WIFI_PA_LNA_CONFIG`）。
pub fn mt76_pa_lna(eeprom: &[u8]) -> Option<u8> {
    eeprom.get(MT_EE_WIFI_CONF + 6).map(|b| b & 0x3)
}

/// 把 `mt76_pa_lna()` 的两位翻成人话（对应内核 `MT7996_FEM_*`）。
pub fn mt76_fem_name(v: u8) -> &'static str {
    match v {
        0 => "内建 PA/LNA（2i5i）",
        3 => "外置 PA/LNA（2e5e）",
        _ => "混合 / 未知",
    }
}

/// 方式一：**原样截取**。直接把厂商 1 353 728 B 的前 7 680 B 当 mt76 EEPROM。
///
/// 保真但不一定可用：芯片 ID / 版本 / 两个 MAC / WIFI_CONF 都对，可
/// 0x1000–0x1DFF 在厂商文件里是它自己的位图编码（几乎全是 0xFF 加上零星
/// 清位），不是 mt76 要的 dBm 值。mt76 读到 `TX0_POWER = 0xFF` 会当成
/// 目标功率 −1 dBm，发射功率会塌掉。只建议用来做字节级对照。
pub fn mt76_carve(eeprom: &[u8]) -> Vec<u8> {
    let mut out = vec![0xffu8; MT76_EE_SIZE];
    let n = eeprom.len().min(MT76_EE_SIZE);
    out[..n].copy_from_slice(&eeprom[..n]);
    out
}

/// 方式二：**官方模板打底 + 注入实机身份**（推荐）。
///
/// 以 `template`（官方 7 680 B bin）为底，只把「必须跟着这台机器走」的字段
/// 覆盖成实机值：
///
/// | 偏移 | 长度 | 含义 |
/// |------|------|------|
/// | 0x004 | 6 | 2.4G MAC（`MT_EE_MAC_ADDR`） |
/// | 0x00A | 6 | 5G MAC（`MT_EE_MAC_ADDR2`） |
/// | 0x190 | 6 | `WIFI_CONF`，决定频段数 / 收发链 / 空间流 |
/// | 0x196 | 2 | PA/LNA 变体（`MT_EE_WIFI_PA_LNA_CONFIG`） |
///
/// 其余（0x1000 起的 byrate 功率表、RATE_DELTA）保留模板默认值 —— 那是
/// mt76 能解释的编码，硬塞厂商位图只会更糟。
pub fn mt76_from_template(template: &[u8], eeprom: &[u8]) -> Result<Vec<u8>, String> {
    if template.len() < MT76_EE_SIZE {
        return Err(format!(
            "模板只有 {} 字节，不足 {MT76_EE_SIZE}",
            template.len()
        ));
    }
    if eeprom.len() < MT_EE_WIFI_CONF + 8 {
        return Err(format!(
            "实机 EEPROM 只有 {} 字节，读不到 WIFI_CONF",
            eeprom.len()
        ));
    }
    let mut out = template[..MT76_EE_SIZE].to_vec();
    // 芯片 ID / 版本：实机与模板都是 0x7993 / 2，取实机值更稳（同芯片不同批次改过版本）。
    out[MT_EE_CHIP_ID..MT_EE_CHIP_ID + 2].copy_from_slice(&eeprom[MT_EE_CHIP_ID..MT_EE_CHIP_ID + 2]);
    out[MT_EE_VERSION..MT_EE_VERSION + 2].copy_from_slice(&eeprom[MT_EE_VERSION..MT_EE_VERSION + 2]);
    out[MT_EE_MAC_ADDR..MT_EE_MAC_ADDR + 6]
        .copy_from_slice(&eeprom[AN_EEPROM_MAC_2G4..AN_EEPROM_MAC_2G4 + 6]);
    out[MT_EE_MAC_ADDR2..MT_EE_MAC_ADDR2 + 6]
        .copy_from_slice(&eeprom[AN_EEPROM_MAC_5G..AN_EEPROM_MAC_5G + 6]);
    out[MT_EE_WIFI_CONF..MT_EE_WIFI_CONF + 8]
        .copy_from_slice(&eeprom[MT_EE_WIFI_CONF..MT_EE_WIFI_CONF + 8]);
    Ok(out)
}

/// 一行字段对照，给界面表格用。
pub struct Mt76Field {
    pub off: usize,
    pub name: &'static str,
    /// 实机 1.35 MB 里的原始字节。
    pub real: String,
    /// 7 680 B 成品里的字节。
    pub bin: String,
    /// true = 成品采用了实机值；false = 保留模板默认。
    pub injected: bool,
}

/// 生成「实机 vs 成品」的字段对照表（界面展示 + 回归断言用）。
pub fn mt76_fields(eeprom: &[u8], bin: &[u8]) -> Vec<Mt76Field> {
    const ROWS: [(usize, usize, &str); 8] = [
        (MT_EE_CHIP_ID, 2, "芯片 ID"),
        (MT_EE_VERSION, 2, "版本"),
        (MT_EE_MAC_ADDR, 6, "2.4G MAC"),
        (MT_EE_MAC_ADDR2, 6, "5G MAC"),
        (MT_EE_WIFI_CONF, 6, "WIFI_CONF"),
        (MT_EE_WIFI_CONF + 6, 2, "PA/LNA 变体"),
        (MT_EE_MAC_ADDR3, 6, "6G MAC（本机无第三频段）"),
        (MT_EE_TX0_POWER_2G, 1, "2.4G 目标功率"),
    ];
    ROWS.iter()
        .map(|&(off, len, name)| {
            let hex = |d: &[u8]| -> String {
                d.get(off..off + len)
                    .map(|s| s.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(" "))
                    .unwrap_or_else(|| "—".into())
            };
            let real = hex(eeprom);
            let out = hex(bin);
            Mt76Field {
                off,
                name,
                injected: real == out,
                real,
                bin: out,
            }
        })
        .collect()
}

/// 7 680 B 成品里「与实机不同」的字节数，按 0x1000 前缀 / 0x1000 之后分开统计。
///
/// 返回 `(前缀差异, 0x1000 起差异)`；用来在界面上直说：头部只有十来个字节对不上，
/// 功率区则是整块换了编码。
pub fn mt76_diff(eeprom: &[u8], bin: &[u8]) -> (usize, usize) {
    let n = eeprom.len().min(bin.len()).min(MT76_EE_SIZE);
    let mut head = 0;
    let mut tail = 0;
    for i in 0..n {
        if eeprom[i] != bin[i] {
            if i < MT76_HEAD_SIZE {
                head += 1;
            } else {
                tail += 1;
            }
        }
    }
    (head, tail)
}

/// 网络侧派生表：(UCI 键名, 相对基准 MAC 的偏移)。
const AN_DERIVED: [(&str, u8); 4] = [
    ("internetmac", 1),
    ("tr069mac", 2),
    ("voipmac", 3),
    ("priprotocolmac", 8),
];

/// LAN 侧：ethNmac = LAN 基址 + (N-1)。
const AN_ETH_NAMES: [&str; 5] = [
    "eth1mac", "eth2mac", "eth3mac", "eth4mac", "eth5mac",
];

/// 容器里的一个出厂文件。
#[derive(Clone)]
pub struct AnFile {
    pub name: String,
    pub data: Vec<u8>,
}

/// AN4475 的可编辑状态。
#[derive(Clone)]
pub struct An4475 {
    pub device_sn: String,
    pub gpon_sn: [u8; 8],
    pub gpon_pw: String,
    pub base_mac: [u8; 6],
    pub eth_mac: [u8; 6],
    /// Wi-Fi 2.4G MAC，写 EEPROM +0x04 与 `APMAC_1`。
    pub wifi_2g4: [u8; 6],
    /// Wi-Fi 5G MAC，写 EEPROM +0x0A 与 `APMAC_2`。
    pub wifi_5g: [u8; 6],
    /// Wi-Fi EEPROM 头里的 device id（MT7993 = 0x7993，只读展示）。
    pub eeprom_id: u16,
    pub vendor: String,
    /// BOSA 型号，来自 `olt_bosa_info` 的 `olt_bosa_id`（真机 WTD01）。
    pub bosa: String,
    pub files: Vec<AnFile>,
}

impl Default for An4475 {
    fn default() -> Self {
        Self::new()
    }
}

impl An4475 {
    pub fn new() -> Self {
        An4475 {
            device_sn: String::new(),
            gpon_sn: [0xff; 8],
            gpon_pw: String::new(),
            base_mac: [0x02, 0, 0, 0, 0, 0],
            eth_mac: [0x02, 0, 0, 0, 1, 0],
            wifi_2g4: mac_add(&[0x02, 0, 0, 0, 0, 0], AN_WIFI_OFFSET_2G4),
            wifi_5g: mac_add(&[0x02, 0, 0, 0, 0, 0], AN_WIFI_OFFSET_5G),
            eeprom_id: 0,
            vendor: "FH".to_string(),
            bosa: String::new(),
            files: Vec::new(),
        }
    }

    pub fn has_files(&self) -> bool {
        !self.files.is_empty()
    }

    /// 网络侧 4 个派生 MAC。
    pub fn derive_net(&self) -> Vec<(&'static str, [u8; 6])> {
        AN_DERIVED
            .iter()
            .map(|(name, off)| (*name, mac_add(&self.base_mac, *off)))
            .collect()
    }

    /// LAN 侧 5 个派生 MAC。
    pub fn derive_lan(&self) -> Vec<(&'static str, [u8; 6])> {
        AN_ETH_NAMES
            .iter()
            .enumerate()
            .map(|(i, name)| (*name, mac_add(&self.eth_mac, i as u8)))
            .collect()
    }

    pub fn file_names(&self) -> Vec<String> {
        let mut v: Vec<String> = self.files.iter().map(|f| f.name.clone()).collect();
        v.sort();
        v
    }

    pub fn payload_size(&self) -> usize {
        self.files.iter().map(|f| f.data.len()).sum()
    }

    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str("机型       : AN4475 (10G PON)\n");
        s.push_str(&format!("设备序列号 : {}\n", self.device_sn));
        s.push_str(&format!("GPON SN    : {}\n", hex_upper(&self.gpon_sn)));
        s.push_str(&format!("GPON 密码  : {}\n", self.gpon_pw));
        s.push_str(&format!("基准 MAC   : {}\n", mac_to_string(&self.base_mac)));
        s.push_str(&format!("LAN 基址   : {}\n", mac_to_string(&self.eth_mac)));
        s.push_str(&format!("2.4G MAC   : {}\n", mac_to_string(&self.wifi_2g4)));
        s.push_str(&format!("5G MAC     : {}\n", mac_to_string(&self.wifi_5g)));
        if self.eeprom_id != 0 {
            s.push_str(&format!(
                "Wi-Fi EEPROM: device id 0x{:04X}{}\n",
                self.eeprom_id,
                if self.eeprom_id == AN_MT7993_ID {
                    ""
                } else {
                    "（非 MT7993）"
                }
            ));
        }
        if !self.bosa.is_empty() {
            s.push_str(&format!("BOSA       : {}\n", self.bosa));
        }
        s.push_str(&format!(
            "出厂文件   : {} 个，原始共 {} 字节\n",
            self.files.len(),
            self.payload_size()
        ));
        for f in &self.files {
            s.push_str(&format!("  {:<24} {:>9}\n", f.name, f.data.len()));
        }
        s
    }

    /// 打包 2 MiB 容器。
    ///
    /// 身份字段会先回写 `factory_conf`（保持文本与头部一致），再压缩入包。
    pub fn encode_compact(&self) -> Result<Vec<u8>, String> {
        let files = apply_identity(
            &self.files,
            &self.device_sn,
            &self.gpon_sn,
            &self.gpon_pw,
            &self.base_mac,
            &self.eth_mac,
            &self.wifi_2g4,
            &self.wifi_5g,
        );
        let mut names: Vec<&String> = files.keys().collect();
        names.sort();
        if names.len() > AN_MAX_FILES {
            return Err(format!(
                "文件数 {} 超过 TOC 上限 {}",
                names.len(),
                AN_MAX_FILES
            ));
        }

        let mut img = vec![0xffu8; AN_COMPACT_SIZE];
        img[0..8].copy_from_slice(&AN_COMPACT_MAGIC);
        put_u32(&mut img, 0x08, AN_COMPACT_VERSION);
        put_u32(&mut img, 0x0c, names.len() as u32);
        put_u32(&mut img, 0x10, AN_TOC_OFF as u32);
        put_u32(&mut img, 0x14, AN_PAYLOAD_OFF as u32);

        let sn = self.device_sn.as_bytes();
        let n = sn.len().min(AN_SN_SIZE);
        img[AN_H_SN..AN_H_SN + n].copy_from_slice(&sn[..n]);
        img[AN_H_GPON_SN..AN_H_GPON_SN + AN_GPON_SN_SIZE].copy_from_slice(&self.gpon_sn);
        img[AN_H_BASE_MAC..AN_H_BASE_MAC + 6].copy_from_slice(&self.base_mac);
        img[AN_H_ETH_MAC..AN_H_ETH_MAC + 6].copy_from_slice(&self.eth_mac);
        img[AN_H_OUI..AN_H_OUI + 3].copy_from_slice(&self.base_mac[..3]);
        img[AN_H_VENDOR..AN_H_VENDOR + 2].copy_from_slice(b"FH");
        let pw = self.gpon_pw.as_bytes();
        let n = pw.len().min(AN_GPON_PW_SIZE);
        img[AN_H_GPON_PW..AN_H_GPON_PW + n].copy_from_slice(&pw[..n]);
        let mut idx = 0;
        for (_name, mac) in self.derive_net().into_iter().chain(self.derive_lan()) {
            let off = AN_H_MACS + idx * 6;
            img[off..off + 6].copy_from_slice(&mac);
            idx += 1;
        }
        img[AN_H_WIFI_2G4..AN_H_WIFI_2G4 + 6].copy_from_slice(&self.wifi_2g4);
        img[AN_H_WIFI_5G..AN_H_WIFI_5G + 6].copy_from_slice(&self.wifi_5g);

        // EEPROM 放固定偏移、原样存；其余文件从 0x2000 起顺序压缩摆放。
        let mut cursor = AN_PAYLOAD_OFF;
        let mut max_end = AN_PAYLOAD_OFF;
        for (i, name) in names.iter().enumerate() {
            let raw = &files[*name];
            let (blob, compr, at) = if name.as_str() == AN_EEPROM {
                if AN_EEPROM_FIXED_OFF + raw.len() > AN_COMPACT_SIZE {
                    return Err(format!(
                        "{name} 有 {} 字节，放不进 0x{:06X} 起的固定区",
                        raw.len(),
                        AN_EEPROM_FIXED_OFF
                    ));
                }
                max_end = max_end.max(AN_EEPROM_FIXED_OFF + raw.len());
                (raw.clone(), 0u8, AN_EEPROM_FIXED_OFF)
            } else if let Some(at) = cal_offset(name) {
                // PON 校准表：固定槽 + 原样，槽内剩余保持 0xFF。
                if raw.len() > AN_CAL_SLOT {
                    return Err(format!(
                        "{name} 有 {} 字节，超出 {} 字节的校准槽",
                        raw.len(),
                        AN_CAL_SLOT
                    ));
                }
                max_end = max_end.max(at + raw.len());
                (raw.clone(), 0u8, at)
            } else {
                let packed = zlib(raw);
                // 压缩收益不足 16 字节就原样存，省得白压一遍。
                let (blob, compr) = if packed.len() + 16 < raw.len() {
                    (packed, 1u8)
                } else {
                    (raw.clone(), 0u8)
                };
                if cursor + blob.len() > AN_EEPROM_FIXED_OFF {
                    return Err(format!(
                        "0x2000–0x{:06X} 区放不下：{name} 需要写到 0x{:06X}",
                        AN_EEPROM_FIXED_OFF,
                        cursor + blob.len()
                    ));
                }
                let at = cursor;
                cursor += blob.len();
                max_end = max_end.max(cursor);
                (blob, compr, at)
            };
            img[at..at + blob.len()].copy_from_slice(&blob);
            let ent = AN_TOC_OFF + i * AN_TOC_ENTRY;
            let nb = name.as_bytes();
            let n = nb.len().min(AN_NAME_MAX - 1);
            img[ent..ent + n].copy_from_slice(&nb[..n]);
            put_u32(&mut img, ent + AN_NAME_MAX, (at - AN_PAYLOAD_OFF) as u32);
            put_u32(&mut img, ent + AN_NAME_MAX + 4, raw.len() as u32);
            put_u32(&mut img, ent + AN_NAME_MAX + 8, blob.len() as u32);
            img[ent + AN_NAME_MAX + 12] = compr;
        }
        // 载荷长度取「最远写入位置」，固定区的 EEPROM 也算在内。
        put_u32(&mut img, 0x18, (max_end - AN_PAYLOAD_OFF) as u32);
        Ok(img)
    }
}

// ---- 读回 ---------------------------------------------------------------

pub fn is_compact(data: &[u8]) -> bool {
    (data.len() >= AN_COMPACT_SIZE && data.starts_with(&AN_COMPACT_MAGIC))
        || (data.len() >= AN_COMPACT_SIZE_V1 && data.starts_with(&AN_COMPACT_MAGIC_V1))
}

/// 旧版 1 MiB 容器的长度，仅用于识别。
pub const AN_COMPACT_SIZE_V1: usize = 0x100000;

/// 容器 magic 的可读名字（状态栏显示用）。
pub fn compact_magic_name(data: &[u8]) -> &'static str {
    if data.starts_with(&AN_COMPACT_MAGIC_V1) {
        "AN4-1MiB 容器（旧版，导出将升级为 2 MiB）"
    } else {
        "AN4-2MiB 容器"
    }
}

/// 是不是 mtd9 备份：UBI 魔数 + 卷表里有 `fhdata`。
pub fn looks_like_volume(data: &[u8]) -> bool {
    if data.len() < AN_UBI_PEB || !data.starts_with(b"UBI#") {
        return false;
    }
    data[..AN_UBI_PEB]
        .windows(6)
        .any(|w| w == b"fhdata")
}

pub fn parse_compact(data: &[u8]) -> Option<An4475> {
    if !is_compact(data) {
        return None;
    }
    let count = u32_at(data, 0x0c).unwrap_or(0) as usize;
    let mut files = Vec::new();
    for i in 0..count.min(AN_MAX_FILES) {
        let ent = AN_TOC_OFF + i * AN_TOC_ENTRY;
        let name = read_cstr(&data[ent..ent + AN_NAME_MAX]);
        if name.is_empty() {
            continue;
        }
        let off = u32_at(data, ent + AN_NAME_MAX).unwrap_or(0) as usize;
        let raw_len = u32_at(data, ent + AN_NAME_MAX + 4).unwrap_or(0) as usize;
        let stored = u32_at(data, ent + AN_NAME_MAX + 8).unwrap_or(0) as usize;
        let compr = data[ent + AN_NAME_MAX + 12];
        let start = AN_PAYLOAD_OFF + off;
        if start + stored > data.len() {
            continue;
        }
        let mut blob = data[start..start + stored].to_vec();
        if compr == 1 {
            blob = match inflate_zlib(&blob) {
                Some(b) => b,
                None => Vec::new(),
            };
        }
        blob.truncate(raw_len);
        files.push(AnFile { name, data: blob });
    }
    files.sort_by(|a, b| a.name.cmp(&b.name));

    let mut gpon_sn = [0xffu8; 8];
    gpon_sn.copy_from_slice(&data[AN_H_GPON_SN..AN_H_GPON_SN + AN_GPON_SN_SIZE]);
    let mut base_mac = [0u8; 6];
    base_mac.copy_from_slice(&data[AN_H_BASE_MAC..AN_H_BASE_MAC + 6]);
    let mut eth_mac = [0u8; 6];
    eth_mac.copy_from_slice(&data[AN_H_ETH_MAC..AN_H_ETH_MAC + 6]);
    // 头部有就信头部；早期镜像那两位是 0xFF，退回 EEPROM 里读。
    let mut wifi_2g4 = [0u8; 6];
    wifi_2g4.copy_from_slice(&data[AN_H_WIFI_2G4..AN_H_WIFI_2G4 + 6]);
    let mut wifi_5g = [0u8; 6];
    wifi_5g.copy_from_slice(&data[AN_H_WIFI_5G..AN_H_WIFI_5G + 6]);
    let eep = files.iter().find(|f| f.name == AN_EEPROM);
    if wifi_2g4 == [0xff; 6] || wifi_5g == [0xff; 6] {
        if let Some((a, b)) = eep.and_then(|f| eeprom_macs(&f.data)) {
            wifi_2g4 = a;
            wifi_5g = b;
        }
    }
    let eeprom_id = eep.map(|f| eeprom_device_id(&f.data)).unwrap_or(0);
    Some(An4475 {
        device_sn: read_ascii(&data[AN_H_SN..AN_H_SN + AN_SN_CAP]),
        gpon_sn,
        gpon_pw: read_ascii(&data[AN_H_GPON_PW..AN_H_GPON_PW + AN_GPON_PW_SIZE]),
        base_mac,
        eth_mac,
        wifi_2g4,
        wifi_5g,
        eeprom_id,
        vendor: read_ascii(&data[AN_H_VENDOR..AN_H_VENDOR + 2]),
        bosa: String::new(),
        files,
    })
}

/// 解析 mtd9 备份（UBI + UBIFS），取出身份字段与全部出厂文件。
pub fn parse_mtd9(data: &[u8]) -> Option<An4475> {
    let files = read_files(data);
    if files.is_empty() {
        return None;
    }
    let conf = files
        .get(AN_MAIN_CONF)
        .map(|v| String::from_utf8_lossy(v).to_string())
        .unwrap_or_default();
    let gpon_hex = uci_get(&conf, "GponSN").unwrap_or_default();
    let pw_hex = uci_get(&conf, "GPONPassWord").unwrap_or_default();
    let bosa_conf = files
        .get("olt_bosa_info")
        .map(|v| String::from_utf8_lossy(v).to_string())
        .unwrap_or_default();

    let mut gpon_sn = [0xffu8; 8];
    if let Some(v) = parse_hex(&gpon_hex) {
        let n = v.len().min(8);
        gpon_sn[..n].copy_from_slice(&v[..n]);
    }
    let gpon_pw = parse_hex(&pw_hex)
        .map(|v| String::from_utf8_lossy(&v).trim().to_string())
        .unwrap_or_default();

    let mut list: Vec<AnFile> = files
        .into_iter()
        .map(|(name, data)| AnFile { name, data })
        .collect();
    list.sort_by(|a, b| a.name.cmp(&b.name));

    let base_mac = uci_get(&conf, "base_mac")
        .and_then(|s| parse_mac(&s))
        .unwrap_or([0u8; 6]);
    // EEPROM 里的值才是驱动实际读到的，优先用它；缺失时退回 APMAC_1/2，
    // 再退到按基准 MAC 推算的约定值。
    let eep = list.iter().find(|f| f.name == AN_EEPROM);
    let (mut wifi_2g4, mut wifi_5g) = match eep.and_then(|f| eeprom_macs(&f.data)) {
        Some((a, b)) => (a, b),
        None => (
            uci_get(&conf, AN_APMAC_2G4)
                .and_then(|s| parse_mac(&s))
                .unwrap_or_else(|| mac_add(&base_mac, AN_WIFI_OFFSET_2G4)),
            uci_get(&conf, AN_APMAC_5G)
                .and_then(|s| parse_mac(&s))
                .unwrap_or_else(|| mac_add(&base_mac, AN_WIFI_OFFSET_5G)),
        ),
    };
    if wifi_2g4 == [0u8; 6] {
        wifi_2g4 = mac_add(&base_mac, AN_WIFI_OFFSET_2G4);
    }
    if wifi_5g == [0u8; 6] {
        wifi_5g = mac_add(&base_mac, AN_WIFI_OFFSET_5G);
    }
    let eeprom_id = eep.map(|f| eeprom_device_id(&f.data)).unwrap_or(0);

    Some(An4475 {
        device_sn: uci_get(&conf, "SerialNumber").unwrap_or_default(),
        gpon_sn,
        gpon_pw,
        base_mac,
        eth_mac: uci_get(&conf, "eth1mac")
            .and_then(|s| parse_mac(&s))
            .unwrap_or([0u8; 6]),
        wifi_2g4,
        wifi_5g,
        eeprom_id,
        vendor: uci_get(&conf, "VendorCode").unwrap_or_else(|| "FH".to_string()),
        bosa: uci_get(&bosa_conf, "olt_bosa_id").unwrap_or_default(),
        files: list,
    })
}

// ---- UBIFS --------------------------------------------------------------

/// 从 UBI 设备的每个已用 PEB 里扫 UBIFS 节点，按 sqnum 保留最新副本。
fn read_files(data: &[u8]) -> HashMap<String, Vec<u8>> {
    // name -> (sqnum, inum)
    let mut dents: HashMap<String, (u64, u64)> = HashMap::new();
    // (inum, block) -> (sqnum, size, compr, payload)
    let mut blobs: HashMap<(u32, u32), (u64, u32, u16, Vec<u8>)> = HashMap::new();

    let pebs = data.len() / AN_UBI_PEB;
    for i in 0..pebs {
        let peb = i * AN_UBI_PEB;
        if !data[peb..peb + 4].starts_with(b"UBI#") {
            continue;
        }
        let vid = peb + AN_UBI_VID_OFF;
        if !data[vid..vid + 4].starts_with(b"UBI!") {
            continue; // 空闲 PEB
        }
        let base = peb + AN_UBI_DATA_OFF;
        let end = peb + AN_UBI_PEB;
        let mut p = base;
        while p + 24 <= end {
            if !data[p..p + 4].starts_with(&AN_UBIFS_MAGIC) {
                p += 8;
                continue;
            }
            let sqnum = u64_at(data, p + 8).unwrap_or(0);
            let nlen = u32_at(data, p + 0x10).unwrap_or(0) as usize;
            if nlen < 24 || nlen > AN_UBI_PEB || p + nlen > end {
                p += 8;
                continue;
            }
            let ntype = data[p + 0x14];
            match ntype {
                2 => {
                    // dent：inum u64 @+0x28，名长 u16 @+0x32，名字 @+0x38
                    let inum = u64_at(data, p + 0x28).unwrap_or(0);
                    let nl = u16_at(data, p + 0x32).unwrap_or(0) as usize;
                    if p + 0x38 + nl <= end {
                        let name = String::from_utf8_lossy(&data[p + 0x38..p + 0x38 + nl])
                            .to_string();
                        let slot = dents.entry(name).or_insert((0, 0));
                        if slot.0 < sqnum {
                            *slot = (sqnum, inum);
                        }
                    }
                }
                1 => {
                    // data：key.ino u32 @+0x18，块号 key[4..8]，len @+0x28，compr @+0x2c
                    let inum = u32_at(data, p + 0x18).unwrap_or(0);
                    let block = u32::from_be_bytes([
                        data[p + 0x1c],
                        data[p + 0x1d],
                        data[p + 0x1e],
                        data[p + 0x1f],
                    ]);
                    let size = u32_at(data, p + 0x28).unwrap_or(0);
                    let compr = u16_at(data, p + 0x2c).unwrap_or(0);
                    let payload = data[p + 0x30..p + nlen].to_vec();
                    let slot = blobs.entry((inum, block)).or_insert((0, 0, 0, Vec::new()));
                    if slot.0 < sqnum {
                        *slot = (sqnum, size, compr, payload);
                    }
                }
                _ => {}
            }
            // UBIFS 节点按 8 字节对齐：len 不一定是对齐后的值，直接 p+=len 会
            // 落在半字节上，导致后面整片节点被跳过（实测 1345 个节点只走到 18 个）。
            p = (p + nlen + 7) & !7usize;
        }
    }

    let mut out: HashMap<String, Vec<u8>> = HashMap::new();
    for (name, (_sq, inum)) in dents.iter() {
        if name.starts_with('.') {
            continue; // UCI 临时文件
        }
        let mut keys: Vec<&(u32, u32)> = blobs.keys().filter(|k| k.0 as u64 == *inum).collect();
        keys.sort_by_key(|k| k.1);
        let mut buf: Vec<u8> = Vec::new();
        for k in keys {
            let (_sq2, size, compr, payload) = &blobs[k];
            let chunk = match compr {
                0 => payload.clone(),
                2 => inflate_raw(payload).unwrap_or_default(),
                _ => Vec::new(), // lzo / zstd：不支持就留空，不猜
            };
            let take = (*size as usize).min(chunk.len());
            buf.extend_from_slice(&chunk[..take]);
        }
        out.insert(name.clone(), buf);
    }
    out
}

// ---- UCI 文本 -----------------------------------------------------------

/// 取 `config key 'X'` 段里第一条 `option value '...'` 的值。
fn uci_get(text: &str, key: &str) -> Option<String> {
    let want = format!("config key '{key}'");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim() == want {
            let mut j = i + 1;
            while j < lines.len() {
                let s = lines[j].trim();
                if s.starts_with("config ") {
                    break;
                }
                if let Some(rest) = s.strip_prefix("option value '") {
                    if let Some(v) = rest.strip_suffix('\'') {
                        return Some(v.to_string());
                    }
                }
                j += 1;
            }
            return None;
        }
        i += 1;
    }
    None
}

/// 改 `config key 'X'` 段里的 `option value`，其余字节原样保留。
fn uci_set(text: &str, key: &str, value: &str) -> String {
    let want = format!("config key '{key}'");
    let lines: Vec<&str> = text.split('\n').collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    let mut i = 0;
    while i < lines.len() {
        let line = lines[i];
        out.push(line.to_string());
        i += 1;
        if line.trim() != want {
            continue;
        }
        while i < lines.len() && !lines[i].trim_start().starts_with("config ") {
            let cur = lines[i];
            let s = cur.trim();
            if s.starts_with("option value '") && s.ends_with('\'') {
                let indent = &cur[..cur.len() - cur.trim_start().len()];
                out.push(format!("{indent}option value '{value}'"));
            } else {
                out.push(cur.to_string());
            }
            i += 1;
        }
    }
    out.join("\n")
}

/// 把身份字段回写 `factory_conf`，保证容器头部与文本内容一致。
fn apply_identity(
    files: &[AnFile],
    sn: &str,
    gpon_sn: &[u8; 8],
    gpon_pw: &str,
    base_mac: &[u8; 6],
    eth_mac: &[u8; 6],
    wifi_2g4: &[u8; 6],
    wifi_5g: &[u8; 6],
) -> HashMap<String, Vec<u8>> {
    let mut map: HashMap<String, Vec<u8>> =
        files.iter().map(|f| (f.name.clone(), f.data.clone())).collect();
    let conf = match map.get(AN_MAIN_CONF) {
        Some(v) => String::from_utf8_lossy(v).to_string(),
        None => return map,
    };
    let mut pairs: Vec<(&str, String)> = vec![
        ("SerialNumber", sn.to_string()),
        ("VendorCode", "FH".to_string()),
        ("GponSN", hex_upper(gpon_sn)),
        ("GPONPassWord", hex_upper(gpon_pw.as_bytes())),
        ("ManufacturerOUI", hex_upper(&base_mac[..3])),
        ("base_mac", mac_to_string(base_mac)),
        ("brmac", mac_to_string(base_mac)),
        ("ponmac", mac_to_string(base_mac)),
    ];
    for (name, off) in AN_DERIVED {
        pairs.push((name, mac_to_string(&mac_add(base_mac, off))));
    }
    for (i, name) in AN_ETH_NAMES.iter().enumerate() {
        pairs.push((name, mac_to_string(&mac_add(eth_mac, i as u8))));
    }
    pairs.push((AN_APMAC_2G4, mac_to_string(wifi_2g4)));
    pairs.push((AN_APMAC_5G, mac_to_string(wifi_5g)));
    let mut text = conf;
    for (k, v) in pairs {
        text = uci_set(&text, k, &v);
    }
    map.insert(AN_MAIN_CONF.to_string(), text.into_bytes());

    // Wi-Fi EEPROM 里同样的两位 MAC 也跟着走，否则驱动可能读到旧值。
    if let Some(eep) = map.get(AN_EEPROM).cloned() {
        if let Some(fixed) = eeprom_set_macs(&eep, wifi_2g4, wifi_5g) {
            map.insert(AN_EEPROM.to_string(), fixed);
        }
    }
    map
}

// ---- Wi-Fi EEPROM -------------------------------------------------------

/// 读 EEPROM 里的 (2.4G, 5G) MAC；太短或全 0/全 FF 时返回 None。
pub fn eeprom_macs(eeprom: &[u8]) -> Option<([u8; 6], [u8; 6])> {
    if eeprom.len() < AN_EEPROM_MAC_5G + 6 {
        return None;
    }
    let mut a = [0u8; 6];
    let mut b = [0u8; 6];
    a.copy_from_slice(&eeprom[AN_EEPROM_MAC_2G4..AN_EEPROM_MAC_2G4 + 6]);
    b.copy_from_slice(&eeprom[AN_EEPROM_MAC_5G..AN_EEPROM_MAC_5G + 6]);
    if a == [0u8; 6] || a == [0xff; 6] {
        return None;
    }
    Some((a, b))
}

/// 写回 EEPROM 的两位 MAC，其余字节（含 1.3 MiB 校准表）原样保留。
pub fn eeprom_set_macs(
    eeprom: &[u8],
    mac_2g4: &[u8; 6],
    mac_5g: &[u8; 6],
) -> Option<Vec<u8>> {
    if eeprom.len() < AN_EEPROM_MAC_5G + 6 {
        return None;
    }
    let mut buf = eeprom.to_vec();
    buf[AN_EEPROM_MAC_2G4..AN_EEPROM_MAC_2G4 + 6].copy_from_slice(mac_2g4);
    buf[AN_EEPROM_MAC_5G..AN_EEPROM_MAC_5G + 6].copy_from_slice(mac_5g);
    Some(buf)
}

/// EEPROM 头 2 字节小端 device id。
pub fn eeprom_device_id(eeprom: &[u8]) -> u16 {
    u16_at(eeprom, AN_EEPROM_DEVICE_ID).unwrap_or(0)
}

// ---- 小工具 -------------------------------------------------------------

/// 末字节加法（带进位），用于派生 +1/+2/+3/+8 与 ethN。
pub fn mac_add(base: &[u8; 6], n: u8) -> [u8; 6] {
    let mut v = [0u8; 6];
    v.copy_from_slice(base);
    let mut carry = n as u16;
    for i in (0..6).rev() {
        let s = v[i] as u16 + carry;
        v[i] = s as u8;
        carry = s >> 8;
    }
    v
}

pub fn mac_to_string(m: &[u8; 6]) -> String {
    format!(
        "{:02X}:{:02X}:{:02X}:{:02X}:{:02X}:{:02X}",
        m[0], m[1], m[2], m[3], m[4], m[5]
    )
}

pub fn parse_mac(s: &str) -> Option<[u8; 6]> {
    let hex: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if hex.len() != 12 {
        return None;
    }
    let mut m = [0u8; 6];
    for i in 0..6 {
        m[i] = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(m)
}

fn hex_upper(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02X}")).collect()
}

fn parse_hex(s: &str) -> Option<Vec<u8>> {
    let t: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if t.len() % 2 != 0 {
        return None;
    }
    (0..t.len() / 2)
        .map(|i| u8::from_str_radix(&t[i * 2..i * 2 + 2], 16).ok())
        .collect()
}

fn read_ascii(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0 || c == 0xff).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

/// 读名字：TOC 用 `0xFF` 填充（不是 0x00），两种终止符都要认。
fn read_cstr(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0 || c == 0xff).unwrap_or(b.len());
    String::from_utf8_lossy(&b[..end]).trim().to_string()
}

fn u32_at(b: &[u8], off: usize) -> Option<u32> {
    b.get(off..off + 4)
        .map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u16_at(b: &[u8], off: usize) -> Option<u16> {
    b.get(off..off + 2)
        .map(|s| u16::from_le_bytes([s[0], s[1]]))
}

fn u64_at(b: &[u8], off: usize) -> Option<u64> {
    b.get(off..off + 8).map(|s| {
        let mut a = [0u8; 8];
        a.copy_from_slice(s);
        u64::from_le_bytes(a)
    })
}

fn put_u32(b: &mut [u8], off: usize, v: u32) {
    b[off..off + 4].copy_from_slice(&v.to_le_bytes());
}

fn zlib(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
    if e.write_all(data).is_err() {
        return Vec::new();
    }
    e.finish().unwrap_or_default()
}

fn inflate_zlib(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(data).read_to_end(&mut out).ok()?;
    Some(out)
}

/// UBIFS 里是**裸 deflate**（没有 0x78 头），必须用 raw 解码器。
fn inflate_raw(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(data)
        .read_to_end(&mut out)
        .ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mac_rules() {
        let base = [0xd4, 0x9f, 0x29, 0x04, 0xe7, 0x50];
        let eth = [0xd4, 0xa0, 0x29, 0x04, 0xe7, 0x71];
        let mut a = An4475::new();
        a.base_mac = base;
        a.eth_mac = eth;
        assert_eq!(mac_to_string(&mac_add(&base, 8)), "D4:9F:29:04:E7:58");
        assert_eq!(mac_to_string(&mac_add(&eth, 4)), "D4:A0:29:04:E7:75");
        assert_eq!(a.derive_net().len(), 4);
        assert_eq!(a.derive_lan().len(), 5);
    }

    #[test]
    fn mac_carry() {
        let base = [0x00, 0x11, 0x22, 0x33, 0x44, 0xff];
        assert_eq!(mac_to_string(&mac_add(&base, 1)), "00:11:22:33:45:00");
    }

    #[test]
    fn uci_roundtrip() {
        let t = "config key 'A'\n\toption value '1'\n\nconfig key 'B'\n\toption value '2'\n";
        assert_eq!(uci_get(t, "B").unwrap(), "2");
        let t2 = uci_set(t, "B", "9");
        assert_eq!(uci_get(&t2, "B").unwrap(), "9");
        assert_eq!(uci_get(&t2, "A").unwrap(), "1");
    }

    #[test]
    fn mt76_template_is_sane() {
        // 官方 bin 必须正好 7680 B，芯片 ID 是 0x7993，PA/LNA 变体为内建。
        assert_eq!(MT76_DEFAULT_IPA.len(), MT76_EE_SIZE);
        assert_eq!(MT76_DEFAULT_EPA.len(), MT76_EE_SIZE);
        assert_eq!(
            u16::from_le_bytes([MT76_DEFAULT_IPA[0], MT76_DEFAULT_IPA[1]]),
            AN_MT7993_ID
        );
        assert_eq!(mt76_pa_lna(MT76_DEFAULT_IPA), Some(0));
        assert_eq!(mt76_pa_lna(MT76_DEFAULT_EPA), Some(3));
    }

    #[test]
    fn mt76_carve_and_inject() {
        // 造一份「实机」EEPROM：芯片 ID + 版本 + 两个 MAC + WIFI_CONF。
        let mut ee = vec![0u8; 0x100];
        ee[0..2].copy_from_slice(&AN_MT7993_ID.to_le_bytes());
        ee[2..4].copy_from_slice(&2u16.to_le_bytes());
        ee[4..10].copy_from_slice(&[0xd4, 0x9f, 0x29, 0x04, 0xe7, 0x56]);
        ee[0x0a..0x10].copy_from_slice(&[0xd4, 0x9f, 0x29, 0x04, 0xe7, 0x57]);
        ee.extend_from_slice(&MT76_DEFAULT_IPA[0x100..MT76_EE_SIZE]); // 补到 7680
        ee[MT_EE_WIFI_CONF..MT_EE_WIFI_CONF + 8]
            .copy_from_slice(&MT76_DEFAULT_IPA[MT_EE_WIFI_CONF..MT_EE_WIFI_CONF + 8]);

        let carved = mt76_carve(&ee);
        assert_eq!(carved.len(), MT76_EE_SIZE);
        assert_eq!(&carved[4..10], &[0xd4, 0x9f, 0x29, 0x04, 0xe7, 0x56]);

        let bin = mt76_from_template(MT76_DEFAULT_IPA, &ee).unwrap();
        assert_eq!(bin.len(), MT76_EE_SIZE);
        assert_eq!(&bin[4..10], &[0xd4, 0x9f, 0x29, 0x04, 0xe7, 0x56]);
        assert_eq!(&bin[0x0a..0x10], &[0xd4, 0x9f, 0x29, 0x04, 0xe7, 0x57]);
        // 功率表必须还是模板原样（没被实机的位图覆盖）。
        assert_eq!(
            &bin[MT_EE_TX0_POWER_2G..MT_EE_TX0_POWER_2G + 16],
            &MT76_DEFAULT_IPA[MT_EE_TX0_POWER_2G..MT_EE_TX0_POWER_2G + 16]
        );
        assert!(mt76_diff(&ee, &bin).0 <= 10);
    }
}
