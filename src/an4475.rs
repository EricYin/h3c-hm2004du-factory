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
//! 0x1AB000      mt76 直读槽：7 680 B，由 1.35 MB 模板打底派生，给 OpenWrt 用
//! 0x1ACE00      直读区结束
//! ---------- 其余至 0x200000 全 0xFF（332 KiB）----------
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

/// 头部 `0xA2` 起 12 字节：GPON SN 的 **12 字符 ASCII 视图**（`FHTTC26A69FF`）。
///
/// 为什么要多存一份：PonWrt 的 identity 机制只有 `pon-ascii12` / `mac-colon17` /
/// `mac-binary6` 三种编码，`0x38` 那种「4 字节 ASCII 厂商码 + 4 字节二进制序列号」
/// 它表达不了，而 C 侧不改。于是把同一份 SN 在这块空闲区按 `pon-ascii12` 能认的
/// 形状再存一遍：PonWrt 只读写这 12 字节，`0x38` 与 `factory_conf` 的 `GponSN`
/// 由本工具在打包/打开时双向同步。
///
/// 实测 `0xA2`–`0xFFF` 在真机上全为 `0xFF`，不与任何既有字段重叠。
pub const AN_H_GPON_SN_VIEW: usize = 0xa2;
pub const AN_GPON_SN_VIEW_SIZE: usize = 12;

/// 主配置文件名。身份字段全在这一个文件的 UCI 文本里。
pub const AN_MAIN_CONF: &str = "factory_conf";
/// Wi-Fi 校准文件名（1.32 MiB，容器里必须压缩存放）。
pub const AN_EEPROM: &str = "MT7993_EEPROM.bin";
/// 光组件（BOSA）信息表：`olt_bosa_id` 等三项 UCI 文本，唯一的型号来源。
pub const AN_BOSA_INFO: &str = "olt_bosa_info";
/// BOB 校准表的文件头魔数：`fiberhomebdbob` + 16 字节版本戳（如 `202404291559VB`）。
pub const AN_BOB_MAGIC: &[u8; 14] = b"fiberhomebdbob";
/// 版本戳在 BOB 头里的偏移（魔数 14 B + 2 B 对齐填充）。
pub const AN_BOB_VERSION_OFF: usize = 0x10;

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

/// `AN_CAL_TABLES` 的前 6 个是 **BDBOB 格式的 PON 校准表**；其余 5 个是 APD 偏压、
/// BOSA 信息、RSSI 与光口 TX 校准（UCI 文本或裸二进制），不参与 APONCAL 转换。
///
/// 顺序：5 张上行（10ge_asym / 10ge_sym / xg / xgs / table）+ 1 张下行（olt_table）。
pub const AN_BOB_TABLES: usize = 6;

/// 上行 5 张 BDBOB 表 = `AN_CAL_TABLES` 的前 5 个。
///
/// `CompactOptions::pon_aponcal` 只动这 5 张。第 6 张 `olt_table` 是下行、
/// chip id 0x27（UX3326S1），APONCAL 枚举里没有对应值 —— 硬转只会让 PonWrt
/// 走错 apply 分支，所以无论开关怎么设，槽里都保持原厂 BDBOB 原样。
pub const AN_UPLINK_BOB: [&str; 5] = [
    "10ge_asym_table",
    "10ge_sym_table",
    "xg_table",
    "xgs_table",
    "table",
];

/// 这张表是不是「打包时要转 APONCAL」的上行表。
pub fn is_uplink_bob(name: &str) -> bool {
    AN_UPLINK_BOB.contains(&name)
}

/// 打包 2 MiB 容器时的 PON 区策略。默认 = 出厂原样。
#[derive(Clone, Copy, Debug, Default)]
pub struct CompactOptions {
    /// 上行 5 张 BDBOB 表写槽前先按 fiberhome-factory 流程转成 APONCAL。
    ///
    /// 打开后产出的镜像 PON 区是 PonWrt 直接能读的格式；原厂 `mt_wifi`
    /// 只认 BDBOB，所以这种镜像**不适合回刷原厂固件**。
    pub pon_aponcal: bool,
}

/// 单张上行表打包时的处理结果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PonInlineAction {
    /// 按 fiberhome 流程转成 APONCAL 后写入槽。
    Converted(PonCalStatus),
    /// 槽里本来就是 APONCAL，原样保留（重复打包时）。
    AlreadyApconal,
    /// 流程没走通，保留原表 —— 半张 APONCAL 比原表更危险。
    Kept(PonCalStatus),
}

impl PonInlineAction {
    pub fn ok(self) -> bool {
        matches!(self, Self::Converted(_) | Self::AlreadyApconal)
    }

    pub fn label(self) -> String {
        match self {
            Self::Converted(st) => format!("APONCAL · {}", st.label()),
            Self::AlreadyApconal => "已是 APONCAL".to_string(),
            Self::Kept(st) => format!("保留原表 · {}", st.label()),
        }
    }
}

/// 一张上行表的打包明细。
pub struct PonInline {
    pub table: String,
    pub offset: usize,
    pub action: PonInlineAction,
}

/// 单张表的内联处理：能转就转，转不了原样退回。
fn inline_aponcal(raw: &[u8]) -> (Vec<u8>, PonInlineAction) {
    if raw.starts_with(PON_CAL_MAGIC) {
        return (raw.to_vec(), PonInlineAction::AlreadyApconal);
    }
    let (status, out) = bob_to_aponcal(raw);
    match out {
        Some(d) if status.exportable() => (d, PonInlineAction::Converted(status)),
        _ => (raw.to_vec(), PonInlineAction::Kept(status)),
    }
}

/// 校准表名 → 直读区偏移；不是固定槽里的表就返回 `None`（走压缩区）。
pub fn cal_offset(name: &str) -> Option<usize> {    AN_CAL_TABLES
        .iter()
        .position(|n| *n == name)
        .map(|i| AN_CAL_FIXED_OFF + i * AN_CAL_SLOT)
}

/// **mt76 直读槽**起点：紧接 11 张 PON 校准表之后（`0x1AB000`）。
///
/// 容器里那 1.35 MB 是原厂 `mt_wifi` 的格式，mt76 只认其中前 7 680 B、且
/// 功率段编码不同。为了「一块 factory 同时喂饱两套固件」，打包时在这里再放
/// 一份用官方模板打底派生出来的 7 680 B：OpenWrt 的 DTS 直接把 nvmem 指到
/// `0x1AB000`，长度 `0x1E00` 即可，不用再跑一次转换工具。
///
/// 尾部到 `0x200000` 还剩 340 KiB，放得下且不影响任何既有偏移。
pub const AN_MT76_FIXED_OFF: usize = AN_CAL_FIXED_OFF + AN_CAL_TABLES.len() * AN_CAL_SLOT;
/// mt76 EEPROM 的固定长度（与 `MT76_EE_SIZE` 同值，单独命名便于出现在布局表里）。
pub const AN_MT76_SIZE: usize = 0x1E00;

/// 从容器尾部取出随包存放的 mt76 EEPROM；没有（旧镜像）时返回 `None`。
pub fn compact_mt76(data: &[u8]) -> Option<&[u8]> {
    let s = data.get(AN_MT76_FIXED_OFF..AN_MT76_FIXED_OFF + AN_MT76_SIZE)?;
    // 尾部原本是 0xFF；校验 chip id + 至少有一个非 0xFF 字节，避免把空白当数据。
    if s.len() == AN_MT76_SIZE
        && s.starts_with(&AN_MT7990_ID.to_le_bytes())
        && s.iter().any(|b| *b != 0xff)
    {
        Some(s)
    } else {
        None
    }
}

// Wi-Fi EEPROM 头很短：device id + 版本 + 两个 MAC，0x10..0x190 全 0，
// 0x190 起才是射频校准表。出厂值仍是芯片厂默认（2.4G/5G 分别
// 00:0C:43:26:60:10 / :11，00:0C:43 是 MediaTek 的 OUI），产线没写这两位。
pub const AN_EEPROM_DEVICE_ID: usize = 0x00; // u16 LE，MT7990AN = 0x7993
pub const AN_EEPROM_VERSION: usize = 0x02; // u16 LE，实测 2
pub const AN_EEPROM_MAC_2G4: usize = 0x04;
pub const AN_EEPROM_MAC_5G: usize = 0x0a;

/// `factory_conf` 里对应的两个键。EEPROM 与应用层各存一份，必须一起改。
pub const AN_APMAC_2G4: &str = "APMAC_1";
pub const AN_APMAC_5G: &str = "APMAC_2";

/// 原厂 Wi-Fi MAC 派生规则，出自 rootfs 里的两个 shell 脚本：
///
/// * `/fhrom/fhshell/fac_start_wifi.sh`（产测模式，生成 `/var/wlan/apcfg`）
/// * `/fhrom/fhshell/generate_wifi_mac.sh`（生成 `mt7990.1.dualband.dat`）
///
/// 两处的算法逐字相同：
///
/// ```sh
/// mac2 = brmac[3:5] + 1      # 第 2 字节 +1
/// ra0_mac  = mac1:mac2:mac3:mac4:mac5:mac6            # 2.4G，末字节不变
/// rai0_mac = mac1:mac2:mac3:mac4:mac5:(mac6 + 8)      # 5G，末字节 +8
/// ```
///
/// 也就是说 **EEPROM 里的 `+0x04` / `+0x0A` 原厂从不写**（实机仍是 MediaTek
/// 默认的 `00:0C:43:26:60:10/11`），Wi-Fi MAC 是在用户态由 `brmac` 现算、
/// 写进 profile 的 `MacAddress` / `MacAddress1`。本工具沿用同一算法。
pub fn an_wifi_mac(base: &[u8; 6], band_5g: bool) -> [u8; 6] {
    let mut v = *base;
    v[1] = v[1].wrapping_add(1);
    if band_5g {
        v[5] = v[5].wrapping_add(8);
    }
    v
}

