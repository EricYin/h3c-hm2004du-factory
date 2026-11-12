# AN758X Factory — H3C HM2004-DU / HL202-DU / AN4475 出厂分区编辑器

> 项目统一命名为 **AN758X-factory**（原 `h3c-hm2004du-factory`）：可执行文件、CI 产物、
> 包名、Python 伴生脚本全部同步改名，功能与数据布局不变。

Rust + egui 图形工具，用于**查看 / 编辑 / 对照** H3C FTTR 设备的出厂分区里的设备身份与校准数据。支持三种机型，在顶栏第二行的**机型按钮**上点击展开列表切换：

| 机型 | 角色 | 出厂分区 | 下行 PON 校准 | 紧凑镜像 |
|---|---|---|---|---|
| **HM2004-DU** | 主网关（AN7581） | mtd14 `reservearea` + mtd13 `ubifs1` | 有（mtd13，UX3326，288 B） | 1 MiB，`H3C-1MiB` |
| **HL202-DU** | 从机 | mtd12 `reservearea` | **无**（该机型没有此分区） | 512 KiB，`HL2-512K` |
| **AN4475** | 10G PON ONT | mtd9（UBI 卷 `fhdata` / UBIFS） | 以 6 张 BOSA 校准表形式存在 | 2 MiB，`AN4-2MiB`（容器） |

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

界面仿上游 FiberHome 编辑器：顶栏第一行是标题，第二行是**机型按钮**（`机型：…  ▼`，点击就地展开 `主机 HM2004-DU` / `从机 HL202-DU` / `AN4475 (10G PON)` 三个选项，也可用 `Ctrl+1/2/3` 直接切换）与动作按钮（`新建` / `打开…` / `打开对应分区` / `保存`）；按钮组左对齐、自动换行，窗口再窄也不会有按钮被挤出可视区；整个界面在滚动区里，内容超出时右侧常显竖向滚动条（滚轮 / 拖动均可）。下面依次是状态行、「身份信息」区（标签 MAC / 设备序列号，各带随机按钮）、派生 MAC 只读表、PON 校准与 Wi-Fi EEPROM 的「替换 / 导出 / 清空」分组行。H3C 两套面板都显示派生 MAC 表（末字节 +1/+2/+4/+5），从机面板另外只读展示 GUD 出厂字符串，AN4475 面板另有 Wi-Fi 2.4G/5G MAC 输入框与 MT7990AN device id 提示。

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
an758x-factory.exe mtd14_reservearea.bin mtd13_h3c.bin
# Linux/macOS
./an758x-factory mtd14_reservearea.bin mtd13_h3c.bin
```

状态栏下方会显示两个分区各自的来源文件名，`<未载入>` 表示该分区尚未打开。

界面固定为**亮色主题**（白色）。这个"白色"由三处配合完成，缺一不可 —— 前三处都是对 eframe/egui 0.36.2 源码逐行核实后的结论：

1. **重写 `App::clear_color` 返回纯白** —— 这是关键。eframe 的默认清屏色是**硬编码的深色** `(12,12,12)`@alpha180，完全无视主题（`eframe/src/epi.rs` 里写着 "a bright gray makes the shadows of the windows look weird"）；而 egui 0.36 传给 `App::ui` 的根 `Ui` **自己不画背景**（官方注释："has no margin or background color"），于是这个深色透出来占满整个窗口。只改 `Visuals` 是治标不治本：控件会变白，窗口依旧发黑。
2. **`apply_light_theme` 把 dark / light 两个 style 都写成亮色** —— `theme_preference` 默认是 `System`，系统为深色时 `Options::theme()` 解析成 `Dark`，Ui 会一直按 dark style 取色；两个 style 都写亮，解析结果就无所谓了。顺带 `set_theme(Light)` 固定偏好。stock 亮色盘的两处也顺手调了：`weak_text_color`（默认由 `text_color * 0.6` 派生，白底上发灰）钉成 110，三态按钮底色 230/220/165 拉开成 238/205/170。
3. **`ui.set_style(...)` 钉住当前帧** —— 根 `Ui` 的 style 在帧开始时就被快照成 `Arc<Style>`，只改 ctx 要等下一帧才生效，首帧会闪一下深色；在 `App::ui` 开头把根 Ui 的 style 也换掉，首帧起就是白的。

已知限制：eframe 0.36.2 不转发 `ViewportCommand::SetTheme` 给原生窗口，所以**窗口标题栏仍跟随系统主题**（仅装饰，应用内容全部为白色）。

## Python 伴生工具（无需 Rust）

`an758x_factory_demo.py` 是命令行版，逻辑与 `src/model.rs::encode()` 完全一致：

```bash
# 生成 1 MiB 紧凑镜像（默认）
python3 an758x_factory_demo.py gen --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a -o factory-h3c-1mib.bin

# 生成真实偏移布局镜像（~1.75 MiB）
python3 an758x_factory_demo.py gen --layout true --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a -o factory-h3c-layout.bin

# 从真实 reservearea 备份解析
python3 an758x_factory_demo.py parse --bin mtd14_reservearea.bin

# 从 mtd13 解析下行 PON 口校准
python3 an758x_factory_demo.py parse --bin mtd13_h3c.bin

# 把下行 PON 口校准一并打进 1 MiB 紧凑镜像（--poncal 接受 mtd13 或裸 288 字节 blob）
python3 an758x_factory_demo.py gen --sn H3CT0005EBF0 --mac 44:76:09:65:06:4a \
    --poncal mtd13_h3c.bin -o factory-h3c-1mib.bin
