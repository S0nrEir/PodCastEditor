mod playback;
mod timeline;
use crate::{
    audio::{Track, peak_stride, processed},
    edit::{Plan, Span, stamp, time_span},
};
use anyhow::{Result, anyhow};
use eframe::egui::{self, Color32, Context, FontFamily, Frame, RichText, Stroke, Ui, vec2};
use playback::Playback;
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};
use timeline::{Gesture, Timeline};

pub fn run_studio(path: Option<PathBuf>) -> std::process::ExitCode {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1110.0, 720.0])
            .with_min_inner_size([840.0, 600.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    match eframe::run_native(
        "声剪 · 播客编辑器",
        options,
        Box::new(move |cc| Ok(Box::new(Studio::new(cc, path)))),
    ) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            rfd::MessageDialog::new()
                .set_title("声剪启动失败")
                .set_description(error.to_string())
                .set_level(rfd::MessageLevel::Error)
                .show();
            std::process::ExitCode::FAILURE
        }
    }
}

const BASE: Color32 = Color32::from_rgb(20, 28, 35);
const PANEL: Color32 = Color32::from_rgb(26, 36, 44);
const PLOT: Color32 = Color32::from_rgb(14, 22, 29);
const LINE: Color32 = Color32::from_rgb(50, 65, 74);
const MUTED: Color32 = Color32::from_rgb(158, 177, 185);
const ACCENT: Color32 = Color32::from_rgb(119, 223, 191);
const CUT: Color32 = Color32::from_rgb(247, 164, 136);

struct View {
    plan: Plan,
    window: Span,
    indexed_peaks: Vec<(i16, i16)>,
    index_stride: u64,
    peaks: Vec<(i16, i16)>,
}
impl View {
    fn new(track: &Track, plan: Plan) -> Result<Self> {
        let full = 0..plan.frames;
        let index_stride = peak_stride(plan.frames).min(256);
        let indexed_peaks = track.waveform_range(&plan, full.clone(), index_stride)?;
        let mut view = Self {
            plan,
            window: 0..0,
            indexed_peaks,
            index_stride,
            peaks: Vec::new(),
        };
        view.set_window(track, full)?;
        Ok(view)
    }

    fn set_window(&mut self, track: &Track, range: Span) -> Result<()> {
        if range == self.window {
            return Ok(());
        }
        let stride = peak_stride(range.end - range.start);
        // Wide views use the precomputed index. At finer scales read only the visible PCM.
        self.peaks = if stride < self.index_stride {
            track.waveform_range(&self.plan, range.clone(), stride)?
        } else {
            range
                .clone()
                .step_by(stride as usize)
                .map(|start| {
                    let first = (start / self.index_stride) as usize;
                    let end =
                        ((start + stride).min(range.end)).div_ceil(self.index_stride) as usize;
                    self.indexed_peaks[first..end]
                        .iter()
                        .fold((0i16, 0i16), |(lo, hi), &(a, b)| (lo.min(a), hi.max(b)))
                })
                .collect()
        };
        self.window = range;
        Ok(())
    }

    fn reveal(&mut self, track: &Track, mark: &Span) -> Result<()> {
        if mark.start >= self.window.start && mark.end <= self.window.end {
            return Ok(());
        }
        let mark_len = mark.end - mark.start;
        let width = (self.window.end - self.window.start).max(mark_len);
        let start = mark
            .start
            .saturating_sub((width - mark_len) / 2)
            .min(self.plan.frames - width);
        self.set_window(track, start..start + width)
    }
}

struct Document {
    track: Arc<Track>,
    original: View,
    result: Option<View>,
    show_result: bool,
    marks: Vec<Span>,
    start: String,
    end: String,
    cursor: u64,
}

impl Document {
    fn open(path: PathBuf) -> Result<Self> {
        let track = Arc::new(Track::open(&path)?);
        let original = View::new(&track, track.original())?;
        Ok(Self {
            track,
            original,
            result: None,
            show_result: false,
            marks: Vec::new(),
            start: "00:00.000".into(),
            end: String::new(),
            cursor: 0,
        })
    }
    fn view(&self) -> &View {
        if self.show_result
            && let Some(result) = &self.result
        {
            return result;
        }
        &self.original
    }
    fn selection(&self) -> Result<Span> {
        time_span(&self.start, &self.end, self.track.rate, self.track.frames)
    }
    fn select(&mut self, range: Span) {
        self.show_result = false;
        self.cursor = range.start;
        self.start = stamp(range.start, self.track.rate);
        self.end = stamp(range.end, self.track.rate);
    }
}

