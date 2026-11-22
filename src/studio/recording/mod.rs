mod input;
#[cfg(target_os = "macos")]
mod permission;
#[cfg(test)]
mod tests;

use super::{ACCENT, BASE, CUT, MUTED, PANEL, PLOT, clock, hint, panel};
use anyhow::{Result, anyhow};
use eframe::egui::{self, Context, Event, Key, Modifiers, RichText, text::LayoutJob};
use input::{Capture, Draft};
use std::{
    path::PathBuf,
    sync::{Arc, mpsc},
    time::Duration,
};

type Manuscript = (PathBuf, LayoutJob);

#[derive(Default)]
pub(super) struct Recording {
    script: Option<Manuscript>,
    offset: f32,
    reset_scroll: bool,
    draft: Option<Arc<Draft>>,
    capture: Option<Capture>,
    export: Option<mpsc::Receiver<Result<PathBuf>>>,
    status: String,
    error: bool,
    permission: Option<mpsc::Receiver<bool>>,
}

impl Recording {
    fn has_audio(&self) -> bool {
        self.draft.as_ref().is_some_and(|d| d.frames() > 0)
    }
    pub fn exporting(&self) -> bool {
        self.export.is_some()
    }

    pub fn load(&mut self, script: Option<Manuscript>) -> bool {
        let changed = self.script.as_ref().map(|s| &s.0) != script.as_ref().map(|s| &s.0);
        if changed && !self.confirm_leave() {
            return false;
        }
        if changed {
            self.reset_scroll = true;
            self.status.clear();
            self.error = false;
        }
        self.script = script.map(|(path, mut job)| {
            for section in &mut job.sections {
                section.format.font_id.size *= 1.22;
            }
            (path, job)
        });
        true
    }

    fn problem(&mut self, error: impl std::fmt::Display) {
        self.status = format!("{error:#}");
        self.error = true;
    }

    fn toggle(&mut self) {
        if self.capture.is_some() {
            self.stop();
            return;
        }
        #[cfg(target_os = "macos")]
        match permission::request() {
            Ok(Some(rx)) => {
                self.permission = Some(rx);
                self.status = "等待系统麦克风授权…".into();
                self.error = false;
                return;
            }
            Ok(None) => {}
            Err(e) => {
                self.problem(e);
                return;
            }
        }
        self.start();
    }

    fn start(&mut self) {
        match Capture::start(self.draft.as_ref()) {
            Ok((capture, draft)) => {
                self.capture = Some(capture);
                self.draft = Some(draft);
                self.status.clear();
                self.error = false;
            }
            Err(e) => self.problem(e),
        }
    }

    pub fn stop(&mut self) {
        if self.permission.take().is_some() {
            self.status.clear();
        }
        if let Some(capture) = self.capture.take() {
            match capture.stop() {
                Err(e) => self.problem(e),
                Ok(()) if !self.has_audio() => {
                    self.problem("没有收到麦克风音频，请检查输入设备和系统麦克风权限")
                }
                Ok(()) => {
                    self.status = "已停止，可继续录制或完成导出".into();
                    self.error = false;
                }
            }
        }
    }

    fn destination(&self) -> Option<(PathBuf, bool)> {
        let stem = self
            .script
            .as_ref()
            .and_then(|(p, _)| p.file_stem())
            .and_then(|s| s.to_str())
            .unwrap_or("播客");
        let mut path = rfd::FileDialog::new()
            .set_title("导出录音")
            .add_filter("WAV 音频", &["wav"])
            .set_file_name(format!("{stem}-录音.wav"))
            .save_file()?;
        let overwrite = path.extension().is_some() && path.exists();
        if path.extension().is_none() {
            path.set_extension("wav");
        }
        Some((path, overwrite))
    }

    // A close or manuscript replacement needs a definite outcome before proceeding.
    pub fn confirm_leave(&mut self) -> bool {
        self.stop();
        if !self.has_audio() {
            return true;
        }
        match rfd::MessageDialog::new()
            .set_title("导出本次录音？")
            .set_description(
                "本次录音尚未导出。选择“是”保存 WAV，选择“否”放弃录音，或取消以保留本次录音。",
            )
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show()
        {
            rfd::MessageDialogResult::Yes => {
                let Some((path, overwrite)) = self.destination() else {
                    return false;
                };
                if let Err(e) = self.draft.as_ref().unwrap().export(&path, overwrite) {
                    self.problem(&e);
                    rfd::MessageDialog::new()
                        .set_title("录音导出失败")
                        .set_description(&self.status)
                        .show();
                    return false;
                }
            }
            rfd::MessageDialogResult::No => {}
            _ => return false,
        }
        self.draft = None;
        true
    }