```

### HL202-DU（从机）

```bash
# 解析从机 mtd12（同时打印 GUD 全部字段）
python3 an758x_factory_demo.py hl202 --bin mtd12.bin

# 从 mtd12 直接转成从机 512 KiB 镜像
python3 an758x_factory_demo.py hl202 --bin mtd12.bin -o factory-hl202-512k.bin

# 只换 SN / MAC 再生成
python3 an758x_factory_demo.py hl202 --bin mtd12.bin \
    --sn H3CT091030F8 --mac 44:76:09:10:30:f8 -o factory-hl202-512k.bin
```

### AN4475（10G PON）

```bash
# 解析 mtd9 备份（打印身份字段、9 个派生 MAC、21 个出厂文件）
python3 an758x_factory_demo.py an4475 --bin mtd9_factory.bin

# 直接转成 2 MiB 容器
python3 an758x_factory_demo.py an4475 --bin mtd9_factory.bin -o factory-an4475-2mib.bin

# 换身份再生成（出厂文件仍取自备份，只改身份）
python3 an758x_factory_demo.py an4475 --bin mtd9_factory.bin \
    --sn F3200D49F2904E800 --mac D4:9F:29:04:E8:00 \
    --eth-mac D4:A0:29:04:E9:00 --gpon-sn 46485454C26A6A00 \
    --gpon-pw HG26099999 --wifi-24 D4:9F:29:04:E8:06 \
    --wifi-5g D4:9F:29:04:E8:07 -o factory-an4475-2mib.bin

# 也可以直接读回已生成的 2 MiB 容器
python3 an758x_factory_demo.py an4475 --bin factory-an4475-2mib.bin

# GUI 程序同样带一个无界面批处理入口（产线批量转换用）
an758x-factory --export-an mtd9_factory.bin factory-an4475-2mib.bin
an758x-factory --export-mt76 factory-an4475-2mib.bin mt7990_eeprom.bin

# 打包时就把上行 PON 按 fiberhome-factory 流程转成 APONCAL，省一步
an758x-factory --export-an mtd9_factory.bin factory-an4475-aponcal.bin --pon-aponcal
python3 an758x_factory_demo.py an4475 --bin mtd9_factory.bin \
    -o factory-an4475-aponcal.bin --pon-aponcal

# GPON SN 两种写法都收（`FHTT` + 8 位 hex 或 16 位 hex），规则同 fiberhome-factory
an758x-factory --export-an mtd9_factory.bin out.bin --gpon-sn FHTTC26A69FF
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

### PON / 光组件：上行与下行是两条独立的路

这台是 **FTTR 主网关**：上行做 ONT 接运营商 OLT，下行 FTTR 口反过来做 OLT 带从 ONU。两个方向各有一颗光组件和一颗 BOSA 驱动芯片，界面按方向分开列：

| 方向 | 光模块（`factory_conf`） | 驱动芯片 | 校准表 |
|---|---|---|---|
| 上行 PON（ONT） | `PLDM588-815-C10` · SN `000059BTK47068533` | UX3363（chip id `0x2E`） | `10ge_asym` / `10ge_sym` / `xg` / `xgs` / `table`（5 张） |
| 下行 FTTR PON（OLT） | `PLDM586-552` · SN `000102ADL16001446` | UX3326S1（chip id `0x27`） | `olt_table` + `olt_bosa_apd_*` + `olt_bosa_rssi` + `LanOpticTxCalib`（6 张） |

以上都不是猜的，三条硬证据：

1. **`factory_conf` 明文分列**：`transceiver_type` / `transceiver_serial` 是上行光模块，`lan_optic_type` / `lan_optic_serial` 是下行光模块。
2. **校准表头 `0x30` 就是 BOSA 驱动 chip id**：上行 5 张全是 `0x2E`，`olt_table` 是 `0x27`。`/etc/rc.d/rcS` 里 `i2c_debug getchipid_down` 读到 `0x27` 才 `cp /etc/table.ux3326s /fhdata/olt_table`，与本机 `olt_table` 的头字节完全吻合。
3. **出厂默认表成对存在**：`/etc/table.ux3361`（`202404291559VB`、`0x2E`）与 `/etc/table.ux3326s`（`202409230900VA`、`0x27`），文件名直接写着驱动型号；`i2c_debug` 支持的芯片串是 `UX3320C / UX3363 / UX33650 / UX3326S1`。

`fsl_core_fttr_chip.ko` 的 `description=Fisilink CRANEV2R2 Sys Driver v1.0(2019.02.26)`、`ae_wan.ko` 的 `xsi_set_xsgmii_serdes_an8831_phy_preset` 说明下行还有 FTTR ASIC 与外置 10G PHY 的代码路径，但这些**不在 21 个出厂文件里**，界面只显示校准包能证实的东西，不画板级拓扑。

#### 几处容易误判

- **不是「一颗芯片两个方向」**。`olt_*` 这组确实挂在下行那条路上（`rcS` 的 `#2. olt bosa_driver` 整段包在 `if [ -f /fhconf/multi_lan ]` 里），`olt_bosa_id = WTD01` 是**下行 BOSA 的 APD 偏压/温补型号**，不是上行模块厂商。
- **`olt_table` 被产线重写过**：它与 `/etc/table.ux3326s` 相差 214 字节（版本戳相同）；而 `LanOpticTxCalib`（536 B）、`olt_bosa_apd_WTD01`（298 B）、`olt_bosa_rssi`（204 B）与 `/fhrom/fhconf/` 出厂默认**逐字节相同** —— 产线只重写了主表。
- **三份表内容完全相同**：`10ge_sym_table`、`xgs_table`、`table` 逐字节一致（`155a901a…`），对称 10G 与通用模式复用同一份 BOB 校准；`10ge_asym_table`（`86139327…`）与 `xg_table`（`680669fd…`）只差 11 字节元数据（`0x74` 是 payload CRC32）。

