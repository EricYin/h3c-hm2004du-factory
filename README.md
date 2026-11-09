# H3C HM2004-DU / HL202-DU / AN4475 Factory Editor

Rust + egui 图形工具，用于**查看 / 编辑 / 对照** H3C FTTR 设备的出厂分区里的设备身份与校准数据。支持三种机型（顶栏下拉切换）：

| 机型 | 角色 | 出厂分区 | 下行 PON 校准 | 紧凑镜像 |
|---|---|---|---|---|
| **HM2004-DU** | 主网关（AN7581） | mtd14 `reservearea` + mtd13 `ubifs1` | 有（mtd13，UX3326，288 B） | 1 MiB，`H3C-1MiB` |
| **HL202-DU** | 从机 | mtd12 `reservearea` | **无**（该机型没有此分区） | 512 KiB，`HL2-512K` |
| **AN4475** | 10G PON ONT | mtd9（UBI 卷 `fhdata` / UBIFS） | 以 6 张 BOSA 校准表形式存在 | 1 MiB，`AN4-1MiB`（容器） |

本项目是 `fiberhome-factory` 的 **H3C 改写版**：布局、导入逻辑、MAC 派生规则全部针对上述机型重新实现。

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
| `0x14102c` | 16 | Device SN 槽位 | 原厂放 16 字节 `H3CT0005EBF068CU`，但**标签上的 SN 只有 12 字符** `H3CT0005EBF0`；尾部 `68CU` 是厂商附加字段，本工具只写前 12 字节，尾部保持原样/留 `0xFF` |
| `0x14105f` | 12 | Device SN（短） | ASCII `H3CT0005EBF0`；原厂如此，其后紧跟明文 `ctromfile_GUI.cfg`，故写入上限同样是 12 |
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

**两台机型共用同一套偏移**（`src/model.rs` 的 `OFF_*` 常量），主机与从机的 512 KiB / 1 MiB 紧凑镜像都在 `label MAC` 之后各写 4 个派生值。

> ⚠️ 证据强度要分开看：主机的规则来自 H3C `h3c_mac_offset`；从机这台 mtd12 里**只有 label MAC 一个明文值**（`AL-MAC="44:76:09:10:30:f8"`），romfile 里那两条 `mw_mac_offset="7"/"4"` 属于 WAN 连接（PVC）字段、与接口派生无关，syslog 区也没有接口 MAC 记录。所以从机的派生值是**沿用同厂商约定的推定值，尚无真机实证**；如果你的从机实测接口 MAC 与此不符，告诉我实际值，我改成从机专属偏移。

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

 界面仿上游 FiberHome 编辑器：顶栏「新建 / 打开 / 保存 1 MiB / 保存真实布局」+ 状态行，「身份信息」区（标签 MAC / 设备序列号，各带随机按钮），派生 MAC 只读表，PON 校准与 Wi-Fi EEPROM 为「替换 / 导出 / 清空」分组行。顶栏的机型是**下拉框**（`主机 HM2004-DU` / `从机 HL202-DU` / `AN4475 (10G PON)`）；H3C 两套面板都显示派生 MAC 表（末字节 +1/+2/+4/+5），从机面板另外只读展示 GUD 出厂字符串，AN4475 面板另有 Wi-Fi 2.4G/5G MAC 输入框与 MT7993 device id 提示。

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
h3c-factory.exe mtd14_reservearea.bin mtd13_h3c.bin
# Linux/macOS
./h3c-factory mtd14_reservearea.bin mtd13_h3c.bin
```

状态栏下方会显示两个分区各自的来源文件名，`<未载入>` 表示该分区尚未打开。

界面固定为**亮色主题**（白色）。这个"白色"由三处配合完成，缺一不可 —— 前三处都是对 eframe/egui 0.36.2 源码逐行核实后的结论：

1. **重写 `App::clear_color` 返回纯白** —— 这是关键。eframe 的默认清屏色是**硬编码的深色** `(12,12,12)`@alpha180，完全无视主题（`eframe/src/epi.rs` 里写着 "a bright gray makes the shadows of the windows look weird"）；而 egui 0.36 传给 `App::ui` 的根 `Ui` **自己不画背景**（官方注释："has no margin or background color"），于是这个深色透出来占满整个窗口。只改 `Visuals` 是治标不治本：控件会变白，窗口依旧发黑。
2. **`apply_light_theme` 把 dark / light 两个 style 都写成亮色** —— `theme_preference` 默认是 `System`，系统为深色时 `Options::theme()` 解析成 `Dark`，Ui 会一直按 dark style 取色；两个 style 都写亮，解析结果就无所谓了。顺带 `set_theme(Light)` 固定偏好。stock 亮色盘的两处也顺手调了：`weak_text_color`（默认由 `text_color * 0.6` 派生，白底上发灰）钉成 110，三态按钮底色 230/220/165 拉开成 238/205/170。
3. **`ui.set_style(...)` 钉住当前帧** —— 根 `Ui` 的 style 在帧开始时就被快照成 `Arc<Style>`，只改 ctx 要等下一帧才生效，首帧会闪一下深色；在 `App::ui` 开头把根 Ui 的 style 也换掉，首帧起就是白的。

已知限制：eframe 0.36.2 不转发 `ViewportCommand::SetTheme` 给原生窗口，所以**窗口标题栏仍跟随系统主题**（仅装饰，应用内容全部为白色）。

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

### HL202-DU（从机）

```bash
# 解析从机 mtd12（同时打印 GUD 全部字段）
python3 h3c_factory_demo.py hl202 --bin mtd12.bin

