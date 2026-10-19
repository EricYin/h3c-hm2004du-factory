# H3C HM2004-DU Factory Editor

Rust + egui 图形工具，用于**查看 / 编辑 / 对照** H3C HM2004-DU（AN7581 FTTR 主网关）出厂分区（factory）里的设备身份与校准数据。
本项目是 `fiberhome-factory` 的 **H3C 改写版**：布局、导入逻辑、MAC 派生规则全部针对 HM2004-DU 重新实现。

> ⚠️ **重要声明**
> - 本工具仅供 **研究 / 备份查看 / 对照学习** 使用，与新华三（H3C）无任何关联，未包含任何厂商闭源代码。
> - 导出的「布局映射镜像」**不含 UBI 卷元数据、不对 ctromfile 重压缩**，**不可直接刷写 NAND**。要落盘到真机需用 `ubiupdatevol` 写入真实的 `ubi-volume-factory` 卷（见下文「进阶」）。
> - 修改设备出厂数据可能导致设备无法启动或违反当地法规，请仅在自己拥有的设备上操作。

## 出厂分区布局（H3C HM2004-DU）

HM2004-DU 的 factory 是一个 UBI 卷（`ubi-volume-factory`，2_359_296 字节）。可读字段的真实偏移：

| 偏移 | 长度 | 字段 | 说明 |
|------|------|------|------|
| `0x140000` | 4 | magic | `21 43 34 12` (`0x12344321`) |
| `0x140005` | 6 | base MAC 占位 | `02:00:00:XX:00:00`（非真实地址） |
| `0x14102c` | 12 | Device SN | ASCII，重复于 `0x14105f` |
| `0x80000` | — | ctromfile (gzip) | 真实 label MAC 在 `AL-MAC="..."` 内 |
| `0x1c0400` | 0x200 | 上行 PON 校准 | EN7572 / UX3363 BOSA |
| `0x1c1000` | 0x120 | 下行 PON 口校准 | UX3326 BOSA（源自 mtd13，见下节） |
| `0x1c1120` | 6 | 下行 BOSA 标量 | OBosaMode / BosaFlag / TempFlag，各 u16 LE |
| `0x4c000` | 0x1000 | Wi-Fi EEPROM | MT7916 DBDC |

真实 label MAC **不**存在可寻址的 factory 字段里：原厂把出厂区相应槽位留空，MAC 只以 `AL-MAC` 形式存在于 `0x80000` 的 gzip ctromfile 中（与 `02_network::h3c_hm2004du_base_mac` 一致）。
因此本工具**导入时解 gzip 取 `AL-MAC`**，**导出时在 `0x80000` 写明文 ctromfile 块**（便于对照，不重压缩）。

## 下行 PON 口校准（mtd13 / UBIFS）

下行 PON 口的光模块是 **UX3326 BOSA**，它的校准数据**不在 factory 分区里**，而在 **mtd13**。mtd13 是一个 4 MiB（`0x400000`）的 UBI 镜像，32 个 PEB（每个 `0x20000`），内含名为 `ubifs1` 的 UBIFS 动态卷，内含 `/bosa/` 目录树：

| UBIFS inode | 文件 | 内容 |
|------|------|------|
| `0x45` | `mmodified_UX3326.bin` | `0x120`（288）字节校准 blob |
| `0x42` | `OBosaMode` | u16 |
| `0x43` | `BosaFlag` | u16 |
| `0x44` | `TempFlag` | u16 |

### 厂商魔改的 UBIFS 节点头

与标准 UBIFS 不同（CRC32 也对不上，故本工具**不做校验**），实测布局为：

| 偏移 | 字段 |
|------|------|
| `+0x00` | magic u32 = `0x06101831` |
| `+0x04` | crc u32（不可校验，跳过） |
| `+0x08` | sqnum u64 |
| `+0x10` | len u32 |
| `+0x14` | node_type u8（1=LEAF/DATA，2=DENT，5=PAD，6=SB，7=CMT） |
| `+0x15` | group_type u8 |
| `+0x18` | ino u32 |
| `+0x28` | nodelen u32（= payload 长度，不含头部） |
| `+0x30` | payload |

### 两个必须注意的坑

1. **必须按 `node_type == 1` 过滤。** mtd13 里有大量 CMT（`node_type=7`）提交节点，其载荷会被误读成 `ino`/`nodelen`，凭空造出几百条假记录并盖掉真实数据。
2. **必须按 `sqnum` 取最新，而不是物理位置最后一份。** UBIFS 是日志结构，同一 inode 会有多个副本。样本 mtd13 中 ino `0x45` 有 10 个副本（`0x2210a8` sq=40 … `0x225800` sq=165），**最新的是 `0x225800`**。

### 288 字节 blob 结构