BOB 表头固定：`fiberhomebdbob`（14 B）+ 16 B 版本戳 + `0x30` chip id + `0x70` payload 长度 + `0x74` CRC32，载荷从 `0x80` 起 1920 B。上行表的型号明文在 `0x95`（`ux3363`），下行在 `0x1C4`（`UX3326S1`），工具扫 `0x80–0x1FF` 的 ASCII 串，两条都能取到。

### PON 校准 → APONCAL：按上游 fiberhome-factory 的流程逐张处理

上游 `fiberhome-factory` 只处理**一张**表：读 `fhdata/table` → `validate_bdbob()` 校验长度 / 魔数 / 两处 CRC → 按头 `0x30` 的 chip id 映射成 `GN28L95` 或 `UX3363` → `encode_pon_calibration()` 重封成 2048 B 的 `APONCAL` 容器，落在固定 `0x0000`。

AN4475 有 6 个 PON 槽，本工具把同一套流程**逐槽跑一遍**，每张都留结果：

```bash
# Rust（GUI 同一份代码，另有无界面入口）
./an758x-factory --export-pon-cal factory-an4475-2mib.bin out/

# Python 伴生工具
python3 an758x_factory_demo.py an4475 --bin factory-an4475-2mib.bin --export-pon-cal out/
```

GUI 里在「PON / 光组件」块的表末多一列状态，块底有「全部导出 APONCAL…」按钮，选目录即写 6 个文件 + 报告。

#### 打包时就内联：`--pon-aponcal`

逐张导出适合「镜像已经有了、只想把表抽出来」；如果镜像本来就要重新打包，可以一步到位：

```bash
an758x-factory --export-an mtd9_factory.bin out.bin --pon-aponcal
```

打包时对上行 5 张各跑一遍同一套流程，成功了就写进各自的固定槽（`0x1A0000` 起），失败则**保留原表**并在明细里标「保留原表 · 原因」—— 半张 APONCAL 比原表更危险。下行 `olt_table` 无论开关都保持原厂 BDBOB。

| 模式 | 与「原样打包」的差异 |
|---|---|
| 不开（默认） | PON 区逐字节相同（实机镜像实测 0 差异） |
| 开 | 只动 5 个上行槽，共 200 B（每槽的 magic / chip / 版本 / 两处 CRC）；`olt_table` 槽不变 |

重复打包是幂等的：对已内联的镜像再跑一次，明细显示「已是 APONCAL」，产物逐字节相同。

> 这种镜像的 PON 区不再是 BDBOB，**原厂 `mt_wifi` 固件不认**，只适合 PonWrt / OpenWrt。
> GUI 里对应「保存 2 MiB 时把上行 5 张内联为 APONCAL」复选框。

真机（`factory-an4475-2mib.bin`）实测结果：

| 槽 | 表名 | chip | 状态 | 命中种子 | 输出 |
|---|---|---|---|---:|---|
| `0x1A0000` | `10ge_asym_table` | `0x2E` | PASS·换种子 | 标准 zlib | `5441ba07…` |
| `0x1A1000` | `10ge_sym_table` | `0x2E` | PASS | 原厂 | `82ade014…` |
| `0x1A2000` | `xg_table` | `0x2E` | PASS·换种子 | 标准 zlib | `d736edeb…` |
| `0x1A3000` | `xgs_table` | `0x2E` | PASS | 原厂 | `82ade014…` |
| `0x1A4000` | `table` | `0x2E` | PASS | 原厂 | `82ade014…` |
| `0x1A5000` | `olt_table` | `0x27` | chip 不在枚举 | 标准 zlib | 不产出 |

（md5 是导出后的 2048 B 整文件。`10ge_sym` / `xgs` / `table` 三张 payload 逐字节相同，所以成品也逐字节相同。）

**可交付 PonWrt：5/6 张。**

#### 两处与上游不同的地方

上游只看一张表，这两件事它碰不到；AN4475 会碰到，所以必须显式处理。

**① CRC 有两套种子，不能只用一套。**

| 种子 | 实现 | 用它的表 |
|---|---|---|
| 原厂（u-boot 口径） | `crc32_register(d, 0) ^ 0xFFFFFFFF` | `10ge_sym` / `xgs` / `table` |
| 标准 zlib 口径 | `crc32_register(d, 0xFFFFFFFF) ^ 0xFFFFFFFF` | `10ge_asym` / `xg` / `olt_table`（含 `mtd9` 的 `table`） |

上游 `fiberhome_crc32()` 只实现了第一种。直接套用会把 `10ge_asym` / `xg` / `olt_table` 判成「header CRC mismatch」，看着像三张坏表，其实完好。工具先按原厂种子验，不过再按标准种子验，报告里标出实际命中的那套；只要有一套对上就照常导出，**payload 一个字节不动**。

**② chip id `0x27`（UX3326S1）不在 APONCAL 枚举里。**

