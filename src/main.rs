#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! AN758X Factory — egui port of `fiberhome-factory`.
//!
//! The UI deliberately mirrors the upstream FiberHome editor: a top bar with
//! 新建 / 打开 / 保存, a status line, an 身份信息 section with per-field 随机
//! buttons, and framed 替换 / 导出 / 清空 rows for the binary blobs.
//!
//! Note: eframe 0.36 replaced `App::update(ctx, frame)` with
//! `App::ui(&mut self, ui: &mut Ui, frame)` — the panel is provided, so there
//! is no `CentralPanel` here.
//!
//! Build: `cargo build --release`
//! Run:   `cargo run --release`

mod an4475;
mod font;
mod hl202;
mod import;
mod model;

use an4475::{An4475, AN_SN_SIZE};
use eframe::egui;
use hl202::{
    parse_compact as hl_parse_compact, Hl202, HL_BOB_SIZE, HL_EEPROM_SIZE, HL_PON_CHIP, HL_SN_SIZE,
};
use model::{
    detect_downlink_chip, detect_pon_chip, derive_mac, mac_to_string, parse_mac, random_mac,
    random_sn, Factory, PON_DOWNLINK_CAL_SIZE, CALIBRATION_SIZE, OFF_ETH0, OFF_PON0,
    OFF_WIFI_24G, OFF_WIFI_5G, WIFI_EEPROM_SIZE,
};
use rfd::FileDialog;
use std::path::Path;

/// 当前编辑的机型。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// H3C HM2004-DU（主机）：mtd14 出厂区 + mtd13 下行校准。
    Hm2004,
    /// H3C HL202-DU（从机）：只有 mtd12（reservearea），无下行校准。
    Hl202,
    /// AN4475（10G PON）：mtd9（UBI 卷 fhdata / UBIFS）→ 2 MiB 容器。
    An4475,
}

impl Kind {
    /// 下拉框里的显示名。三套布局互不通用，靠 magic 区分。
    const ALL: [Kind; 3] = [Kind::Hm2004, Kind::Hl202, Kind::An4475];

    fn label(&self) -> &'static str {
        match self {
            Kind::Hm2004 => "主机 HM2004-DU",
            Kind::Hl202 => "从机 HL202-DU",
            Kind::An4475 => "AN4475 (10G PON)",
        }
    }
}

struct Editor {
    kind: Kind,
    /// 机型列表是否展开（内联展开，不用 ComboBox 的跨帧弹层）。
    model_open: bool,
    factory: Factory,
    /// 从机数据。切到 HL202 机型时使用，与 `factory` 互不影响。
    hl202: Hl202,
    /// AN4475 数据：出厂文件 + 身份字段。
    an: An4475,
    /// AN4475 的 LAN 基址 MAC（eth1mac），与基准 MAC 是两段地址。
    eth_mac: String,
    /// AN4475 的 GPON SN（16 位 hex，含 "FHTT" 厂商码）。
    gpon_sn: String,
    /// AN4475 的 GPON 密码（10 字符 ASCII）。
    gpon_pw: String,
    /// AN4475 的 Wi-Fi 2.4G MAC（写 EEPROM +0x04 与 APMAC_1）。
    wifi_2g4: String,
    /// AN4475 的 Wi-Fi 5G MAC（写 EEPROM +0x0A 与 APMAC_2）。
    wifi_5g: String,
    device_sn: String,
    label_mac: String,
    msg: String,
    /// 已载入的出厂区（mtd14 / reservearea）文件名，用于状态栏与摘要。
    src_factory: Option<String>,
    /// 已载入的下行 PON 分区（mtd13）文件名。
    src_downlink: Option<String>,
    /// 从机 GUD 字符串（只读展示，不参与导出）。
    gud: Vec<(&'static str, String)>,
}

impl Editor {
    fn sync_from_factory(&mut self) {
        self.device_sn = self.factory.device_sn.clone();
        self.label_mac = mac_to_string(&self.factory.label_mac);
    }

    fn apply_to_factory(&mut self) {
        if let Some(m) = parse_mac(&self.label_mac) {
            self.factory.label_mac = m;
        }
        self.factory.device_sn = self.device_sn.clone();
    }

    fn sync_from_hl202(&mut self) {
        self.device_sn = self.hl202.device_sn.clone();
        self.label_mac = mac_to_string(&self.hl202.label_mac);
    }

    fn apply_to_hl202(&mut self) {
        if let Some(m) = parse_mac(&self.label_mac) {
            self.hl202.label_mac = m;
        }
        self.hl202.device_sn = self.device_sn.clone();
    }

    fn sync_from_an(&mut self) {
        self.device_sn = self.an.device_sn.clone();
        self.label_mac = an4475::mac_to_string(&self.an.base_mac);
        self.eth_mac = an4475::mac_to_string(&self.an.eth_mac);
        self.gpon_sn = self.an.gpon_sn.iter().map(|b| format!("{b:02X}")).collect();
        self.gpon_pw = self.an.gpon_pw.clone();
        self.wifi_2g4 = an4475::mac_to_string(&self.an.wifi_2g4);
        self.wifi_5g = an4475::mac_to_string(&self.an.wifi_5g);
    }

    fn apply_to_an(&mut self) {
        if let Some(m) = an4475::parse_mac(&self.label_mac) {
            self.an.base_mac = m;
        }
        if let Some(m) = an4475::parse_mac(&self.eth_mac) {
            self.an.eth_mac = m;
        }
        if let Some(m) = an4475::parse_mac(&self.wifi_2g4) {
            self.an.wifi_2g4 = m;
        }
        if let Some(m) = an4475::parse_mac(&self.wifi_5g) {
            self.an.wifi_5g = m;
        }
        self.an.device_sn = self.device_sn.trim().to_string();
        self.an.gpon_pw = self.gpon_pw.trim().to_string();
        // GPON SN 允许输入 16 位 hex；不足则保持原值，免得手滑把身份写坏。
        let hex: String = self
            .gpon_sn
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        if hex.len() == 16 {
            let mut sn = [0u8; 8];
            for i in 0..8 {
                if let Ok(b) = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16) {
                    sn[i] = b;
                }
            }
            self.an.gpon_sn = sn;
        }
    }