/// 本机 Wi-Fi 方案的数字基带是 **MT7990AN**（配套射频前端 MT7976CN，双频
/// 2.4G/5G、无 6G），device id 即 `0x7993`。
///
/// 开源 mt76 的 `mt7996` 驱动把 `0x7993` 登记为 `MT7990_DEVICE_ID`，
/// `mt7996_check_eeprom()` 认它 —— 所以 Linux 下按 MT7990 跑，默认 bin 名
/// 是 `mt7990_eeprom_2i5i.bin`，不是 `mt7996_*`。
///
/// 厂商文件名 `MT7993_EEPROM.bin` 沿用出厂的命名，与实际芯片型号无关。
pub const AN_MT7990_ID: u16 = 0x7993;

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

/// 从文件表里取 BOSA 型号（`olt_bosa_info` → `olt_bosa_id`）。
///
/// 这是整份校准包里**唯一**的光组件型号来源：`10ge_*` / `xg*` / `table` 这些
/// BOB 表只有魔数和版本戳，没有器件型号。两种入口（mtd9 / 2 MiB 容器）都走
/// 这里，免得从容器读入时 BOSA 一栏莫名消失。
pub fn bosa_from_files(files: &[AnFile]) -> String {
    files
        .iter()
        .find(|f| f.name == AN_BOSA_INFO)
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .and_then(|t| uci_get(&t, "olt_bosa_id"))
        .unwrap_or_default()
}

/// 从 `factory_conf` 里取一个 UCI 键（`pon_mode` / `uplink_mode` 等）。
fn conf_value(files: &[AnFile], key: &str) -> String {
    files
        .iter()
        .find(|f| f.name == AN_MAIN_CONF)
        .map(|f| String::from_utf8_lossy(&f.data).to_string())
        .and_then(|t| uci_get(&t, key))
        .unwrap_or_default()
}

/// BOB 校准表头里的版本戳（`fiberhomebdbob` 后 16 字节，如 `202404291559VB`）。
///
/// 不是 BOB 表（或长度不够）时返回 `None`，这样界面上不会把配置文件
/// 当成校准表显示版本。
pub fn bob_version(data: &[u8]) -> Option<String> {
    if data.len() < AN_BOB_VERSION_OFF + 16 || !data.starts_with(AN_BOB_MAGIC) {
        return None;
    }
    let s: String = data[AN_BOB_VERSION_OFF..AN_BOB_VERSION_OFF + 16]
        .iter()
        .take_while(|b| **b != 0 && b.is_ascii_graphic())
        .map(|b| *b as char)
        .collect();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

/// BOB 表头里的「BOSA 驱动 chip id」字节。
///
/// 0x30 这一字节就是光组件驱动芯片的 ID，实机可直接对照 `/etc/rc.d/rcS`：
/// `i2c_debug getchipid_down` 读到 `0x27` 就 `cp /etc/table.ux3326s /fhdata/olt_table`，
/// 而 mtd9 里的 `olt_table` 第 0x30 字节正是 0x27。上行 5 张表是 0x2E，
/// 对应出厂默认表 `/etc/table.ux3361`。
pub const AN_BOSA_CHIP_ID_OFF: usize = 0x30;

/// 取 BOB 表里的驱动 chip id；非 BOB 表返回 `None`。
pub fn bosa_chip_id(data: &[u8]) -> Option<u8> {
    if data.len() <= AN_BOSA_CHIP_ID_OFF || !data.starts_with(AN_BOB_MAGIC) {
        return None;
    }
    Some(data[AN_BOSA_CHIP_ID_OFF])
}

/// chip id → 驱动型号。取值来自 `/etc/table.*` 的默认表名与 `rcS` 的分支。
pub fn bosa_chip_name(id: u8) -> &'static str {
    match id {
        0x2E => "UX3363（与 UX3361 同驱动）",
        0x27 => "UX3326S1",
        0x26 => "UX3326",
        0x58 => "UX3368",
        0x99 => "25L99",
        0x81 => "GN25L81",
        _ => "未知型号",
    }
}

/// BOB 表载荷里的芯片型号明文：上行表在 `0x95`（`ux3363`），
/// 下行表在 `0x1C4`（`UX3326S1`）。只扫 0x80–0x1FF，避开表头的版本戳。
pub fn bosa_tag(data: &[u8]) -> Option<String> {
    let end = data.len().min(0x200);
    let mut i = 0x80;
    while i < end {
        if data[i].is_ascii_alphabetic() {
            let start = i;
            let mut j = i + 1;
            while j < end && (data[j].is_ascii_alphanumeric() || data[j] == b'-') {
                j += 1;
            }
            if j - start >= 5 {
                return Some(String::from_utf8_lossy(&data[start..j]).to_string());
            }
            i = j;
        } else {
            i += 1;
        }
    }
    None
}

/// 12 个出厂 MAC（基准 + 4 个网络侧派生 + 5 个 LAN + 2 个 Wi-Fi）的重复检查。
///
/// 返回 `(MAC, 用到它的字段列表)`，只列出现 ≥2 次的地址。全 0 / 全 FF 视为
/// 「没填」，不算冲突 —— 那种情况该由输入校验挡，不是地址规划问题。
pub fn mac_conflicts_of(
    base: &[u8; 6],
    eth: &[u8; 6],
    wifi_2g4: &[u8; 6],
    wifi_5g: &[u8; 6],
) -> Vec<(String, Vec<String>)> {
    // 未填的段（全 0 / 全 FF）整段跳过：基址没填时派生出来的 :01/:02 会互撞，
    // 那是「还没输入」，不是地址规划问题，报出来只会盖住真正的冲突。
    let filled = |m: &[u8; 6]| *m != [0u8; 6] && *m != [0xff; 6];

    let mut items: Vec<(String, String)> = Vec::new();
    if filled(base) {
        items.push((mac_to_string(base), "基准 MAC".to_string()));
        for (name, off) in AN_DERIVED {
            items.push((mac_to_string(&mac_add(base, off)), name.to_string()));
        }
    }
    if filled(eth) {
        for (i, name) in AN_ETH_NAMES.iter().enumerate() {
            items.push((mac_to_string(&mac_add(eth, i as u8)), (*name).to_string()));
        }
    }
    if filled(wifi_2g4) {
        items.push((mac_to_string(wifi_2g4), "Wi-Fi 2.4G".to_string()));
    }
    if filled(wifi_5g) {
        items.push((mac_to_string(wifi_5g), "Wi-Fi 5G".to_string()));
    }

    let mut out: Vec<(String, Vec<String>)> = Vec::new();
    for (mac, user) in items {
        match out.iter_mut().find(|(m, _)| *m == mac) {
            Some((_, users)) => users.push(user),
            None => out.push((mac, vec![user])),
        }
    }
    out.retain(|(_, users)| users.len() > 1);
    out
}

// ===== PON 校准表 → APONCAL（与上游 fiberhome-factory 同语义）=====
//
// 上游只处理**一张**表：读 `fhdata/table`，`validate_bdbob()` 校验长度/魔数/
// 两处 CRC，`convert_bdbob()` 按头 0x30 的 chip id 映射成 GN28L95 或 UX3363，
// `encode_pon_calibration()` 重封成 2048 B 的 APONCAL 容器。
//
// AN4475 有 6 个 PON 槽，所以这里把同一套流程做成「逐槽跑一遍」。同时把上游
// 隐含假设、但本机并不成立的两件事显式化：
//
//   1. **CRC 种子有两套**。上游固定用 `crc32_register(d, 0) ^ !0`（u-boot 口径），
//      而本机 `10ge_asym_table` / `xg_table` / `olt_table` 是标准 zlib 口径
//      （`crc32_register(d, !0) ^ !0`）。严格照抄会判一半的表 CRC 失败，所以
//      先按上游种子验，不过再按标准种子验，并把实际命中的那套报出来。
//   2. **chip id 0x27 不在 APONCAL 枚举里**。枚举只有 1=GN28L95、2=UX3363，
//      PonWrt 的 `bosa_load_calibration()` 也只认这两个值。下行 `olt_table`
//      因此判 `ChipUnsupported`，不臆造一个值去糊弄驱动。

/// APONCAL 容器魔数（8 B，含结尾 NUL）。
pub const PON_CAL_MAGIC: &[u8; 8] = b"APONCAL\0";
/// APONCAL 版本，固定 1。
pub const PON_CAL_VERSION: u16 = 1;
/// APONCAL 头长度，固定 0x80；payload 从 0x80 起。
pub const PON_CAL_HEADER_SIZE: usize = 0x80;
/// `format` 字段：1 = 原生 payload（原样搬运，不重排）。
pub const PON_CAL_FORMAT_NATIVE: u32 = 1;
/// APONCAL 容器总长，与 BDBOB 一致（2048 B）。
pub const PON_CAL_SIZE: usize = 0x800;

/// BDBOB 头里四个长度/CRC 字段的偏移。
const BOB_DATA_SIZE_OFF: usize = 0x70;
const BOB_DATA_CRC_OFF: usize = 0x74;
const BOB_HEADER_SIZE_OFF: usize = 0x78;
const BOB_HEADER_CRC_OFF: usize = 0x7c;

/// 反射 CRC-32 的寄存器值（不补初值、不取反），与上游 `crc32_register` 同实现。
pub fn crc32_register(data: &[u8], mut crc: u32) -> u32 {
    for byte in data {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0u32.wrapping_sub(crc & 1));
        }
    }
    crc
}

/// 原厂 BDBOB 表用的 CRC：初值 0、末尾取反（等同 u-boot `crc32(0, …)`）。
pub fn fiberhome_crc32(data: &[u8]) -> u32 {
    crc32_register(data, 0) ^ u32::MAX
}

/// APONCAL 容器用的 CRC：初值全 1、末尾取反（等同标准 zlib `crc32()`）。
pub fn calibration_crc32(data: &[u8]) -> u32 {
    crc32_register(data, u32::MAX) ^ u32::MAX
}

/// APONCAL 的 `chip` 枚举（u32）。只有这两个值，PonWrt 驱动按它选 apply 分支。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PonCalChip {
    Gn28l95,
    Ux3363,
}

impl PonCalChip {
    pub fn id(self) -> u32 {
        match self {
            Self::Gn28l95 => 1,
            Self::Ux3363 => 2,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Gn28l95 => "GN28L95",
            Self::Ux3363 => "UX3363",
        }
    }