枚举只有 `1 = GN28L95`、`2 = UX3363`，PonWrt 的 `bosa_load_calibration()` 也只接受这两个值。下行 `olt_table` 的 chip 是 `0x27`，所以判 `chip 不在枚举`、**不产出文件** —— 硬填 `2` 会让驱动走 UX3363 的 apply 分支，calibration 静默不生效，比不加载更难查。

#### 成品结构

```
0000  APONCAL\0           magic（8 B）
0008  01 00               version = 1
000a  80 00               hdrsize = 0x80
000c  02 00 00 00         chip = 2（UX3363）
0010  01 00 00 00         format = 1（原生 payload，不重排）
0014  80 07 00 00         payload 长度 = 1920
0018  xx xx xx xx         payload CRC（标准 zlib）
007c  xx xx xx xx         头 CRC（标准 zlib，覆盖 0x00–0x7B）
0080  ...                 BDBOB 的 1920 B payload，原样搬运
```

总长固定 2048 B，`0x800` 之后全 `0xFF`。两个 CRC 都按标准 zlib 算 —— 直接抄原表 `0x74` 那个值会判 CRC 失败、整段丢弃。

导出时还会按 PonWrt `bosa_load_calibration()` 的规则再复核一遍（magic / version / hdrsize / format / chip ∈ {1,2} / `plen ≤ cell-0x80` / `plen ≥ 0x780` / 两个 CRC），任何一项不过都会写进报告。

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

#### GPON SN 的两种写法（与 fiberhome-factory 一致）

同一个 8 字节有两种说法，工具两种都收、也都能显示：

| 写法 | 例子 | 用在哪 |
|---|---|---|
| 厂商码 + 8 位 hex | `FHTTC26A69FF` | 贴纸、工单、`random_sn()` |
| 16 位 hex | `46485454C26A69FF` | UCI `GponSN`、头部 `0x38` 原样 |

解析顺序照搬上游 `display_sn()`：**先**看是不是 12 个字符且前 4 个是 ASCII 字母数字，是就按「4 字节 ASCII + 4 字节 hex」拆；**否则**整串（容忍 `:` `-`）当 16 位 hex 解 8 字节，并拒绝全 `00` / 全 `FF`。

读回时反过来：前 4 字节都是字母数字就拼成 `FHTT` + 8 位 hex，否则给 16 位 hex（厂商码不是 ASCII 的老设备）。

界面上输入框右侧实时显示「写进去的 8 字节」，格式不对直接红字，**保存会被拒绝**（旧版是静默保留原值，漏一位都看不出来）。`随机` 按钮同上游 `random_sn()`：`FHTT` + 4 字节随机。

命令行的 `--gpon-sn` 走同一套解析，两种写法产出的镜像逐字节相同：

```bash
an758x-factory --export-an mtd9_factory.bin out.bin --gpon-sn FHTTC26A69FF
an758x-factory --export-an mtd9_factory.bin out.bin --gpon-sn 46485454C26A69FF  # 等价
```

#### 给 PonWrt 的 12 字符视图（头部 `0xA2`）

PonWrt 的 board 表只提供三种 identity 编码：`pon-ascii12`（12 字符 ASCII）、`mac-colon17`、`mac-binary6`。`0x38` 那种「4 字节 ASCII 厂商码 + 4 字节二进制序列号」**没有一种装得下**，而 C 侧不改。

解法是把同一份 SN 在容器头部空闲区再存一份 12 字符文本：

| 位置 | 长度 | 内容 | 谁来读写 |
|---|---|---|---|
| `0x38` | 8 | 二进制：`FHTT` + `C2 6A 69 FF` | 厂商固件 / 本工具 |
| `0xA2` | 12 | ASCII：`FHTTC26A69FF` | **PonWrt**（`pon-ascii12`） |

`0xA2`–`0xFFF` 在真机上全是 `0xFF`，不与任何既有字段重叠（头部字段到 `0xA2` 就结束了）。

同步规则只有一条：**视图有效就以视图为准**。

- 打包时：`0x38` 写 8 字节二进制，`0xA2` 写 12 字符文本，`factory_conf` 的 `GponSN` 写 16 位 hex —— 三处同源；
- 打开时：若 `0xA2` 是 12 个可打印 ASCII 且能解析，**压过 `0x38`**（说明 PonWrt 改过了）；否则（老镜像的 `0xFF`、或厂商码非 ASCII 写不下）退回 `0x38`；
- 于是「PonWrt 改 `0xA2` → 用本工具打开并保存」就把 `0x38` 与 `factory_conf` 一起带过来了。

PonWrt 侧只需在 `case "$board"` 里补一段，不动 C：

```sh
	# 0xA2: 12-char ASCII view of the 8-byte binary GPON SN at 0x38 (0xA2 = 162).
	# an758x-factory keeps 0x38 / 0xA2 / factory_conf GponSN in sync on (re)pack.
	fiberhome,an4475-g2)
		identity_available=1
		json_add_object identity
		json_add_object targets
		json_add_object factory
		json_add_object fields
		identity_field pon_sn pon-sn 162 12 pon-ascii12
		identity_field pon_mac mac 64 6 mac-binary6
		identity_field lan_base_mac mac 70 6 mac-binary6
		json_close_object
		json_close_object
		json_close_object
		json_close_object
		;;
```

副作用要知道：**导出的镜像比输入多 12 字节差异**（就在 `0xA2`），其余字节不变。老镜像 `0xA2` 是 `0xFF`，写进去就是这 12 个字节；不想要这份视图时把 `AN_H_GPON_SN_VIEW` 那段写入去掉即可。

