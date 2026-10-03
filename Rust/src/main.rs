#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod processor;

use eframe::egui::{self, Align2, Color32, FontId, RichText, Sense, Stroke};
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};

const BLUE: Color32 = Color32::from_rgb(0x1f, 0x53, 0x8d);
const GREEN: Color32 = Color32::from_rgb(0x2e, 0xcc, 0x71);
const ORANGE: Color32 = Color32::from_rgb(0xe6, 0x7e, 0x22);
const IDLE_TEXT: &str = "ここに圧縮ファイルをドラッグ＆ドロップ\n（クリックで選択・複数同時に処理可能）";

enum Msg {
    Log(String),
    Status(String),
    Progress(f32),
    Done(Option<PathBuf>),
    Error(String),
}

struct App {
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    processing: bool,
    quality: u8,
    log: String,
    status: String,
    progress: f32,
    last_dir: Option<PathBuf>,
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        let mut v = egui::Visuals::dark();
        v.panel_fill = Color32::from_rgb(0x1a, 0x1b, 0x1e);
        v.selection.bg_fill = BLUE;
        v.widgets.noninteractive.rounding = 8.0.into();
        v.widgets.inactive.rounding = 8.0.into();
        v.widgets.hovered.rounding = 8.0.into();
        v.widgets.active.rounding = 8.0.into();
        cc.egui_ctx.set_visuals(v);
        cc.egui_ctx.style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(10.0, 10.0);
            s.spacing.button_padding = egui::vec2(14.0, 7.0);
        });
        let (tx, rx) = channel();
        Self {
            tx,
            rx,
            processing: false,
            quality: 50,
            log: String::new(),
            status: "処理待機中...".into(),
            progress: 0.0,
            last_dir: None,
        }
    }

    fn start(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>) {
        self.processing = true;
        self.progress = 0.0;
        self.last_dir = None;
        let (tx, ctx, q) = (self.tx.clone(), ctx.clone(), self.quality as f32);
        std::thread::spawn(move || worker(paths, q, tx, ctx));
    }

    fn drain(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Log(s) => self.log.push_str(&s),
                Msg::Status(s) => self.status = s,
                Msg::Progress(p) => self.progress = p,
                Msg::Done(dir) => {
                    self.processing = false;
                    self.progress = 1.0;
                    self.status = "すべての処理が完了しました".into();
                    self.last_dir = dir;
                }
                Msg::Error(e) => {
                    self.log.push_str(&format!("[エラー] {e}\n\n"));
                    self.status = "エラーが発生しました".into();
                    self.processing = false;
                }
            }
        }
    }
}