    /// BDBOB 头 0x30 的 chip id → APONCAL 枚举。上游把 0 与 0x28 都当 GN28L95。
    fn from_bob_id(id: u8) -> Option<Self> {
        match id {
            0 | 0x28 => Some(Self::Gn28l95),
            0x2e => Some(Self::Ux3363),
            _ => None,
        }
    }
}

/// BDBOB 两处 CRC 用的是哪套种子。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BobCrcSeed {
    /// 原厂口径（`fiberhome_crc32`）。上游只认这一套。
    FiberHome,
    /// 标准 zlib 口径。本机 `10ge_asym_table` / `xg_table` / `olt_table` 用这套。
    Standard,
}

impl BobCrcSeed {
    pub fn name(self) -> &'static str {
        match self {
            Self::FiberHome => "原厂口径",
            Self::Standard => "标准 zlib 口径",
        }
    }

    fn crc(self, data: &[u8]) -> u32 {
        match self {
            Self::FiberHome => fiberhome_crc32(data),
            Self::Standard => calibration_crc32(data),
        }
    }
}

/// 逐槽处理的结果状态。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PonCalStatus {
    /// 完全按上游流程通过：长度、魔数、两处 CRC 都用原厂种子。
    OkStrict,
    /// 原厂种子不过，换标准 zlib 种子通过；payload 本身完好。
    OkAltSeed(BobCrcSeed),
    /// chip id 是 0x27（UX3326S1），APONCAL 没有对应枚举值。
    ChipUnsupported,
    /// 不是 BDBOB：长度不是 2048 或魔数不对。
    NotBob,
    /// 头里的长度字段自相矛盾。
    BadFields,
    /// 两套 CRC 种子都对不上。
    CrcFail,
    /// 槽里已经是 APONCAL —— 镜像由 `--pon-aponcal` 打包，无需再转。
    AlreadyApconal,
}

impl PonCalStatus {
    /// 能否产出可交给 PonWrt 的 2048 B 文件。
    pub fn exportable(self) -> bool {
        matches!(
            self,
            Self::OkStrict | Self::OkAltSeed(_) | Self::AlreadyApconal
        )
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::OkStrict => "PASS",
            Self::OkAltSeed(_) => "PASS·换种子",
            Self::ChipUnsupported => "chip 不在枚举",
            Self::NotBob => "非 BDBOB",
            Self::BadFields => "长度字段异常",
            Self::CrcFail => "CRC 不匹配",
            Self::AlreadyApconal => "已是 APONCAL",
        }
    }
}

/// 一个 PON 槽跑完上游流程后的结果。
pub struct PonCalRow {
    pub table: String,
    pub offset: usize,
    /// BDBOB 头 0x30 的原始 chip id。
    pub chip_id: u8,
    /// 映射到的 APONCAL 枚举；`None` 表示不在枚举内。
    pub chip: Option<PonCalChip>,
    pub version: Option<String>,
    pub status: PonCalStatus,
    /// 转换出来的 2048 B（仅 `status.exportable()` 时非空）。
    pub data: Option<Vec<u8>>,
}

/// 把一张 2048 B 的 BDBOB 校准表按上游流程封成 APONCAL。
pub fn bob_to_aponcal(data: &[u8]) -> (PonCalStatus, Option<Vec<u8>>) {
    if data.len() != PON_CAL_SIZE || !data.starts_with(AN_BOB_MAGIC) {
        return (PonCalStatus::NotBob, None);
    }
    let (Some(dsize), Some(dcrc), Some(hsize), Some(hcrc)) = (
        u32_at(data, BOB_DATA_SIZE_OFF),
        u32_at(data, BOB_DATA_CRC_OFF),
        u32_at(data, BOB_HEADER_SIZE_OFF),
        u32_at(data, BOB_HEADER_CRC_OFF),
    ) else {
        return (PonCalStatus::BadFields, None);
    };
    let (dsize, hsize) = (dsize as usize, hsize as usize);
    if hsize != PON_CAL_HEADER_SIZE || hsize + dsize != data.len() {
        return (PonCalStatus::BadFields, None);
    }

    // 先按上游口径验，不过再按标准 zlib 验 —— 两套种子在本机都真实存在。
    let mut seed = None;
    for candidate in [BobCrcSeed::FiberHome, BobCrcSeed::Standard] {
        let hdr_ok = candidate.crc(&data[..BOB_HEADER_CRC_OFF]) == hcrc;
        let pay_ok = candidate.crc(&data[hsize..hsize + dsize]) == dcrc;
        if hdr_ok && pay_ok {
            seed = Some(candidate);
            break;
        }
    }
    let Some(seed) = seed else {
        return (PonCalStatus::CrcFail, None);
    };
    let Some(chip) = PonCalChip::from_bob_id(data[0x30]) else {
        return (PonCalStatus::ChipUnsupported, None);
    };

    let payload = &data[hsize..hsize + dsize];
    let mut out = vec![0xff; PON_CAL_SIZE];
    out[..PON_CAL_HEADER_SIZE].fill(0);
    out[..PON_CAL_MAGIC.len()].copy_from_slice(PON_CAL_MAGIC);
    out[0x08..0x0a].copy_from_slice(&PON_CAL_VERSION.to_le_bytes());
    out[0x0a..0x0c].copy_from_slice(&(PON_CAL_HEADER_SIZE as u16).to_le_bytes());
    out[0x0c..0x10].copy_from_slice(&chip.id().to_le_bytes());
    out[0x10..0x14].copy_from_slice(&PON_CAL_FORMAT_NATIVE.to_le_bytes());
    out[0x14..0x18].copy_from_slice(&(payload.len() as u32).to_le_bytes());
    out[0x18..0x1c].copy_from_slice(&calibration_crc32(payload).to_le_bytes());
    out[PON_CAL_HEADER_SIZE..PON_CAL_HEADER_SIZE + payload.len()].copy_from_slice(payload);
    let new_hcrc = calibration_crc32(&out[..BOB_HEADER_CRC_OFF]);
    out[BOB_HEADER_CRC_OFF..BOB_HEADER_CRC_OFF + 4].copy_from_slice(&new_hcrc.to_le_bytes());

    let status = match seed {
        BobCrcSeed::FiberHome => PonCalStatus::OkStrict,
        other => PonCalStatus::OkAltSeed(other),
    };
    (status, Some(out))
}

/// 用户侧校验：按 PonWrt `bosa_load_calibration()` 的要求复核一遍成品。
///
/// 与上游只做「编码」不同，这里把驱动侧那道门也走一遍 —— 编码对了但驱动不收
/// （版本、头长、format、chip 枚举、长度上限）在实机上是一样跑不起来的。
pub fn verify_aponcal(data: &[u8]) -> Vec<(&'static str, bool)> {
    let mut out = Vec::new();
    out.push(("magic == APONCAL", data.starts_with(PON_CAL_MAGIC)));
    let ver = u16_at(data, 0x08).unwrap_or(0);
    let hs = u16_at(data, 0x0a).unwrap_or(0);
    let chip = u32_at(data, 0x0c).unwrap_or(0);
    let fmt = u32_at(data, 0x10).unwrap_or(0);
    let plen = u32_at(data, 0x14).unwrap_or(0) as usize;
    let pcrc = u32_at(data, 0x18).unwrap_or(0);
    let hcrc = u32_at(data, BOB_HEADER_CRC_OFF).unwrap_or(0);
    out.push(("version == 1", ver == PON_CAL_VERSION));
    out.push(("hdrsize == 0x80", hs as usize == PON_CAL_HEADER_SIZE));
    out.push(("format == 1", fmt == PON_CAL_FORMAT_NATIVE));
    out.push(("chip ∈ {1,2}", chip == 1 || chip == 2));
    out.push(("payload ≤ cell-0x80", plen <= data.len().saturating_sub(PON_CAL_HEADER_SIZE)));
    out.push(("payload ≥ 0x780", plen >= 0x780));
    out.push((
        "header crc",
        calibration_crc32(&data[..BOB_HEADER_CRC_OFF]) == hcrc,
    ));
    out.push((
        "payload crc",
        data.get(PON_CAL_HEADER_SIZE..PON_CAL_HEADER_SIZE + plen)
            .map(|p| calibration_crc32(p) == pcrc)
            .unwrap_or(false),
    ));
    out
}

/// APONCAL 头 0x0C 的 chip 枚举（u32）→ 对应的 BOSA 驱动类型。
///
/// 只有 1 = GN28L95、2 = UX3363；读回内联镜像时用它在界面上还原芯片型号。
pub fn aponcal_chip(data: &[u8]) -> Option<PonCalChip> {
    match u32_at(data, 0x0c)? {
        1 => Some(PonCalChip::Gn28l95),
        2 => Some(PonCalChip::Ux3363),
        _ => None,
    }
}

/// APONCAL 头 0x08 的版本号（u16，固定 1）。
pub fn aponcal_version(data: &[u8]) -> Option<u16> {
    u16_at(data, 0x08)
}

/// 空字符串显示成 `—`，避免界面上出现「pon_mode=」这种半截话。
fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

/// 直读区里一张 PON 校准表的展示信息（只读，不改字节）。
pub struct CalTableInfo {
    pub name: String,
    /// 镜像内偏移 = `AN_CAL_FIXED_OFF + 槽号 × 0x1000`。
    pub offset: usize,
    pub size: usize,
    /// BOB 头里的版本戳；非 BOB 表为 `None`。
    pub version: Option<String>,
    /// 与它逐字节相同的另一张表。实机上 `10ge_sym_table` / `xgs_table` /
    /// `table` 三份内容完全一致 —— 对称 10G 与通用模式复用同一份校准。
    pub same_as: Option<String>,
    /// 表头 0x30 的 BOSA 驱动 chip id（上行 0x2E / 下行 0x27）。
    pub chip_id: Option<u8>,
    /// 表载荷里的型号明文（`ux3363` / `UX3326S1`）。
    pub tag: Option<String>,
    /// 槽里已经是 APONCAL（而不是出厂 BDBOB）—— 由 `--pon-aponcal` 打包的镜像。
    pub aponcal: bool,
}