`pon_mac` / `lan_base_mac` 用 `mac-binary6` 指向 `0x40` / `0x46` 是原生 6 字节，PonWrt 可以直接读写；但 `factory_conf` 里那 12 套 MAC 是派生值，改动后仍需本工具重新派生一次才一致。

工具在派生表下方做唯一性自检：12 个地址（基准 + 4 个网络侧 + 5 个 LAN + 2 个 Wi-Fi）出现重复就红字点名，例如「MAC 冲突：D4:A0:29:04:E7:71 同时用于 eth1mac / Wi-Fi 2.4G」。原厂这套分配本身不冲突（`D4:9F` 段占 50–53/58，`D4:A0` 段占 50/58 与 71–75，两个前缀互不相干），冲突只可能来自手改；另一种「冲突」是整片镜像刷到第二台机 —— 那时 12 个地址全都重复，只能靠每台重新生成，工具没法代劳。

### 2 MiB 容器（`AN4-2MiB`）

与 H3C 两台机型不同，这里**不能**全用固定偏移摆放：单机唯一数据共 1.40 MB，其中 Wi-Fi EEPROM 一个文件就 1.32 MiB，1 MiB 装不下（真实分区按 2 MiB 规划）。因此改成自描述的容器 —— 头部固定偏移放身份，**EEPROM 和 11 张 PON 校准表也各占固定槽、原样不压缩**（都能 hex 直读），只有 9 个配置文件进 TOC + zlib 载荷：

| 偏移 | 长度 | 内容 |
|---|---|---|
| `0x000000` | 8 | magic `AN4-2MiB`（旧镜像 `AN4-1MiB` 仍可读） |
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
| `0x0000A2` | 12 | **GPON SN 的 12 字符视图**（`FHTTC26A69FF`），给 PonWrt 的 `pon-ascii12` 用；老镜像此区为 `0xFF` |
| `0x0000AE`–`0x000FFF` | 3402 | 空闲，全 `0xFF` |
| `0x001000` | 48/项 | TOC：名 32 + `off`/`raw_len`/`stored_len` 各 4 + 压缩标志 1 |
| `0x002000` | — | 其余 9 个配置文件（能压就压，压不动原样存），实测占到 `0x003CAB` |
| `0x040000` | 1 353 728 | **`MT7993_EEPROM.bin` 原样存放、不压缩**（Wi-Fi 校准区只读这个偏移即可） |
| `0x18A800` | — | EEPROM 结束 |
| `0x1A0000` | 11 × 4 KiB | **PON 校准表直读区**，每张一个 `0x1000` 槽、原样不压缩，见下表 |
| `0x1AB000` | 7 680 | **mt76 直读槽**：打包时由 1.35 MB 自动派生（官方模板打底），OpenWrt 的 DTS nvmem 指到这里即可 |
| `0x1ACE00` | — | 之后到 `0x200000` 全 `0xFF`（332 KiB 空闲） |

实测载荷 **1 748 480 字节**（`0x1AAE00` 处结束）—— 其中 EEPROM 1 353 728 B、11 张 PON 校准表 13 856 B、mt76 槽 7 680 B 都是原样占用、压不动，真正被压缩的 9 个配置文件只有 **7 339 B**（原始 34 809 B）。`0x1ACE00` 之后到 `0x200000` 全部是 `0xFF`。再加一份 7 680 B 的 mt76 副本只吃掉 7.5 KiB，尾部余量仍然充足。若换用更大的 EEPROM 导致越界，导出会直接报错并指出是哪个文件、写到哪个偏移，不会静默截断。

**Wi-Fi 校准可直接读**：EEPROM 不压缩，固定摆在 `0x040000`，长度 1 353 728 字节，头部 `0x7993` 就是 device ID。用任何 hex 工具打开 2 MiB 镜像、跳到 `0x40000` 就是完整校准区（`+0x04`/`+0x0A` 是 2.4G/5G MAC）。

**一块 factory 喂两套固件**：`0x1AB000` 的 7 680 B 是给开源 mt76 用的成品（模板打底，功率表是驱动能解释的 dBm 编码），原厂那份 1.35 MB 原封不动留在 `0x40000`。OpenWrt 侧只要：

```dts
eeprom_factory: eeprom@1ab000 { reg = <0x1ab000 0x1e00>; };

&pcie0 {
    wifi@0,0 {
        nvmem-cells = <&eeprom_factory>;
        nvmem-cell-names = "eeprom";
    };
};
```

`--export-mt76` 读新镜像时直接取这个槽（打印「取自容器 0x1AB000 的 mt76 槽」）；读旧镜像没有槽，就现场按模板打底算一份，两者结果逐字节相同。

**PON 校准表也可直接读**：11 张表各占一个 4 KiB 槽，槽内有效字节之后填 `0xFF`：

| 槽偏移 | 文件 | 有效长度 | 内容特征 |
|---|---|---:|---|
| `0x1A0000` | `10ge_asym_table` | 2 048 | 表头 `fiberhome` |
| `0x1A1000` | `10ge_sym_table` | 2 048 | 表头 `fiberhome` |
| `0x1A2000` | `xg_table` | 2 048 | 表头 `fiberhome` |
| `0x1A3000` | `xgs_table` | 2 048 | 表头 `fiberhome` |
| `0x1A4000` | `table` | 2 048 | 表头 `fiberhome` |
| `0x1A5000` | `olt_table` | 2 048 | 表头 `fiberhome` |
| `0x1A6000` | `olt_bosa_apd_WTD01` | 298 | 文本 `-40:-609` 起（APD 温补表） |
| `0x1A7000` | `olt_bosa_apd_YS01` | 300 | 文本 `-40:-639` 起 |
| `0x1A8000` | `olt_bosa_info` | 230 | 文本 `config k...` 起，含 BOSA 型号 |
| `0x1A9000` | `olt_bosa_rssi` | 204 | 文本 `1:-20:34` 起（RSSI 表） |
| `0x1AA000` | `LanOpticTxCalib` | 536 | 文本 `config k...` 起（光口 TX 校准） |