    /// 机型切换：三套布局互不通用（magic 不同），切过去只是换面板，
    /// 已载入的数据各自留在 factory / hl202 / an 里，不会被覆盖。
    fn switch_kind(&mut self, kind: Kind) {
        self.kind = kind;
        self.model_open = false;
        self.src_factory = None;
        self.src_downlink = None;
        self.gud.clear();
        match kind {
            Kind::Hm2004 => {
                self.sync_from_factory();
                self.msg = "已切换到主机 HM2004-DU".into();
            }
            Kind::Hl202 => {
                self.sync_from_hl202();
                self.msg = "已切换到从机 HL202-DU".into();
            }
            Kind::An4475 => {
                self.sync_from_an();
                self.msg = "已切换到 AN4475（需先打开 mtd9 备份才有出厂文件）".into();
            }
        }
    }

    fn save_image(&mut self, img: Vec<u8>, default_name: &str) {
        if let Some(p) = FileDialog::new().set_file_name(default_name).save_file() {
            match std::fs::write(&p, &img) {
                Ok(_) => self.msg = format!("已保存 {} 字节 -> {}", img.len(), p.display()),
                Err(e) => self.msg = format!("保存失败: {e}"),
            }
        }
    }

    /// Load the **factory / reservearea** partition (mtd14): identity, uplink
    /// PON calibration and Wi-Fi EEPROM.
    ///
    /// The downlink block is deliberately *not* clobbered when this dump has
    /// none: a real mtd14 has no downlink data, and overwriting a good mtd13
    /// import with 0xFF would silently undo it. Only a dump that actually
    /// carries downlink bytes is allowed to replace them.
    fn open_factory(&mut self, bytes: &[u8], name: &str) {
        let mut parsed = import::parse_factory_volume(bytes);
        let parsed_has_downlink = parsed.pon_downlink_cal.iter().any(|&b| b != 0xff);
        if !parsed_has_downlink {
            // Preserve whatever mtd13 gave us.
            parsed.pon_downlink_cal = std::mem::take(&mut self.factory.pon_downlink_cal);
            parsed.pon_downlink_flags = self.factory.pon_downlink_flags;
        }
        self.factory = parsed;
        self.sync_from_factory();
        self.src_factory = Some(name.to_string());
        self.msg = format!("已打开出厂区: {name}");
        if let Some(d) = &self.src_downlink {
            self.msg.push_str(&format!(" · 下行 PON 保留自 {d}"));
        }
    }

    /// Load the **downlink PON** partition (mtd13): the UX3326 BOSA blob plus
    /// the OBosaMode / BosaFlag / TempFlag scalars. Nothing else is touched, so
    /// it is safe to open in either order.
    fn open_downlink(&mut self, bytes: &[u8], name: &str) {
        match import::parse_mtd13(bytes) {
            Some((cal, flags, off)) => {
                self.factory.pon_downlink_cal = cal;
                self.factory.pon_downlink_flags = flags;
                self.src_downlink = Some(name.to_string());
                self.msg =
                    format!("已打开下行 PON 分区: {name} · UBIFS 记录 0x{off:06x}（sqnum 最新副本）");
            }
            None => {
                self.msg = format!("已打开: {name} · 但未找到 /bosa/ 校准表（不是 mtd13？）");
            }
        }
    }

    /// Load the **from-unit** reservearea partition (HL202-DU mtd12).
    ///
    /// 从机与主机共用 magic 与 2.25 MB 尺寸，靠 `looks_like_hl202_volume`
    /// 里 GUD 配置名（`ctromfile_GUD.cfg` vs `ctromfile_GUI.cfg`）区分。
    fn open_hl202(&mut self, bytes: &[u8], name: &str) {
        let compact = hl202::is_compact(bytes);
        self.hl202 = if compact {
            hl_parse_compact(bytes)
        } else {
            hl202::parse_hl202_volume(bytes)
        };
        self.kind = Kind::Hl202;
        self.sync_from_hl202();
        self.src_factory = Some(name.to_string());
        self.src_downlink = None;
        self.gud = if compact {
            Vec::new()
        } else {
            hl202::read_gud_strings(bytes)
        };
        self.msg = format!(
            "已打开从机 HL202-DU ({}): {name}",
            if compact { "HL2-512K 紧凑镜像" } else { "mtd12 reservearea" }
        );
    }

    /// 载入 AN4475 的 mtd9 备份，或本工具生成的 1/2 MiB 容器。
    ///
    /// mtd9 不是裸分区而是整个 UBI 设备（25 MiB / 200 个 PEB），里面只有一个
    /// 卷 `fhdata`，卷上是 UBIFS。解析见 `an4475::parse_mtd9`。
    fn open_an(&mut self, bytes: &[u8], name: &str) {
        let compact = an4475::is_compact(bytes);
        let parsed = if compact {
            an4475::parse_compact(bytes)
        } else {
            an4475::parse_mtd9(bytes)
        };
        match parsed {
            Some(a) => {
                let n = a.files.len();
                let bosa = a.bosa.clone();
                self.an = a;
                self.kind = Kind::An4475;
                self.sync_from_an();
                self.src_factory = Some(name.to_string());
                self.src_downlink = None;
                self.gud.clear();
                self.msg = format!(
                    "已打开 AN4475 ({}): {name} · {n} 个出厂文件{}",
                    if compact {
                        an4475::compact_magic_name(bytes)
                    } else {
                        "mtd9 UBI/UBIFS"
                    },
                    if bosa.is_empty() {
                        String::new()
                    } else {
                        format!(" · BOSA {bosa}")
                    }
                );
            }
            None => self.msg = format!("已打开: {name} · 但没找到 AN4475 出厂数据（不是 mtd9？）"),
        }
    }