enum Finished {
    Imported(Document),
    Processed(View),
    Exported(PathBuf),
}
enum Action {
    Pick,
    Import(PathBuf),
    Mark,
    Remove(usize),
    Recall(usize),
    Process,
    Export,
    Play,
    Audition,
    Seek(u64),
    Window(Span),
    Select(Span),
    Switch(bool),
}

pub struct Studio {
    doc: Option<Document>,
    worker: Option<mpsc::Receiver<Result<Finished>>>,
    player: Option<Playback>,
    status: String,
    error: bool,
    close_after_work: bool,
}

impl Studio {
    pub fn new(cc: &eframe::CreationContext<'_>, path: Option<PathBuf>) -> Self {
        theme(&cc.egui_ctx);
        let mut app = Self {
            doc: None,
            worker: None,
            player: None,
            status: "就绪".into(),
            error: false,
            close_after_work: false,
        };
        if let Some(path) = path {
            app.import(&cc.egui_ctx, path);
        }
        app
    }

    fn work(
        &mut self,
        ctx: &Context,
        message: &str,
        task: impl FnOnce() -> Result<Finished> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        self.worker = Some(rx);
        self.player = None;
        self.status = message.to_owned();
        self.error = false;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(task());
            ctx.request_repaint();
        });
    }

    fn import(&mut self, ctx: &Context, path: PathBuf) {
        self.work(ctx, "正在读取音频并生成波形…", move || {
            Ok(Finished::Imported(Document::open(path)?))
        });
    }

    fn poll(&mut self) {
        let Some(rx) = &self.worker else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(value) => value,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow!("后台任务意外结束")),
        };
        self.worker = None;
        match result {
            Ok(Finished::Imported(doc)) => {
                self.doc = Some(doc);
                self.status = "音频已导入".into();
            }
            Ok(Finished::Processed(view)) => {
                if let Some(doc) = &mut self.doc {
                    doc.result = Some(view);
                    doc.show_result = true;
                    doc.cursor = 0;
                }
                self.status = "标记已处理，可试听并导出结果".into();
            }
            Ok(Finished::Exported(path)) => self.status = format!("已导出到 {}", path.display()),
            Err(e) => {
                self.error = true;
                self.status = format!("{e:#}");
            }
        }
    }

    fn act(&mut self, ctx: &Context, action: Action) -> Result<()> {
        if self.worker.is_some() {
            return Ok(());
        }
        self.error = false;
        match action {
            Action::Pick => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("导入播客音频")
                    .add_filter("音频", &["wav", "mp3", "flac", "m4a", "aac", "ogg", "aiff"])
                    .add_filter("所有文件", &["*"])
                    .pick_file()
                {
                    self.import(ctx, path);
                }
                return Ok(());
            }
            Action::Import(path) => {
                self.import(ctx, path);
                return Ok(());
            }
            _ => {}
        }
        let Some(doc) = &mut self.doc else {
            return Ok(());
        };
        match action {
            Action::Mark => {
                let range = doc.selection()?;
                if doc.marks.contains(&range) {
                    return Err(anyhow!("这个范围已有剪辑标记"));
                }
                doc.marks.push(range);
                doc.marks.sort_by_key(|r| r.start);
                doc.result = None;
                self.status = format!("已添加剪辑标记，共 {} 个", doc.marks.len());
            }
            Action::Remove(i) => {
                if doc.show_result {
                    doc.cursor = 0;
                }
                doc.marks.remove(i);
                doc.result = None;
                doc.show_result = false;
                self.player = None;
                self.status = "已移除标记".into();
            }
            Action::Recall(i) => {
                doc.original.reveal(&doc.track, &doc.marks[i])?;
                self.player = None;
                doc.select(doc.marks[i].clone());
            }
            Action::Select(range) => {
                self.player = None;
                doc.select(range);
            }
            Action::Seek(frame) => {
                self.player = None;
                doc.cursor = frame;
            }
            Action::Window(range) => {
                let view = if doc.show_result {
                    doc.result.as_mut().expect("result view exists")
                } else {
                    &mut doc.original
                };
                view.set_window(&doc.track, range)?;
            }
            Action::Switch(result) => {
                self.player = None;
                doc.show_result = result;
                doc.cursor = doc.view().window.start;
            }
            Action::Play => {
                if self.player.take().is_none() {
                    let plan = &doc.view().plan;
                    let from = if doc.cursor >= plan.frames {
                        0
                    } else {
                        doc.cursor
                    };
                    self.player = Some(Playback::play(&doc.track, plan, from, plan.frames)?);
                }
            }
            Action::Audition => {
                let range = doc.selection()?;
                doc.original.reveal(&doc.track, &range)?;
                self.player = None;
                doc.show_result = false;
                doc.cursor = range.start;
                self.player = Some(Playback::play(
                    &doc.track,
                    &doc.original.plan,
                    range.start,
                    range.end,
                )?);
            }
            Action::Process => {
                let plan = processed(doc.track.frames, &doc.marks)?;
                let track = doc.track.clone();
                self.work(ctx, "正在处理剪辑标记…", move || {
                    Ok(Finished::Processed(View::new(&track, plan)?))
                });
            }
            Action::Export => {
                let Some(view) = &doc.result else {
                    return Ok(());
                };
                let stem = doc
                    .track
                    .path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy();
                let Some(mut path) = rfd::FileDialog::new()
                    .set_title("导出剪辑结果")
                    .add_filter("WAV 音频", &["wav"])
                    .set_file_name(format!("{stem}-剪辑.wav"))
                    .save_file()
                else {
                    return Ok(());
                };
                let overwrite = path.extension().is_some() && path.try_exists()?;
                if path.extension().is_none() {
                    path.set_extension("wav");
                }
                let track = doc.track.clone();
                let plan = view.plan.clone();
                self.work(ctx, "正在写入 WAV 文件…", move || {
                    track.export(&plan, &path, overwrite)?;
                    Ok(Finished::Exported(path))
                });
            }
            Action::Pick | Action::Import(_) => unreachable!(),
        }
        Ok(())
    }
}