| 偏移 | 内容 |
|------|------|
| `+0x00` | 器件/工厂参数区（0x70 字节） |
| `+0x70` | **84 项 u16 接收灵敏度 LUT，注意是大端** |
| `+0x118` | 尾部保留 |

灵敏度 LUT 用 `struct.unpack_from(">84H", cal, 0x70)` 读取，实测值域 `220..318`（`0x00dc..0x013e`），小端读出来是乱序。工具据此识别光模块型号——常量填充（例如全 `0x5a`）虽然也满足「非递减」，但缺少正值起点、无 `0xFFFF`、且尾部不爬升，因此不会被误判。

样本 mtd13 中该 LUT 的偏移、项数、字节序在**全部 10 个 ino `0x45` 副本上完全一致**（早期副本值域 `316..456`，晚期 `220..318`）。

> 注意：`0x1c1000` / `0x2200` 这些偏移描述的是**本工具自己的导出格式**——mtd13 不属于 `ubi-volume-factory`，没有可对应的真实分区偏移。原始定位由 `parse_mtd13()` 完成。

## MAC 派生（末字节偏移）

与 `02_network::h3c_mac_offset` 一致：

| 接口 | 偏移 | 示例（label = `44:76:09:65:06:4A`） |
|------|------|------|
| label / br-lan / LAN | 0 | `44:76:09:65:06:4A` |
| eth0 | +1 | `44:76:09:65:06:4B` |
| pon0 / wan | +2 | `44:76:09:65:06:4C` |
| 5G Wi-Fi | +4 | `44:76:09:65:06:4E` |
| 2.4G Wi-Fi | +5 | `44:76:09:65:06:4F` |

（FiberHome 原版用 `2.4G=base+1` / `5G=base+8`，规则不同。）

## 构建与运行（图形界面，需 Rust 工具链）

```bash
cargo build --release
cargo run --release

# 也可以直接带文件启动（或把 .bin 拖到 exe 上）
cargo run --release -- mtd14_reservearea.bin
```

仓库附带 `Cargo.lock`，锁定 egui / eframe **0.36.2**（本代码所针对的 API）。上游 `eframe` 0.35→0.36 有破坏性变更（`App::update` → `App::ui`、`Frame::group` → `ui.group`），升级依赖时请同步核对。

Linux 构建需显式开启窗口后端（与上游一致）：

```bash
sudo apt-get install -y build-essential libx11-dev libxkbcommon-dev pkg-config
cargo build --release --features linux-x11
```

 界面仿上游 FiberHome 编辑器：顶栏「新建 / 打开 / 保存 1 MiB / 保存真实布局」+ 状态行，「身份信息」区（标签 MAC / 设备序列号，各带随机按钮），派生 MAC 只读表，PON 校准与 Wi-Fi EEPROM 为「替换 / 导出 / 清空」分组行。

**PON 硬件由导入数据决定**：`上行 PON 校准` 行的标题会显示从校准 blob 识别出的芯片——H3C 真机格式把型号以填充 ASCII 写在块内（实测 `0x1c0428` = `EN7572`，厂商 `ECONET` 因无数字被跳过）；若导入的是 FiberHome 的 `APONCAL` 格式则读 `0x0c` 芯片 ID（1=GN28L95、2=UX3363）；都识别不出时只显示「上行 PON 校准」。这样换用其它光模块的备份会自动显示对应型号。

**出厂校准分散在两个分区**，因此顶栏提供两个独立入口，**任意顺序打开都互不覆盖**：

| 按钮 | 目标分区 | 载入内容 |
|------|------|------|
| `打开出厂区 (mtd14)` | `mtd14_reservearea.bin` | 标签 MAC、设备 SN、上行 PON 校准、Wi-Fi EEPROM |
| `打开下行校准 (mtd13)` | `mtd13_h3c.bin` | 下行 PON 口校准 blob + OBosaMode/BosaFlag/TempFlag |
| `打开…` | 自动按内容识别 | 等价于上面两个之一 |

关键行为：**真实 mtd14 不含下行数据**，所以打开它时不会用 `0xFF` 抹掉已经从 mtd13 载入的下行校准；反之打开 mtd13 也不会动身份信息与上行校准。导出的 1 MiB 镜像同时包含两个分区的数据。

也可在命令行一次传两个文件（顺序任意）：

```bash
h3c-hm2004du-factory.exe mtd14_reservearea.bin mtd13_h3c.bin
# Linux/macOS
./h3c-hm2004du-factory mtd14_reservearea.bin mtd13_h3c.bin
```

状态栏下方会显示两个分区各自的来源文件名，`<未载入>` 表示该分区尚未打开。