    /// Dispatch on content so the user can pick either file in the same dialog.
    fn open_auto(&mut self, bytes: &[u8], name: &str) {
        // AN4475 必须排在 mtd13 之前：mtd9 是**整片 NAND**，开头就是 `UBI#`，
        // 而它唯一的卷 fhdata 本身又是 UBIFS —— `looks_like_mtd13`（长度≥mtd13
        // 且以 UBI# 开头）会把 25 MiB 的 mtd9 直接认成 mtd13，然后报
        // “未找到 /bosa/ 校准表”。用卷名 `fhdata` 先把它挑出来。
        if an4475::is_compact(bytes) || an4475::looks_like_volume(bytes) {
            self.open_an(bytes, name);
        } else if import::looks_like_mtd13(bytes) {
            self.open_downlink(bytes, name);
        } else if hl202::is_compact(bytes) || hl202::looks_like_hl202_volume(bytes) {
            self.open_hl202(bytes, name);
        } else {
            self.open_factory(bytes, name);
        }
    }
}

impl eframe::App for Editor {
    /// 窗口清屏色。
    ///
    /// **这是「界面改为白色」一直没生效的真正原因**：eframe 的默认实现返回一个
    /// 硬编码的深色 `(12,12,12)`@alpha180，完全无视主题（源码里写着 "a bright
    /// gray makes the shadows of the windows look weird"）；而 egui 0.36 传给
    /// `App::ui` 的根 `Ui` 自己不画背景（官方注释："has no margin or background
    /// color"），于是这个深色透过整个窗口 —— 表现就是控件都变浅了、窗口还是黑的。
    /// 钉成纯白即可，且不依赖 `visuals` 参数（它在每帧 `ctx.run()` 之前取样，
    /// 首帧可能还是深色 style）。
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        egui::Color32::WHITE.to_normalized_gamma_f32()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // 每帧重申亮色主题。两件事都要做，缺一不可：
        //
        // 1. `apply_light_theme` 改 ctx：`theme_preference` 默认是 `System`，
        //    系统深色时 `Options::theme()` 解析成 `Dark`，Ui 会按 dark style
        //    取色。把 dark / light 两个 style 都写成亮色，解析结果就无所谓了。
        // 2. `ui.set_style` 改当前帧：根 `Ui` 的 style 在帧开始就被快照成
        //    `Arc<Style>`，只改 ctx 对本帧无效（要等下一帧）。钉一下，首帧就不闪。
        ui.set_style(apply_light_theme(ui.ctx()));

        // ---- 顶栏：标题 ----
        ui.horizontal(|ui| {
            ui.heading("AN758X Factory");
            ui.label(
                egui::RichText::new(concat!("v", env!("CARGO_PKG_VERSION")))
                    .weak()
                    .small(),
            );
            ui.separator();
            ui.label(
                egui::RichText::new("H3C HM2004-DU / HL202-DU / AN4475 出厂分区编辑器").weak(),
            );
        });

        // 机型快捷键兜底：Ctrl+1/2/3（mac 上是 ⌘+1/2/3）。
        // 远程桌面 / 部分窗口管理器下弹出层可能拿不到鼠标焦点，键盘仍可用；
        // 文本框里按 Ctrl+数字不会产生字符，所以不会干扰 SN 编辑。
        let hotkey = ui.ctx().input(|i| {
            if !i.modifiers.command && !i.modifiers.alt {
                return None;
            }
            [
                (egui::Key::Num1, Kind::Hm2004),
                (egui::Key::Num2, Kind::Hl202),
                (egui::Key::Num3, Kind::An4475),
            ]
            .into_iter()
            .find(|(key, _)| i.key_pressed(*key))
            .map(|(_, k)| k)
        });
        if let Some(k) = hotkey {
            if k != self.kind {
                self.switch_kind(k);
            }
        }

