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
  python3 h3c_factory_demo.py gen --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a -o factory-h3c-demo.bin

  # 从真实备份解析字段
  python3 h3c_factory_demo.py parse --bin "G:/下载/H3C-HM2004-DU-TELNET备份/mtd14_reservearea.bin"
"""
import argparse
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
DEVICE_SN_SIZE = 0x0C
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
    snb = sn.encode("ascii")[:DEVICE_SN_SIZE]
    img[DEVICE_SN_OFFSET:DEVICE_SN_OFFSET + len(snb)] = snb
    img[DEVICE_SN_DUP_OFFSET:DEVICE_SN_DUP_OFFSET + len(snb)] = snb
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
    else:
        ap.print_help()


if __name__ == "__main__":
    main()