界面固定为**亮色主题**，且每帧重申一次（`apply_light_theme`）。原因：egui 的 `Options`（含 `theme_preference`）会经 eframe persistence 落盘并在启动时恢复，而 `theme_preference` 默认是 `System` —— 系统为深色时 `Options::theme()` 返回 `Dark`，`Ui` 按 dark style 取色，表现为「背景浅、文字深」的半成品外观。仅在启动回调里设置一次挡不住持久化恢复与系统主题变更事件，故在每帧开头幂等重申。

## Python 伴生工具（无需 Rust）

`h3c_factory_demo.py` 是命令行版，逻辑与 `src/model.rs::encode()` 完全一致：

```bash
# 生成 1 MiB 紧凑镜像（默认）
python3 h3c_factory_demo.py gen --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a -o factory-h3c-1mib.bin

# 生成真实偏移布局镜像（~1.75 MiB）
python3 h3c_factory_demo.py gen --layout true --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a -o factory-h3c-layout.bin

# 从真实 reservearea 备份解析
python3 h3c_factory_demo.py parse --bin mtd14_reservearea.bin

# 从 mtd13 解析下行 PON 口校准
python3 h3c_factory_demo.py parse --bin mtd13_h3c.bin

# 把下行 PON 口校准一并打进 1 MiB 紧凑镜像（--poncal 接受 mtd13 或裸 288 字节 blob）
python3 h3c_factory_demo.py gen --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a \
    --poncal mtd13_h3c.bin -o factory-h3c-1mib.bin
```

示例产物见 [`examples/factory-h3c-demo.bin`](examples/factory-h3c-demo.bin)。

工具支持两种导出格式（解析时按 magic 自动识别）：

**1. 1 MiB 紧凑镜像（默认）** — 1,048,576 字节。真实 H3C 字段位于 `0x140000+`（超出 1 MiB），无法按真实偏移塞进 1 MiB，故采用自描述紧凑布局（magic `H3C-1MiB`），**所有字段全部保留**：

| 偏移 | 长度 | 字段 |
|------|------|------|
| `0x0000` | 8 | magic `H3C-1MiB` |
| `0x0010` | 6 | label MAC |
| `0x0020` | 6 | eth0 = label+1 |
| `0x0030` | 6 | pon0/wan = label+2 |
| `0x0040` | 6 | 5G = label+4 |
| `0x0050` | 6 | 2.4G = label+5 |
| `0x0060` | 12 | Device SN (ASCII) |
| `0x1000` | 0x1000 | Wi-Fi EEPROM (MT7916) |
| `0x2000` | 0x200 | 上行 PON 校准 (EN7572) |
| `0x2200` | 0x120 | **下行 PON 口校准 (UX3326)** |
| `0x2400` | 6 | 下行 BOSA 标量 OBosaMode / BosaFlag / TempFlag |
| 其余 | — | `0xFF` 填充 |

**2. 真实偏移布局镜像** — 1,847,296 字节（`0x1c3000`），字段落在与真机 factory 卷一致的偏移上，适合严格对照。

两种都**不可直接刷写 NAND**（不含 UBI 元数据、不重压缩 ctromfile）。

## 目录结构

```
h3c-hm2004du-factory/
├── Cargo.toml
├── Cargo.lock                 # 锁定 egui/eframe 0.36.2
├── LICENSE                    # GPL-2.0-only
├── README.md
├── .gitignore
├── .github/workflows/build.yml # GitHub Actions：三平台构建（产物见 Actions Artifacts）
├── src/
│   ├── font.rs      # CJK 字体安装（fontdb，移植自上游）
│   ├── main.rs      # egui 编辑器
│   ├── model.rs     # H3C 布局常量 + Factory + encode
│   └── import.rs    # 解析真实 factory 卷
├── h3c_factory_demo.py
└── examples/
    └── factory-h3c-demo.bin
```

## 进阶：生成可刷写卷

当前 `encode()` 输出演示镜像。若要生成可被 `ubiupdatevol` 写入的整卷，需要：

1. 用 `ubinfo` 取得真实卷大小并对齐到 NAND 页 / 块；
2. 在 `0x80000` 处重新 gzip 压缩 ctromfile（保留 `AL-MAC` / `SerialNumber`）；
3. 由 ubi 工具写入卷，而不是裸写文件。

欢迎 PR。

## 许可证

**[GPL-2.0-only](LICENSE)**。

本项目是上游 `fiberhome-factory` 项目的 **H3C 改写 / 重实现**：整体架构与出厂分区编辑思路借鉴自上游，`src/font.rs` 的 CJK 字体安装逻辑直接移植自上游 `font.rs`。上游以 **GPL-2.0-only** 发布，故本派生作品沿用同一许可证。

若需改用其他许可证（如 MIT），请先移除所有源自上游的代码。

感谢原作者的工作。