        // ---- 机型选择 ----
        // 这里**故意不用 `egui::ComboBox`**。它的弹层是一个独立 `Area`，开合状态
        // 存在 `Memory` 里、跨帧读取；在部分 Windows / 远程桌面环境下，点击后
        // 窗口会进入等待状态，要等下一次鼠标移动才重绘，表现就是"点了没反应"。
        // 改成内联展开后，列表在**点击的同一帧**就画出来了，不依赖任何跨帧
        // 弹层状态，也不依赖重绘调度：
        //   - 整个按钮（含"机型："三个字）都是点击区，点文字也能展开；
        //   - 条目是普通 Button，整行 292×30 都可点；
        //   - 选中后立即收起。
        ui.horizontal(|ui| {
            let label = format!("机型：{}   ▼", self.kind.label());
            let btn = egui::Button::new(egui::RichText::new(label).size(15.0));
            if ui.add_sized([300.0, 34.0], btn).clicked() {
                self.model_open = !self.model_open;
                // 打开/收起后立刻要一帧，避免窗口在等待输入时"看着没动"。
                ui.ctx().request_repaint();
            }
            ui.label(egui::RichText::new("点击展开（Ctrl+1/2/3）").weak().small());
        });
        if self.model_open {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_width(300.0);
                for k in Kind::ALL {
                    let selected = self.kind == k;
                    let text = if selected {
                        egui::RichText::new(format!("✔ {}", k.label())).size(15.0)
                    } else {
                        egui::RichText::new(format!("   {}", k.label())).size(15.0)
                    };
                    let item = egui::Button::new(text)
                        .selected(selected)
                        .wrap_mode(egui::TextWrapMode::Extend);
                    if ui.add_sized([292.0, 30.0], item).clicked() {
                        self.switch_kind(k);
                        self.model_open = false;
                    }
                }
            });
        }
        // 动作按钮：左对齐 + 自动换行。
        // 原先是「右对齐单行」，760 px 下 8 个按钮总宽超出，右对齐会把超出
        // 的部分推到可视区左侧之外 —— 表现为「保存 1 MiB」只剩半个字。
        // `horizontal_wrapped` 让窗口再窄也只会换行，不会有按钮消失。
        ui.horizontal_wrapped(|ui| {
            if ui.button("新建").clicked() {
                match self.kind {
                    Kind::Hm2004 => {
                        self.factory = Factory::new();
                        self.sync_from_factory();
                        self.msg = "新建 HM2004-DU Factory".into();
                    }
                    Kind::Hl202 => {
                        self.hl202 = Hl202::new();
                        self.sync_from_hl202();
                        self.msg = "新建 HL202-DU Factory".into();
                    }
                    Kind::An4475 => {
                        self.an = An4475::new();
                        self.sync_from_an();
                        self.msg = "新建 AN4475（出厂文件需从 mtd9 备份导入）".into();
                    }
                }
                self.src_factory = None;
                self.src_downlink = None;
                self.gud.clear();
            }
            if ui.button("打开…").clicked() {
                if let Some(p) = FileDialog::new().add_filter("bin", &["bin"]).pick_file() {
                    match std::fs::read(&p) {
                        Ok(b) => {
                            let name = p.display().to_string();
                            self.open_auto(&b, &name);
                        }
                        Err(e) => self.msg = format!("读取失败: {e}"),
                    }
                }
            }
            match self.kind {
                Kind::Hm2004 => {
                    // 出厂校准分散在两个分区里，故给两个明确的入口：顺序任意，
                    // 互不覆盖（见 open_factory / open_downlink）。
                    if ui.button("打开出厂区 (mtd14)").clicked() {
                        if let Some(p) = FileDialog::new().add_filter("bin", &["bin"]).pick_file()
                        {
                            match std::fs::read(&p) {
                                Ok(b) => {
                                    let name = p.display().to_string();
                                    self.open_factory(&b, &name);
                                }
                                Err(e) => self.msg = format!("读取失败: {e}"),
                            }
                        }
                    }
                    if ui.button("打开下行校准 (mtd13)").clicked() {
                        if let Some(p) = FileDialog::new().add_filter("bin", &["bin"]).pick_file()
                        {
                            match std::fs::read(&p) {
                                Ok(b) => {
                                    let name = p.display().to_string();
                                    self.open_downlink(&b, &name);
                                }
                                Err(e) => self.msg = format!("读取失败: {e}"),
                            }
                        }
                    }
                }
                Kind::Hl202 => {
                    if ui.button("打开 mtd12").clicked() {
                        if let Some(p) = FileDialog::new().add_filter("bin", &["bin"]).pick_file()
                        {
                            match std::fs::read(&p) {
                                Ok(b) => {
                                    let name = p.display().to_string();
                                    self.open_hl202(&b, &name);
                                }
                                Err(e) => self.msg = format!("读取失败: {e}"),
                            }
                        }
                    }
                }
                Kind::An4475 => {
                    if ui.button("打开 mtd9").clicked() {
                        if let Some(p) = FileDialog::new().add_filter("bin", &["bin"]).pick_file()
                        {
                            match std::fs::read(&p) {
                                Ok(b) => {
                                    let name = p.display().to_string();
                                    self.open_an(&b, &name);
                                }
                                Err(e) => self.msg = format!("读取失败: {e}"),
                            }
                        }
                    }
                }
            }
            match self.kind {
                Kind::Hm2004 => {
                    if ui.button("保存真实布局").clicked() {
                        self.apply_to_factory();
                        let img = self.factory.encode();
                        self.save_image(img, "factory-h3c-layout.bin");
                    }
                    if ui
                        .add(egui::Button::new(egui::RichText::new("保存 1 MiB").strong()))
                        .clicked()
                    {
                        self.apply_to_factory();
                        let img = self.factory.encode_1mib();
                        self.save_image(img, "factory-h3c-1mib.bin");
                    }
                }
                Kind::Hl202 => {
                    if ui
                        .add(egui::Button::new(egui::RichText::new("保存 512 KiB").strong()))
                        .clicked()
                    {
                        self.apply_to_hl202();
                        let img = self.hl202.encode_compact();
                        self.save_image(img, "factory-hl202-512k.bin");
                    }
                }
                Kind::An4475 => {
                    if ui
                        .add(egui::Button::new(egui::RichText::new("保存 2 MiB").strong()))
                        .clicked()
                    {
                        self.apply_to_an();
                        match self.an.encode_compact() {
                            Ok(img) => self.save_image(img, "factory-an4475-2mib.bin"),
                            Err(e) => self.msg = format!("生成失败: {e}"),
                        }
                    }
                }
            }
        });
        ui.label(&self.msg);
        ui.separator();

        match self.kind {
            Kind::Hm2004 => self.ui_hm2004(ui),
            Kind::Hl202 => self.ui_hl202(ui),
            Kind::An4475 => self.ui_an4475(ui),
        }
    }
}