fn theme(ctx: &Context) {
    let mut fonts = egui::FontDefinitions::default();
    let win = std::env::var_os("WINDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("C:/Windows"));
    let paths = [
        win.join("Fonts/msyh.ttc"),
        win.join("Fonts/simhei.ttf"),
        PathBuf::from("/System/Library/Fonts/PingFang.ttc"),
        PathBuf::from("/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc"),
    ];
    if let Some(data) = paths.iter().find_map(|p| std::fs::read(p).ok()) {
        fonts
            .font_data
            .insert("cjk".into(), egui::FontData::from_owned(data).into());
        for family in [FontFamily::Proportional, FontFamily::Monospace] {
            fonts.families.entry(family).or_default().push("cjk".into());
        }
    }
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    let mut v = egui::Visuals::dark();
    v.panel_fill = BASE;
    v.window_fill = PANEL;
    v.extreme_bg_color = PLOT;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.inactive.bg_fill = LINE;
    v.widgets.inactive.weak_bg_fill = LINE;
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(63, 85, 94);
    v.selection.bg_fill = Color32::from_rgb(47, 97, 93);
    v.override_text_color = Some(Color32::from_rgb(230, 238, 239));
    style.visuals = v;
    style.spacing.item_spacing = vec2(8.0, 7.0);
    style.spacing.button_padding = vec2(12.0, 6.0);
    ctx.set_style(style);
}

fn title(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(19.0).strong());
}
fn hint(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).size(12.0).color(MUTED));
}
fn panel(color: Color32, padding: i8) -> Frame {
    Frame::new().fill(color).inner_margin(padding)
}
fn clock(frames: u64, rate: u32) -> String {
    let t = stamp(frames, rate);
    t.split_once('.')
        .map(|(a, b)| format!("{a}.{}", &b[..b.len().min(3)]))
        .unwrap_or(t)
}
fn logo(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(r, 9, ACCENT);
    for (i, height) in [5., 12., 19., 9., 15.].iter().enumerate() {
        p.vline(
            r.left() + 7.0 + i as f32 * 5.0,
            (r.center().y - height * 0.5)..=(r.center().y + height * 0.5),
            Stroke::new(2.5_f32, PLOT),
        );
    }
}