槽号顺序固定（`AN_CAL_TABLES`），不随文件名字典序变化；TOC 里仍记录每个文件的真实偏移，所以**解析不依赖槽位**，旧工具读新镜像、新工具读旧镜像都不受影响。

**旧镜像兼容**：早先生成的 1 MiB 容器（magic `AN4-1MiB`）头部偏移与本版完全一致，工具仍能读入并显示，状态栏会标注「旧版，导出将升级为 2 MiB」；重新保存即得到 2 MiB + `AN4-2MiB`。

**身份回写**：导出前会把 SN / MAC / GPON SN / 密码写回 `factory_conf` 的对应 UCI 段（只改 `option value`，其余字节与缩进原样保留），保证文本与头部一致。来回解析 21 个文件逐字节一致（已验证）。

### Wi-Fi 方案：MT7990AN + MT7976CN

本机是 WiFi 7 双频方案：数字基带 **MT7990AN**（EEPROM device id `0x7993`，mt76 里登记为 `MT7990_DEVICE_ID`），射频前端 **MT7976CN**，模块按**内建 PA（iPA）**配置。

- 频段只有 2.4G + 5G（BE3600，2×2 + 2×2），**没有 6G** —— EEPROM 里 `MT_EE_TX0_POWER_6G`（`0x1310`）与 `RATE_DELTA_6G`（`0x154A`）不参与，取什么值都无所谓
- `WIFI_CONF`（`0x190`）= `11 10 03 1a 10 02`：band0 2T2R、band1 3 链 2 流、band2 未启用
- `0x196`/`0x197` = `08 08` → `MT_EE_WIFI_PA_LNA_CONFIG = 0` → `FEM_INT`，只能配官方 `_2i5i`（iPA）模板；`_2e5e`（ePA）会被 `mt7996_eeprom_variant_valid()` 判为变体不符
- MT7976CN 在 EEPROM 里**没有独立字段**，只通过上述 FEM 位与通道/功率表体现

### Wi-Fi EEPROM 里的 MAC

`MT7993_EEPROM.bin` 的头很短，前面就是两个 MAC 槽，后面才是射频校准：

| 偏移 | 长度 | 内容 | 真机原值 |
|---|---|---|---|
| `+0x00` | 2 | device id（u16 LE） | `0x7993` = MT7990AN（基带） |
| `+0x02` | 2 | 格式版本（u16 LE） | `2` |
| `+0x04` | 6 | 2.4G MAC | `00:0C:43:26:60:10` |
| `+0x0A` | 6 | 5G MAC | `00:0C:43:26:60:11` |
| `+0x10` | 384 | 保留，全 `0x00` | — |
| `+0x190` | — | 射频校准表起点 | — |

`00:0C:43` 是 **MediaTek 自己的 OUI**，两位只差末尾 `…10`/`…11` —— 这是芯片厂模板里的默认值，说明**产线没写这两位**。`factory_conf` 里的 `APMAC_1` / `APMAC_2` 同样是占位值（`00:0a:c2:d6:00:00`），两处互相也不一致。

因此本工具把 Wi-Fi MAC 当成身份的一部分来处理：

- 界面上有 `Wi-Fi 2.4G MAC` / `Wi-Fi 5G MAC` 两个框，可手填，也可点「按基准 MAC 派生」
- 导出时**三处一起写**：容器头部 `0x96`/`0x9C`、EEPROM `+0x04`/`+0x0A`、`factory_conf` 的 `APMAC_1`/`APMAC_2`
- EEPROM 其余 1.32 MiB（含全部校准表）逐字节原样保留，只动这 12 字节
- 导入时若 EEPROM 的 device id 不是 `0x7993`，界面会标出来

> **原厂规则（脚本实证）**：rootfs 里 `fhrom/fhshell/fac_start_wifi.sh` 与 `generate_wifi_mac.sh` 两处算法逐字相同 —— `brmac` 的**第 2 字节 +1**，5G 再把**末字节 +8**，2.4G 末字节不变；两个字节都是 u8 回绕，不向邻近字节进位。以本机 `brmac = D4:9F:29:04:E7:50` 为例：
>
> | 接口 | 值 | 算法 |
> |---|---|---|
> | `ra0`（2.4G） | `D4:A0:29:04:E7:50` | 第 2 字节 `9F`→`A0` |
> | `rai0`（5G） | `D4:A0:29:04:E7:58` | 第 2 字节 `9F`→`A0`，末字节 `50`→`58` |
>
> 原厂**不写** EEPROM 的 `+0x04/+0x0A`（实机仍是 MediaTek 默认值），也不写 `APMAC_1/APMAC_2`（仍是 `00:0a:c2:d6:00:00` 占位），只把派生值写进 profile 的 `MacAddress`。本工具沿用同一算法，但为了 mt76 侧也能取到，额外把结果写进 EEPROM 与 `factory_conf`。