impl Editor {
    /// 主机 HM2004-DU 面板：身份 + 派生 MAC + 上行/下行校准 + EEPROM。
    fn ui_hm2004(&mut self, ui: &mut egui::Ui) {
        // 已载入来源：让用户一眼看出两个分区是否都到位
        egui::Grid::new("sources")
            .num_columns(2)
            .spacing([16.0, 2.0])
            .show(ui, |ui| {
                ui.weak("出厂区 (mtd14)");
                ui.monospace(self.src_factory.as_deref().unwrap_or("<未载入>"));
                ui.end_row();
                ui.weak("下行校准 (mtd13)");
                ui.monospace(self.src_downlink.as_deref().unwrap_or("<未载入>"));
                ui.end_row();
            });
        ui.separator();

        // 折叠式摘要：打开文件后一眼看清各项是否真的载入（含下行 PON 校准）
        egui::CollapsingHeader::new("当前镜像摘要")
            .default_open(false)
            .show(ui, |ui| {
                ui.monospace(self.factory.summary());
            });
        ui.separator();

        // ---- 身份信息 ----
        ui.heading("身份信息");
        const FIELD: f32 = 280.0;
        ui.horizontal(|ui| {
            ui.label("标签 MAC");
            ui.add(egui::TextEdit::singleline(&mut self.label_mac).desired_width(FIELD));
            if ui.button("随机").clicked() {
                self.factory.label_mac = random_mac();
                self.label_mac = mac_to_string(&self.factory.label_mac);
            }
        });
        ui.horizontal(|ui| {
            ui.label("设备序列号");
            ui.add(egui::TextEdit::singleline(&mut self.device_sn).desired_width(FIELD));
            if ui.button("随机").clicked() {
                self.factory.device_sn = random_sn();
                self.device_sn = self.factory.device_sn.clone();
            }
        });

        ui.add_space(6.0);
        let base = parse_mac(&self.label_mac).unwrap_or(self.factory.label_mac);
        derived_mac_grid(ui, &base, "derived_macs");

        // ---- 数据区 ----
        ui.add_space(8.0);
        // The chip is named by the imported calibration, not hardcoded.
        let pon_title = match detect_pon_chip(&self.factory.calibration) {
            Some(chip) => format!("上行 PON 校准 ({chip})"),
            None => "上行 PON 校准".to_string(),
        };
        blob_section(
            ui,
            &pon_title,
            &mut self.factory.calibration,
            CALIBRATION_SIZE,
            &mut self.msg,
        );

        // ---- 下行 PON 口校准 (mtd13 / UX3326 BOSA) ----
        let down_title = match detect_downlink_chip(&self.factory.pon_downlink_cal) {
            Some(chip) => format!("下行 PON 口校准 ({chip} BOSA)"),
            None => "下行 PON 口校准".to_string(),
        };
        blob_section(
            ui,
            &down_title,
            &mut self.factory.pon_downlink_cal,
            PON_DOWNLINK_CAL_SIZE,
            &mut self.msg,
        );
        let f = self.factory.pon_downlink_flags;
        egui::Grid::new("bosa_flags")
            .num_columns(2)
            .spacing([16.0, 3.0])
            .show(ui, |ui| {
                ui.weak("OBosaMode");
                ui.monospace(format!("0x{:04X} ({})", f.obosa_mode, f.obosa_mode));
                ui.end_row();
                ui.weak("BosaFlag");
                ui.monospace(format!("0x{:04X} ({})", f.bosa_flag, f.bosa_flag));
                ui.end_row();
                ui.weak("TempFlag");
                ui.monospace(format!("0x{:04X} ({})", f.temp_flag, f.temp_flag));
                ui.end_row();
            });

        blob_section(
            ui,
            "Wi-Fi EEPROM (MT7916)",
            &mut self.factory.wifi_eeprom,
            WIFI_EEPROM_SIZE,
            &mut self.msg,
        );

        ui.add_space(8.0);
        ui.weak("出厂校准分散在两个分区：上行/身份/EEPROM 在 mtd14（reservearea），下行 PON 口校准在 mtd13（UBIFS）。两个文件可任意顺序打开，互不覆盖。");
        ui.weak("导出为演示镜像（不含 UBI 元数据、不重压缩 ctromfile），不可直接刷写 NAND。");
        ui.weak("真实偏移: SN@0x14102c · magic@0x140000 · AL-MAC@gzip 0x80000 · 上行校准@0x1c0400 · 下行校准@0x1c1000 · EEPROM@0x4c000");
        ui.weak("下行 PON 校准原始来源为 mtd13 的 UBIFS 卷 ubifs1（/bosa/mmodified_UX3326.bin，288 字节，取 sqnum 最新副本）。");
    }

    /// 从机 HL202-DU 面板：身份 + BOB + EEPROM + GUD 只读。
    ///
    /// 从机没有下行 PON 校准分区，所以这里**不出现下行那一栏** —— 免得用户以为
    /// 漏载了文件。
    fn ui_hl202(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("hl_sources")
            .num_columns(2)
            .spacing([16.0, 2.0])
            .show(ui, |ui| {
                ui.weak("reservearea (mtd12)");
                ui.monospace(self.src_factory.as_deref().unwrap_or("<未载入>"));
                ui.end_row();
                ui.weak("下行校准");
                ui.weak("本机型无此分区");
                ui.end_row();
            });
        ui.separator();

        egui::CollapsingHeader::new("当前镜像摘要")
            .default_open(false)
            .show(ui, |ui| {
                ui.monospace(self.hl202.summary());
            });
        ui.separator();

        ui.heading("身份信息");
        const FIELD: f32 = 280.0;
        ui.horizontal(|ui| {
            ui.label("标签 MAC");
            ui.add(egui::TextEdit::singleline(&mut self.label_mac).desired_width(FIELD));
            if ui.button("随机").clicked() {
                self.hl202.label_mac = random_mac();
                self.label_mac = mac_to_string(&self.hl202.label_mac);
            }
        });
        ui.horizontal(|ui| {
            ui.label(format!("设备序列号（{} 字符）", HL_SN_SIZE));
            ui.add(egui::TextEdit::singleline(&mut self.device_sn).desired_width(FIELD));
            if ui.button("随机").clicked() {
                self.hl202.device_sn = random_sn();
                self.device_sn = self.hl202.device_sn.clone();
            }
        });

        ui.add_space(6.0);
        let base = parse_mac(&self.label_mac).unwrap_or(self.hl202.label_mac);
        derived_mac_grid(ui, &base, "hl_derived_macs");

        ui.add_space(8.0);
        blob_section(
            ui,
            &format!("PON BOB 校准 ({HL_PON_CHIP})"),
            &mut self.hl202.bob,
            HL_BOB_SIZE,
            &mut self.msg,
        );
        blob_section(
            ui,
            "Wi-Fi EEPROM (MT7916D)",
            &mut self.hl202.wifi_eeprom,
            HL_EEPROM_SIZE,
            &mut self.msg,
        );

        if !self.gud.is_empty() {
            ui.add_space(6.0);
            egui::CollapsingHeader::new("GUD 出厂字符串（只读，不写入 512 KiB）")
                .default_open(false)
                .show(ui, |ui| {
                    egui::Grid::new("gud_grid")
                        .num_columns(2)
                        .spacing([16.0, 2.0])
                        .show(ui, |ui| {
                            for (k, v) in &self.gud {
                                ui.weak(*k);
                                ui.monospace(v.as_str());
                                ui.end_row();
                            }
                        });
                });
        }

        ui.add_space(8.0);
        ui.weak("从机 reservearea 在 mtd12（2.25 MB），没有下行 PON 校准分区，也不需要 mtd13。");
        ui.weak("512 KiB 镜像用专属 magic `HL2-512K`，与主机的 `H3C-1MiB` 不通用。");
        ui.weak("真实偏移: SN@0x14102c · magic@0x140000 · AL-MAC@gzip 0x80000 · BOB@0x1c0400 (0xE1) · EEPROM@0x4c000");
        ui.weak("GUD 字符串里只搬运 SN 和 MAC；SSID/密码/生产日期等留在原分区，不写入 512 KiB。");
    }

