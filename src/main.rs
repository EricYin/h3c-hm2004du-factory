#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! H3C HM2004-DU Factory Editor — egui port of `fiberhome-factory`.
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

mod font;
mod import;
mod model;

use eframe::egui;
use model::{
    detect_downlink_chip, detect_pon_chip, derive_mac, mac_to_string, parse_mac, random_mac,
    random_sn, Factory, PON_DOWNLINK_CAL_SIZE, CALIBRATION_SIZE, OFF_ETH0, OFF_PON0,
    OFF_WIFI_24G, OFF_WIFI_5G, WIFI_EEPROM_SIZE,
};
use rfd::FileDialog;
use std::path::Path;

struct Editor {
    factory: Factory,
    device_sn: String,
    label_mac: String,
    msg: String,
    /// 已载入的出厂区（mtd14 / reservearea）文件名，用于状态栏与摘要。
    src_factory: Option<String>,
    /// 已载入的下行 PON 分区（mtd13）文件名。
    src_downlink: Option<String>,
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

    /// Dispatch on content so the user can pick either file in the same dialog.
    fn open_auto(&mut self, bytes: &[u8], name: &str) {
        if import::looks_like_mtd13(bytes) {
            self.open_downlink(bytes, name);
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

        // ---- 顶栏：标题 + 动作 ----
        ui.horizontal(|ui| {
            ui.heading("H3C Factory");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("保存 1 MiB").clicked() {
                    self.apply_to_factory();
                    let img = self.factory.encode_1mib();
                    self.save_image(img, "factory-h3c-1mib.bin");
                }
                if ui.button("保存真实布局").clicked() {
                    self.apply_to_factory();
                    let img = self.factory.encode();
                    self.save_image(img, "factory-h3c-layout.bin");
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
                // 出厂校准分散在两个分区里，故给两个明确的入口：顺序任意，
                // 互不覆盖（见 open_factory / open_downlink）。
                if ui.button("打开出厂区 (mtd14)").clicked() {
                    if let Some(p) = FileDialog::new().add_filter("bin", &["bin"]).pick_file() {
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
                    if let Some(p) = FileDialog::new().add_filter("bin", &["bin"]).pick_file() {
                        match std::fs::read(&p) {
                            Ok(b) => {
                                let name = p.display().to_string();
                                self.open_downlink(&b, &name);
                            }
                            Err(e) => self.msg = format!("读取失败: {e}"),
                        }
                    }
                }
                if ui.button("新建").clicked() {
                    self.factory = Factory::new();
                    self.sync_from_factory();
                    self.src_factory = None;
                    self.src_downlink = None;
                    self.msg = "新建 Factory".into();
                }
            });
        });
        ui.label(&self.msg);
        ui.separator();

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
            ui.add(
                egui::TextEdit::singleline(&mut self.label_mac).desired_width(FIELD),
            );
            if ui.button("随机").clicked() {
                self.factory.label_mac = random_mac();
                self.label_mac = mac_to_string(&self.factory.label_mac);
            }
        });
        ui.horizontal(|ui| {
            ui.label("设备序列号");
            ui.add(
                egui::TextEdit::singleline(&mut self.device_sn).desired_width(FIELD),
            );
            if ui.button("随机").clicked() {
                self.factory.device_sn = random_sn();
                self.device_sn = self.factory.device_sn.clone();
            }
        });

        ui.add_space(6.0);
        ui.label("派生 MAC（末字节偏移，随标签 MAC 实时计算）");
        let base = parse_mac(&self.label_mac).unwrap_or(self.factory.label_mac);
        egui::Grid::new("derived_macs")
            .num_columns(2)
            .spacing([16.0, 3.0])
            .show(ui, |ui| {
                ui.label("eth0  (+1)");
                ui.monospace(mac_to_string(&derive_mac(&base, OFF_ETH0)));
                ui.end_row();
                ui.label("pon0  (+2)");
                ui.monospace(mac_to_string(&derive_mac(&base, OFF_PON0)));
                ui.end_row();
                ui.label("5G    (+4)");
                ui.monospace(mac_to_string(&derive_mac(&base, OFF_WIFI_5G)));
                ui.end_row();
                ui.label("2.4G  (+5)");
                ui.monospace(mac_to_string(&derive_mac(&base, OFF_WIFI_24G)));
                ui.end_row();
            });

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
/// 在 `egui::Visuals::light()` 基础上改三处：
/// - `weak_text_color`：默认由 `text_color * weak_text_alpha(0.6)` 派生，白底上
///   发灰、几乎不可读，钉成 110。
/// - 三态按钮底色 230 / 220 / 165 太接近，一眼分不出，拉开成 238 / 205 / 170。
/// - `panel_fill` / `window_fill` 钉成纯白。
fn light_style() -> std::sync::Arc<egui::Style> {
    static STYLE: std::sync::OnceLock<std::sync::Arc<egui::Style>> = std::sync::OnceLock::new();
    STYLE
        .get_or_init(|| {
            let mut v = egui::Visuals::light();
            v.weak_text_color = Some(egui::Color32::from_gray(110));
            v.widgets.inactive.bg_fill = egui::Color32::from_gray(238);
            v.widgets.inactive.weak_bg_fill = egui::Color32::from_gray(238);
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
            .with_inner_size([720.0, 760.0])
            .with_min_inner_size([640.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "H3C Factory 编辑器",
        options,
        Box::new(|cc| {
            font::install(&cc.egui_ctx);
            apply_light_theme(&cc.egui_ctx);
            let mut ed = Editor {
                factory: Factory::new(),
                device_sn: String::new(),
                label_mac: String::new(),
                msg: "新建 Factory".to_string(),
                src_factory: None,
                src_downlink: None,
            };
            ed.sync_from_factory();
            // Like upstream: allow `h3c-hm2004du-factory.exe backup.bin`
            // (or dragging a .bin onto the exe) to open a file directly.
            // Multiple paths may be given, which is how a real unit's two
            // partitions get loaded in one go:
            //   h3c-hm2004du-factory.exe mtd14_reservearea.bin mtd13_h3c.bin
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
