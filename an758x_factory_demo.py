#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
H3C HM2004-DU factory 工具 (Rust egui 编辑器的 Python 伴生版 / CLI)。

与 src/model.rs::encode() 完全一致地生成「布局映射演示镜像」:
  - 所有字段落在 H3C 真实偏移上
  - 不含 UBI EC/VID 头, AL-MAC 区以明文(非 gzip)写入
  - 仅用于查看/对照, 不可直接刷写 NAND

同时支持从真实 reservearea 备份 (.bin) 解析 SN / AL-MAC / 校准 / EEPROM,
等价于 src/import.rs::parse_factory_volume。

用法:
  # 用已知设备身份生成演示镜像
  python3 an758x_factory_demo.py gen --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a -o factory-h3c-demo.bin

  # 从真实备份解析字段
  python3 an758x_factory_demo.py parse --bin "G:/下载/H3C-HM2004-DU-TELNET备份/mtd14_reservearea.bin"
"""
import argparse
import os
import struct
import sys
import zlib

FACTORY_VOLUME_SIZE = 0x240000
# 布局映射演示镜像大小：最高使用字段向上取整到 4 KiB。
# 下行 PON 校准在 0x1c1000，长 0x120，紧跟其后 6 字节 BOSA 标量。
DEMO_SIZE = 0x1C3000

MAGIC_OFFSET = 0x140000
MAGIC = bytes([0x21, 0x43, 0x34, 0x12])
BASE_MAC_OFFSET = 0x140005
DEVICE_SN_OFFSET = 0x14102C
DEVICE_SN_DUP_OFFSET = 0x14105F
# 槽位物理宽度 16 字节：原厂 mtd14 在 0x14102c 放的是 H3CT0005EBF068CU。
# 但机子标签上的 SN 是 12 字符 H3CT0005EBF0，尾部 68CU 属于厂商附加字段，
# 不是 SN 的一部分，本工具不解析也不生成。
DEVICE_SN_SLOT_SIZE = 0x10
# 实际 SN 长度上限 = 12，与标签以及 0x14105f 处的副本一致。
DEVICE_SN_SIZE = 0x0C
# 副本槽同样是 12 字符短 SN，其后紧跟明文 ctromfile_GUI.cfg，两处上限必须一致。
DEVICE_SN_DUP_SIZE = DEVICE_SN_SIZE
ALMAC_REGION_OFFSET = 0x80000
ALMAC_REGION_SIZE = 0x4000
CALIBRATION_OFFSET = 0x1C0400
CALIBRATION_SIZE = 0x200
WIFI_EEPROM_OFFSET = 0x4C000
WIFI_EEPROM_SIZE = 0x1000

# ---- 下行 PON 口校准（mtd13 / UBIFS `ubifs1` 卷）----
#
# 下行 PON 口的光模块校准不在 factory 分区里，而是在 mtd13 —— 一个 4 MiB 的
# UBI 镜像，内含名为 `ubifs1` 的 UBIFS 动态卷。逆向该卷得到目录树：
#
#   /bosa/OBosaMode              u16
#   /bosa/BosaFlag               u16
#   /bosa/TempFlag               u16
#   /bosa/mmodified_UX3326.bin   0x120 字节 —— 下面这块
#
# mtd13 不属于 ubi-volume-factory，没有真实分区偏移；这里的常量描述的是我们
# 自己的导出格式，原始定位由 parse_mtd13() 完成。
PON_DOWNLINK_CAL_OFFSET = 0x1C1000
PON_DOWNLINK_CAL_SIZE = 0x120
PON_DOWNLINK_FLAGS_SIZE = 0x6

# 1MiB 紧凑布局（自描述，不可刷写）：真实 H3C 身份字段在 0x140000+（超 1MiB），
# 故按文档化偏移重排，magic "H3C-1MiB" 用于解析时自动识别。
COMPACT_SIZE = 0x100000
COMPACT_MAGIC = b"H3C-1MiB"
C_LABEL_MAC_OFFSET = 0x0010
C_ETH0_MAC_OFFSET = 0x0020
C_PON0_MAC_OFFSET = 0x0030
C_WIFI5G_MAC_OFFSET = 0x0040
C_WIFI24_MAC_OFFSET = 0x0050
C_DEVICE_SN_OFFSET = 0x0060
C_WIFI_EEPROM_OFFSET = 0x1000
C_CALIBRATION_OFFSET = 0x2000
C_PON_DOWNLINK_CAL_OFFSET = 0x2200
C_PON_DOWNLINK_FLAGS_OFFSET = 0x2400

# MAC 派生偏移 (末字节)
OFF_ETH0, OFF_PON0, OFF_WIFI_5G, OFF_WIFI_24G = 1, 2, 4, 5

# ===================== HL202-DU（从机，mtd12）=====================
#
# 从机的 reservearea 挂在 mtd12（不是主机的 mtd14），大小同为 0x240000，
# magic 也一样是 21 43 34 12，但内容结构不同：
#
#   0x04c000  Wi-Fi EEPROM, 0x1000      (MT7916D，与主机同偏移同长度)
#   0x080000  gzip(ctromfile.cfg)       (label MAC 在 AL-MAC="..." 内)
#   0x100000  syslog                     (可丢弃)
#   0x140000  GUD 字段索引表
#   0x141010  GUD 出厂字符串, 151 字节   (SN / SSID / 密码 / 日期)
#   0x1c0400  PON BOB 校准, 0xE1 (225)  (EN7571，长度逐台实测)
#
# 与主机最关键的差异：**没有下行 PON 校准**（整分区无 UX3326/OBosaMode 等），
# 所以从机不需要 mtd13。
HL_FACTORY_SIZE = 0x240000
HL_MAGIC_OFFSET = 0x140000
HL_MAGIC = bytes([0x21, 0x43, 0x34, 0x12])
HL_GUD_OFFSET = 0x141010
HL_GUD_SIZE = 151
HL_SN_OFFSET = 0x14102C
HL_SN_DUP_OFFSET = 0x14105F
HL_SN_SIZE = 0x0C
HL_ALMAC_OFFSET = 0x80000
HL_ALMAC_SIZE = 0x4000
HL_EEPROM_OFFSET = 0x4C000
HL_EEPROM_SIZE = 0x1000
HL_BOB_OFFSET = 0x1C0400
HL_BOB_SIZE = 0xE1

# 从机专属 512 KiB 紧凑布局，magic 与主机不同，两套互不通用。
# 有效数据只到 0x20E1（EEPROM 4 KiB + BOB 225 B），512 KiB 已有 8 倍余量。
HL_COMPACT_SIZE = 0x80000
HL_COMPACT_MAGIC = b"HL2-512K"
# 旧版 1 MiB 镜像的 magic，仅用于读取兼容（偏移与本版一致，仍可解析）。
HL_COMPACT_MAGIC_LEGACY = b"HL2-1MiB"
HL_C_LABEL_MAC_OFFSET = 0x0010
HL_C_SN_OFFSET = 0x0020
# 派生 MAC 四连，偏移与主机一致（末字节 +1/+2/+4/+5）。
# 旧版镜像这里是全 0xFF，读取时忽略、显示时现算。
HL_C_ETH0_MAC_OFFSET = 0x0030
HL_C_PON0_MAC_OFFSET = 0x0040
HL_C_WIFI5G_MAC_OFFSET = 0x0050
HL_C_WIFI24_MAC_OFFSET = 0x0060
HL_C_EEPROM_OFFSET = 0x1000
HL_C_BOB_OFFSET = 0x2000

# GUD 字符串区字段边界（实测）。注意 ctromfile_GUD.cfg 是 17 字符而非 16，
# 所以它之后的字段比常见资料写的偏移多 1 字节。
HL_GUD_FIELDS = [
    ("GUDID",     0x141010, 28),
    ("SN",        0x14102C, 12),
    ("reserved",  0x141038, 2),
    ("SSID24",    0x14103A, 7),
    ("PW24",      0x141041, 8),
    ("SSID5G",    0x141049, 10),
    ("PW5G",      0x141053, 8),
    ("HW",        0x14105B, 4),
    ("SNdup",     0x14105F, 12),
    ("cfgname",   0x14106B, 17),
    ("PWdup",     0x14107C, 8),
    ("key",       0x141084, 12),
    ("PONPW",     0x141090, 10),
    ("flag",      0x14109A, 3),
    ("date",      0x14109D, 10),
]


def parse_mac(s):
    s = s.strip()
    if ":" in s:
        parts = s.split(":")
    elif "-" in s:
        parts = s.split("-")
    elif len(s) == 12:
        parts = [s[i:i + 2] for i in range(0, 12, 2)]
    else:
        return None
    if len(parts) != 6:
        return None
    try:
        m = bytes(int(p, 16) for p in parts)
    except ValueError:
        return None
    if m == b"\x00" * 6 or m == b"\xff" * 6:
        return None
    if m[0] & 0x01:
        return None
    return m


def mac_to_string(m):
    return ":".join(f"{b:02X}" for b in m)


def derive_mac(base, off):
    m = bytearray(base)
    m[5] = (m[5] + off) & 0xFF
    return bytes(m)


# 厂商名（无数字、不代表芯片），识别型号时跳过
VENDOR_TOKENS = ("ECONET",)


def _take_token(run):
    s = bytes(run).decode("ascii", "ignore").strip()
    run.clear()
    if len(s) < 4:
        return None
    has_alpha = any(c.isalpha() for c in s)
    has_digit = any(c.isdigit() for c in s)
    if has_alpha and has_digit and s.upper() not in VENDOR_TOKENS:
        return s
    return None


def detect_pon_chip(cal):
    """从校准 blob 识别 PON 前端芯片（硬件由导入数据决定）。

    - FiberHome/APONCAL 风格：0x0c 处 u32 LE 芯片 ID（1=GN28L95, 2=UX3363）
    - H3C 自有格式：blob 内填充的 ASCII 型号名（实测 0x1c0428 = "EN7572"）
    - 两者都不是 -> None（未设置 / 未识别）
    """
    if not cal or all(b == 0xFF for b in cal):
        return None
    if cal[:8] == b"APONCAL\x00" and len(cal) >= 0x10:
        cid = int.from_bytes(cal[0x0C:0x10], "little")
        return {1: "GN28L95", 2: "UX3363"}.get(cid, "未知芯片 #%d" % cid)
    run = bytearray()
    for b in list(cal) + [0x0A]:
        if 0x20 <= b < 0x7F:
            run.append(b)
        else:
            t = _take_token(run)
            if t:
                return t
    return None


# ---- 下行 PON 口校准：mtd13 解析 -------------------------------------------------
#
# mtd13 是 4 MiB 的 UBI 镜像（32 个 PEB，每个 0x20000），内含 UBIFS 动态卷
# `ubifs1`。厂商魔改了 UBIFS 节点头布局（与标准不符，CRC 也对不上，故不做
# 校验），实测为：
#
#   +0x00 magic u32 = 0x06101831
#   +0x04 crc   u32（不可校验，跳过）
#   +0x08 sqnum  u64（越大越新）
#   +0x10 len    u32
#   +0x14 node_type u8（1=LEAF/DATA，2=DENT，7=CMT 提交节点）
#   +0x15 group_type u8
#   +0x18 ino    u32
#   +0x28 nodelen u32
#   +0x30 payload
#
# 关键点：同一 inode 会有多个副本（UBIFS 的日志语义），必须按 sqnum 取最新，
# 而不是物理位置最靠后的那一份。

UBIFS_NODE_MAGIC = 0x06101831
UBIFS_NODE_HDR = 0x30
UBIFS_NODE_LEAF = 1
MTD13_SIZE = 0x400000
INO_UX3326_BIN = 0x45
INO_OBOSA_MODE = 0x42
INO_BOSA_FLAG = 0x43
INO_TEMP_FLAG = 0x44

# UX3326 接收灵敏度 LUT：84 项 u16，**大端**，位于 blob +0x70。
# 该结论在本 mtd13 的全部 10 个 ino 0x45 副本上一致。
PON_LUT_ENTRIES = 84
PON_LUT_OFFSET = 0x70
PON_LUT_SEARCH_FROM = 0x60


def looks_like_mtd13(data):
    return len(data) >= MTD13_SIZE and data[:4] == b"UBI#"


def _scan_ubifs_leaves(data):
    """扫描 UBIFS LEAF 节点，同一 ino 按 sqnum 取最新。"""
    best = {}
    for off in range(0, len(data) - UBIFS_NODE_HDR, 4):
        if int.from_bytes(data[off:off + 4], "little") != UBIFS_NODE_MAGIC:
            continue
        # 必须过滤 node_type==LEAF：CMT(7) 提交节点的载荷会被误读成 ino/nodelen，
        # 凭空造出几百条假记录并盖掉真实数据。
        if data[off + 0x14] != UBIFS_NODE_LEAF:
            continue
        sqnum = int.from_bytes(data[off + 0x08:off + 0x10], "little")
        ino = int.from_bytes(data[off + 0x18:off + 0x1C], "little")
        nodelen = int.from_bytes(data[off + 0x28:off + 0x2C], "little")
        # 厂商魔改：nodelen 是 payload 长度（不含 0x30 头部），故上界按
        # off + 0x30 + nodelen 判断，而不是 off + nodelen。
        if nodelen > MTD13_SIZE or off + UBIFS_NODE_HDR + nodelen > len(data):
            continue
        prev = best.get(ino)
        if prev is None or sqnum > prev[0]:
            best[ino] = (sqnum, off, data[off + UBIFS_NODE_HDR:off + UBIFS_NODE_HDR + nodelen])
    return best


def _u16(payload):
    return int.from_bytes(payload[:2], "little") if payload and len(payload) >= 2 else None


def parse_mtd13(data):
    """从 mtd13 提取下行 PON 校准。

    返回 (cal_blob, flags_dict, source_offset)；非 mtd13 或找不到时返回 None。
    """
    if not looks_like_mtd13(data):
        return None
    nodes = _scan_ubifs_leaves(data)
    cal = nodes.get(INO_UX3326_BIN)
    if cal is None or len(cal[2]) < PON_DOWNLINK_CAL_SIZE:
        return None
    flags = {
        "OBosaMode": _u16(nodes.get(INO_OBOSA_MODE, (0, 0, b""))[2]),
        "BosaFlag": _u16(nodes.get(INO_BOSA_FLAG, (0, 0, b""))[2]),
        "TempFlag": _u16(nodes.get(INO_TEMP_FLAG, (0, 0, b""))[2]),
    }
    return cal[2][:PON_DOWNLINK_CAL_SIZE], flags, cal[1]


def pack_pon_downlink_flags(flags):
    """flags dict -> 6 字节（OBosaMode/BosaFlag/TempFlag，各 u16 LE）。"""
    out = bytearray()
    for k in ("OBosaMode", "BosaFlag", "TempFlag"):
        out += (int(flags.get(k) or 0) & 0xFFFF).to_bytes(2, "little")
    return bytes(out)


def unpack_pon_downlink_flags(raw):
    if not raw or len(raw) < PON_DOWNLINK_FLAGS_SIZE:
        return None
    return {
        "OBosaMode": int.from_bytes(raw[0:2], "little"),
        "BosaFlag": int.from_bytes(raw[2:4], "little"),
        "TempFlag": int.from_bytes(raw[4:6], "little"),
    }


def _is_sensitivity_lut(values):
    if len(values) < PON_LUT_ENTRIES or values[0] == 0 or 0xFFFF in values:
        return False
    if not all(values[i] <= values[i + 1] for i in range(len(values) - 1)):
        return False
    # 尾部必须真的在爬升，否则常量填充（如 0x5a）会被误判成真实光模块
    tail = values[-(PON_LUT_ENTRIES - 8):]
    return any(tail[i] < tail[i + 1] for i in range(len(tail) - 1))


def detect_downlink_chip(cal):
    """从下行校准 blob 识别光模块（实测 UX3326）。"""
    if not cal or len(cal) < PON_DOWNLINK_CAL_SIZE or all(b == 0xFF for b in cal):
        return None
    n = PON_LUT_ENTRIES
    for start in range(PON_LUT_SEARCH_FROM, PON_DOWNLINK_CAL_SIZE - n * 2 + 1, 2):
        vals = [int.from_bytes(cal[start + i * 2:start + i * 2 + 2], "big")
                for i in range(n)]
        if _is_sensitivity_lut(vals):
            return "UX3326"
    return None


def encode(sn, label_mac, calibration=None, eeprom=None, downlink=None, downlink_flags=None):
    img = bytearray(b"\xff" * DEMO_SIZE)
    img[MAGIC_OFFSET:MAGIC_OFFSET + 4] = MAGIC
    img[BASE_MAC_OFFSET:BASE_MAC_OFFSET + 6] = label_mac
    # 只写标签上的 12 字符 SN；0x14102c 槽位尾部 4 字节是厂商字段，留 0xFF。
    snb = sn.encode("ascii")[:DEVICE_SN_SIZE]
    img[DEVICE_SN_OFFSET:DEVICE_SN_OFFSET + len(snb)] = snb
    # 副本槽同为 12 字符，其后是明文 ctromfile_GUI.cfg，绝不能多写。
    dnb = sn.encode("ascii")[:DEVICE_SN_DUP_SIZE]
    img[DEVICE_SN_DUP_OFFSET:DEVICE_SN_DUP_OFFSET + len(dnb)] = dnb
    block = (
        '<ctromfile>\n  <mapcfg AL-MAC="%s" />\n'
        '  <sysinfo SerialNumber="%s" />\n</ctromfile>\n'
        % (mac_to_string(label_mac), sn)
    ).encode("utf-8")
    img[ALMAC_REGION_OFFSET:ALMAC_REGION_OFFSET + len(block)] = block
    if calibration:
        img[CALIBRATION_OFFSET:CALIBRATION_OFFSET + len(calibration)] = calibration
    if eeprom:
        img[WIFI_EEPROM_OFFSET:WIFI_EEPROM_OFFSET + len(eeprom)] = eeprom
    _write_downlink(img, PON_DOWNLINK_CAL_OFFSET, downlink, downlink_flags)
    return bytes(img)


def _write_downlink(img, base, downlink, downlink_flags, flags_offset=None):
    """写入下行 PON 校准 blob 与 3×u16 BOSA 标量。

    `flags_offset` 省略时标量紧贴 blob 尾部（真实偏移布局）；紧凑布局传入
    独立的对齐偏移。
    """
    if downlink:
        blob = bytes(downlink)[:PON_DOWNLINK_CAL_SIZE]
        img[base:base + len(blob)] = blob
    if downlink_flags is not None:
        fb = pack_pon_downlink_flags(downlink_flags)
        at = base + PON_DOWNLINK_CAL_SIZE if flags_offset is None else flags_offset
        img[at:at + len(fb)] = fb


def encode_1mib(sn, label_mac, calibration=None, eeprom=None, downlink=None,
                downlink_flags=None):
    """1MiB 紧凑镜像：全部字段按文档化偏移重排进 1MiB，含 5 条 MAC 表。"""
    img = bytearray(b"\xff" * COMPACT_SIZE)
    img[0:len(COMPACT_MAGIC)] = COMPACT_MAGIC
    for off, mac in (
        (C_LABEL_MAC_OFFSET, label_mac),
        (C_ETH0_MAC_OFFSET, derive_mac(label_mac, OFF_ETH0)),
        (C_PON0_MAC_OFFSET, derive_mac(label_mac, OFF_PON0)),
        (C_WIFI5G_MAC_OFFSET, derive_mac(label_mac, OFF_WIFI_5G)),
        (C_WIFI24_MAC_OFFSET, derive_mac(label_mac, OFF_WIFI_24G)),
    ):
        img[off:off + 6] = mac
    snb = sn.encode("ascii")[:DEVICE_SN_SIZE]
    img[C_DEVICE_SN_OFFSET:C_DEVICE_SN_OFFSET + len(snb)] = snb
    if eeprom:
        img[C_WIFI_EEPROM_OFFSET:C_WIFI_EEPROM_OFFSET + len(eeprom)] = eeprom
    if calibration:
        img[C_CALIBRATION_OFFSET:C_CALIBRATION_OFFSET + len(calibration)] = calibration
    # 紧凑布局里 flags 有独立的对齐偏移（0x2400），而非紧贴 blob
    _write_downlink(img, C_PON_DOWNLINK_CAL_OFFSET, downlink, downlink_flags,
                    flags_offset=C_PON_DOWNLINK_FLAGS_OFFSET)
    return bytes(img)


def _find_subseq(hay, needle):
    return hay.find(needle)


def _find_al_mac(buf):
    tag = b'AL-MAC="'
    from_ = 0
    while from_ + len(tag) + 17 <= len(buf):
        rel = buf.find(tag, from_)
        if rel < 0:
            break
        at = rel + len(tag)
        m = parse_mac(buf[at:at + 17].decode("utf-8", "ignore"))
        if m:
            return m
        from_ = at


def read_almac_bytes(data):
    """AL-MAC 区内容：真实备份为 gzip ctromfile(解压后)，演示镜像为明文原文。

    用 decompressobj 而非 zlib.decompress：前者在 gzip 成员结束处停下，
    不会因为成员后紧随的 0xff 填充而报错。
    """
    if len(data) <= ALMAC_REGION_OFFSET:
        return None
    region = data[ALMAC_REGION_OFFSET:]
    idx = region.find(b"\x1f\x8b\x08")
    if idx >= 0:
        d = zlib.decompressobj(16 + zlib.MAX_WBITS)
        try:
            out = d.decompress(region[idx:]) + d.flush()
        except Exception:
            out = b""
        if out:
            return out
    return region


def _find_serial_number(buf):
    tag = b'SerialNumber="'
    i = buf.find(tag)
    if i < 0:
        return None
    at = i + len(tag)
    j = buf.find(b'"', at)
    if j < 0 or j == at:
        return None
    return buf[at:j].decode("utf-8", "ignore")


def read_label_mac(data):
    text = read_almac_bytes(data)
    if text:
        m = _find_al_mac(text)
        if m:
            return m
    if len(data) >= BASE_MAC_OFFSET + 6:
        m = data[BASE_MAC_OFFSET:BASE_MAC_OFFSET + 6]
        if any(b not in (0xFF, 0x00) for b in m):
            return bytes(m)
    return None


def parse_compact(data):
    """解析 encode_1mib 生成的 1MiB 紧凑镜像。"""
    label = data[C_LABEL_MAC_OFFSET:C_LABEL_MAC_OFFSET + 6]
    label = bytes(label) if any(b != 0xFF for b in label) else None
    sn = ""
    if len(data) >= C_DEVICE_SN_OFFSET + DEVICE_SN_SIZE:
        sn = data[C_DEVICE_SN_OFFSET:C_DEVICE_SN_OFFSET + DEVICE_SN_SIZE].decode(
            "utf-8", "ignore"
        ).strip("\x00 ").strip()
    cal = data[C_CALIBRATION_OFFSET:C_CALIBRATION_OFFSET + CALIBRATION_SIZE]
    eep = data[C_WIFI_EEPROM_OFFSET:C_WIFI_EEPROM_OFFSET + WIFI_EEPROM_SIZE]
    dn = data[C_PON_DOWNLINK_CAL_OFFSET:C_PON_DOWNLINK_CAL_OFFSET + PON_DOWNLINK_CAL_SIZE]
    dnf = unpack_pon_downlink_flags(
        data[C_PON_DOWNLINK_FLAGS_OFFSET:C_PON_DOWNLINK_FLAGS_OFFSET + PON_DOWNLINK_FLAGS_SIZE]
    )
    return sn, label, cal, eep, dn, dnf


# ===================== HL202-DU（从机）解析 / 打包 =====================

def looks_like_hl202_volume(data):
    """判断一个 2.25 MB dump 是不是 HL202-DU 的 mtd12。

    不能只看 magic —— 主机 mtd14 也是 21 43 34 12 @0x140000、SN 也在 0x14102c。
    可靠判据是 GUD 区里的"配置名"：主机是 ctromfile_GUI.cfg，从机是
    ctromfile_GUD.cfg。兜底再看上行校准区有没有明文 ECONET（主机有，从机没有）。
    """
    if len(data) < HL_MAGIC_OFFSET + 4:
        return False
    if data[HL_MAGIC_OFFSET:HL_MAGIC_OFFSET + 4] != HL_MAGIC:
        return False
    gud = data[HL_GUD_OFFSET:HL_GUD_OFFSET + 0x100]
    # 注意 _find_subseq 未命中返回 -1（不是 None），必须用 >= 0 判断。
    if _find_subseq(gud, b"ctromfile_GUD.cfg") >= 0:
        return True
    if _find_subseq(gud, b"ctromfile_GUI.cfg") >= 0:
        return False
    return data[HL_BOB_OFFSET + 0x14:HL_BOB_OFFSET + 0x1A] != b"ECONET"


def _hl_read_sn(data):
    if len(data) < HL_SN_OFFSET + HL_SN_SIZE:
        return ""
    return data[HL_SN_OFFSET:HL_SN_OFFSET + HL_SN_SIZE].decode(
        "utf-8", "ignore"
    ).strip("\x00 ").strip()


def parse_hl202_volume(data):
    """解析 HL202-DU 的 mtd12，返回 (sn, label_mac, bob, eeprom)。"""
    sn = _hl_read_sn(data)
    alm = read_almac_bytes(data)
    if not sn and alm:
        found = _find_serial_number(alm)
        if found:
            sn = found
    label = None
    if alm:
        label = _find_al_mac(alm)
    if not label:
        label = read_label_mac(data)
    bob = data[HL_BOB_OFFSET:HL_BOB_OFFSET + HL_BOB_SIZE]
    eep = data[HL_EEPROM_OFFSET:HL_EEPROM_OFFSET + HL_EEPROM_SIZE]
    return sn, label, bob, eep


def is_hl202_compact(data):
    """一段数据是不是从机的紧凑镜像（新版 512 KiB 或旧版 1 MiB）。"""
    return data[:8] in (HL_COMPACT_MAGIC, HL_COMPACT_MAGIC_LEGACY)


def derive_macs(label_mac):
    """四个派生接口 MAC：(eth0, pon0, 5G, 2.4G)，末字节分别 +1/+2/+4/+5。"""
    return tuple(derive_mac(label_mac, off) for off in (OFF_ETH0, OFF_PON0, OFF_WIFI_5G, OFF_WIFI_24G))


def encode_hl202_512k(sn, label_mac, bob=None, eeprom=None):
    """生成从机专属 512 KiB 紧凑镜像（magic `HL2-512K`，与主机不通用）。"""
    img = bytearray(b"\xff" * HL_COMPACT_SIZE)
    img[0:len(HL_COMPACT_MAGIC)] = HL_COMPACT_MAGIC
    for off, mac in zip(
        (HL_C_LABEL_MAC_OFFSET, HL_C_ETH0_MAC_OFFSET, HL_C_PON0_MAC_OFFSET,
         HL_C_WIFI5G_MAC_OFFSET, HL_C_WIFI24_MAC_OFFSET),
        (label_mac, *derive_macs(label_mac)),
    ):
        img[off:off + 6] = mac
    snb = sn.encode("ascii")[:HL_SN_SIZE]
    img[HL_C_SN_OFFSET:HL_C_SN_OFFSET + len(snb)] = snb
    if eeprom:
        n = min(len(eeprom), HL_EEPROM_SIZE)
        img[HL_C_EEPROM_OFFSET:HL_C_EEPROM_OFFSET + n] = eeprom[:n]
    if bob:
        n = min(len(bob), HL_BOB_SIZE)
        img[HL_C_BOB_OFFSET:HL_C_BOB_OFFSET + n] = bob[:n]
    return bytes(img)


def parse_hl202_compact(data):
    """解析 HL2-512K / 旧版 HL2-1MiB 紧凑镜像，返回 (sn, label_mac, bob, eeprom)。"""
    label = data[HL_C_LABEL_MAC_OFFSET:HL_C_LABEL_MAC_OFFSET + 6]
    label = bytes(label) if any(b != 0xFF for b in label) else None
    sn = ""
    if len(data) >= HL_C_SN_OFFSET + HL_SN_SIZE:
        sn = data[HL_C_SN_OFFSET:HL_C_SN_OFFSET + HL_SN_SIZE].decode(
            "utf-8", "ignore"
        ).strip("\x00 ").strip()
    bob = data[HL_C_BOB_OFFSET:HL_C_BOB_OFFSET + HL_BOB_SIZE]
    eep = data[HL_C_EEPROM_OFFSET:HL_C_EEPROM_OFFSET + HL_EEPROM_SIZE]
    return sn, label, bob, eep


def read_hl_gud(data):
    """读出 GUD 字符串区所有字段（只读展示，不参与导出）。"""
    out = []
    for name, off, ln in HL_GUD_FIELDS:
        raw = data[off:off + ln]
        out.append((name, raw.decode("utf-8", "ignore").strip("\x00 ").strip()))
    return out


def parse_volume(data):
    # mtd13：下行 PON 校准的真实来源，直接走 UBIFS 解析
    if looks_like_mtd13(data):
        sn, label, cal, eep = "", None, b"", b""
        parsed = parse_mtd13(data)
        if parsed:
            dn, dnf, _ = parsed
        else:
            dn, dnf = b"", None
        return sn, label, cal, eep, dn, dnf
    # 1MiB 紧凑镜像自动识别
    if data[:8] == COMPACT_MAGIC:
        return parse_compact(data)
    sn = ""
    if len(data) >= DEVICE_SN_OFFSET + DEVICE_SN_SIZE:
        sn = data[DEVICE_SN_OFFSET:DEVICE_SN_OFFSET + DEVICE_SN_SIZE].decode(
            "utf-8", "ignore"
        ).strip("\x00 ").strip()
    # 0x14102c 槽位可能为空，回退到 ctromfile 里的 SerialNumber
    if not sn:
        text = read_almac_bytes(data)
        if text:
            found = _find_serial_number(text)
            if found:
                sn = found
    label = read_label_mac(data)
    cal = data[CALIBRATION_OFFSET:CALIBRATION_OFFSET + CALIBRATION_SIZE] if len(data) >= CALIBRATION_OFFSET + CALIBRATION_SIZE else b""
    eep = data[WIFI_EEPROM_OFFSET:WIFI_EEPROM_OFFSET + WIFI_EEPROM_SIZE] if len(data) >= WIFI_EEPROM_OFFSET + WIFI_EEPROM_SIZE else b""
    dn = b""
    dnf = None
    if len(data) >= PON_DOWNLINK_CAL_OFFSET + PON_DOWNLINK_CAL_SIZE:
        dn = data[PON_DOWNLINK_CAL_OFFSET:PON_DOWNLINK_CAL_OFFSET + PON_DOWNLINK_CAL_SIZE]
        dnf = unpack_pon_downlink_flags(
            data[PON_DOWNLINK_CAL_OFFSET + PON_DOWNLINK_CAL_SIZE:
                 PON_DOWNLINK_CAL_OFFSET + PON_DOWNLINK_CAL_SIZE + PON_DOWNLINK_FLAGS_SIZE]
        )
    return sn, label, cal, eep, dn, dnf


# ===========================================================================
# AN4475：mtd9（UBI 卷 fhdata / UBIFS）→ 2 MiB 紧凑容器
# ===========================================================================
#
# 真机 mtd9 是一整个 UBI 设备（PEB 128 KiB，共 200 个 = 25 MiB），里面只有
# 一个动态卷 `fhdata`（178 PEB ≈ 22.25 MiB），卷上是 UBIFS，装着 21 个出厂
# 文件（另有 15 个 UCI 临时文件 `.factory_conf.uci-XXXXXX`，属写入中间态，
# 不入镜像）。
#
# 与 H3C 两台机型最大的不同：AN4475 的出厂数据**全部是文本 UCI + 定长表**，
# SN/MAC 都在 `factory_conf` 里明文躺着，不需要从 AL-MAC 里反推。但也因此
# 单机唯一数据（12 套 MAC、GPON SN/密码、BOSA 校准表、MT7993 EEPROM）体积
# 达到 1.4 MB，其中 EEPROM 一个文件就 1.32 MiB —— 超过 1 MiB 镜像容量，
# 所以镜像采用「头部身份 + TOC + 载荷」的容器结构。EEPROM 与 11 张 PON 校准
# 表都放固定偏移、原样不压缩（可直接 hex 直读），只有 9 个配置文件仍 zlib。
# 容器长度按 2 MiB 规划（magic `AN4-2MiB`），旧版 1 MiB 容器仍可读入。

AN_UBI_PEB = 0x20000
AN_UBI_VID_OFF = 0x800
AN_UBI_DATA_OFF = 0x1000
AN_UBIFS_MAGIC = b"\x31\x18\x10\x06"

AN_COMPACT_SIZE = 0x200000
AN_COMPACT_MAGIC = b"AN4-2MiB"
AN_COMPACT_MAGIC_V1 = b"AN4-1MiB"      # 旧版 1 MiB 容器，仅用于读取兼容
AN_COMPACT_SIZE_V1 = 0x100000
# 2 MiB 容器里 EEPROM 的固定偏移（不压缩，方便直接用 hex 工具看校准区）。
AN_EEPROM_FIXED_OFF = 0x40000
# PON 校准表直读区：0x1A0000 起，每张 4 KiB 槽，原样不压缩。
AN_CAL_FIXED_OFF = 0x1A0000
AN_CAL_SLOT = 0x1000
AN_CAL_TABLES = [
    "10ge_asym_table", "10ge_sym_table",
    "xg_table", "xgs_table",
    "table", "olt_table",
    "olt_bosa_apd_WTD01", "olt_bosa_apd_YS01",
    "olt_bosa_info", "olt_bosa_rssi",
    "LanOpticTxCalib",
]


def cal_offset(name):
    """校准表名 -> 直读区偏移；不在槽表里返回 None（走压缩区）。"""
    if name in AN_CAL_TABLES:
        return AN_CAL_FIXED_OFF + AN_CAL_TABLES.index(name) * AN_CAL_SLOT
    return None


# mt76 直读槽：紧接 11 张 PON 校准表之后（0x1AB000）。
AN_MT76_FIXED_OFF = AN_CAL_FIXED_OFF + len(AN_CAL_TABLES) * AN_CAL_SLOT
AN_MT76_SIZE = 0x1E00


def compact_mt76(data):
    """从容器尾部取出随包存放的 mt76 EEPROM；旧镜像返回 None。"""
    if len(data) < AN_MT76_FIXED_OFF + AN_MT76_SIZE:
        return None
    s = data[AN_MT76_FIXED_OFF:AN_MT76_FIXED_OFF + AN_MT76_SIZE]
    if s[:2] == b"\x93\x79" and any(b != 0xFF for b in s):
        return bytes(s)
    return None
AN_COMPACT_VERSION = 1
AN_TOC_OFF = 0x1000
AN_PAYLOAD_OFF = 0x2000
AN_TOC_ENTRY = 48
AN_NAME_MAX = 32
AN_MAX_FILES = (AN_PAYLOAD_OFF - AN_TOC_OFF) // AN_TOC_ENTRY

AN_SN_SIZE = 17
AN_SN_CAP = 24
AN_GPON_SN_SIZE = 8
AN_GPON_PW_SIZE = 10

AN_H_SN = 0x20
AN_H_GPON_SN = 0x38
AN_H_BASE_MAC = 0x40
AN_H_ETH_MAC = 0x46
AN_H_OUI = 0x4C
AN_H_VENDOR = 0x4F
AN_H_GPON_PW = 0x51
AN_H_MACS = 0x60
# 0x60 + 9×6 = 0x96，Wi-Fi 两位 MAC 紧接其后，到 0x1000 前还有很大余量。
AN_H_WIFI_2G4 = 0x96
AN_H_WIFI_5G = 0x9C

# 城市宽带（CT）ONU 的 MAC 分配：基准 MAC 往上 +1/+2/+3/+8 依次是上网、
# TR-069、VoIP、私有协议；LAN 口另取一个基址（真机上与基准 MAC 差
# 0x0100_0021，属另一段预留），eth1..eth5 在自身基址上 +0..+4。
AN_DERIVED = (
    ("internetmac", 1),
    ("tr069mac", 2),
    ("voipmac", 3),
    ("priprotocolmac", 8),
)
AN_ETH_NAMES = ("eth1mac", "eth2mac", "eth3mac", "eth4mac", "eth5mac")
AN_MAIN_CONF = "factory_conf"

# Wi-Fi 校准文件里也带 MAC。MT7993 的 EEPROM 头很短：device id + 版本 + 两个
# MAC，之后 0x10..0x190 全 0，0x190 起才是射频校准。出厂值仍是芯片厂默认
# （2.4G/5G 分别 00:0C:43:26:60:10 / :11，00:0C:43 是 MediaTek 的 OUI），
# 说明产线没写这两位，得由本工具补。
AN_WLAN_EEPROM = "MT7993_EEPROM.bin"
AN_EEPROM_DEVICE_ID = 0x00          # u16 LE，MT7990AN = 0x7993
AN_EEPROM_VERSION = 0x02            # u16 LE，实测 2
AN_EEPROM_MAC_2G4 = 0x04            # 6 字节
AN_EEPROM_MAC_5G = 0x0A             # 6 字节
AN_EEPROM_MAC_AREA = 0x10           # 0x10 起为 0 填充，MAC 区到此为止

# factory_conf 里对应的两段。EEPROM 与应用层各存一份，必须一起改，
# 否则驱动取哪个说不准。
AN_APMAC_2G4 = "APMAC_1"
AN_APMAC_5G = "APMAC_2"

# 本工具约定的 Wi-Fi MAC 偏移（真机未写，无出厂实证，可在界面改）。
AN_WIFI_OFFSET_2G4 = 6
AN_WIFI_OFFSET_5G = 7


def _mac_add(mac, n):
    v = int.from_bytes(mac, "big") + n
    return (v & 0xFFFFFFFFFFFF).to_bytes(6, "big")


def an4475_derive(base_mac, eth_mac):
    """按真机规律派生：网络侧 +1/+2/+3/+8，LAN 侧 +0..+4。"""
    net = [(name, _mac_add(base_mac, off)) for name, off in AN_DERIVED]
    lan = [(name, _mac_add(eth_mac, i)) for i, name in enumerate(AN_ETH_NAMES)]
    return net + lan


def an4475_wifi_defaults(base_mac):
    """Wi-Fi 两位 MAC 的默认值。

    真机出厂时 EEPROM 和 factory_conf 里都是芯片厂/OEM 占位值，没有可参照的
    真实值，所以这里按基准 MAC 往后取两个未被占用的偏移（+6/+7）。纯约定，
    界面上可改。
    """
    return (_mac_add(base_mac, AN_WIFI_OFFSET_2G4),
            _mac_add(base_mac, AN_WIFI_OFFSET_5G))


def an4475_eeprom_device_id(eeprom):
    """EEPROM 头 2 字节小端 device id，MT7990AN 为 0x7993。"""
    if len(eeprom) < 2:
        return 0
    return int.from_bytes(eeprom[:2], "little")


def an4475_eeprom_macs(eeprom):
    """读 EEPROM 里的 (2.4G, 5G) MAC；太短或全 0 时返回 None。"""
    if len(eeprom) < AN_EEPROM_MAC_5G + 6:
        return None
    a = bytes(eeprom[AN_EEPROM_MAC_2G4:AN_EEPROM_MAC_2G4 + 6])
    b = bytes(eeprom[AN_EEPROM_MAC_5G:AN_EEPROM_MAC_5G + 6])
    if a in (b"\x00" * 6, b"\xff" * 6):
        return None
    return a, b


def an4475_eeprom_set_macs(eeprom, mac_2g4, mac_5g):
    """写回 EEPROM 的两位 MAC，其余字节（含校准表）原样保留。"""
    if len(eeprom) < AN_EEPROM_MAC_5G + 6:
        raise ValueError("Wi-Fi EEPROM 只有 %d 字节，放不下 MAC 头" % len(eeprom))
    buf = bytearray(eeprom)
    buf[AN_EEPROM_MAC_2G4:AN_EEPROM_MAC_2G4 + 6] = mac_2g4
    buf[AN_EEPROM_MAC_5G:AN_EEPROM_MAC_5G + 6] = mac_5g
    return bytes(buf)


# ---- mt76（开源驱动）视角：定长 7 680 B ------------------------------------
#
# 内核 mt7996/eeprom.h 里 __MT_EE_MAX = 0x1DFF，mt7996.h 里
# MT7996_EEPROM_SIZE = 7680 —— mt76 无论走 efuse 还是外部 EEPROM 都只读
# 这么长，/lib/firmware 里的默认 bin 也必须是 7680 B。
# 厂商 MT7993_EEPROM.bin 是 1 353 728 B，是同一份数据的超集：
#   0x0000-0x0FFF  小 EE 头，两边布局一致（实测只差 10 字节）
#   0x1000-0x1DFF  mt76 放 byrate 功率表 / RATE_DELTA；厂商放自己的位图
#   0x1E00-0x14A7FF mt_wifi 私有校准区，mt76 不读
MT76_EE_SIZE = 0x1E00
MT76_HEAD_SIZE = 0x1000
MT76_DEFAULT_IPA_PATH = os.path.join(
    os.path.dirname(os.path.abspath(__file__)), "assets", "mt7990_eeprom_2i5i.bin")
MT_EE_CHIP_ID = 0x000
MT_EE_VERSION = 0x002
MT_EE_MAC_ADDR = 0x004
MT_EE_MAC_ADDR2 = 0x00A
MT_EE_WIFI_CONF = 0x190
MT_EE_MAC_ADDR3 = 0x2C0
MT_EE_TX0_POWER_2G = 0x1300
MT_EE_RATE_DELTA_2G = 0x1400
MT76_FEM_NAMES = {0: "内建 PA/LNA（2i5i）", 3: "外置 PA/LNA（2e5e）"}


def mt76_pa_lna(eeprom):
    """PA/LNA 变体：WIFI_CONF+6 的低 2 位（MT_EE_WIFI_PA_LNA_CONFIG）。"""
    if len(eeprom) < MT_EE_WIFI_CONF + 7:
        return None
    return eeprom[MT_EE_WIFI_CONF + 6] & 0x3


_MT76_TEMPLATE_CACHE = None


def _mt76_template():
    """官方 iPA 模板（缓存，批量生成时不用反复读盘）。"""
    global _MT76_TEMPLATE_CACHE
    if _MT76_TEMPLATE_CACHE is None:
        with open(MT76_DEFAULT_IPA_PATH, "rb") as f:
            _MT76_TEMPLATE_CACHE = f.read()
    return _MT76_TEMPLATE_CACHE


def mt76_carve(eeprom):
    """原样截取：厂商文件前 7 680 B。保真，但功率区是厂商编码，mt76 读不动。"""
    out = bytearray(b"\xff" * MT76_EE_SIZE)
    n = min(len(eeprom), MT76_EE_SIZE)
    out[:n] = eeprom[:n]
    return bytes(out)


def mt76_from_template(template, eeprom):
    """官方 7 680 B 模板打底，只覆盖芯片 ID/版本/两个 MAC/WIFI_CONF/PA-LNA。"""
    if len(template) < MT76_EE_SIZE:
        raise ValueError("模板只有 %d 字节，不足 %d" % (len(template), MT76_EE_SIZE))
    if len(eeprom) < MT_EE_WIFI_CONF + 8:
        raise ValueError("实机 EEPROM 只有 %d 字节，读不到 WIFI_CONF" % len(eeprom))
    out = bytearray(template[:MT76_EE_SIZE])
    out[MT_EE_CHIP_ID:MT_EE_CHIP_ID + 2] = eeprom[MT_EE_CHIP_ID:MT_EE_CHIP_ID + 2]
    out[MT_EE_VERSION:MT_EE_VERSION + 2] = eeprom[MT_EE_VERSION:MT_EE_VERSION + 2]
    out[MT_EE_MAC_ADDR:MT_EE_MAC_ADDR + 6] = eeprom[AN_EEPROM_MAC_2G4:AN_EEPROM_MAC_2G4 + 6]
    out[MT_EE_MAC_ADDR2:MT_EE_MAC_ADDR2 + 6] = eeprom[AN_EEPROM_MAC_5G:AN_EEPROM_MAC_5G + 6]
    out[MT_EE_WIFI_CONF:MT_EE_WIFI_CONF + 8] = eeprom[MT_EE_WIFI_CONF:MT_EE_WIFI_CONF + 8]
    return bytes(out)


def mt76_diff(eeprom, bin_):
    """返回 (前缀差异字节数, 0x1000 起差异字节数)。"""
    n = min(len(eeprom), len(bin_), MT76_EE_SIZE)
    head = sum(1 for i in range(n) if i < MT76_HEAD_SIZE and eeprom[i] != bin_[i])
    tail = sum(1 for i in range(n) if i >= MT76_HEAD_SIZE and eeprom[i] != bin_[i])
    return head, tail


# ---- UBIFS 读取 ----------------------------------------------------------

def _ubifs_collect(data):
    """扫 UBI 里所有 UBIFS 节点，按 sqnum 取每个 key 的最新副本。

    返回 (dents, datas)：dents = {name: inum}，datas = {(inum, block): (size, compr, payload)}
    """
    dents, datas = {}, {}
    n_peb = len(data) // AN_UBI_PEB
    for i in range(n_peb):
        peb = i * AN_UBI_PEB
        if data[peb:peb + 4] != b"UBI#":
            continue
        if data[peb + AN_UBI_VID_OFF:peb + AN_UBI_VID_OFF + 4] != b"UBI!":
            continue  # 未使用的 PEB
        base = peb + AN_UBI_DATA_OFF
        end = base + AN_UBI_PEB - AN_UBI_DATA_OFF
        for p in range(base, end - 24, 8):
            if data[p:p + 4] != AN_UBIFS_MAGIC:
                continue
            sqnum = struct.unpack("<Q", data[p + 8:p + 16])[0]
            nlen = struct.unpack("<I", data[p + 0x10:p + 0x14])[0]
            if not 24 <= nlen <= AN_UBI_PEB:
                continue
            ntype = data[p + 0x14]
            key = data[p + 0x18:p + 0x28]
            if ntype == 2:  # dent
                inum = struct.unpack("<Q", data[p + 0x28:p + 0x30])[0]
                nl = struct.unpack("<H", data[p + 0x32:p + 0x34])[0]
                name = data[p + 0x38:p + 0x38 + nl].decode("utf-8", "replace")
                if name not in dents or dents[name][0] < sqnum:
                    dents[name] = (sqnum, inum)
            elif ntype == 1:  # data
                inum = struct.unpack("<I", key[0:4])[0]
                blk = key[4:8]
                size = struct.unpack("<I", data[p + 0x28:p + 0x2C])[0]
                compr = struct.unpack("<H", data[p + 0x2C:p + 0x2E])[0]
                k = (inum, blk)
                if k not in datas or datas[k][0] < sqnum:
                    datas[k] = (sqnum, size, compr, data[p + 0x30:p + nlen])
    return dents, datas


def _ubifs_inflate(payload, compr):
    if compr == 0:
        return payload
    if compr == 2:  # zlib —— UBIFS 存的是裸 deflate（无 0x78 头）
        try:
            return zlib.decompress(payload, -15)
        except zlib.error:
            return b""
    return b""  # 1=lzo / 3=zstd：本工具不支持，宁可缺文件也不猜


def an4475_read_files(data):
    """把 fhdata 卷还原成 {文件名: 内容}，跳过 UCI 临时文件。"""
    dents, datas = _ubifs_collect(data)
    out = {}
    for name, (_sq, inum) in dents.items():
        if name.startswith("."):
            continue
        blocks = sorted(
            ((k[1], v) for k, v in datas.items() if k[0] == inum),
            key=lambda x: x[0],
        )
        buf = bytearray()
        for _blk, (_sq2, size, compr, payload) in blocks:
            chunk = _ubifs_inflate(payload, compr)
            buf.extend(chunk[:size] if size <= len(chunk) else chunk)
        out[name] = bytes(buf)
    return out


# ---- UCI 文本读写 --------------------------------------------------------

def _uci_get(text, key):
    want = "config key '%s'" % key
    lines = text.split("\n")
    for i, line in enumerate(lines):
        if line.strip() != want:
            continue
        for nxt in lines[i + 1:]:
            s = nxt.strip()
            if s.startswith("config "):
                break
            if s.startswith("option value '") and s.endswith("'"):
                return s[len("option value '"):-1]
        break
    return ""


def _uci_set(text, key, value):
    """把 `config key '<key>'` 段里的 option value 换掉，其余字节保持原样。"""
    want = "config key '%s'" % key
    lines = text.split("\n")
    out, i = [], 0
    while i < len(lines):
        line = lines[i]
        out.append(line)
        i += 1
        if line.strip() != want:
            continue
        while i < len(lines) and not lines[i].lstrip().startswith("config "):
            cur = lines[i]
            s = cur.strip()
            if s.startswith("option value '") and s.endswith("'"):
                indent = cur[:len(cur) - len(cur.lstrip())]
                out.append("%soption value '%s'" % (indent, value))
            else:
                out.append(cur)
            i += 1
    return "\n".join(out)


def apply_an4475_identity(files, sn, gpon_sn, gpon_pw, base_mac, eth_mac,
                          wifi_2g4=None, wifi_5g=None):
    """把身份字段写回 factory_conf（以及 Wi-Fi EEPROM），保证三处一致。

    只改**单例**字段；SSID / 密码 / 光模块序列号 / BOSA 校准原样保留。
    `wifi_2g4` / `wifi_5g` 为 None 时不动 Wi-Fi 那两位。
    """
    conf = files.get(AN_MAIN_CONF)
    if conf is None:
        return dict(files)
    text = conf.decode("utf-8", "ignore")
    pairs = [
        ("SerialNumber", sn),
        ("VendorCode", "FH"),
        ("GponSN", gpon_sn.hex().upper()),
        ("GPONPassWord", gpon_pw.encode("ascii").hex().upper()),
        ("ManufacturerOUI", base_mac[:3].hex().upper()),
        ("base_mac", mac_to_string(base_mac)),
        ("brmac", mac_to_string(base_mac)),
        ("ponmac", mac_to_string(base_mac)),
    ]
    for name, mac in an4475_derive(base_mac, eth_mac):
        pairs.append((name, mac_to_string(mac)))
    if wifi_2g4 is not None:
        pairs.append((AN_APMAC_2G4, mac_to_string(wifi_2g4)))
    if wifi_5g is not None:
        pairs.append((AN_APMAC_5G, mac_to_string(wifi_5g)))
    for key, value in pairs:
        text = _uci_set(text, key, value)
    out = dict(files)
    out[AN_MAIN_CONF] = text.encode("utf-8")

    # Wi-Fi EEPROM 里同样的两位 MAC 也要跟着走，否则驱动可能读到旧值。
    if (wifi_2g4 is not None or wifi_5g is not None) and AN_WLAN_EEPROM in out:
        cur = an4475_eeprom_macs(out[AN_WLAN_EEPROM]) or an4475_wifi_defaults(base_mac)
        out[AN_WLAN_EEPROM] = an4475_eeprom_set_macs(
            out[AN_WLAN_EEPROM],
            cur[0] if wifi_2g4 is None else wifi_2g4,
            cur[1] if wifi_5g is None else wifi_5g,
        )
    return out


# ---- mtd9 解析 -----------------------------------------------------------

def looks_like_an4475_volume(data):
    """mtd9 备份判据：UBI 魔数 + 卷表里出现 fhdata。"""
    if len(data) < AN_UBI_PEB or data[:4] != b"UBI#":
        return False
    return b"fhdata" in data[:AN_UBI_PEB]


def parse_an4475_mtd9(data):
    """解析 mtd9 备份，返回身份字段与全部出厂文件。"""
    files = an4475_read_files(data)
    text = files.get(AN_MAIN_CONF, b"").decode("utf-8", "ignore")
    gpon_raw = _uci_get(text, "GponSN")
    pw_raw = _uci_get(text, "GPONPassWord")
    try:
        gpon_sn = bytes.fromhex(gpon_raw)[:AN_GPON_SN_SIZE]
    except ValueError:
        gpon_sn = b"\xff" * AN_GPON_SN_SIZE
    try:
        gpon_pw = bytes.fromhex(pw_raw).decode("ascii", "ignore")[:AN_GPON_PW_SIZE]
    except ValueError:
        gpon_pw = ""
    eeprom = files.get(AN_WLAN_EEPROM, b"")
    eep_macs = an4475_eeprom_macs(eeprom)
    base_mac = parse_mac(_uci_get(text, "base_mac")) or b"\x00" * 6
    # EEPROM 里的值才是驱动实际读到的，优先用它；缺失时退回 factory_conf 的
    # APMAC_1/2，再退到按基准 MAC 推算的约定值。
    d24, d5g = an4475_wifi_defaults(base_mac)
    wifi_2g4 = (eep_macs[0] if eep_macs else None) or parse_mac(
        _uci_get(text, AN_APMAC_2G4)) or d24
    wifi_5g = (eep_macs[1] if eep_macs else None) or parse_mac(
        _uci_get(text, AN_APMAC_5G)) or d5g
    return {
        "sn": _uci_get(text, "SerialNumber")[:AN_SN_SIZE],
        "gpon_sn": gpon_sn,
        "gpon_pw": gpon_pw,
        "base_mac": base_mac,
        "eth_mac": parse_mac(_uci_get(text, "eth1mac")) or b"\x00" * 6,
        "wifi_2g4": wifi_2g4,
        "wifi_5g": wifi_5g,
        "eeprom_device_id": an4475_eeprom_device_id(eeprom),
        "eeprom_size": len(eeprom),
        "vendor": _uci_get(text, "VendorCode"),
        "bosa": _uci_get(files.get("olt_bosa_info", b"").decode("utf-8", "ignore"),
                         "olt_bosa_id"),
        "files": files,
    }


# ---- 2 MiB 容器 ----------------------------------------------------------

def encode_an4475_2mib(sn, gpon_sn, gpon_pw, base_mac, eth_mac, files,
                       wifi_2g4=None, wifi_5g=None):
    """打包 2 MiB 容器。文件按「能压就压」存放，超出容量直接报错。"""
    d24, d5g = an4475_wifi_defaults(base_mac)
    wifi_2g4 = wifi_2g4 or d24
    wifi_5g = wifi_5g or d5g
    files = apply_an4475_identity(files, sn, gpon_sn, gpon_pw, base_mac, eth_mac,
                                  wifi_2g4, wifi_5g)
    names = sorted(files)
    if len(names) > AN_MAX_FILES:
        raise ValueError("文件数 %d 超过 TOC 上限 %d" % (len(names), AN_MAX_FILES))

    img = bytearray(b"\xff" * AN_COMPACT_SIZE)
    img[0:8] = AN_COMPACT_MAGIC
    struct.pack_into("<I", img, 0x08, AN_COMPACT_VERSION)
    struct.pack_into("<I", img, 0x0C, len(names))
    struct.pack_into("<I", img, 0x10, AN_TOC_OFF)
    struct.pack_into("<I", img, 0x14, AN_PAYLOAD_OFF)

    snb = sn.encode("ascii", "ignore")[:AN_SN_SIZE]
    img[AN_H_SN:AN_H_SN + len(snb)] = snb
    img[AN_H_GPON_SN:AN_H_GPON_SN + AN_GPON_SN_SIZE] = gpon_sn[:AN_GPON_SN_SIZE]
    img[AN_H_BASE_MAC:AN_H_BASE_MAC + 6] = base_mac
    img[AN_H_ETH_MAC:AN_H_ETH_MAC + 6] = eth_mac
    img[AN_H_OUI:AN_H_OUI + 3] = base_mac[:3]
    img[AN_H_VENDOR:AN_H_VENDOR + 2] = b"FH"
    pwb = gpon_pw.encode("ascii", "ignore")[:AN_GPON_PW_SIZE]
    img[AN_H_GPON_PW:AN_H_GPON_PW + len(pwb)] = pwb
    for i, (_name, mac) in enumerate(an4475_derive(base_mac, eth_mac)):
        off = AN_H_MACS + i * 6
        img[off:off + 6] = mac
    img[AN_H_WIFI_2G4:AN_H_WIFI_2G4 + 6] = wifi_2g4
    img[AN_H_WIFI_5G:AN_H_WIFI_5G + 6] = wifi_5g

    # EEPROM 与 11 张 PON 校准表走固定偏移、原样存；其余配置文件从 0x2000 起压缩摆放。
    cursor = AN_PAYLOAD_OFF
    max_end = AN_PAYLOAD_OFF
    for idx, name in enumerate(names):
        raw = files[name]
        if name == AN_WLAN_EEPROM:
            if AN_EEPROM_FIXED_OFF + len(raw) > AN_COMPACT_SIZE:
                raise ValueError("%s 有 %d 字节，放不进 0x%06X 起的固定区"
                                 % (name, len(raw), AN_EEPROM_FIXED_OFF))
            at, use, compr = AN_EEPROM_FIXED_OFF, raw, 0
            max_end = max(max_end, at + len(use))
        elif cal_offset(name) is not None:
            # PON 校准表：固定槽 + 原样，槽内剩余保持 0xFF。
            at = cal_offset(name)
            if len(raw) > AN_CAL_SLOT:
                raise ValueError("%s 有 %d 字节，超出 %d 字节的校准槽"
                                 % (name, len(raw), AN_CAL_SLOT))
            use, compr = raw, 0
            max_end = max(max_end, at + len(use))
        else:
            packed = zlib.compress(raw, 9)
            use = packed if len(packed) < len(raw) - 16 else raw
            compr = 1 if use is packed else 0
            if cursor + len(use) > AN_EEPROM_FIXED_OFF:
                raise ValueError("0x2000–0x%06X 区放不下：%s 需要写到 0x%06X"
                                 % (AN_EEPROM_FIXED_OFF, name, cursor + len(use)))
            at = cursor
            cursor += len(use)
            max_end = max(max_end, cursor)
        img[at:at + len(use)] = use
        ent = AN_TOC_OFF + idx * AN_TOC_ENTRY
        nb = name.encode("utf-8")[:AN_NAME_MAX - 1]
        img[ent:ent + len(nb)] = nb
        struct.pack_into("<III", img, ent + AN_NAME_MAX, at - AN_PAYLOAD_OFF,
                         len(raw), len(use))
        img[ent + AN_NAME_MAX + 12] = compr

    # 尾部再放一份 mt76 能直接加载的 7 680 B（模板打底），OpenWrt 侧 nvmem
    # 指到 AN_MT76_FIXED_OFF 即可，不必现场转换。
    if AN_WLAN_EEPROM in files:
        bin_ = mt76_from_template(_mt76_template(), files[AN_WLAN_EEPROM])
        end = AN_MT76_FIXED_OFF + len(bin_)
        if end > AN_COMPACT_SIZE:
            raise ValueError("mt76 槽 0x%06X 放不下 %d 字节"
                             % (AN_MT76_FIXED_OFF, AN_MT76_SIZE))
        img[AN_MT76_FIXED_OFF:end] = bin_
        max_end = max(max_end, end)

    struct.pack_into("<I", img, 0x18, max_end - AN_PAYLOAD_OFF)
    return bytes(img)


def is_an4475_compact(data):
    """2 MiB 新版或旧版 1 MiB 容器都算。"""
    return ((len(data) >= AN_COMPACT_SIZE and data[:8] == AN_COMPACT_MAGIC)
            or (len(data) >= AN_COMPACT_SIZE_V1 and data[:8] == AN_COMPACT_MAGIC_V1))


def parse_an4475_compact(data):
    """读回容器：头部身份字段 + 解压后的文件表（1/2 MiB 布局相同）。"""
    files = {}
    count = struct.unpack("<I", data[0x0C:0x10])[0]
    for idx in range(min(count, AN_MAX_FILES)):
        ent = AN_TOC_OFF + idx * AN_TOC_ENTRY
        raw_name = data[ent:ent + AN_NAME_MAX].split(b"\x00")[0]
        name = raw_name.decode("utf-8", "ignore")
        off, raw_len, stored_len = struct.unpack(
            "<III", data[ent + AN_NAME_MAX:ent + AN_NAME_MAX + 12]
        )
        compr = data[ent + AN_NAME_MAX + 12]
        blob = data[AN_PAYLOAD_OFF + off:AN_PAYLOAD_OFF + off + stored_len]
        if compr == 1:
            try:
                blob = zlib.decompress(blob)
            except zlib.error:
                blob = b""
        files[name] = blob[:raw_len]
    sn = data[AN_H_SN:AN_H_SN + AN_SN_CAP].split(b"\x00")[0].decode("ascii", "ignore")
    base_mac = bytes(data[AN_H_BASE_MAC:AN_H_BASE_MAC + 6])
    # 头部有就信头部；旧镜像那两位是 0xFF，退回 EEPROM 里读。
    wifi_2g4 = bytes(data[AN_H_WIFI_2G4:AN_H_WIFI_2G4 + 6])
    wifi_5g = bytes(data[AN_H_WIFI_5G:AN_H_WIFI_5G + 6])
    if wifi_2g4 == b"\xff" * 6 or wifi_5g == b"\xff" * 6:
        eep = an4475_eeprom_macs(files.get(AN_WLAN_EEPROM, b""))
        if eep:
            wifi_2g4, wifi_5g = eep
    return {
        "sn": sn.strip(),
        "gpon_sn": bytes(data[AN_H_GPON_SN:AN_H_GPON_SN + AN_GPON_SN_SIZE]),
        "gpon_pw": data[AN_H_GPON_PW:AN_H_GPON_PW + AN_GPON_PW_SIZE]
        .split(b"\x00")[0].decode("ascii", "ignore").strip(),
        "base_mac": base_mac,
        "eth_mac": bytes(data[AN_H_ETH_MAC:AN_H_ETH_MAC + 6]),
        "wifi_2g4": wifi_2g4,
        "wifi_5g": wifi_5g,
        "eeprom_device_id": an4475_eeprom_device_id(files.get(AN_WLAN_EEPROM, b"")),
        "files": files,
    }



def main():
    ap = argparse.ArgumentParser(description="H3C HM2004-DU factory 工具")
    sub = ap.add_subparsers(dest="cmd")

    g = sub.add_parser("gen", help="生成演示镜像 (1mib 紧凑 / true 真实布局)")
    g.add_argument("--sn", required=True)
    g.add_argument("--mac", required=True)
    g.add_argument("--cal", help="上行 PON 校准 bin (可选)")
    g.add_argument("--eeprom", help="EEPROM bin (可选)")
    g.add_argument(
        "--poncal",
        help="下行 PON 口校准：mtd13_h3c.bin，或已抽出的 0x120 字节 UX3326 blob (可选)",
    )
    g.add_argument(
        "--layout",
        choices=["1mib", "true"],
        default="1mib",
        help="1mib=1MiB 紧凑镜像(默认), true=真实 H3C 偏移布局(~1.75MiB)",
    )
    g.add_argument("-o", "--out", default="factory-h3c-1mib.bin")

    p = sub.add_parser("parse", help="解析真实 reservearea 备份或 mtd13")
    p.add_argument("--bin", required=True)

    h = sub.add_parser("hl202", help="HL202-DU（从机）: 解析 mtd12 / 生成 512 KiB")
    h.add_argument("--bin", help="mtd12 reservearea 备份 (.bin)，给定时解析")
    h.add_argument("--sn", help="生成用 SN（12 字符），省略则沿用 --bin 里的")
    h.add_argument("--mac", help="生成用 label MAC，省略则沿用 --bin 里的")
    h.add_argument("-o", "--out", help="输出 512 KiB 镜像路径")

    a = sub.add_parser("an4475", help="AN4475: 解析 mtd9 备份 / 生成 2 MiB 容器")
    a.add_argument("--bin", help="mtd9 备份 (.bin) 或已生成的 AN4-2MiB / AN4-1MiB 镜像")
    a.add_argument("--sn", help="设备序列号（17 字符）")
    a.add_argument("--gpon-sn", help="GPON SN，16 位 hex（含 FHTT 厂商码）")
    a.add_argument("--gpon-pw", help="GPON 密码（10 字符 ASCII）")
    a.add_argument("--mac", help="基准 MAC（base_mac / brmac / ponmac）")
    a.add_argument("--eth-mac", help="LAN 基址 MAC（eth1mac）")
    a.add_argument("--wifi-24", help="2.4G Wi-Fi MAC（写 EEPROM +4 与 APMAC_1）")
    a.add_argument("--wifi-5g", help="5G Wi-Fi MAC（写 EEPROM +0x0A 与 APMAC_2）")
    a.add_argument("-o", "--out", help="输出 2 MiB 镜像路径")
    a.add_argument("--export-mt76", metavar="BIN",
                   help="额外导出 mt76 用的 7 680 B EEPROM（默认官方模板打底）")
    a.add_argument("--mt76-carve", action="store_true",
                   help="--export-mt76 改成原样截取（前 7 680 B，保真但功率区不可用）")
    a.add_argument("--mt76-template", metavar="BIN",
                   help="--export-mt76 打底用的模板（默认 assets/mt7990_eeprom_2i5i.bin）")

    args = ap.parse_args()
    if args.cmd == "gen":
        mac = parse_mac(args.mac)
        if not mac:
            sys.exit("MAC 无效: %s" % args.mac)
        if len(args.sn) != DEVICE_SN_SIZE:
            print(
                "警告: SN 长度 %d != %d, 高位将被截断"
                % (len(args.sn), DEVICE_SN_SIZE)
            )
        cal = open(args.cal, "rb").read() if args.cal else None
        eep = open(args.eeprom, "rb").read() if args.eeprom else None
        dn, dnf = None, None
        if args.poncal:
            raw = open(args.poncal, "rb").read()
            if raw[:4] == b"UBI#":
                parsed = parse_mtd13(raw)
                if not parsed:
                    sys.exit("未在 %s 中找到 UX3326 下行校准" % args.poncal)
                dn, dnf, src = parsed
                print("下行校准来源: %s +0x30 (%d 字节)" % (args.poncal, len(dn)))
            else:
                dn = raw[:PON_DOWNLINK_CAL_SIZE]
            print(
                "下行 PON 校准: %d 字节, OBosaMode=%s BosaFlag=%s TempFlag=%s"
                % (
                    len(dn),
                    (dnf or {}).get("OBosaMode"),
                    (dnf or {}).get("BosaFlag"),
                    (dnf or {}).get("TempFlag"),
                )
            )
        if args.layout == "1mib":
            img = encode_1mib(args.sn, mac, cal, eep, dn, dnf)
        else:
            img = encode(args.sn, mac, cal, eep, dn, dnf)
        open(args.out, "wb").write(img)
        e0 = derive_mac(mac, OFF_ETH0)
        p0 = derive_mac(mac, OFF_PON0)
        w5 = derive_mac(mac, OFF_WIFI_5G)
        w24 = derive_mac(mac, OFF_WIFI_24G)
        print("已写出 %d 字节 -> %s" % (len(img), args.out))
        print("  Device SN : %s" % args.sn)
        print("  Label MAC : %s" % mac_to_string(mac))
        print("  eth0  +1  : %s" % mac_to_string(e0))
        print("  pon0  +2  : %s" % mac_to_string(p0))
        print("  5G    +4  : %s" % mac_to_string(w5))
        print("  2.4G  +5  : %s" % mac_to_string(w24))
        if dn is not None:
            print("  下行 PON  : %s, %d 字节" % (detect_downlink_chip(dn) or "<未识别>", len(dn)))
    elif args.cmd == "parse":
        data = open(args.bin, "rb").read()
        src = ""
        if looks_like_mtd13(data):
            parsed = parse_mtd13(data)
            if parsed:
                _, _, off = parsed
                src = "  (UBIFS 最新副本 @0x%06x +0x30, sqnum 取最大)" % off
        sn, label, cal, eep, dn, dnf = parse_volume(data)
        print("文件: %s (%d 字节)%s" % (args.bin, len(data), src))
        print("  Device SN : %r" % sn)
        print("  Label MAC : %s" % (mac_to_string(label) if label else "<未找到>"))
        if label:
            print("  eth0  +1  : %s" % mac_to_string(derive_mac(label, OFF_ETH0)))
            print("  pon0  +2  : %s" % mac_to_string(derive_mac(label, OFF_PON0)))
            print("  5G    +4  : %s" % mac_to_string(derive_mac(label, OFF_WIFI_5G)))
            print("  2.4G  +5  : %s" % mac_to_string(derive_mac(label, OFF_WIFI_24G)))
        chip = detect_pon_chip(cal)
        print("  上行 PON 芯片: %s" % (chip if chip else "<未识别/未设置>"))
        print("  上行 calibration: %d 字节" % len(cal))
        print("  eeprom    : %d 字节" % len(eep))
        print("  下行 PON 芯片: %s" % (detect_downlink_chip(dn) or "<未识别/未设置>"))
        print("  下行 calibration: %d 字节" % len(dn))
        if dnf:
            print(
                "  BOSA 标志: OBosaMode=%(OBosaMode)s BosaFlag=%(BosaFlag)s "
                "TempFlag=%(TempFlag)s" % dnf
            )
    elif args.cmd == "hl202":
        sn, label, bob, eep = "", None, None, None
        if args.bin:
            data = open(args.bin, "rb").read()
            if is_hl202_compact(data):
                sn, label, bob, eep = parse_hl202_compact(data)
                print("文件: %s (%d 字节) · HL2-512K 紧凑镜像" % (args.bin, len(data)))
            else:
                if not looks_like_hl202_volume(data):
                    print("警告: %s 不像 HL202-DU 的 mtd12（magic/GUD 配置名不匹配）"
                          % args.bin)
                sn, label, bob, eep = parse_hl202_volume(data)
                print("文件: %s (%d 字节) · mtd12 reservearea"
                      % (args.bin, len(data)))
                print("  GUD 字符串:")
                for k, v in read_hl_gud(data):
                    if v:
                        print("    %-10s %s" % (k, v))
            print("  Device SN : %r" % sn)
            print("  Label MAC : %s"
                  % (mac_to_string(label) if label else "<未找到>"))
            if label:
                e0, p0, w5, w24 = derive_macs(label)
                print("  eth0  (+1): %s" % mac_to_string(e0))
                print("  pon0  (+2): %s" % mac_to_string(p0))
                print("  5G    (+4): %s" % mac_to_string(w5))
                print("  2.4G  (+5): %s" % mac_to_string(w24))
            print("  PON BOB   : %d 字节 (EN7571, 非FF=%d)"
                  % (len(bob), sum(1 for b in bob if b != 0xFF)))
            print("  EEPROM    : %d 字节" % len(eep))
            print("  下行 PON  : 本机型无此分区")
        if args.sn is not None:
            if len(args.sn) != HL_SN_SIZE:
                print("警告: SN 长度 %d != %d, 将被截断" % (len(args.sn), HL_SN_SIZE))
            sn = args.sn
        if args.mac is not None:
            m = parse_mac(args.mac)
            if not m:
                sys.exit("MAC 无效: %s" % args.mac)
            label = m
        if args.out:
            if not sn or not label:
                sys.exit("生成 512 KiB 需要 SN 和 MAC（用 --bin 或 --sn/--mac 提供）")
            img = encode_hl202_512k(sn, label, bob, eep)
            open(args.out, "wb").write(img)
            print("已写出 %d 字节 -> %s" % (len(img), args.out))
            print("  magic     : %r" % img[:8])
            print("  SN        : %s" % sn)
            print("  Label MAC : %s" % mac_to_string(label))
    elif args.cmd == "an4475":
        sn, gpon_sn, gpon_pw = "", b"\xff" * AN_GPON_SN_SIZE, ""
        base = eth = wifi24 = wifi5g = None
        files = {}
        devid = 0
        if args.bin:
            data = open(args.bin, "rb").read()
            if is_an4475_compact(data):
                info = parse_an4475_compact(data)
                magic = ("AN4-1MiB 容器（旧版，导出将升级为 2 MiB）"
                         if data[:8] == AN_COMPACT_MAGIC_V1 else "AN4-2MiB 容器")
                print("文件: %s (%d 字节) · %s" % (args.bin, len(data), magic))
            else:
                if not looks_like_an4475_volume(data):
                    print("警告: %s 不像 AN4475 的 mtd9（缺 UBI 魔数或 fhdata 卷）"
                          % args.bin)
                info = parse_an4475_mtd9(data)
                print("文件: %s (%d 字节) · mtd9 (UBI 卷 fhdata / UBIFS)"
                      % (args.bin, len(data)))
            sn = info["sn"]
            gpon_sn, gpon_pw = info["gpon_sn"], info["gpon_pw"]
            base, eth = info["base_mac"], info["eth_mac"]
            wifi24, wifi5g = info["wifi_2g4"], info["wifi_5g"]
            devid = info.get("eeprom_device_id", 0)
            files = info["files"]
            eep = files.get(AN_WLAN_EEPROM)
            if eep is not None:
                print("  Wi-Fi     : %s，%d 字节，device id 0x%04X"
                      % (AN_WLAN_EEPROM, len(eep), devid))
                if devid != 0x7993:
                    print("    警告: device id 0x%04X != 0x7993，可能不是 MT7990AN"
                          % devid)
            print("  出厂文件  : %d 个，共 %d 字节"
                  % (len(files), sum(len(v) for v in files.values())))
            for name in sorted(files):
                print("    %-24s %8d" % (name, len(files[name])))
        if args.sn:
            sn = args.sn
        if args.gpon_sn:
            gpon_sn = bytes.fromhex(args.gpon_sn)
        if args.gpon_pw:
            gpon_pw = args.gpon_pw
        if args.mac:
            base = parse_mac(args.mac)
            if not base:
                sys.exit("MAC 无效: %s" % args.mac)
        if args.eth_mac:
            eth = parse_mac(args.eth_mac)
            if not eth:
                sys.exit("MAC 无效: %s" % args.eth_mac)
        for opt, val, name in ((args.wifi_24, "wifi24", "2.4G"), (args.wifi_5g, "wifi5g", "5G")):
            if not opt:
                continue
            m = parse_mac(opt)
            if not m:
                sys.exit("%s MAC 无效: %s" % (name, opt))
            if val == "wifi24":
                wifi24 = m
            else:
                wifi5g = m
        if base and eth:
            print("  Serial    : %s" % sn)
            print("  GPON SN   : %s" % gpon_sn.hex().upper())
            print("  GPON 密码 : %s" % gpon_pw)
            print("  基准 MAC  : %s" % mac_to_string(base))
            print("  LAN 基址  : %s" % mac_to_string(eth))
            for name, mac in an4475_derive(base, eth):
                print("    %-16s %s" % (name, mac_to_string(mac)))
            d24, d5g = an4475_wifi_defaults(base)
            print("  2.4G MAC  : %s%s"
                  % (mac_to_string(wifi24 or d24),
                     "" if wifi24 else "（约定默认 +%d）" % AN_WIFI_OFFSET_2G4))
            print("  5G MAC    : %s%s"
                  % (mac_to_string(wifi5g or d5g),
                     "" if wifi5g else "（约定默认 +%d）" % AN_WIFI_OFFSET_5G))
        if args.out:
            if not (sn and base and eth and files):
                sys.exit("生成 2 MiB 需要 --bin 提供出厂文件与身份"
                         "（可再用 --sn/--mac 覆盖）")
            img = encode_an4475_2mib(sn, gpon_sn, gpon_pw, base, eth, files,
                                     wifi24, wifi5g)
            open(args.out, "wb").write(img)
            print("已写出 %d 字节 -> %s" % (len(img), args.out))
            print("  magic     : %r  载荷 %d 字节"
                  % (img[:8], int.from_bytes(img[0x18:0x1C], "little")))
        if args.export_mt76:
            ee = files.get(AN_WLAN_EEPROM) if files else None
            if ee is None:
                sys.exit("导出 mt76 EEPROM 需要 --bin 里有 %s" % AN_WLAN_EEPROM)
            slot = None
            if args.bin and not args.mt76_carve:
                try:
                    with open(args.bin, "rb") as f:
                        slot = compact_mt76(f.read())
                except OSError:
                    slot = None
            if slot is not None:
                bin_ = slot
                mode = "取自容器 0x%06X 的 mt76 槽" % AN_MT76_FIXED_OFF
            elif args.mt76_carve:
                bin_ = mt76_carve(ee)
                mode = "原样截取"
            else:
                path = args.mt76_template or MT76_DEFAULT_IPA_PATH
                try:
                    tpl = open(path, "rb").read()
                except OSError as exc:
                    sys.exit("读不到 mt76 模板 %s: %s" % (path, exc))
                bin_ = mt76_from_template(tpl, ee)
                mode = "模板打底" if not args.mt76_template else "模板打底(%s)" % args.mt76_template
            open(args.export_mt76, "wb").write(bin_)
            head, tail = mt76_diff(ee, bin_)
            pa = mt76_pa_lna(ee)
            print("已写出 mt76 EEPROM %d 字节 -> %s（%s）"
                  % (len(bin_), args.export_mt76, mode))
            print("  源       : %d 字节" % len(ee))
            print("  PA/LNA   : %s"
                  % ("<读不到>" if pa is None
                     else "0x%02X %s" % (pa, MT76_FEM_NAMES.get(pa, "混合/未知"))))
            print("  0x0000-0x0FFF 差 %d 字节；0x1000 起差 %d 字节" % (head, tail))
            print("  芯片 ID  : %s" % bin_[:2].hex(" ").upper())
            print("  2.4G MAC : %s" % mac_to_string(bin_[MT_EE_MAC_ADDR:MT_EE_MAC_ADDR + 6]))
            print("  5G   MAC : %s" % mac_to_string(bin_[MT_EE_MAC_ADDR2:MT_EE_MAC_ADDR2 + 6]))
    else:
        ap.print_help()


if __name__ == "__main__":
    main()