    /// AN4475 面板：身份 + 9 个派生 MAC + 出厂文件清单。
    ///
    /// 与 H3C 两台机型的最大区别是「数据在文件里」：SN / MAC / GPON SN 全都在
    /// `factory_conf` 这段 UCI 文本中，导出时由 `an4475::encode_compact` 回写，
    /// 界面上的输入框就是这份文本的视图。
    fn ui_an4475(&mut self, ui: &mut egui::Ui) {
        egui::Grid::new("an_sources")
            .num_columns(2)
            .spacing([16.0, 2.0])
            .show(ui, |ui| {
                ui.weak("mtd9 备份 / 2 MiB 镜像");
                ui.monospace(self.src_factory.as_deref().unwrap_or("<未载入>"));
                ui.end_row();
                ui.weak("出厂文件");
                ui.monospace(format!(
                    "{} 个，原始 {} 字节",
                    self.an.files.len(),
                    self.an.payload_size()
                ));
                ui.end_row();
            });
        ui.separator();

        egui::CollapsingHeader::new("当前镜像摘要")
            .default_open(false)
            .show(ui, |ui| {
                ui.monospace(self.an.summary());
            });
        ui.separator();

        ui.heading("身份信息");
        const FIELD: f32 = 280.0;
        ui.horizontal(|ui| {
            ui.label(format!("设备序列号（{AN_SN_SIZE} 字符）"));
            ui.add(egui::TextEdit::singleline(&mut self.device_sn).desired_width(FIELD));
            if ui.button("按 MAC 生成").clicked() {
                // 真机 SN = "F3200" + 基准 MAC 的 12 位 hex
                // （F3200 + D49F2904E750 = F3200D49F2904E750），实测吻合。
                let mac: String = self
                    .an
                    .base_mac
                    .iter()
                    .map(|b| format!("{b:02X}"))
                    .collect();
                self.device_sn = format!("F3200{mac}");
                self.an.device_sn = self.device_sn.clone();
            }
        });
        ui.horizontal(|ui| {
            ui.label("基准 MAC (base/br/pon)");
            ui.add(egui::TextEdit::singleline(&mut self.label_mac).desired_width(FIELD));
            if ui.button("随机").clicked() {
                // 保留 OUI 只换后 3 字节：工厂换机不换厂商前缀。
                let r = random_mac();
                let old = self.an.base_mac;
                let mut m = old;
                m[3] = r[3];
                m[4] = r[4];
                m[5] = r[5];
                self.an.base_mac = m;
                self.label_mac = an4475::mac_to_string(&m);
                // Wi-Fi 两位若还停在旧的派生值上，就跟着一起走；手改过的不动。
                if self.an.wifi_2g4 == an4475::mac_add(&old, an4475::AN_WIFI_OFFSET_2G4) {
                    self.an.wifi_2g4 = an4475::mac_add(&m, an4475::AN_WIFI_OFFSET_2G4);
                    self.wifi_2g4 = an4475::mac_to_string(&self.an.wifi_2g4);
                }
                if self.an.wifi_5g == an4475::mac_add(&old, an4475::AN_WIFI_OFFSET_5G) {
                    self.an.wifi_5g = an4475::mac_add(&m, an4475::AN_WIFI_OFFSET_5G);
                    self.wifi_5g = an4475::mac_to_string(&self.an.wifi_5g);
                }
            }
        });
        ui.horizontal(|ui| {
            ui.label("LAN 基址 MAC (eth1)");
            ui.add(egui::TextEdit::singleline(&mut self.eth_mac).desired_width(FIELD));
            if ui.button("随机").clicked() {
                let r = random_mac();
                let mut m = self.an.eth_mac;
                m[3] = r[3];
                m[4] = r[4];
                m[5] = r[5];
                self.an.eth_mac = m;
                self.eth_mac = an4475::mac_to_string(&m);
            }
        });
        ui.horizontal(|ui| {
            ui.label("GPON SN（16 位 hex）");
            ui.add(egui::TextEdit::singleline(&mut self.gpon_sn).desired_width(FIELD));
        });
        ui.horizontal(|ui| {
            ui.label("GPON 密码（10 字符）");
            ui.add(egui::TextEdit::singleline(&mut self.gpon_pw).desired_width(FIELD));
        });

        ui.add_space(6.0);
        ui.separator();
        // 用 Grid 而不是两个 horizontal：两个标签字数不同，输入框左边缘会错开
        // 4 px（截图里很明显）；Grid 自动把三列对齐。
        egui::Grid::new("an_wifi_mac")
            .num_columns(3)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                ui.label("Wi-Fi 2.4G MAC");
                // Grid 单元格的可用宽度首帧是 0，`desired_width` 会被压缩成
                // 40 px；用 add_sized 钉死宽高，列宽才会按 280 px 撑开。
                ui.add_sized(
                    [FIELD, 22.0],
                    egui::TextEdit::singleline(&mut self.wifi_2g4),
                );
                if ui.button("按基准 MAC 派生").clicked() {
                    let b = an4475::parse_mac(&self.label_mac).unwrap_or(self.an.base_mac);
                    self.an.wifi_2g4 = an4475::mac_add(&b, an4475::AN_WIFI_OFFSET_2G4);
                    self.an.wifi_5g = an4475::mac_add(&b, an4475::AN_WIFI_OFFSET_5G);
                    self.wifi_2g4 = an4475::mac_to_string(&self.an.wifi_2g4);
                    self.wifi_5g = an4475::mac_to_string(&self.an.wifi_5g);
                }
                ui.end_row();

                ui.label("Wi-Fi 5G MAC");
                ui.add_sized(
                    [FIELD, 22.0],
                    egui::TextEdit::singleline(&mut self.wifi_5g),
                );
                ui.label("");
                ui.end_row();
            });
        ui.add_space(5.0);
        ui.weak(format!(
            "写 EEPROM +{:#04X}/+{:#04X} 与 factory_conf 的 APMAC_1/APMAC_2；device id {}",
            an4475::AN_EEPROM_MAC_2G4,
            an4475::AN_EEPROM_MAC_5G,
            if self.an.eeprom_id == 0 {
                "<未载入>".to_string()
            } else if self.an.eeprom_id == an4475::AN_MT7993_ID {
                format!("0x{:04X} (MT7993)", self.an.eeprom_id)
            } else {
                format!("0x{:04X}（非 MT7993，请核对）", self.an.eeprom_id)
            }
        ));

        ui.add_space(6.0);
        let base = an4475::parse_mac(&self.label_mac).unwrap_or(self.an.base_mac);
        let eth = an4475::parse_mac(&self.eth_mac).unwrap_or(self.an.eth_mac);
        ui.label("派生 MAC（随上面两个基址实时计算）");
        egui::Grid::new("an_derived")
            .num_columns(2)
            .spacing([16.0, 3.0])
            .show(ui, |ui| {
                for (name, off) in [
                    ("internetmac", 1u8),
                    ("tr069mac", 2),
                    ("voipmac", 3),
                    ("priprotocolmac", 8),
                ] {
                    ui.label(name);
                    ui.monospace(an4475::mac_to_string(&an4475::mac_add(&base, off)));
                    ui.end_row();
                }
                for (i, name) in ["eth1mac", "eth2mac", "eth3mac", "eth4mac", "eth5mac"]
                    .iter()
                    .enumerate()
                {
                    ui.label(*name);
                    ui.monospace(an4475::mac_to_string(&an4475::mac_add(&eth, i as u8)));
                    ui.end_row();
                }
            });

        ui.add_space(8.0);
        egui::CollapsingHeader::new(format!("出厂文件（{} 个）", self.an.files.len()))
            .default_open(false)
            .show(ui, |ui| {
                egui::Grid::new("an_files")
                    .num_columns(2)
                    .spacing([16.0, 2.0])
                    .show(ui, |ui| {
                        for f in &self.an.files {
                            ui.monospace(f.name.as_str());
                            ui.monospace(format!("{} 字节", f.data.len()));
                            ui.end_row();
                        }
                    });
            });

        ui.add_space(8.0);
        ui.weak("mtd9 = 整个 UBI 设备（25 MiB / 200 个 128 KiB PEB），只有一个动态卷 fhdata（178 LEB），卷上是 UBIFS。");
        ui.weak("2 MiB 镜像用专属 magic `AN4-2MiB`：头部固定偏移放身份，其余文件进 TOC + zlib 载荷；旧版 `AN4-1MiB` 仍可读入，导出时自动升级。");
        ui.weak("身份字段（SN / MAC / GPON SN / 密码）导出时会回写 factory_conf，保持文本与头部一致。");
        ui.weak("导入时按 sqnum 取 UBIFS 每个 key 的最新副本；`.factory_conf.uci-*` 是写入中间态，自动跳过。");
    }
}