fn controls(ui: &mut Ui, doc: &mut Document, action: &mut Option<Action>) {
    let rate = doc.track.rate;
    let width = ui.available_width();
    title(ui, "范围选择");
    hint(ui, "在原音轨上拖选，或输入起止时间");
    ui.add_space(8.0);
    ui.add_enabled_ui(!doc.show_result, |ui| {
        for (name, value) in [("开始时间", &mut doc.start), ("结束时间", &mut doc.end)] {
            hint(ui, name);
            ui.add(
                egui::TextEdit::singleline(value)
                    .font(egui::TextStyle::Monospace)
                    .hint_text("分:秒.毫秒")
                    .desired_width(f32::INFINITY)
                    .margin(vec2(9.0, 7.0)),
            );
        }
        let sel = doc.selection();
        if let Ok(span) = &sel {
            hint(
                ui,
                &format!("选区时长   {}", clock(span.end - span.start, rate)),
            );
        } else if !doc.end.is_empty() {
            ui.label(
                RichText::new(sel.as_ref().unwrap_err().to_string())
                    .size(12.0)
                    .color(CUT),
            );
        } else {
            hint(ui, "选区时长   —");
        }
        if ui
            .add_enabled(
                sel.is_ok(),
                egui::Button::new("试听选区").min_size(vec2(width, 27.0)),
            )
            .clicked()
        {
            *action = Some(Action::Audition);
        }
        if ui
            .add_enabled(
                sel.is_ok(),
                egui::Button::new(RichText::new("＋  添加剪辑标记").color(CUT))
                    .min_size(vec2(width, 31.0)),
            )
            .clicked()
        {
            *action = Some(Action::Mark);
        }
    });
    ui.add_space(12.0);
    ui.separator();
    ui.add_space(7.0);
    title(ui, "剪辑汇总");
    let keep = Plan::from_marks(doc.track.frames, &doc.marks)
        .expect("validated marks")
        .frames;
    for (name, frames, color) in [
        ("原始时长", doc.track.frames, MUTED),
        ("剪去时长", doc.track.frames - keep, CUT),
        ("保留时长", keep, ACCENT),
    ] {
        ui.horizontal(|ui| {
            hint(ui, name);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(RichText::new(clock(frames, rate)).monospace().color(color));
            });
        });
    }
    ui.add_space(6.0);
    if ui
        .add_enabled(
            !doc.marks.is_empty() && doc.result.is_none(),
            egui::Button::new(
                RichText::new(if doc.result.is_some() {
                    "标记已处理"
                } else {
                    "处理所有标记"
                })
                .color(PLOT)
                .strong(),
            )
            .fill(ACCENT)
            .min_size(vec2(width, 37.0)),
        )
        .clicked()
    {
        *action = Some(Action::Process);
    }
}

fn editor(ui: &mut Ui, doc: &Document, playing: bool, action: &mut Option<Action>) {
    let rate = doc.track.rate;
    let name = doc
        .track
        .path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    ui.add(egui::Label::new(RichText::new(name.as_ref()).size(22.0).strong()).truncate())
        .on_hover_text(doc.track.path.display().to_string());
    hint(
        ui,
        &format!("{} Hz    /    {} 声道", rate, doc.track.channels),
    );
    ui.add_space(5.0);
    let view = doc.view();
    ui.horizontal(|ui| {
        if ui.selectable_label(!doc.show_result, "原始音轨").clicked() && doc.show_result {
            *action = Some(Action::Switch(false));
        }
        if ui
            .add_enabled(
                doc.result.is_some(),
                egui::SelectableLabel::new(doc.show_result, "剪辑结果"),
            )
            .clicked()
            && !doc.show_result
        {
            *action = Some(Action::Switch(true));
        }
        hint(ui, "Ctrl + 滚轮缩放");
        if view.window != (0..view.plan.frames) && ui.small_button("显示全轨").clicked() {
            *action = Some(Action::Window(0..view.plan.frames));
        }
    });
    let wave = Timeline {
        peaks: &view.peaks,
        frames: view.plan.frames,
        window: &view.window,
        rate,
        marks: if doc.show_result { &[] } else { &doc.marks },
        selection: if doc.show_result {
            None
        } else {
            doc.selection().ok()
        },
        cursor: doc.cursor,
        editable: !doc.show_result,
    };
    if let Some(gesture) = wave.draw(ui) {
        *action = Some(match gesture {
            Gesture::Seek(f) => Action::Seek(f),
            Gesture::Zoom(range) => Action::Window(range),
            Gesture::Select(r) => Action::Select(r),
        });
    }
    ui.horizontal(|ui| {
        if ui
            .button(if playing { "暂停" } else { "播放" })
            .on_hover_text("空格键播放或暂停")
            .clicked()
        {
            *action = Some(Action::Play);
        }
        ui.label(
            RichText::new(format!(
                "{}  /  {}",
                clock(doc.cursor, rate),
                clock(view.plan.frames, rate)
            ))
            .monospace()
            .size(13.0),
        );
    });
    ui.add_space(9.0);
    ui.separator();
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        title(ui, "剪辑标记");
        hint(ui, &format!("{} 段", doc.marks.len()));
    });
    if doc.marks.is_empty() {
        hint(ui, "尚无标记。选中要删除的片段后，点击「添加剪辑标记」。");
        return;
    }
    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            for (i, mark) in doc.marks.iter().enumerate() {
                ui.allocate_ui_with_layout(
                    vec2(ui.available_width(), 30.0),
                    egui::Layout::right_to_left(egui::Align::Center),
                    |ui| {
                        if ui.small_button("移除").clicked() {
                            *action = Some(Action::Remove(i));
                        }
                        ui.with_layout(egui::Layout::top_down_justified(egui::Align::Min), |ui| {
                            let selected = wave.selection.as_ref() == Some(mark);
                            ui.visuals_mut().selection.bg_fill = Color32::from_rgb(47, 82, 109);
                            let range = format!(
                                "剪辑    {}  –  {}",
                                stamp(mark.start, rate),
                                stamp(mark.end, rate)
                            );
                            let text = RichText::new(range).monospace();
                            let text = if selected {
                                text.strong().color(Color32::LIGHT_BLUE)
                            } else {
                                text
                            };
                            if ui
                                .selectable_label(selected, text)
                                .on_hover_text("在原始音轨中选中此范围")
                                .clicked()
                            {
                                *action = Some(Action::Recall(i));
                            }
                        });
                    },
                );
                ui.separator();
            }
        });
}