**导入时会把占位值当「未写」处理**：`00:0C:43`（MediaTek OUI）、`00:0A:C2`（原厂 APMAC 占位）、全 `00`、全 `FF` 一律判为占位，改用 `brmac` 现算，避免整批机器带着同一个 MAC 出厂。判断逻辑在 `an4475::is_placeholder_mac()`，Python 侧同名函数 `an4475_is_placeholder_mac()`。

> 压缩实现差异：Rust 侧用 `miniz_oxide`、Python 侧用 `zlib`，同一份输入打出来的**压缩字节可能不同**（本次实测载荷 218 787 vs 210 969 字节），**解压后的文件内容完全一致**。跨语言比对请比内容，不要比整个镜像的 MD5。

### 原厂是怎么处理 Wi-Fi 校准的（完整备份实证）

拿到整机 7 个分区的原厂备份后，机制可以逐条落地了 —— **原厂跑的是 `mt_wifi`（厂商驱动），不是 mt76**，1.35 MB 与 7 680 B 是**同一文件的两个阶段**，不是两种格式的两份文件。

**分区表**（地址取自 `mtd2_KernelA` 里的 FDT）：

| 分区 | 文件 | 大小 | flash 偏移 |
|---|---|---|---|
| `mtd0` | `bootloader.bin` | 2.5 MiB | `0x0` |
| `mtd1` | `bootflag.bin` | 512 KiB | — |
| `mtd2` | `KernelA.bin` | 32 MiB | —（FDT 在此分区内，分区表就从这里读出） |
| `mtd3` | `RootfsA.bin` | 108 MiB | `0x2300000`（squashfs，xz） |
| `mtd6` | `apps.bin` | 132 MiB | `0x11B00000`（UBI） |
| `mtd7` | `frameworka.bin` | 16 MiB | `0x19F00000`（squashfs） |
| `mtd9` | `factory.bin` | 25 MiB | `0x1BF00000`（UBI，烧的是 `fhdata_ubi.img`） |

**mtd9 内部**：UBI（PEB 128 KiB × 200），只挂一个动态卷 `fhdata`，卷上是 UBIFS，共 21 个出厂文件。**`MT7993_EEPROM.bin` 位于 `/fhdata/wlan/` 下**，1 353 728 B，在 UBIFS 里按 zlib 分成 331 个数据块存放（整卷实际只占约 1.6 MB，其余 23 MB 是空的）。

**驱动怎么找校准**（`mt_wifi.ko` 里的字符串即证据）：

```
/fhdata/wlan/%s                                    ← 第一优先（产测写入的 1.35 MB）
/lib/firmware/MT7993_EEPROM.bin                    ← 默认 bin（7 680 B）
/lib/firmware/MT7993_MT7976_EEPROM_BE3600_iPAiLNA.bin
/lib/firmware/MT7993_MT7976_EEPROM_BE3600_ePAeLNA.bin
"Use default BIN from:%s." / "Unexpected error with default BIN."
```

启动脚本 `/etc/insmod_wifi7_BE3600.sh` 先把固件搜索路径改到 `/etc/wireless`，再依次 `insmod` `mt_wifi.ko` → `mtk_hwifi.ko` → `connac_if.ko` → `mtk_pci.ko` → `mt7993.ko`（`option_type=1 rro_mode=5`）→ `mt799b.ko`。

**rootfs 里那三份 7 680 B 的关系**（与你手头的 zip 逐字节一致）：

| 路径 | MD5 | 作用 |
|---|---|---|
| `/lib/firmware/MT7993_EEPROM.bin` | `74abc9a1…` | = `_iPAiLNA`，驱动默认 bin |
| `/lib/firmware/…_BE3600_iPAiLNA.bin` | `74abc9a1…` | 同一文件，按 FEM 变体命名的一份 |
| `/lib/firmware/…_BE3600_ePAeLNA.bin` | `9ff6b18e…` | ePA 变体，本机 `0x196=08` 不匹配，不会用 |
| `/etc/MT7993_EEPROM.bin` | `6b7aa278…` | 产测占位用，与默认版差 342 B，全在功率表（`0x1300` 低 2、`RATE_DELTA` 低 1~6） |

**产测流程**（`fac_start_wifi.sh` 的 `copy_cal_file`）：

1. 只有存在 `/fhdata/factorymodeflag` 时才执行
2. 若 `/fhdata/wlan/MT7993_EEPROM.bin` 不存在 → `cp /etc/MT7993_EEPROM.bin /fhdata/wlan/`，即先用 7 680 B 占位，保证驱动能起来
3. 产线工具随后把逐台校准的 1.35 MB 覆盖上去（实机就是这么来的：`factorymodeflag` 还在，文件已是 1.35 MB）

**关键结论**：产测覆盖后，**前 `0x1000` 只动了 10 个字节**（`0x44C–0x450`、`0xB91/B95/B99`、`0xBA8/BAA`，逐台通道偏置），而 **`0x1000–0x1DFF` 被整段换成厂商自己的编码**（`0xFF` 占 80%，与默认版 `0x00` 占 72% 完全不同）。也就是说 `0x1000` 往后在 `mt_wifi` 眼里根本不是 mt76 的 dBm 功率表 —— 这也解释了为什么把 1.35 MB 前 7 680 B 直接喂给 mt76 不能用。

> 反过来看 `0x1E00–0x14A7FF` 那 1.34 MB：按 2 KiB 切片统计，每片稳定是 `0xFF` 80% 上下、数据 5–16%，是典型的「逐信道 × 逐速率稀疏表」，与 `0x1000` 段同一种编码，只是往后延续。原厂把它当私有校准区，mt76 连读都不读。