/// 派生 MAC 只读表。两台机型规则相同（末字节 +1/+2/+4/+5），故共用一份绘制逻辑；
/// `grid_id` 用来区分两套面板里的 Grid，避免 egui 记忆错位。
fn derived_mac_grid(ui: &mut egui::Ui, base: &[u8; 6], grid_id: &str) {
    ui.label("派生 MAC（末字节偏移，随标签 MAC 实时计算）");
    egui::Grid::new(grid_id)
        .num_columns(2)
        .spacing([16.0, 3.0])
        .show(ui, |ui| {
            ui.label("eth0  (+1)");
            ui.monospace(mac_to_string(&derive_mac(base, OFF_ETH0)));
            ui.end_row();
            ui.label("pon0  (+2)");
            ui.monospace(mac_to_string(&derive_mac(base, OFF_PON0)));
            ui.end_row();
            ui.label("5G    (+4)");
            ui.monospace(mac_to_string(&derive_mac(base, OFF_WIFI_5G)));
            ui.end_row();
            ui.label("2.4G  (+5)");
            ui.monospace(mac_to_string(&derive_mac(base, OFF_WIFI_24G)));
            ui.end_row();
        });
}

/// One framed blob row, styled like the upstream editor: bold title, a
/// 未设置/已载入 status, and right-aligned 替换 / 导出 / 清空 buttons.
fn blob_section(
    ui: &mut egui::Ui,
    title: &str,
    data: &mut Vec<u8>,
    cap: usize,
    msg: &mut String,
) {
    let loaded = data.len() > cap || data.iter().any(|&b| b != 0xff);
    let status = if loaded {
        format!("已载入 {} 字节", data.len())
    } else {
        "未设置".to_string()
    };
    ui.group(|ui| {
        ui.set_min_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.strong(title);
            ui.weak(status);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("清空").clicked() {
                    *data = vec![0xff; cap];
                    *msg = format!("已清空: {title}");
                }
                if ui.button("导出").clicked() {
                    if let Some(p) = FileDialog::new()
                        .set_file_name("blob.bin")
                        .save_file()
                    {
                        match std::fs::write(&p, data.as_slice()) {
                            Ok(_) => {
                                *msg = format!("已导出 {} 字节 -> {}", data.len(), p.display())
                            }
                            Err(e) => *msg = format!("导出失败: {e}"),
                        }
                    }
                }
                if ui.button("替换").clicked() {
                    if let Some(p) = FileDialog::new().pick_file() {
                        match std::fs::read(&p) {
                            Ok(mut b) => {
                                b.truncate(cap);
                                *data = b;
                                *msg = format!("已载入: {}", p.display());
                            }
                            Err(e) => *msg = format!("读取失败: {e}"),
                        }
                    }
                }
            });
        });
    });
    ui.add_space(4.0);
}