/// 一个 PON 方向的软件可见信息。
///
/// 只列「能从校验包里读出来」的东西：光模块型号与序列号来自 `factory_conf`，
/// 驱动型号来自校准表的 chip id 字节，表名来自固定槽。板级拓扑（Fisilink
/// CRANE V2R2、AN8831X 等）不在校验包里，界面不臆造。
pub struct PonPort {
    /// 「上行 PON（ONT · 接运营商 OLT）」这类标题。
    pub title: &'static str,
    /// 光模块 / BOSA 型号，如 `PLDM588-815-C10`。
    pub module: String,
    /// 模块序列号（`transceiver_serial` / `lan_optic_serial`）。
    pub module_sn: String,
    /// 驱动型号 + chip id，如 `UX3326S1（chip id 0x27）`。
    pub driver: String,
    /// 属于这个方向的校准表名。
    pub tables: Vec<String>,
}

/// AN4475 的可编辑状态。
#[derive(Clone)]
pub struct An4475 {    pub device_sn: String,
    pub gpon_sn: [u8; 8],
    pub gpon_pw: String,
    pub base_mac: [u8; 6],
    pub eth_mac: [u8; 6],
    /// Wi-Fi 2.4G MAC，写 EEPROM +0x04 与 `APMAC_1`。
    pub wifi_2g4: [u8; 6],
    /// Wi-Fi 5G MAC，写 EEPROM +0x0A 与 `APMAC_2`。
    pub wifi_5g: [u8; 6],
    /// Wi-Fi EEPROM 头里的 device id（MT7990AN = 0x7993，只读展示）。
    pub eeprom_id: u16,
    pub vendor: String,
    /// BOSA 型号，来自 `olt_bosa_info` 的 `olt_bosa_id`（真机 WTD01）。
    /// 这是校准包里**唯一**的光组件型号来源；PON MAC 在 SoC 内，没有型号字段。
    pub bosa: String,
    /// PON 模式（`factory_conf` 的 `pon_mode`，真机 7，原厂编码，含义未公开）。
    pub pon_mode: String,
    /// 上联模式（`factory_conf` 的 `uplink_mode`，真机 4）。
    pub uplink_mode: String,
    /// 上行光模块型号（`factory_conf` 的 `transceiver_type`，真机 `PLDM588-815-C10`）。
    pub transceiver_type: String,
    /// 上行光模块序列号（`transceiver_serial`）。
    pub transceiver_sn: String,
    /// 下行 FTTR 光模块型号（`lan_optic_type`，真机 `PLDM586-552`）。
    pub lan_optic_type: String,
    /// 下行 FTTR 光模块序列号（`lan_optic_serial`）。
    pub lan_optic_sn: String,
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
            wifi_2g4: an_wifi_mac(&[0x02, 0, 0, 0, 0, 0], false),
            wifi_5g: an_wifi_mac(&[0x02, 0, 0, 0, 0, 0], true),
            eeprom_id: 0,
            vendor: "FH".to_string(),
            bosa: String::new(),
            pon_mode: String::new(),
            uplink_mode: String::new(),
            transceiver_type: String::new(),
            transceiver_sn: String::new(),
            lan_optic_type: String::new(),
            lan_optic_sn: String::new(),
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

    /// 上行 / 下行两个 PON 方向的软件可见信息。
    ///
    /// 本机是 FTTR 主网关：上行做 ONT（接运营商 OLT），下行做 OLT（带从 ONU），
    /// 两条路各有一颗 BOSA 驱动芯片，靠校准表的 chip id 字节区分 —— 不是
    /// 「一颗芯片两个方向」。上行 5 张表的 chip id 都是 0x2E，下行只有
    /// `olt_table` 一张，chip id 0x27。
    pub fn pon_ports(&self) -> Vec<PonPort> {
        let tables = self.cal_tables();
        const UP: [&str; 5] = [
            "10ge_asym_table",
            "10ge_sym_table",
            "xg_table",
            "xgs_table",
            "table",
        ];
        let pick = |names: &[&str]| -> Vec<CalTableInfo> {
            tables
                .iter()
                .filter(|t| names.contains(&t.name.as_str()))
                .map(|t| CalTableInfo {
                    name: t.name.clone(),
                    offset: t.offset,
                    size: t.size,
                    version: t.version.clone(),
                    same_as: t.same_as.clone(),
                    chip_id: t.chip_id,
                    tag: t.tag.clone(),
                    aponcal: t.aponcal,
                })
                .collect()
        };
        let drive = |list: &[CalTableInfo]| -> String {
            match list.first().and_then(|t| t.chip_id) {
                Some(id) => {
                    let name = bosa_chip_name(id);
                    let tag = list.first().and_then(|t| t.tag.clone());
                    match tag {
                        Some(t) if !t.eq_ignore_ascii_case(name.split('（').next().unwrap_or("")) => {
                            format!("{name} · chip id 0x{id:02X} · 表内明文 {t}")
                        }
                        _ => format!("{name} · chip id 0x{id:02X}"),
                    }
                }
                None => "—（未读到校准表）".to_string(),
            }
        };
        // 下行 6 项：主校准表 + 两颗 BOSA 的 APD 偏压表（按 olt_bosa_id 选一张）
        // + RSSI 校准 + LAN 光口 Tx 校准。只有 olt_table 是 BOB 表，其余是 UCI/二进制。
        const DOWN: [&str; 6] = [
            "olt_table",
            "olt_bosa_apd_WTD01",
            "olt_bosa_apd_YS01",
            "olt_bosa_info",
            "olt_bosa_rssi",
            "LanOpticTxCalib",
        ];
        let up = pick(&UP);
        let down = pick(&DOWN);
        vec![
            PonPort {
                title: "上行 PON（ONT · 接运营商 OLT）",
                module: self.transceiver_type.clone(),
                module_sn: self.transceiver_sn.clone(),
                driver: drive(&up),
                tables: up.iter().map(|t| t.name.clone()).collect(),
            },
            PonPort {
                title: "下行 FTTR PON（本机作 OLT · 带从 ONU）",
                module: self.lan_optic_type.clone(),
                module_sn: self.lan_optic_sn.clone(),
                driver: drive(&down),
                tables: down.iter().map(|t| t.name.clone()).collect(),
            },
        ]
    }

    /// 11 张固定槽校准表的展示信息，按槽号顺序；容器里没有的表跳过。
    ///
    /// 只读用途：界面上的「PON / 光组件」块靠它列出表名、偏移、版本戳，
    /// 以及哪几张内容完全相同（真机上 3 张共用一份）。
    pub fn cal_tables(&self) -> Vec<CalTableInfo> {
        AN_CAL_TABLES
            .iter()
            .enumerate()
            .filter_map(|(i, name)| {
                let f = self.files.iter().find(|f| &f.name == name)?;
                // 只跟**槽号更小**的表比，这样三份相同的表统一报「同最早那张」，
                // 不会出现 A 指向 B、B 又指向 C 的绕圈。
                let same_as = AN_CAL_TABLES
                    .iter()
                    .take(i)
                    .find(|other| {
                        self.files
                            .iter()
                            .any(|o| o.name == **other && o.data == f.data && !o.data.is_empty())
                    })
                    .map(|other| (*other).to_string());
                Some(CalTableInfo {
                    name: (*name).to_string(),
                    offset: AN_CAL_FIXED_OFF + i * AN_CAL_SLOT,
                    size: f.data.len(),
                    version: bob_version(&f.data),
                    same_as,
                    chip_id: bosa_chip_id(&f.data),
                    tag: bosa_tag(&f.data),
                    aponcal: f.data.starts_with(PON_CAL_MAGIC),
                })
            })
            .collect()
    }

    /// PON 口 6 张 BDBOB 校准表逐张按上游 fiberhome-factory 的流程处理一遍。
    ///
    /// 上游只有一张表可处理，这里把同一套 validate → chip 映射 → 重封跑 6 次，
    /// 每张都留下状态，便于在界面上直接看到「哪张能交给 PonWrt、哪张不能」。
    pub fn pon_cal_rows(&self) -> Vec<PonCalRow> {
        AN_CAL_TABLES
            .iter()
            .take(AN_BOB_TABLES)
            .enumerate()
            .filter_map(|(i, name)| {
                let f = self.files.iter().find(|f| &f.name == name)?;
                // 已经由本工具内联过 APONCAL 的镜像：槽里就是成品，不再跑一遍流程。
                // 这时 chip / 版本要换成从 APONCAL 头读，否则显示成 0x00 / —。
                let (status, data, chip_id, version) = if f.data.starts_with(PON_CAL_MAGIC) {
                    let chip = aponcal_chip(&f.data);
                    let cid = match chip {
                        Some(PonCalChip::Ux3363) => 0x2e,
                        Some(PonCalChip::Gn28l95) => 0x28,
                        None => 0,
                    };
                    (
                        PonCalStatus::AlreadyApconal,
                        Some(f.data.clone()),
                        cid,
                        aponcal_version(&f.data).map(|v| format!("v{v}")),
                    )
                } else {
                    let (st, data) = bob_to_aponcal(&f.data);
                    let cid = f.data.get(0x30).copied().unwrap_or(0);
                    (st, data, cid, bob_version(&f.data))
                };
                Some(PonCalRow {
                    table: (*name).to_string(),
                    offset: AN_CAL_FIXED_OFF + i * AN_CAL_SLOT,
                    chip_id,
                    chip: PonCalChip::from_bob_id(chip_id),
                    version,
                    status,
                    data,
                })
            })
            .collect()
    }

    /// 镜像内 12 个 MAC 的重复检查，返回 `(MAC, 用到它的字段列表)`。
    ///
    /// 一、原厂规则本身不重复：`D4:9F` 段用 50–53/58，`D4:A0` 段用 50/58（Wi-Fi）
    /// 和 71–75（LAN），两段前缀不同、同段内部也不撞。二、但这些字段都能手改，
    /// 改错就会出现「同一地址挂在两个接口上」——交换机 MAC 表在两端口间来回学，
    /// 表现为丢包/闪断。所以这里按「同地址出现 ≥2 次」列出来，交界面提示。
    pub fn mac_conflicts(&self) -> Vec<(String, Vec<String>)> {
        mac_conflicts_of(&self.base_mac, &self.eth_mac, &self.wifi_2g4, &self.wifi_5g)
    }