# 从 mtd12 直接转成从机 512 KiB 镜像
python3 h3c_factory_demo.py hl202 --bin mtd12.bin -o factory-hl202-512k.bin

# 只换 SN / MAC 再生成
python3 h3c_factory_demo.py hl202 --bin mtd12.bin \
    --sn H3CT091030F8 --mac 44:76:09:10:30:f8 -o factory-hl202-512k.bin
```

### AN4475（10G PON）

```bash
# 解析 mtd9 备份（打印身份字段、9 个派生 MAC、21 个出厂文件）
python3 h3c_factory_demo.py an4475 --bin mtd9_factory.bin

# 直接转成 1 MiB 容器
python3 h3c_factory_demo.py an4475 --bin mtd9_factory.bin -o factory-an4475-1mib.bin

# 换身份再生成（出厂文件仍取自备份，只改身份）
python3 h3c_factory_demo.py an4475 --bin mtd9_factory.bin \
    --sn F3200D49F2904E800 --mac D4:9F:29:04:E8:00 \
    --eth-mac D4:A0:29:04:E9:00 --gpon-sn 46485454C26A6A00 \
    --gpon-pw HG26099999 --wifi-24 D4:9F:29:04:E8:06 \
    --wifi-5g D4:9F:29:04:E8:07 -o factory-an4475-1mib.bin