/// 构造本应用的亮色 [`egui::Style`]（只构造一次，之后每次只是 Arc 克隆）。
///
/// 在 `egui::Visuals::light()` 基础上改四处：
/// - `weak_text_color`：默认由 `text_color * weak_text_alpha(0.6)` 派生，白底上
///   发灰、几乎不可读，钉成 110。
/// - 三态按钮底色 230 / 220 / 165 太接近，一眼分不出，拉开成 238 / 205 / 170。
/// - `panel_fill` / `window_fill` 钉成纯白。
/// - `inactive.bg_stroke`：`Widgets::light()` 里它是 `Stroke::NONE`，而 `TextEdit`
///   不聚焦时正是用它画边框 —— 纯白面板上的输入框会**完全没有边界**，只剩一片白。
///   补一条 gray(190) 描边，与 `Frame::group` 的边框同色，白底上才看得出输入区。
fn light_style() -> std::sync::Arc<egui::Style> {
    static STYLE: std::sync::OnceLock<std::sync::Arc<egui::Style>> = std::sync::OnceLock::new();
    STYLE
        .get_or_init(|| {
            let mut v = egui::Visuals::light();
            v.weak_text_color = Some(egui::Color32::from_gray(110));
            v.widgets.inactive.bg_fill = egui::Color32::from_gray(238);
            v.widgets.inactive.weak_bg_fill = egui::Color32::from_gray(238);
            v.widgets.inactive.bg_stroke =
                egui::Stroke::new(1.0, egui::Color32::from_gray(190));
            v.widgets.hovered.bg_fill = egui::Color32::from_gray(205);
            v.widgets.hovered.weak_bg_fill = egui::Color32::from_gray(205);
            v.widgets.active.bg_fill = egui::Color32::from_gray(170);
            v.widgets.active.weak_bg_fill = egui::Color32::from_gray(170);
            v.panel_fill = egui::Color32::WHITE;
            v.window_fill = egui::Color32::WHITE;
            v.window_stroke.color = egui::Color32::from_gray(150);
            let mut style = egui::Style::default();
            style.visuals = v;
            std::sync::Arc::new(style)
        })
        .clone()
}

/// 把亮色主题强制写到 `ctx` 上，返回该 style 供调用方钉到当前帧的根 `Ui`。
///
/// `Options::theme()` 在 `ThemePreference::System`（默认值）时按系统偏好解析，
/// 系统是深色就会一直选到 `dark_style`；所以这里把 dark / light 两个 style 都
/// 写成亮色，无论解析成什么都不影响，再用 `set_theme` 把偏好固定成 Light。
fn apply_light_theme(ctx: &egui::Context) -> std::sync::Arc<egui::Style> {
    let style = light_style();
    ctx.set_style_of(egui::Theme::Dark, style.clone());
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_theme(egui::Theme::Light);
    style
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // 800 px 起：顶栏按钮一行放得下（680 时会挤成两行，虽不丢按钮但很挤）。
            .with_inner_size([860.0, 800.0])
            .with_min_inner_size([700.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AN758X Factory 编辑器 v0.3.0",
        options,
        Box::new(|cc| {
            font::install(&cc.egui_ctx);
            apply_light_theme(&cc.egui_ctx);
            let mut ed = Editor {
                kind: Kind::Hm2004,
                model_open: false,
                factory: Factory::new(),
                hl202: Hl202::new(),
                an: An4475::new(),
                eth_mac: String::new(),
                gpon_sn: String::new(),
                gpon_pw: String::new(),
                wifi_2g4: String::new(),
                wifi_5g: String::new(),
                device_sn: String::new(),
                label_mac: String::new(),
                msg: "新建 Factory".to_string(),
                src_factory: None,
                src_downlink: None,
                gud: Vec::new(),
            };
            ed.sync_from_factory();
            // Like upstream: allow `an758x-factory.exe backup.bin`
            // (or dragging a .bin onto the exe) to open a file directly.
            // Multiple paths may be given, which is how a real unit's two
            // partitions get loaded in one go:
            //   an758x-factory.exe mtd14_reservearea.bin mtd13_h3c.bin
            let mut loaded = 0usize;
            for path in std::env::args_os().skip(1) {
                match std::fs::read(&path) {
                    Ok(bytes) => {
                        let name = Path::new(&path).display().to_string();
                        ed.open_auto(&bytes, &name);
                        loaded += 1;
                    }
                    Err(e) => ed.msg = format!("读取失败 {}: {e}", Path::new(&path).display()),
                }
            }
            if loaded == 0 {
                ed.msg = "新建 Factory（可打开 mtd14 出厂区 + mtd13 下行校准两个文件）".into();
            }
            Ok(Box::new(ed))
        }),
    )
}