    pub fn summary(&self) -> String {
        let mut s = String::new();
        s.push_str("机型       : AN4475 (10G PON)\n");
        s.push_str(&format!("设备序列号 : {}\n", self.device_sn));
        s.push_str(&format!(
            "GPON SN    : {}{}\n",
            gpon_sn_display(&self.gpon_sn),
            if gpon_sn_is_blank(&self.gpon_sn) {
                "（未写入）"
            } else {
                ""
            }
        ));
        s.push_str(&format!("GPON 密码  : {}\n", self.gpon_pw));
        s.push_str(&format!("基准 MAC   : {}\n", mac_to_string(&self.base_mac)));
        s.push_str(&format!("LAN 基址   : {}\n", mac_to_string(&self.eth_mac)));
        s.push_str(&format!("2.4G MAC   : {}\n", mac_to_string(&self.wifi_2g4)));
        s.push_str(&format!("5G MAC     : {}\n", mac_to_string(&self.wifi_5g)));
        if self.eeprom_id != 0 {
            s.push_str(&format!(
                "Wi-Fi EEPROM: device id 0x{:04X}{}\n",
                self.eeprom_id,
                if self.eeprom_id == AN_MT7990_ID {
                    ""
                } else {
                    "（非 MT7990AN）"
                }
            ));
        }
        if !self.bosa.is_empty() {
            s.push_str(&format!("BOSA APD   : {}\n", self.bosa));
        }
        let ports = self.pon_ports();
        for (label, port) in [("上行光模块", &ports[0]), ("下行光模块", &ports[1])] {
            if port.module.is_empty() {
                continue;
            }
            s.push_str(&format!("{label} : {}", port.module));
            if !port.module_sn.is_empty() {
                s.push_str(&format!(" · SN {}", port.module_sn));
            }
            s.push_str(&format!(" · {}\n", port.driver));
        }
        if !self.pon_mode.is_empty() || !self.uplink_mode.is_empty() {
            s.push_str(&format!(
                "PON 模式   : pon_mode={} uplink_mode={}（原厂编码）\n",
                or_dash(&self.pon_mode),
                or_dash(&self.uplink_mode)
            ));
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

    /// 打包 2 MiB 容器（出厂原样：PON 校准表按 BDBOB 原封不动）。
    ///
    /// 身份字段会先回写 `factory_conf`（保持文本与头部一致），再压缩入包。
    pub fn encode_compact(&self) -> Result<Vec<u8>, String> {
        self.encode_compact_with(CompactOptions::default())
            .map(|(img, _)| img)
    }

    /// 打包 2 MiB 容器，并按 `opts` 决定 PON 区怎么处理。
    ///
    /// 返回 `(镜像, 上行表处理明细)`；明细供界面和 CLI 打印，正常打包可忽略。
    pub fn encode_compact_with(
        &self,
        opts: CompactOptions,
    ) -> Result<(Vec<u8>, Vec<PonInline>), String> {
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
        let mut inlines: Vec<PonInline> = Vec::new();
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
        // 12 字符视图：写不下（厂商码非 ASCII、只能给 16 位 hex）就保持 0xFF，
        // 这时 PonWrt 侧读到的是空值，不会把错的东西写回来。
        let view = gpon_sn_display(&self.gpon_sn);
        if view.len() == AN_GPON_SN_VIEW_SIZE && view.is_ascii() {
            img[AN_H_GPON_SN_VIEW..AN_H_GPON_SN_VIEW + AN_GPON_SN_VIEW_SIZE]
                .copy_from_slice(view.as_bytes());
        }
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
                // PON 校准表：固定槽，槽内剩余保持 0xFF。
                if raw.len() > AN_CAL_SLOT {
                    return Err(format!(
                        "{name} 有 {} 字节，超出 {} 字节的校准槽",
                        raw.len(),
                        AN_CAL_SLOT
                    ));
                }
                let blob = if opts.pon_aponcal && is_uplink_bob(name) {
                    let (data, action) = inline_aponcal(raw);
                    inlines.push(PonInline {
                        table: (*name).clone(),
                        offset: at,
                        action,
                    });
                    data
                } else {
                    raw.clone()
                };
                max_end = max_end.max(at + blob.len());
                (blob, 0u8, at)
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
        // 尾部再塞一份 mt76 能直接加载的 7 680 B：拿上面写好的那份 1.35 MB
        // 做模板打底（只取 MAC / WIFI_CONF / PA-LNA，功率表用官方值），
        // 这样 OpenWrt 侧 nvmem 指到 0x1AB000 就能用，不必现场转换。
        if let Some(raw) = files.get(AN_EEPROM) {
            let bin = mt76_from_template(MT76_DEFAULT_IPA, raw)?;
            let end = AN_MT76_FIXED_OFF + bin.len();
            if end > AN_COMPACT_SIZE {
                return Err(format!(
                    "mt76 槽 0x{:06X} 放不下 {AN_MT76_SIZE} 字节",
                    AN_MT76_FIXED_OFF
                ));
            }
            img[AN_MT76_FIXED_OFF..end].copy_from_slice(&bin);
            max_end = max_end.max(end);
        }

        // 载荷长度取「最远写入位置」，固定区的 EEPROM 也算在内。
        put_u32(&mut img, 0x18, (max_end - AN_PAYLOAD_OFF) as u32);
        Ok((img, inlines))
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
    // 12 字符视图是 PonWrt 侧唯一的写入口：它被改过就以它为准，重新编码回 8 字节。
    // 全 0xFF 的老镜像经 lossy 转换会变成 12 个 U+FFFD，`is_ascii()` 直接挡掉。
    if let Some(sn) = gpon_sn_view(&data) {
        gpon_sn = sn;
    }
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
        bosa: bosa_from_files(&files),
        pon_mode: conf_value(&files, "pon_mode"),
        uplink_mode: conf_value(&files, "uplink_mode"),
        transceiver_type: conf_value(&files, "transceiver_type"),
        transceiver_sn: conf_value(&files, "transceiver_serial"),
        lan_optic_type: conf_value(&files, "lan_optic_type"),
        lan_optic_sn: conf_value(&files, "lan_optic_serial"),
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

    // 原厂存 16 位 hex，但手工改过/老版本的 conf 里可能直接是 `FHTT...` 文本，
    // 所以先走 parse_gpon_sn；解不了再退回「能解多少写多少」，最后才留 FF。
    let gpon_sn = parse_gpon_sn(&gpon_hex).unwrap_or_else(|_| {
        let mut sn = [0xffu8; 8];
        if let Some(v) = parse_hex(&gpon_hex) {
            let n = v.len().min(8);
            sn[..n].copy_from_slice(&v[..n]);
        }
        sn
    });
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
    // 原厂**不写** EEPROM 的 MAC（实机仍是 MediaTek 默认的 00:0C:43:26:60:10/11），
    // 也不写 APMAC_1/2，Wi-Fi MAC 一律由 brmac 现算。所以这里的顺序是：
    // 「真写过」的 EEPROM 值 > 「真写过」的 APMAC > 按 brmac 派生。
    let eep = list.iter().find(|f| f.name == AN_EEPROM);
    let conf_mac = |key: &str| {
        uci_get(&conf, key)
            .and_then(|s| parse_mac(&s))
            .filter(|m| !is_placeholder_mac(m))
    };
    let (mut wifi_2g4, mut wifi_5g) = match eep.and_then(|f| eeprom_macs(&f.data)) {
        Some((a, b)) => (a, b),
        None => (
            conf_mac(AN_APMAC_2G4).unwrap_or_else(|| an_wifi_mac(&base_mac, false)),
            conf_mac(AN_APMAC_5G).unwrap_or_else(|| an_wifi_mac(&base_mac, true)),
        ),
    };
    if wifi_2g4 == [0u8; 6] {
        wifi_2g4 = an_wifi_mac(&base_mac, false);
    }
    if wifi_5g == [0u8; 6] {
        wifi_5g = an_wifi_mac(&base_mac, true);
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
        bosa: bosa_from_files(&list),
        pon_mode: conf_value(&list, "pon_mode"),
        uplink_mode: conf_value(&list, "uplink_mode"),
        transceiver_type: conf_value(&list, "transceiver_type"),
        transceiver_sn: conf_value(&list, "transceiver_serial"),
        lan_optic_type: conf_value(&list, "lan_optic_type"),
        lan_optic_sn: conf_value(&list, "lan_optic_serial"),
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
/// 芯片厂 / OEM 的占位 MAC —— 原厂从不把它们当作「已写」。
///
/// * `00:0C:43` 是 **MediaTek 自己的 OUI**，EEPROM 模板里就是 `…:26:60:10/11`
/// * `00:0A:C2` 是原厂 `factory_conf` 里 `APMAC_1/APMAC_2` 的占位前缀
/// * 全 `00` / 全 `FF` 是未烧写的空值
///
/// 导入备份时遇到这些值，说明产线没写，应按 `brmac` 现算，而不是把它们当身份
/// 原样带进新镜像（否则整批机器同 MAC）。
pub fn is_placeholder_mac(m: &[u8; 6]) -> bool {
    let oui = [m[0], m[1], m[2]];
    oui == [0x00, 0x0C, 0x43] || oui == [0x00, 0x0A, 0xC2] || *m == [0u8; 6] || *m == [0xFFu8; 6]
}

pub fn eeprom_macs(eeprom: &[u8]) -> Option<([u8; 6], [u8; 6])> {
    if eeprom.len() < AN_EEPROM_MAC_5G + 6 {
        return None;
    }
    let mut a = [0u8; 6];
    let mut b = [0u8; 6];
    a.copy_from_slice(&eeprom[AN_EEPROM_MAC_2G4..AN_EEPROM_MAC_2G4 + 6]);
    b.copy_from_slice(&eeprom[AN_EEPROM_MAC_5G..AN_EEPROM_MAC_5G + 6]);
    if is_placeholder_mac(&a) {
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

pub fn hex_upper(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02X}")).collect()
}

// ---------------------------------------------------------------------------
// GPON SN：规则与 fiberhome-factory 的 `display_sn` / `parse_pon_serial` 一致
// ---------------------------------------------------------------------------
//
// 8 字节的 GPON SN 有两种写法，厂商固化的是「4 字节 ASCII 厂商码 + 4 字节二进制序列号」：
//
//   FHTTC26A69FF   ← 12 字符，人看着能读（fiberhome-factory 界面上就是这个）
//   46485454C26A69FF ← 16 位 hex，8 字节原样（UCI `GponSN` 存这个）
//
// 两者是同一份数据，只是展示层不同。fiberhome-factory 的 `display_sn` 先按 12 字符解释，
// 不成立才退到 16 位 hex；本函数照搬这个分支顺序，不自行发明规则。

/// 厂商码长度，也是 8 字节里 ASCII 部分的长度。
pub const AN_GPON_VENDOR_LEN: usize = 4;

/// 解析 GPON SN 文本，返回写进镜像的 8 字节。
///
/// 分支顺序与 fiberhome-factory 的 `display_sn()` 相同：
///
/// 1. **12 个字符、前 4 个是 ASCII 字母数字** → 前 4 字节按 ASCII 原样，
///    后 4 字节按 hex 解码。这是产线上贴纸/工单的写法（`FHTTC26A69FF`）。
/// 2. **其它** → 整串（允许 `:` `-` 分隔）当 16 位 hex 解成 8 字节，
///    并拒绝全 `00` / 全 `FF`（`parse_pon_serial()` 的判定，空 SN 是无意义的）。
///
/// 12 字符分支上游不做全 0/FF 校验，这里也不做，保持行为一致 —— 否则
/// `FHTT00000000` 会被拒，而上游能存。
pub fn parse_gpon_sn(text: &str) -> Result<[u8; 8], String> {
    let t = text.trim();
    if t.len() == 12 && t.is_ascii() && t.as_bytes()[..4].iter().all(u8::is_ascii_alphanumeric) {
        let mut sn = [0u8; 8];
        sn[..AN_GPON_VENDOR_LEN].copy_from_slice(&t.as_bytes()[..AN_GPON_VENDOR_LEN]);
        for i in 0..4 {
            let pair = &t[4 + i * 2..6 + i * 2];
            sn[4 + i] = u8::from_str_radix(pair, 16)
                .map_err(|_| format!("GPON SN 序列号部分不是 hex：{pair}"))?;
        }
        return Ok(sn);
    }

    // 上游 decode_hex() 会剔除 `:` 与 `-`，这里跟着剔，其它字符不放过。
    let compact: String = t.chars().filter(|c| *c != ':' && *c != '-').collect();
    if !compact.is_ascii() || compact.len() != 16 {
        return Err(format!(
            "GPON SN 要 12 位「厂商码+8 位 hex」（如 FHTTC26A69FF）或 16 位 hex，当前 {} 位",
            compact.chars().count()
        ));
    }
    let mut sn = [0u8; 8];
    for i in 0..8 {
        sn[i] = u8::from_str_radix(&compact[i * 2..i * 2 + 2], 16)
            .map_err(|_| format!("GPON SN 含非 hex 字符：{compact}"))?;
    }
    if sn.iter().all(|b| *b == 0x00) || sn.iter().all(|b| *b == 0xff) {
        return Err("GPON SN 不能是 8 字节全 00 或全 FF".to_string());
    }
    Ok(sn)
}

/// 8 字节 → 界面显示文本。
///
/// 与 fiberhome-factory 读回时的判断一致：前 4 字节都是字母数字就拼成
/// `FHTT` + 8 位 hex，否则老老实实给 16 位 hex（厂商码不是 ASCII 的老设备）。
pub fn gpon_sn_display(sn: &[u8; 8]) -> String {
    if sn[..AN_GPON_VENDOR_LEN]
        .iter()
        .all(u8::is_ascii_alphanumeric)
    {
        format!(
            "{}{}",
            String::from_utf8_lossy(&sn[..AN_GPON_VENDOR_LEN]),
            hex_upper(&sn[AN_GPON_VENDOR_LEN..])
        )
    } else {
        hex_upper(sn)
    }
}

/// 全 `00` / 全 `FF` 说明 SN 还没写（新镜像默认就是 8 个 `FF`）。
pub fn gpon_sn_is_blank(sn: &[u8; 8]) -> bool {
    sn.iter().all(|b| *b == 0x00) || sn.iter().all(|b| *b == 0xff)
}

/// 从头部 `0xA2` 的 12 字符视图读 SN。
///
/// 老镜像这块是 `0xFF`，`from_utf8_lossy` 会给出 12 个 U+FFFD，`is_ascii()` 挡掉；
/// 写坏成半截 ASCII 的也不采信。只有「12 个可打印 ASCII 且能按规则解析」才算数。
pub fn gpon_sn_view(data: &[u8]) -> Option<[u8; 8]> {
    let raw = data.get(AN_H_GPON_SN_VIEW..AN_H_GPON_SN_VIEW + AN_GPON_SN_VIEW_SIZE)?;
    let text = String::from_utf8_lossy(raw);
    let text = text.trim();
    if !text.is_ascii() || text.len() != AN_GPON_SN_VIEW_SIZE {
        return None;
    }
    parse_gpon_sn(text).ok()
}

/// 随机 SN，与 fiberhome-factory 的 `random_sn()` 同款：`FHTT` + 4 字节随机 hex。
pub fn random_gpon_sn() -> [u8; 8] {
    let mut sn = [0xffu8; 8];
    sn[..AN_GPON_VENDOR_LEN].copy_from_slice(b"FHTT");
    let mut b = [0u8; 4];
    let _ = getrandom::fill(&mut b);
    sn[AN_GPON_VENDOR_LEN..].copy_from_slice(&b);
    sn
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

    /// 原厂 fac_start_wifi.sh 的实测算例：本机 brmac = D4:9F:29:04:E7:50。
    #[test]
    fn an_wifi_mac_matches_vendor_script() {
        let base = [0xD4, 0x9F, 0x29, 0x04, 0xE7, 0x50];
        assert_eq!(mac_to_string(&an_wifi_mac(&base, false)), "D4:A0:29:04:E7:50");
        assert_eq!(mac_to_string(&an_wifi_mac(&base, true)), "D4:A0:29:04:E7:58");
        // 第二字节与末字节都要按 u8 回绕，不向邻近字节进位。
        let b2 = [0x00, 0xFF, 0x22, 0x33, 0x44, 0xF8];
        assert_eq!(mac_to_string(&an_wifi_mac(&b2, false)), "00:00:22:33:44:F8");
        assert_eq!(mac_to_string(&an_wifi_mac(&b2, true)), "00:00:22:33:44:00");
    }

    #[test]
    fn compact_carries_mt76_slot() {
        // 放一份 EEPROM 进去，打包后尾部应出现 7 680 B 的 mt76 副本。
        let mut a = An4475::new();
        a.files.push(AnFile {
            name: AN_EEPROM.to_string(),
            data: MT76_DEFAULT_IPA.to_vec(),
        });
        let img = a.encode_compact().unwrap();
        let slot = compact_mt76(&img).expect("尾部应带 mt76 槽");
        assert_eq!(slot.len(), AN_MT76_SIZE);
        assert_eq!(slot, MT76_DEFAULT_IPA);
        assert!(AN_MT76_FIXED_OFF + AN_MT76_SIZE <= AN_COMPACT_SIZE);
        // 载荷长度要覆盖到槽尾。
        let payload = u32::from_le_bytes([img[0x18], img[0x19], img[0x1a], img[0x1b]]) as usize;
        assert_eq!(payload, AN_MT76_FIXED_OFF + AN_MT76_SIZE - AN_PAYLOAD_OFF);

        // 没有 EEPROM 就不该有槽（尾部保持 0xFF）。
        let b = An4475::new();
        let img2 = b.encode_compact().unwrap();
        assert!(compact_mt76(&img2).is_none());
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
            AN_MT7990_ID
        );
        assert_eq!(mt76_pa_lna(MT76_DEFAULT_IPA), Some(0));
        assert_eq!(mt76_pa_lna(MT76_DEFAULT_EPA), Some(3));
    }

    #[test]
    fn mt76_carve_and_inject() {
        // 造一份「实机」EEPROM：芯片 ID + 版本 + 两个 MAC + WIFI_CONF。
        let mut ee = vec![0u8; 0x100];
        ee[0..2].copy_from_slice(&AN_MT7990_ID.to_le_bytes());
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
    /// BOB 头解析：真实版本戳能读出来，非 BOB 文件一律 None（否则界面上
    /// 配置文件也会显示「版本」）。
    #[test]
    fn bob_header() {
        let mut t = vec![0u8; 2048];
        t[..14].copy_from_slice(b"fiberhomebdbob");
        t[0x10..0x1e].copy_from_slice(b"202404291559VB");
        assert_eq!(bob_version(&t).as_deref(), Some("202404291559VB"));
        assert_eq!(bob_version(b"config key 'x'"), None);
        assert_eq!(bob_version(&[0xff; 2048]), None);
    }

    /// 三份内容相同的表只报「同最早那张」，且 BOSA 型号能从 UCI 文本里取到。
    #[test]
    fn cal_tables_same_as() {
        let mut a = An4475::new();
        let mut bob = vec![0u8; 2048];
        bob[..14].copy_from_slice(b"fiberhomebdbob");
        bob[0x10..0x1e].copy_from_slice(b"202404291559VB");
        let mk = |n: &str, d: Vec<u8>| AnFile {
            name: n.to_string(),
            data: d,
        };
        a.files = vec![
            mk("10ge_asym_table", vec![0xaa; 2048]),
            mk("10ge_sym_table", bob.clone()),
            mk("xg_table", vec![0xbb; 2048]),
            mk("xgs_table", bob.clone()),
            mk("table", bob),
            mk("olt_bosa_info", b"config key 'olt_bosa_id'\n    option value 'WTD01'\n".to_vec()),
        ];
        let t = a.cal_tables();
        assert_eq!(t.len(), 6);
        assert_eq!(t[1].same_as, None);
        assert_eq!(t[3].same_as.as_deref(), Some("10ge_sym_table"));
        assert_eq!(t[4].same_as.as_deref(), Some("10ge_sym_table"));
        assert_eq!(t[0].same_as, None);
        assert_eq!(bosa_from_files(&a.files), "WTD01");
    }
    /// 原厂分配方案本身不冲突；手改撞车时必须被点名。
    #[test]
    fn mac_conflict_detect() {
        let base = [0xd4, 0x9f, 0x29, 0x04, 0xe7, 0x50];
        let eth = [0xd4, 0xa0, 0x29, 0x04, 0xe7, 0x71];
        let w24 = an_wifi_mac(&base, false);
        let w5 = an_wifi_mac(&base, true);
        assert!(mac_conflicts_of(&base, &eth, &w24, &w5).is_empty());

        // 把 2.4G 改成 LAN 第一个地址：应该正好抓到 eth1mac 与 Wi-Fi 2.4G 撞车。
        let bad = [0xd4, 0xa0, 0x29, 0x04, 0xe7, 0x71];
        let c = mac_conflicts_of(&base, &eth, &bad, &w5);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].0, "D4:A0:29:04:E7:71");
        assert!(c[0].1.contains(&"eth1mac".to_string()));
        assert!(c[0].1.contains(&"Wi-Fi 2.4G".to_string()));

        // 全 0 是「没填」，不算冲突。
        assert!(mac_conflicts_of(&[0; 6], &[0; 6], &[0; 6], &[0; 6]).is_empty());
    }
    /// 表头 0x30 是 BOSA 驱动 chip id，载荷里还有型号明文；两条必须对得上。
    #[test]
    fn bosa_chip_identity() {
        let mut up = vec![0u8; 2048];
        up[..14].copy_from_slice(b"fiberhomebdbob");
        up[0x30] = 0x2E;
        up[0x95..0x9b].copy_from_slice(b"ux3363");
        assert_eq!(bosa_chip_id(&up), Some(0x2E));
        assert_eq!(bosa_tag(&up).as_deref(), Some("ux3363"));
        assert_eq!(bosa_chip_name(0x2E), "UX3363（与 UX3361 同驱动）");

        let mut down = vec![0u8; 2048];
        down[..14].copy_from_slice(b"fiberhomebdbob");
        down[0x30] = 0x27;
        down[0x1C4..0x1CC].copy_from_slice(b"UX3326S1");
        assert_eq!(bosa_chip_id(&down), Some(0x27));
        assert_eq!(bosa_tag(&down).as_deref(), Some("UX3326S1"));
        assert_eq!(bosa_chip_name(0x27), "UX3326S1");

        // 配置文件不是 BOB 表，不该被当成光组件。
        assert_eq!(bosa_chip_id(b"config key 'x'"), None);
        assert_eq!(bosa_tag(&[0xff; 2048]), None);
    }

    /// 上行 / 下行两份表各归各的方向，chip id 不串。
    #[test]
    fn pon_ports_split() {
        let mut a = An4475::new();
        a.transceiver_type = "PLDM588-815-C10".to_string();
        a.transceiver_sn = "SNUP".to_string();
        a.lan_optic_type = "PLDM586-552".to_string();
        a.lan_optic_sn = "SNDOWN".to_string();
        let mk = |name: &str, id: u8, tag: &[u8]| {
            let mut t = vec![0u8; 2048];
            t[..14].copy_from_slice(b"fiberhomebdbob");
            t[0x30] = id;
            t[0x95..0x95 + tag.len()].copy_from_slice(tag);
            AnFile { name: name.to_string(), data: t }
        };
        a.files = vec![
            mk("10ge_asym_table", 0x2E, b"ux3363"),
            mk("10ge_sym_table", 0x2E, b"ux3363"),
            mk("xg_table", 0x2E, b"ux3363"),
            mk("xgs_table", 0x2E, b"ux3363"),
            mk("table", 0x2E, b"ux3363"),
            mk("olt_table", 0x27, b"UX3326S1"),
        ];
        let p = a.pon_ports();
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].module, "PLDM588-815-C10");
        assert_eq!(p[0].tables.len(), 5);
        assert!(p[0].driver.contains("0x2E"), "{}", p[0].driver);
        assert_eq!(p[1].module, "PLDM586-552");
        assert_eq!(p[1].tables, vec!["olt_table".to_string()]);
        assert!(p[1].driver.contains("UX3326S1"), "{}", p[1].driver);
        assert!(p[1].driver.contains("0x27"), "{}", p[1].driver);
    }

    /// 造一张结构合法、CRC 按指定种子填好的 BDBOB 表。
    fn bob_fixture(chip_id: u8, seed: BobCrcSeed) -> Vec<u8> {
        let mut t = vec![0u8; 2048];
        t[..14].copy_from_slice(AN_BOB_MAGIC);
        t[0x10..0x20].copy_from_slice(b"202404291559VB\0\0");
        t[0x30] = chip_id;
        for (i, b) in t[0x80..0x800].iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        t[0x70..0x74].copy_from_slice(&1920u32.to_le_bytes());
        t[0x78..0x7c].copy_from_slice(&0x80u32.to_le_bytes());
        let dcrc = seed.crc(&t[0x80..0x800]);
        t[0x74..0x78].copy_from_slice(&dcrc.to_le_bytes());
        let hcrc = seed.crc(&t[..0x7c]);
        t[0x7c..0x80].copy_from_slice(&hcrc.to_le_bytes());
        t
    }

    #[test]
    fn bob_to_aponcal_both_seeds() {
        for seed in [BobCrcSeed::FiberHome, BobCrcSeed::Standard] {
            let bob = bob_fixture(0x2E, seed);
            let (st, out) = bob_to_aponcal(&bob);
            let out = out.expect("应产出 APONCAL");
            match (seed, st) {
                (BobCrcSeed::FiberHome, PonCalStatus::OkStrict) => {}
                (BobCrcSeed::Standard, PonCalStatus::OkAltSeed(BobCrcSeed::Standard)) => {}
                _ => panic!("状态不对: {st:?} / {seed:?}"),
            }
            assert_eq!(out.len(), PON_CAL_SIZE);
            assert_eq!(&out[..8], PON_CAL_MAGIC);
            assert_eq!(u32_at(&out, 0x0c), Some(2), "chip 应为 UX3363");
            assert_eq!(u32_at(&out, 0x14), Some(1920));
            // payload 必须原样搬运，一个字节都不许动
            assert_eq!(&out[0x80..0x800], &bob[0x80..0x800]);
            let checks = verify_aponcal(&out);
            assert!(checks.iter().all(|(_, ok)| *ok), "{checks:?}");
        }
    }

    #[test]
    fn bob_to_aponcal_rejects_downlink_chip() {
        // 下行 olt_table 的 0x27（UX3326S1）不在 APONCAL 枚举里，不能硬填 2。
        let bob = bob_fixture(0x27, BobCrcSeed::Standard);
        let (st, out) = bob_to_aponcal(&bob);
        assert_eq!(st, PonCalStatus::ChipUnsupported);
        assert!(out.is_none());
    }

    #[test]
    fn bob_to_aponcal_rejects_corrupt() {
        let mut bob = bob_fixture(0x2E, BobCrcSeed::FiberHome);
        bob[0x300] ^= 0xff;
        assert_eq!(bob_to_aponcal(&bob).0, PonCalStatus::CrcFail);
        assert_eq!(bob_to_aponcal(&[0u8; 16]).0, PonCalStatus::NotBob);
    }

    #[test]
    fn pon_cal_rows_covers_all_pob_slots() {
        let mut a = An4475::new();
        a.files = vec![
            AnFile { name: "10ge_asym_table".into(), data: bob_fixture(0x2E, BobCrcSeed::Standard) },
            AnFile { name: "10ge_sym_table".into(), data: bob_fixture(0x2E, BobCrcSeed::FiberHome) },
            AnFile { name: "xg_table".into(), data: bob_fixture(0x2E, BobCrcSeed::Standard) },
            AnFile { name: "xgs_table".into(), data: bob_fixture(0x2E, BobCrcSeed::FiberHome) },
            AnFile { name: "table".into(), data: bob_fixture(0x2E, BobCrcSeed::FiberHome) },
            AnFile { name: "olt_table".into(), data: bob_fixture(0x27, BobCrcSeed::Standard) },
        ];
        let rows = a.pon_cal_rows();
        assert_eq!(rows.len(), AN_BOB_TABLES);
        assert_eq!(rows[0].offset, AN_CAL_FIXED_OFF);
        assert_eq!(rows[5].offset, AN_CAL_FIXED_OFF + 5 * AN_CAL_SLOT);
        assert_eq!(rows[5].status, PonCalStatus::ChipUnsupported);
        assert!(!rows[5].status.exportable());
        // 5 张上行全部可导出，且种子分别命中两套
        assert!(rows[..5].iter().all(|r| r.status.exportable()));
        assert_eq!(rows[1].status, PonCalStatus::OkStrict);
        assert_eq!(rows[0].status, PonCalStatus::OkAltSeed(BobCrcSeed::Standard));
    }

    /// 打开 `pon_aponcal`：5 张上行槽写 APONCAL，下行 olt_table 一字节不动。
    #[test]
    fn inline_aponcal_touches_uplink_only() {
        let mut a = An4475::new();
        a.files = vec![
            AnFile { name: "10ge_asym_table".into(), data: bob_fixture(0x2E, BobCrcSeed::Standard) },
            AnFile { name: "10ge_sym_table".into(), data: bob_fixture(0x2E, BobCrcSeed::FiberHome) },
            AnFile { name: "xg_table".into(), data: bob_fixture(0x2E, BobCrcSeed::Standard) },
            AnFile { name: "xgs_table".into(), data: bob_fixture(0x2E, BobCrcSeed::FiberHome) },
            AnFile { name: "table".into(), data: bob_fixture(0x2E, BobCrcSeed::FiberHome) },
            AnFile { name: "olt_table".into(), data: bob_fixture(0x27, BobCrcSeed::Standard) },
        ];
        let (img, inlines) = a
            .encode_compact_with(CompactOptions { pon_aponcal: true })
            .expect("应能打包");
        assert_eq!(inlines.len(), 5, "只处理上行 5 张");
        assert!(inlines.iter().all(|r| r.action.ok()), "{:?}", inlines.iter().map(|r| r.action.label()).collect::<Vec<_>>());

        let slot = |i: usize| {
            let at = AN_CAL_FIXED_OFF + i * AN_CAL_SLOT;
            &img[at..at + PON_CAL_SIZE]
        };
        for i in 0..5 {
            assert!(slot(i).starts_with(PON_CAL_MAGIC), "槽 {i} 应为 APONCAL");
            let checks = verify_aponcal(slot(i));
            assert!(checks.iter().all(|(_, ok)| *ok), "槽 {i} 驱动侧复核: {checks:?}");
        }
        assert!(
            slot(5).starts_with(&AN_BOB_MAGIC[..]),
            "下行 olt_table（chip 0x27）必须保持原厂 BDBOB"
        );
    }

    /// 对已经内联过的镜像再打一次包：不重复转换，产物逐字节相同。
    #[test]
    fn inline_aponcal_is_idempotent() {
        let mut a = An4475::new();
        a.files = vec![
            AnFile { name: "10ge_asym_table".into(), data: bob_fixture(0x2E, BobCrcSeed::Standard) },
            AnFile { name: "table".into(), data: bob_fixture(0x2E, BobCrcSeed::FiberHome) },
            AnFile { name: "olt_table".into(), data: bob_fixture(0x27, BobCrcSeed::Standard) },
        ];
        let opts = CompactOptions { pon_aponcal: true };
        let (img1, _) = a.encode_compact_with(opts).expect("第一次打包");

        let b = parse_compact(&img1).expect("产物应能读回");
        assert!(b.cal_tables().iter().any(|t| t.aponcal), "读回时应标出槽里已是 APONCAL");
        let (img2, inlines) = b.encode_compact_with(opts).expect("第二次打包");
        assert_eq!(img1, img2, "重复打包必须幂等");
        assert!(
            inlines.iter().all(|r| r.action == PonInlineAction::AlreadyApconal),
            "{:?}",
            inlines.iter().map(|r| r.action.label()).collect::<Vec<_>>()
        );
    }

    /// 转换失败的表保留原样 —— 半张 APONCAL 比原表更危险。
    #[test]
    fn inline_aponcal_keeps_corrupt_table() {
        let mut bad = bob_fixture(0x2E, BobCrcSeed::FiberHome);
        bad[0x300] ^= 0xff;
        let mut a = An4475::new();
        a.files = vec![AnFile { name: "10ge_asym_table".into(), data: bad.clone() }];
        let (img, inlines) = a
            .encode_compact_with(CompactOptions { pon_aponcal: true })
            .expect("坏表不应中断打包");
        assert_eq!(inlines.len(), 1);
        assert_eq!(
            inlines[0].action,
            PonInlineAction::Kept(PonCalStatus::CrcFail)
        );
        assert_eq!(
            &img[AN_CAL_FIXED_OFF..AN_CAL_FIXED_OFF + 2048],
            &bad[..],
            "坏表必须原样留在槽里"
        );
    }

    /// webif 那行 `GponSN` 就是 16 位 hex；贴纸/工单是 `FHTT` + 8 位 hex。
    /// 两种写法必须落到同一份 8 字节。
    #[test]
    fn gpon_sn_both_spellings_agree() {
        let hex = parse_gpon_sn("46485454C26A69FF").expect("16 位 hex");
        let readable = parse_gpon_sn("FHTTC26A69FF").expect("12 字符厂商码形式");
        assert_eq!(hex, readable);
        assert_eq!(hex[..4], *b"FHTT");
        assert_eq!(&hex[4..], &[0xC2, 0x6A, 0x69, 0xFF]);
        // 读回按 fiberhome-factory 的 display_sn 反向拼，应还原成可读形式
        assert_eq!(gpon_sn_display(&hex), "FHTTC26A69FF");
    }

    /// 厂商码不是 ASCII 的老设备只能走 16 位 hex，读回也应是 16 位 hex。
    #[test]
    fn gpon_sn_non_ascii_vendor_stays_hex() {
        let sn = parse_gpon_sn("0102030405060708").expect("非 ASCII 厂商码");
        assert_eq!(gpon_sn_display(&sn), "0102030405060708");
    }

    /// 全 00 / 全 FF 在 16 位 hex 分支要拒（上游 parse_pon_serial 的判定）。
    #[test]
    fn gpon_sn_rejects_blank() {
        assert!(parse_gpon_sn("0000000000000000").is_err());
        assert!(parse_gpon_sn("FFFFFFFFFFFFFFFF").is_err());
        assert!(gpon_sn_is_blank(&[0xff; 8]));
        // 12 字符分支上游不校验，保持同一行为
        assert!(parse_gpon_sn("FHTT00000000").is_ok());
    }

    /// 长度/字符不对要报错，不能静默 —— 产线上少一位最难查。
    #[test]
    fn gpon_sn_rejects_bad_input() {
        assert!(parse_gpon_sn("FHTTC26A69F").is_err());
        assert!(parse_gpon_sn("FHTTC26A69FFF").is_err());
        assert!(parse_gpon_sn("").is_err());
        assert!(parse_gpon_sn("FHTTG26A69FF").is_err(), "序列号部分不是 hex");
        // 上游 decode_hex 容许 `:` `-`，这里跟着容许
        let sn = parse_gpon_sn("4648:5454-C26A:69FF").expect("带分隔符");
        assert_eq!(gpon_sn_display(&sn), "FHTTC26A69FF");
    }

    /// 随机 SN 的形状要能直接过解析：`FHTT` + 8 位 hex。
    #[test]
    fn random_gpon_sn_round_trips() {
        let sn = random_gpon_sn();
        assert_eq!(&sn[..4], b"FHTT");
        let text = gpon_sn_display(&sn);
        assert_eq!(text.len(), 12, "{text}");
        assert_eq!(parse_gpon_sn(&text).expect("自己生成的必须能解"), sn);
    }

    /// 打包时在 0xA2 写 12 字符视图，`0x38` 仍是 8 字节二进制 —— 两份数据同一含义。
    #[test]
    fn gpon_sn_view_written() {
        let mut a = An4475::new();
        a.gpon_sn = parse_gpon_sn("FHTTC26A69FF").expect("解析");
        let (img, _) = a
            .encode_compact_with(CompactOptions { pon_aponcal: false })
            .expect("打包");
        assert_eq!(
            &img[AN_H_GPON_SN..AN_H_GPON_SN + AN_GPON_SN_SIZE],
            &[0x46, 0x48, 0x54, 0x54, 0xc2, 0x6a, 0x69, 0xff]
        );
        assert_eq!(
            &img[AN_H_GPON_SN_VIEW..AN_H_GPON_SN_VIEW + AN_GPON_SN_VIEW_SIZE],
            b"FHTTC26A69FF"
        );
        let b = parse_compact(&img).expect("读回");
        assert_eq!(b.gpon_sn, a.gpon_sn);
    }

    /// PonWrt 只会改 0xA2 这 12 个字符，读回时必须让它压过 `0x38`。
    #[test]
    fn gpon_sn_view_wins_over_binary() {
        let mut a = An4475::new();
        a.gpon_sn = parse_gpon_sn("FHTTC26A69FF").expect("解析");
        let (mut img, _) = a
            .encode_compact_with(CompactOptions { pon_aponcal: false })
            .expect("打包");
        img[AN_H_GPON_SN_VIEW..AN_H_GPON_SN_VIEW + AN_GPON_SN_VIEW_SIZE]
            .copy_from_slice(b"FHTT99999999");
        // 模拟「0x38 还是旧值、只有视图被改」
        img[AN_H_GPON_SN..AN_H_GPON_SN + AN_GPON_SN_SIZE]
            .copy_from_slice(&[0x46, 0x48, 0x54, 0x54, 0xc2, 0x6a, 0x69, 0xff]);
        let b = parse_compact(&img).expect("读回");
        assert_eq!(b.gpon_sn, parse_gpon_sn("FHTT99999999").expect("解析"));
        let (img2, _) = b
            .encode_compact_with(CompactOptions { pon_aponcal: false })
            .expect("再打包");
        assert_eq!(
            &img2[AN_H_GPON_SN..AN_H_GPON_SN + AN_GPON_SN_SIZE],
            b"FHTT\x99\x99\x99\x99",
            "再打包时 0x38 必须跟随视图"
        );
    }

    /// 老镜像视图区是 0xFF：不许被 lossy 出来的 12 个 U+FFFD 污染，仍按 0x38 读。
    #[test]
    fn gpon_sn_view_ignores_blank() {
        let mut a = An4475::new();
        a.gpon_sn = parse_gpon_sn("FHTTC26A69FF").expect("解析");
        let (mut img, _) = a
            .encode_compact_with(CompactOptions { pon_aponcal: false })
            .expect("打包");
        img[AN_H_GPON_SN_VIEW..AN_H_GPON_SN_VIEW + AN_GPON_SN_VIEW_SIZE].fill(0xff);
        assert!(gpon_sn_view(&img).is_none());
        let b = parse_compact(&img).expect("读回");
        assert_eq!(
            b.gpon_sn,
            parse_gpon_sn("FHTTC26A69FF").expect("解析"),
            "视图为空时应退回 0x38"
        );
    }

    /// 厂商码非 ASCII 时只能给 16 位 hex，视图放不下就留 0xFF，不能截断成假值。
    #[test]
    fn gpon_sn_view_left_blank_for_hex_only() {
        let mut a = An4475::new();
        a.gpon_sn = parse_gpon_sn("0102030405060708").expect("解析");
        let (img, _) = a
            .encode_compact_with(CompactOptions { pon_aponcal: false })
            .expect("打包");
        assert!(gpon_sn_view(&img).is_none());
        assert_eq!(
            &img[AN_H_GPON_SN_VIEW..AN_H_GPON_SN_VIEW + AN_GPON_SN_VIEW_SIZE],
            &[0xff; AN_GPON_SN_VIEW_SIZE]
        );
    }
}