fn worker(paths: Vec<PathBuf>, quality: f32, tx: Sender<Msg>, ctx: egui::Context) {
    let send = |m: Msg| {
        let _ = tx.send(m);
        ctx.request_repaint();
    };

    let mut valid = Vec::new();
    for p in paths {
        if !p.exists() {
            continue;
        }
        let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
        if ["zip", "rar", "7z"].contains(&ext.as_str()) {
            valid.push(p);
        } else {
            send(Msg::Log(format!(
                "【スキップ】 未対応の拡張子です: {}\n\n",
                p.file_name().unwrap_or_default().to_string_lossy()
            )));
        }
    }
    if valid.is_empty() {
        send(Msg::Error("処理可能な圧縮ファイル (zip, rar, 7z) がありませんでした。".into()));
        return;
    }

    let total = valid.len();
    let mut success = 0;
    let mut last_dir = None;
    for (i, path) in valid.iter().enumerate() {
        let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
        send(Msg::Status(format!("処理中 ({}/{}): {}", i + 1, total, name)));

        let res = processor::process_single_archive(
            path,
            quality,
            &|m| send(Msg::Log(m + "\n")),
            &|cur, tot| {
                let inner = if tot > 0 { cur as f32 / tot as f32 / total as f32 } else { 0.0 };
                send(Msg::Progress(i as f32 / total as f32 + inner));
            },
        );
        match res {
            Ok(out) => {
                success += 1;
                last_dir = out.parent().map(|p| p.to_path_buf());
            }
            Err(e) => send(Msg::Log(format!(
                "[エラー] {name} の処理に失敗しました。\n詳細: {e:#}\n\n"
            ))),
        }
    }
    send(Msg::Log(format!("【完了】 処理に成功したファイル: {success} / {total} 件\n")));
    send(Msg::Done(last_dir));
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        self.drain();

        let dropped: Vec<PathBuf> =
            ctx.input(|i| i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect());
        let hovering = ctx.input(|i| !i.raw.hovered_files.is_empty());

        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(ctx.style().visuals.panel_fill).inner_margin(22.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(RichText::new("Archive Image WebP Converter").size(24.0).strong());
                    ui.label(
                        RichText::new(
                            "圧縮ファイル (zip, rar, 7z) 内の画像を WebP に一括変換し、zip で再圧縮します。\n画像以外のファイルは自動的に除外されます。",
                        )
                        .size(12.0)
                        .color(Color32::from_gray(150)),
                    );
                });

                // ドロップゾーン
                let (rect, resp) =
                    ui.allocate_exact_size(egui::vec2(ui.available_width(), 120.0), Sense::click());
                let (border, text) = if self.processing {
                    (ORANGE, "現在処理中です。しばらくお待ちください...")
                } else if hovering {
                    (GREEN, "ここにドロップして処理を開始")
                } else {
                    (BLUE, IDLE_TEXT)
                };
                ui.painter().rect(rect, 14.0, Color32::from_rgb(0x21, 0x21, 0x25), Stroke::new(2.0_f32, border));
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    text,
                    FontId::proportional(15.0),
                    Color32::WHITE,
                );
                if !self.processing {
                    if resp.hovered() {
                        ctx.set_cursor_icon(egui::CursorIcon::PointingHand);
                    }
                    if resp.clicked() {
                        if let Some(files) = rfd::FileDialog::new()
                            .add_filter("圧縮ファイル", &["zip", "rar", "7z"])
                            .pick_files()
                        {
                            self.start(ctx, files);
                        }
                    }
                }

                // 品質
                ui.add_enabled(
                    !self.processing,
                    egui::Slider::new(&mut self.quality, 5..=100)
                        .suffix("%")
                        .text("WebP 圧縮品質"),
                );

                // 進捗
                ui.label(RichText::new(&self.status).size(12.0));
                ui.add(egui::ProgressBar::new(self.progress).animate(self.processing));

                // ログ
                ui.label(RichText::new("処理ログ").size(12.0).strong());
                let h = (ui.available_height() - 50.0).max(80.0);
                egui::Frame::none()
                    .fill(Color32::from_rgb(0x14, 0x14, 0x16))
                    .rounding(8.0)
                    .inner_margin(8.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .max_height(h)
                            .min_scrolled_height(h)
                            .stick_to_bottom(true)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(&self.log).monospace().size(11.5),
                                    )
                                    .wrap(),
                                );
                            });
                    });

                ui.horizontal(|ui| {
                    if ui.add_enabled(!self.processing, egui::Button::new("ログをクリア")).clicked() {
                        self.log.clear();
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let can_open = !self.processing && self.last_dir.is_some();
                        let btn = egui::Button::new(RichText::new("保存先 (new_temp) を開く").color(Color32::WHITE))
                            .fill(GREEN);
                        if ui.add_enabled(can_open, btn).clicked() {
                            if let Some(d) = &self.last_dir {
                                if let Err(e) = std::process::Command::new("explorer").arg(d).spawn() {
                                    self.log.push_str(&format!("[エラー] 保存先フォルダを開けませんでした。\n詳細: {e}\n\n"));
                                }
                            }
                        }
                    });
                });
            });

        if !self.processing && !dropped.is_empty() {
            self.start(ctx, dropped);
        }
    }
}

fn setup_fonts(ctx: &egui::Context) {
    // exe に埋め込む（fonts/NotoSansJP-Regular.ttf を配置しておくこと）
    const NOTO_SANS_JP: &[u8] = include_bytes!("../fonts/NotoSansJP-Regular.ttf");

    let mut fonts = egui::FontDefinitions::default();
    fonts
        .font_data
        .insert("noto_sans_jp".into(), egui::FontData::from_static(NOTO_SANS_JP));
    // 先頭に挿入して最優先にする（英数字もNoto Sans JPで描画）
    for fam in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.get_mut(&fam).unwrap().insert(0, "noto_sans_jp".into());
    }
    ctx.set_fonts(fonts);
}

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([650.0, 650.0])
            .with_min_inner_size([550.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Archive Image WebP Converter",
        options,
        Box::new(|cc| Ok(Box::new(App::new(cc)))),
    )
}