### mt76 / OpenWrt：7 680 B 的 EEPROM 视图

厂商那份 `MT7993_EEPROM.bin` 是 **1 353 728 B**，而内核 mt76 的 `mt7996` 驱动只要 **7 680 B** —— 不是谁压缩了谁，而是**同一份数据的两个视图**：

| 段 | 长度 | 厂商 1.35 MB 里是什么 | mt76 里是什么 |
|---|---|---|---|
| `0x0000–0x0FFF` | 4 096 | 小 EE 头：芯片 ID / 版本 / 两个 MAC / `WIFI_CONF` / 通道偏置 | **同一套布局**，实测仅差 10 字节 |
| `0x1000–0x1DFF` | 3 584 | 厂商自己的位图编码（近 3 000 字节是 `0xFF` 加零星清位） | byrate 功率表 + `RATE_DELTA`，正常 dBm 值 |
| `0x1E00–0x14A7FF` | 1 346 048 | `mt_wifi` 私有校准区 | **不存在**，驱动不读 |

对齐依据是内核头文件：`mt7996/eeprom.h` 里 `__MT_EE_MAX = 0x1dff`，`mt7996.h` 里 `MT7996_EEPROM_SIZE = 7680`、`MT7996_EEPROM_BLOCK_SIZE = 16`；驱动无论走 efuse 还是外部 EEPROM，都只按 7 680 B 读，`/lib/firmware/mediatek/mt7996/mt7990_eeprom*.bin` 长度不足直接 `Invalid default bin size`。

> 顺带一个坑：`0x7993` 在开源驱动里登记成 **`MT7990_DEVICE_ID`**（`mt7996_check_eeprom()` 认它），所以 MT7990AN 在 Linux 下按 MT7990 跑，默认 bin 名也就成了 `mt7990_eeprom*.bin`。

工具给两种成品（界面「mt76 / OpenWrt EEPROM（7 680 B）」折叠区，或命令行）：

| 方式 | 做法 | 适用 |
|---|---|---|
| **模板打底**（默认） | 官方 `mt7990_eeprom_2i5i.bin` 打底，只覆盖 `0x004`/`0x00A` 的 MAC、`0x190` 的 6 字节 `WIFI_CONF`、`0x196` 的 PA/LNA | 给 mt76 用，功率表是驱动能解释的编码 |
| **原样截取** | 直接取厂商文件前 7 680 B | 只做字节级对照；`0x1000` 起是位图，mt76 会把 `TX0_POWER = 0xFF` 读成 −1 dBm |

> **整份 1.35 MB 直接改名当默认 bin 不可取**：`fw->size` 检查只挡「小于 7 680」，所以能过；但 `mt7996_eeprom_check_or_use_default()` 只 `memcpy(eeprom, fw->data, 7680)`，随后 `release_firmware()` 把扩展区丢掉。主线 6.x 里 `mt7996_eeprom_get_target_power()` / `get_power_delta()` 没有任何调用点（功率来自 DT `power-limits` 或 `phy->txpower`），所以垃圾功率段暂时"看不见"；一旦走旧分支或 vendor 补丁，`0x1300 = 0xFF` 就是 −1 dBm、`0x147D = 0xFF` 就是 +31.5 dB。另外 MAC 仍是 `00:0C:43:26:60:10/11`，设备树没有 `mac-address` 时整批同 MAC。

模板选 **iPA/LNA** 那份是有依据的：实机 `WIFI_CONF+6/+7 = 08 08`，`MT_EE_WIFI_PA_LNA_CONFIG` 两位为 0 = `MT7996_FEM_INT`；换成 ePA 那份（`8B 8B`，`FEM_EXT`）会被 `mt7996_eeprom_variant_valid()` 判变体不符，直接回落默认 bin。两份模板都在 `assets/` 里，`--mt76-template` 可换。

```bash
# 从 2 MiB 容器（或 mtd9 备份、或裸的 1.35 MB EEPROM）导出 7 680 B
./an758x-factory --export-mt76 factory-an4475-2mib.bin mt7990_eeprom.bin
# 原样截取
./an758x-factory --export-mt76 factory-an4475-2mib.bin mt7990_eeprom-carve.bin carve
# Python 侧等价
python3 an758x_factory_demo.py an4475 --bin factory-an4475-2mib.bin --export-mt76 mt7990_eeprom.bin
```

装到设备上：`/lib/firmware/mediatek/mt7996/mt7990_eeprom_2i5i.bin`（内建 PA/LNA 变体）。

> 本机实测（AN4475 真机 EEPROM）：模板打底的产物与官方 `MT7993_MT7976_EEPROM_BE3600_iPAiLNA.bin` **逐字节相同**（MD5 `74abc9a1…`）—— 因为产线连 Wi-Fi MAC 都没写，头段本来就是芯片厂原样。换言之这份板子的默认 bin 不用改，只有真要给每台机器不同 MAC 时才需要注入。

## 目录结构

```
an758x-factory/
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
│   ├── an4475.rs    # AN4475：mtd9 UBI/UBIFS 解析 + 2 MiB 容器
│   └── import.rs    # 解析真实 factory 卷 / mtd13 UBIFS
├── assets/
│   ├── mt7990_eeprom_2i5i.bin      # 官方默认 bin，内建 PA/LNA（7680 B，实机用这份）
│   └── mt7990_eeprom_2e5e.bin      # 官方默认 bin，外置 PA/LNA（7680 B，对照用）
├── an758x_factory_demo.py
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