# 也可以直接读回已生成的 1 MiB 容器
python3 h3c_factory_demo.py an4475 --bin factory-an4475-1mib.bin
```

示例产物见 [`examples/factory-h3c-demo.bin`](examples/factory-h3c-demo.bin)、
[`examples/factory-hl202-demo-512k.bin`](examples/factory-hl202-demo-512k.bin)。

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
| `0x0060` | 16 | Device SN (ASCII，12 字符有效，其余留空) |
| `0x1000` | 0x1000 | Wi-Fi EEPROM (MT7916) |
| `0x2000` | 0x200 | 上行 PON 校准 (EN7572) |
| `0x2200` | 0x120 | **下行 PON 口校准 (UX3326)** |
| `0x2400` | 6 | 下行 BOSA 标量 OBosaMode / BosaFlag / TempFlag |
| 其余 | — | `0xFF` 填充 |

**2. 真实偏移布局镜像** — 1,847,296 字节（`0x1c3000`），字段落在与真机 factory 卷一致的偏移上，适合严格对照。

两种都**不可直接刷写 NAND**（不含 UBI 元数据、不重压缩 ctromfile）。

## HL202-DU（从机）

从机的 `reservearea` 挂在 **mtd12**（不是主机的 mtd14），大小同为 `0x240000`，magic 同样是 `21 43 34 12`，但内容结构不同：

| 偏移 | 长度 | 内容 |
|---|---|---|
| `0x04c000` | 0x1000 | Wi-Fi EEPROM (MT7916D)，与主机同偏移同长度 |
| `0x080000` | — | gzip(ctromfile.cfg)，label MAC 在 `AL-MAC="..."` 内 |
| `0x100000` | — | syslog（可丢弃） |
| `0x140000` | 0x1000 | GUD 字段索引表（uint32 数组） |
| `0x141010` | 151 | GUD 出厂字符串（SN / SSID / 密码 / 型号 / 密钥 / 日期） |
| `0x1c0400` | **0xE1 (225)** | PON BOB 校准 (EN7571)，长度逐台实测 |
| 其余 | — | `0xFF` / `0x00` |

### 与主机最关键的三处差异

1. **没有下行 PON 校准**。整分区搜 `UX3326` / `OBosaMode` / `BosaFlag` / `TempFlag` 全部 0 命中 —— 从机不需要 mtd13，紧凑镜像里也没有这一块。
2. **上行校准长度与结构不同**：主机是 `0x200`（512 B，EN7572，含明文 `ECONET`），从机是 `0xE1`（225 B，EN7571，`+0x40` 处有 `RCF\0` 段标记）。
3. **GUD 字符串语义不同**：主机那段是「SN + 4 字符厂商后缀 + 混淆串」，从机是定长字段顺序拼接。

### 机型自动识别

两台机型的 magic、尺寸、SN 偏移**全部相同**，只看这些会误判。可靠判据是 GUD 区里的「配置名」字段：

| 机型 | 配置名 |
|---|---|
| HM2004-DU | `ctromfile_GUI.cfg` |
| HL202-DU | `ctromfile_GUD.cfg` |

兜底判据：主机上行校准区 `0x1c0414` 有明文 `ECONET`，从机没有。

### 从机 512 KiB 紧凑布局（与主机不通用）

从机走**独立布局**，magic 为 `HL2-512K`，总长 **512 KiB（0x80000）**：

| 偏移 | 长度 | 内容 |
|---|---|---|
| `0x0000` | 8 | magic `HL2-512K` |
| `0x0010` | 6 | label MAC |
| `0x0020` | 12 | Device SN（12 字符） |
| `0x0030` | 6 | eth0 = label+1 |
| `0x0040` | 6 | pon0 = label+2 |
| `0x0050` | 6 | 5G = label+4 |
| `0x0060` | 6 | 2.4G = label+5 |
| `0x1000` | 0x1000 | Wi-Fi EEPROM (MT7916D) |
| `0x2000` | 0xE1 | PON BOB 校准 (EN7571) |
| `0x20E1` 起 | — | `0xFF` 填充 |

尺寸取 512 KiB 而非 1 MiB：有效数据只到 `0x20E1`，512 KiB 已有 8 倍余量；且 512 KiB 能被常见的 NAND 块大小（128 KiB）整除。

**兼容旧版**：早先生成的 1 MiB 镜像（magic `HL2-1MiB`）偏移与本版完全一致，工具仍可正常解析；但新生成一律是 512 KiB + `HL2-512K`。更早的产物 `0x0030` 起是 `0xFF`（还没有派生 MAC 槽位），这类文件同样能读——派生值由 label MAC 现算，界面显示不受影响。

GUD 里**只搬运 SN 和 MAC**（label + 4 个派生值），SSID / 密码 / 生产日期 / 设备密钥 / PON 密码留在原分区，不写入紧凑镜像。

> **GUD 字段偏移注意**：`ctromfile_GUD.cfg` 是 **17 字符**而非 16，因此它之后的字段（密码副本 / 设备密钥 / PON 密码 / 标记 / 生产日期）比常见资料里写的偏移**多 1 字节**。代码里用的是实测值。

## AN4475（10G PON）

AN4475 是第三套完全不同的布局：出厂数据**不在裸分区里**，而是装在一个 UBI 卷里的 UBIFS 上。

| 项目 | 实测值 |
|---|---|
| mtd9 大小 | 25 MiB = 200 × 128 KiB PEB |
| UBI 卷表 | PEB 0 / 1 各一份副本，卷 0 = `fhdata`，178 LEB ≈ 22.25 MiB |
| 文件系统 | UBIFS（节点头 magic `0x06101831`，同级节点按 sqnum 取最新副本） |
| 出厂文件 | 21 个，原始共 1 402 393 字节 |
| 身份字段 | 全在 `factory_conf` 的 UCI 文本里（SN / 12 套 MAC / GPON SN / 密码） |

### 卷内文件

| 文件 | 大小 | 内容 |
|---|---|---|
| `MT7993_EEPROM.bin` | 1 353 728 | Wi-Fi 校准 + 两位 MAC，81% 是 `0xFF` |
| `pre_usrconfig_conf` | 16 961 | 预置用户配置模板 |
| `tr.conf` | 5 069 | 远程管理参数 |
| `pre_attrconfig_conf` | 4 653 | 预置属性模板 |
| `factory_conf` | 3 756 | **身份主文件**：SN / MAC / GPON SN / 密码 / SSID |
| `fi_json.config` | 3 035 | 工厂 JSON 配置 |
| `table` / `xg_table` / `xgs_table` / `10ge_sym_table` / `10ge_asym_table` / `olt_table` | 各 2 048 | 6 张 BOSA 校准表（`fiberhomebdbob` 头 + 14 字符时间戳 + 表体） |
| `sysinfo_conf` | 854 | 软件/硬件版本、区域码、运营商 |
| `LanOpticTxCalib` | 536 | LAN 光模块 TX 温补（8 个 Slope/Offset） |
| `voice_digitmap_conf` | 481 | 语音拨号表 |
| `olt_bosa_apd_WTD01` / `olt_bosa_apd_YS01` | 298 / 300 | BOSA APD 定标（本机 BOSA = `WTD01`） |
| `olt_bosa_info` / `olt_bosa_rssi` | 230 / 204 | BOSA 型号与 RSSI 定标 |
| `wlan` / `factorymodeflag` | 0 | 空旗标文件 |

另有 15 个 `.factory_conf.uci-XXXXXX`，是 UCI 写入时的临时文件（中间态），导入时按「以 `.` 开头」跳过。

### 身份字段与 MAC 规律

`factory_conf` 里 12 个 MAC 全部明文，实测关系如下（真机 `D4:9F:29:04:E7:50` 一组）：

| 字段 | 与基准 MAC 的关系 | 真机值 |
|---|---|---|
| `base_mac` / `brmac` / `ponmac` | — | D4:9F:29:04:E7:50 |
| `internetmac` | +1 | D4:9F:29:04:E7:51 |
| `tr069mac` | +2 | D4:9F:29:04:E7:52 |
| `voipmac` | +3 | D4:9F:29:04:E7:53 |
| `priprotocolmac` | +8 | D4:9F:29:04:E7:58 |
| `eth1mac`…`eth5mac` | **另一段**基址 +0…+4 | D4:A0:29:04:E7:71…75 |

> LAN 段不是基准 MAC 加偏移：真机上两者相差 `0x01_00_00_21`，属于另一段预留地址。所以工具存**两个**基址（基准 MAC + LAN 基址），不是只存一个。

另外两条确有实测依据的规律：

- **SN = `F3200` + 基准 MAC 的 12 位 hex**：`F3200` + `D49F2904E750` = `F3200D49F2904E750`（17 字符）。界面上的「按 MAC 生成」就是这条。
- **GPON SN = ASCII `FHTT` + 4 字节**：真机 `46485454C26A69FF` → `FHTT` + `C2 6A 69 FF`。这 4 字节与 MAC/SN 无对应关系，只能单独填。
- **GPON 密码是 hex 编码的 ASCII**：`48473236303132333435` → `HG26012345`。

### 1 MiB 容器（`AN4-1MiB`）

与 H3C 两台机型不同，这里**不能**用固定偏移摆放：单机唯一数据共 1.40 MB，其中 Wi-Fi EEPROM 一个文件就 1.32 MiB，1 MiB 装不下。因此改成自描述的容器 —— 头部固定偏移放身份（消费方不解析 TOC 也能直接读），其余文件进 TOC + zlib 载荷：

| 偏移 | 长度 | 内容 |
|---|---|---|
| `0x000000` | 8 | magic `AN4-1MiB` |
| `0x000008` | 4 | 版本 = 1 |
| `0x00000C` | 4 | 文件数 |
| `0x000010` | 4 | TOC 偏移 = `0x1000` |
| `0x000014` | 4 | 载荷偏移 = `0x2000` |
| `0x000018` | 4 | 载荷长度 |
| `0x000020` | 24 | 设备序列号（ASCII，17 字符 + `0xFF`） |
| `0x000038` | 8 | GPON SN（二进制，含 `FHTT`） |
| `0x000040` | 6 | 基准 MAC |
| `0x000046` | 6 | LAN 基址 MAC（eth1） |
| `0x00004C` | 3 | OUI（基准 MAC 前三字节） |
| `0x00004F` | 2 | 厂商码 `FH` |
| `0x000051` | 10 | GPON 密码（ASCII） |
| `0x000060` | 54 | 派生 MAC 9 项（internet/tr069/voip/priprotocol + eth1..5） |
| `0x000096` | 6 | Wi-Fi 2.4G MAC（与 EEPROM `+0x04`、`APMAC_1` 同步） |
| `0x00009C` | 6 | Wi-Fi 5G MAC（与 EEPROM `+0x0A`、`APMAC_2` 同步） |
| `0x001000` | 48/项 | TOC：名 32 + `off`/`raw_len`/`stored_len` 各 4 + 压缩标志 1 |
| `0x002000` | — | 载荷（能压就压，压不动原样存） |

实测载荷 **210 962 字节**（EEPROM 从 1.32 MiB 压到约 195 KiB），1 MiB 余量充足。若换用更大的 EEPROM 导致溢出，导出会直接报错并指出是哪个文件、写到哪个偏移，不会静默截断。

**身份回写**：导出前会把 SN / MAC / GPON SN / 密码写回 `factory_conf` 的对应 UCI 段（只改 `option value`，其余字节与缩进原样保留），保证文本与头部一致。来回解析 21 个文件逐字节一致（已验证）。

### Wi-Fi EEPROM 里的 MAC

`MT7993_EEPROM.bin` 的头很短，前面就是两个 MAC 槽，后面才是射频校准：

| 偏移 | 长度 | 内容 | 真机原值 |
|---|---|---|---|
| `+0x00` | 2 | device id（u16 LE） | `0x7993` = MT7993 |
| `+0x02` | 2 | 格式版本（u16 LE） | `2` |
| `+0x04` | 6 | 2.4G MAC | `00:0C:43:26:60:10` |
| `+0x0A` | 6 | 5G MAC | `00:0C:43:26:60:11` |
| `+0x10` | 384 | 保留，全 `0x00` | — |
| `+0x190` | — | 射频校准表起点 | — |

`00:0C:43` 是 **MediaTek 自己的 OUI**，两位只差末尾 `…10`/`…11` —— 这是芯片厂模板里的默认值，说明**产线没写这两位**。`factory_conf` 里的 `APMAC_1` / `APMAC_2` 同样是占位值（`00:0a:c2:d6:00:00`），两处互相也不一致。

因此本工具把 Wi-Fi MAC 当成身份的一部分来处理：

- 界面上有 `Wi-Fi 2.4G MAC` / `Wi-Fi 5G MAC` 两个框，可手填，也可点「按基准 MAC 派生」
- 导出时**三处一起写**：1 MiB 头部 `0x96`/`0x9C`、EEPROM `+0x04`/`+0x0A`、`factory_conf` 的 `APMAC_1`/`APMAC_2`
- EEPROM 其余 1.32 MiB（含全部校准表）逐字节原样保留，只动这 12 字节
- 导入时若 EEPROM 的 device id 不是 `0x7993`，界面会标出来

> 派生偏移取 `+6`/`+7`（基准 MAC 段内 `+1/+2/+3/+8` 已被上网/TR-069/VoIP/私有协议占用，`+4/+5` 在 H3C 机型上是 5G/2.4G，为避开冲突往后取）。**这是本工具的约定，没有出厂实证** —— 真机两处都是厂默认值，无从反推。有实测值就在界面上直接改。

> 压缩实现差异：Rust 侧用 `miniz_oxide`、Python 侧用 `zlib`，同一份输入打出来的**压缩字节可能不同**（本次实测载荷 218 787 vs 210 969 字节），**解压后的文件内容完全一致**。跨语言比对请比内容，不要比整个镜像的 MD5。

## 目录结构

```
h3c-factory/
├── Cargo.toml
├── Cargo.lock                 # 锁定 egui/eframe 0.36.2
├── LICENSE                    # GPL-2.0-only
├── README.md
├── .gitignore
├── .github/workflows/build.yml # GitHub Actions：三平台构建（产物见 Actions Artifacts）
├── src/
│   ├── font.rs      # CJK 字体安装（fontdb，移植自上游）
│   ├── main.rs      # egui 编辑器（机型下拉 + 三套面板）
│   ├── model.rs     # HM2004-DU 布局常量 + Factory + encode
│   ├── hl202.rs     # HL202-DU 布局常量 + Hl202 + encode_compact（512 KiB）
│   ├── an4475.rs    # AN4475：mtd9 UBI/UBIFS 解析 + 1 MiB 容器
│   └── import.rs    # 解析真实 factory 卷 / mtd13 UBIFS
├── h3c_factory_demo.py
└── examples/
    ├── factory-h3c-demo.bin          # 主机 1 MiB 示例
    └── factory-hl202-demo-512k.bin   # 从机 512 KiB 示例
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