impl eframe::App for Studio {
    fn update(&mut self, ctx: &Context, _frame: &mut eframe::Frame) {
        self.poll();
        let busy = self.worker.is_some();
        if let Some(player) = &self.player
            && let Some(doc) = &mut self.doc
        {
            doc.cursor = player.position();
            if player.finished() {
                self.player = None;
            }
            ctx.request_repaint_after(Duration::from_millis(35));
        }
        if busy && ctx.input(|i| i.viewport().close_requested()) {
            self.close_after_work = true;
            self.status = "当前任务完成后将关闭窗口…".into();
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else if !busy && self.close_after_work {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        let mut action = ctx
            .input(|i| i.raw.dropped_files.iter().find_map(|f| f.path.clone()))
            .map(Action::Import);
        egui::TopBottomPanel::top("header")
            .frame(panel(PANEL, 14))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    logo(ui);
                    ui.label(RichText::new("声剪").size(23.0).strong());
                    ui.separator();
                    hint(ui, "播客编辑器");
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui
                            .add_enabled(
                                !busy && self.doc.as_ref().is_some_and(|d| d.result.is_some()),
                                egui::Button::new("导出 WAV…"),
                            )
                            .clicked()
                        {
                            action = Some(Action::Export);
                        }
                        if ui
                            .add_enabled(!busy, egui::Button::new("导入音频…"))
                            .clicked()
                        {
                            action = Some(Action::Pick);
                        }
                    });
                });
            });
        egui::TopBottomPanel::bottom("status")
            .frame(panel(PLOT, 8))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if busy {
                        ui.spinner();
                    }
                    ui.label(RichText::new(&self.status).size(12.0).color(if self.error {
                        CUT
                    } else {
                        MUTED
                    }));
                });
            });
        if let Some(doc) = &mut self.doc {
            egui::SidePanel::right("controls")
                .resizable(false)
                .exact_width(270.0)
                .frame(panel(PANEL, 18))
                .show(ctx, |ui| {
                    ui.add_enabled_ui(!busy, |ui| controls(ui, doc, &mut action));
                });
            egui::CentralPanel::default()
                .frame(panel(BASE, 18))
                .show(ctx, |ui| {
                    ui.add_enabled_ui(!busy, |ui| {
                        editor(ui, doc, self.player.is_some(), &mut action)
                    });
                });
        } else {
            egui::CentralPanel::default()
                .frame(panel(BASE, 24))
                .show(ctx, |ui| {
                    ui.add_space((ui.available_height() * 0.32).max(0.0));
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("导入第一条音轨").size(30.0).strong());
                        hint(ui, "将录音拖到这里，或从本机选择音频文件");
                        ui.add_space(12.0);
                        if ui
                            .add_enabled(
                                !busy,
                                egui::Button::new(RichText::new("选择音频文件…").color(PLOT))
                                    .fill(ACCENT)
                                    .min_size(vec2(156.0, 36.0)),
                            )
                            .clicked()
                        {
                            action = Some(Action::Pick);
                        }
                        hint(ui, "WAV  /  MP3  /  FLAC  /  M4A  /  OGG");
                    });
                });
        }
        if !busy
            && self.doc.is_some()
            && !ctx.wants_keyboard_input()
            && ctx.input(|i| i.key_pressed(egui::Key::Space))
        {
            action = Some(Action::Play);
        }
        if let Some(action) = action {
            if let Err(e) = self.act(ctx, action) {
                self.status = format!("{e:#}");
                self.error = true;
            }
            ctx.request_repaint();
        }
    }
}