    fn finish(
        &mut self,
        ctx: &Context,
        destination: impl FnOnce(&Self) -> Option<(PathBuf, bool)>,
    ) {
        self.stop();
        if !self.has_audio() {
            return;
        }
        let Some((path, overwrite)) = destination(self) else {
            return;
        };
        let draft = self.draft.as_ref().unwrap().clone();
        let (tx, rx) = mpsc::channel();
        self.export = Some(rx);
        self.status = "正在导出 WAV…".into();
        self.error = false;
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = draft.export(&path, overwrite).map(|()| path);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    pub fn poll(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(rx) = &self.permission {
            match rx.try_recv() {
                Ok(true) => {
                    self.permission = None;
                    self.start();
                }
                Err(mpsc::TryRecvError::Empty) => {}
                _ => {
                    self.permission = None;
                    self.problem(permission::DENIED);
                }
            }
        }
        if let Some(rx) = &self.export {
            let result = match rx.try_recv() {
                Ok(r) => r,
                Err(mpsc::TryRecvError::Empty) => return,
                Err(mpsc::TryRecvError::Disconnected) => Err(anyhow!("录音导出意外中断")),
            };
            self.export = None;
            match result {
                Ok(path) => {
                    self.draft = None;
                    self.status = format!("录音已导出到 {}", path.display());
                    self.error = false;
                }
                Err(e) => self.problem(e),
            }
        }
        if let Some(capture) = &self.capture {
            let error = capture.error();
            if error.is_some() || capture.interrupted() {
                self.stop();
                if let Some(e) = error {
                    self.problem(e);
                }
            }
        }
    }

    pub fn show(&mut self, ctx: &Context) {
        let busy = self.exporting() || self.permission.is_some();
        let mut toggle = record_shortcut(ctx);
        let mut finish = false;
        let step = ctx.input_mut(|i| {
            i.count_and_consume_key(Modifiers::NONE, Key::ArrowDown) as f32
                - i.count_and_consume_key(Modifiers::NONE, Key::ArrowUp) as f32
        });
        let live = self.capture.is_some();
        if live || self.permission.is_some() {
            ctx.request_repaint_after(Duration::from_millis(75));
        }
        egui::TopBottomPanel::bottom("recording-controls")
            .frame(panel(PANEL, 16))
            .show(ctx, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let label = if live {
                        "停止录制"
                    } else if self.has_audio() {
                        "继续录制"
                    } else {
                        "开始录制"
                    };
                    toggle |= ui
                        .add_enabled(
                            !busy,
                            egui::Button::new(
                                RichText::new(format!("{label}  R")).strong().color(PLOT),
                            )
                            .fill(if live { CUT } else { ACCENT })
                            .min_size(egui::vec2(125., 38.)),
                        )
                        .clicked();
                    finish = ui
                        .add_enabled(
                            !busy && (live || self.has_audio()),
                            egui::Button::new("完成并导出…").min_size(egui::vec2(110., 38.)),
                        )
                        .clicked();
                    let elapsed = self
                        .draft
                        .as_ref()
                        .map(|d| clock(d.frames(), d.rate))
                        .unwrap_or_else(|| "00:00.000".into());
                    ui.label(RichText::new(elapsed).monospace().size(19.));
                    if let Some(capture) = &self.capture {
                        ui.label(RichText::new("录制中").color(CUT));
                        ui.add(egui::ProgressBar::new(capture.level().sqrt()).desired_width(72.))
                            .on_hover_text("麦克风输入电平");
                        hint(ui, &capture.device_name);
                    } else if busy {
                        ui.spinner();
                    }
                });
                if !self.status.is_empty() {
                    ui.colored_label(if self.error { CUT } else { MUTED }, &self.status);
                }
            });
        egui::CentralPanel::default()
            .frame(panel(BASE, 22))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    hint(ui, "↑ ↓  滚动讲稿");
                    if let Some((path, _)) = &self.script {
                        ui.separator();
                        ui.add(
                            egui::Label::new(
                                path.file_name().unwrap_or_default().to_string_lossy(),
                            )
                            .truncate(),
                        );
                    }
                });
                ui.separator();
                let Some((_, job)) = &self.script else {
                    hint(
                        ui,
                        "可直接录音，也可在讲稿编辑页点击「用此稿录音」载入讲稿。",
                    );
                    return;
                };
                let mut scroll = egui::ScrollArea::vertical()
                    .id_salt("recording-manuscript")
                    .auto_shrink([false, false]);
                if self.reset_scroll || step != 0. {
                    let from = if self.reset_scroll { 0. } else { self.offset };
                    scroll = scroll.vertical_scroll_offset((from + step * 72.).max(0.));
                    self.reset_scroll = false;
                }
                self.offset = scroll
                    .show(ui, |ui| {
                        ui.add(egui::Label::new(job.clone()).wrap().selectable(false));
                    })
                    .state
                    .offset
                    .y;
            });
        if !busy {
            if finish {
                self.finish(ctx, Self::destination);
            } else if toggle {
                self.toggle();
            }
        }
    }
}

fn record_shortcut(ctx: &Context) -> bool {
    ctx.input_mut(|i| {
        let mut toggle = false;
        i.events.retain(|e| {
            if let Event::Key {
                key: Key::R,
                pressed: true,
                repeat,
                modifiers,
                ..
            } = e
                && modifiers.is_none()
            {
                toggle |= !repeat;
                return false;
            }
            true
        });
        toggle
    })
}
