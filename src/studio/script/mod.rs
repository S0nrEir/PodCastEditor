mod document;
mod hint_label;
mod tags;
#[cfg(test)]
mod tests;
mod view;

use super::{ACCENT, BASE, CUT, MUTED, PANEL, PLOT, hint, panel, title};
use anyhow::{Context as _, Result, bail};
use document::{Document, Snapshot, atomic_write};
use eframe::egui::{
    self, Context, Id, Key, Modifiers, RichText, TextEdit, Ui,
    text::{CCursor, CCursorRange},
};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};
use tags::{DEFAULTS, color, valid_name};
use view::{Buffer, View};

const DIGITS: [Key; 9] = [
    Key::Num1,
    Key::Num2,
    Key::Num3,
    Key::Num4,
    Key::Num5,
    Key::Num6,
    Key::Num7,
    Key::Num8,
    Key::Num9,
];

pub fn is_script(path: &Path) -> bool {
    path.extension()
        .is_some_and(|ext| ext.eq_ignore_ascii_case("md") || ext.eq_ignore_ascii_case("markdown"))
}

fn config_path() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(|home| PathBuf::from(home).join("Library/Application Support"))
        .context("找不到用户配置目录")?;
    #[cfg(not(target_os = "macos"))]
    let root = std::env::var_os("APPDATA")
        .or_else(|| std::env::var_os("XDG_CONFIG_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|p| PathBuf::from(p).join(".config")))
        .context("找不到用户配置目录")?;
    Ok(root.join("PodCastEditor").join("script-tags.txt"))
}

fn editor_id() -> Id {
    Id::new("script-content")
}

pub struct Script {
    doc: Option<Document>,
    names: Vec<String>,
    raw: bool,
    manage: bool,
    picker: bool,
    picker_index: usize,
    new_name: String,
    message: String,
    error: bool,
    generation: u64,
    keyboard_hint: bool,
}

impl Default for Script {
    fn default() -> Self {
        let mut names: Vec<_> = DEFAULTS.iter().map(|s| s.to_string()).collect();
        let load = config_path().and_then(|p| match fs::read_to_string(&p) {
            Ok(text) => {
                let mut saved = Vec::new();
                for s in text.lines() {
                    if !valid_name(s) || saved.iter().any(|n| n == s) {
                        bail!("标签配置包含无效或重复的名称");
                    }
                    saved.push(s.to_owned());
                }
                names = saved;
                Ok(())
            }
            Err(e) if e.kind() == ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        });
        Self {
            doc: None,
            names,
            raw: false,
            manage: false,
            picker: false,
            picker_index: 0,
            new_name: String::new(),
            error: load.is_err(),
            message: load.err().map_or_else(String::new, |e| e.to_string()),
            generation: 0,
            keyboard_hint: false,
        }
    }
}

impl Script {
    fn result(&mut self, result: Result<()>, success: &str) {
        self.error = result.is_err();
        self.message = result.map_or_else(|e| format!("{e:#}"), |()| success.into());
    }

    fn save(&mut self) -> bool {
        let Some(doc) = &mut self.doc else {
            return true;
        };
        let result = doc.save();
        self.result(result, "讲稿已保存");
        !self.error
    }

    pub fn confirm_leave(&mut self) -> bool {
        let Some(doc) = &self.doc else { return true };
        if !doc.dirty() {
            return true;
        }
        match rfd::MessageDialog::new()
            .set_title("保存讲稿更改？")
            .set_description(format!(
                "“{}”有未保存的更改。是否保存？",
                doc.path.file_name().unwrap_or_default().to_string_lossy()
            ))
            .set_buttons(rfd::MessageButtons::YesNoCancel)
            .show()
        {
            rfd::MessageDialogResult::Yes => self.save(),
            rfd::MessageDialogResult::No => true,
            _ => false,
        }
    }

    pub fn open(&mut self, ctx: &Context, path: PathBuf) {
        if !self.confirm_leave() {
            return;
        }
        match Document::open(path, &self.names, self.raw) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.generation += 1;
                self.keyboard_hint = false;
                self.picker = false;
                self.message.clear();
                self.error = false;
                egui::text_edit::TextEditState::default().store(ctx, editor_id());
                self.restore_selection(ctx, true);
            }
            Err(e) => self.result(Err(e), ""),
        }
    }

    fn pick(&mut self, ctx: &Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("导入 Markdown 讲稿")
            .add_filter("Markdown 讲稿", &["md", "markdown"])
            .pick_file()
        {
            self.open(ctx, path);
        }
    }

    pub fn reading_copy(&self) -> Option<(PathBuf, egui::text::LayoutJob)> {
        let doc = self.doc.as_ref()?;
        let job = if self.raw {
            View::new(&doc.source, &self.names, false)
                .job
                .borrow()
                .clone()
        } else {
            doc.view.job.borrow().clone()
        };
        Some((doc.path.clone(), job))
    }

    pub fn header(&mut self, ui: &mut Ui) -> bool {
        let record = ui
            .add_enabled(self.doc.is_some(), egui::Button::new("用此稿录音"))
            .clicked();
        if ui
            .add_enabled(self.doc.is_some(), egui::Button::new("保存"))
            .on_hover_text(shortcut_name(ui.ctx(), Key::S))
            .clicked()
        {
            self.save();
        }
        if ui.button("导入讲稿…").clicked() {
            self.pick(ui.ctx());
        }
        record
    }

    fn restore_selection(&self, ctx: &Context, focus: bool) {
        if let Some(doc) = &self.doc {
            let mut state = TextEdit::load_state(ctx, editor_id()).unwrap_or_default();
            let a = doc.view.cursor(doc.selected.start);
            let b = doc.view.cursor(doc.selected.end);
            state
                .cursor
                .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
            state.store(ctx, editor_id());
            if focus {
                ctx.memory_mut(|m| m.request_focus(editor_id()));
            }
            ctx.request_repaint();
        }
    }

    fn rebuild(&mut self, ctx: &Context, focus: bool) {
        if let Some(doc) = &mut self.doc {
            doc.view = View::new(&doc.source, &self.names, self.raw);
            self.restore_selection(ctx, focus);
        }
    }

    fn apply(&mut self, ctx: &Context, index: usize) {
        if let Some(doc) = &mut self.doc {
            let name = &self.names[index];
            let result = doc.wrap(name);
            self.result(result, &format!("已添加「{name}」"));
            self.rebuild(ctx, true);
        }
    }

    fn remove_mark(&mut self, ctx: &Context) {
        if let Some(doc) = &mut self.doc {
            doc.unwrap();
            self.result(Ok(()), "已移除选区标签");
            self.rebuild(ctx, true);
        }
    }

    fn set_names(&mut self, ctx: &Context, names: Vec<String>) {
        let result = config_path().and_then(|path| {
            fs::create_dir_all(path.parent().unwrap())?;
            atomic_write(&path, names.join("\n").as_bytes(), false)
        });
        if result.is_ok() {
            self.names = names;
            self.new_name.clear();
            self.rebuild(ctx, false);
        }
        self.result(result, "标签列表已保存");
    }

    fn add_name(&mut self, ctx: &Context) {
        let name = self.new_name.trim();
        if !valid_name(name) {
            self.result(
                Err(anyhow::anyhow!(
                    "名称须以文字开头，可包含文字、数字、下划线或短横线"
                )),
                "",
            );
        } else if self.names.iter().any(|s| s == name) {
            self.result(Err(anyhow::anyhow!("已有同名标签")), "");
        } else {
            let mut names = self.names.clone();
            names.push(name.into());
            self.set_names(ctx, names);
        }
    }

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            title(ui, "讲稿标签");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = if self.manage { "完成" } else { "管理" };
                ui.toggle_value(&mut self.manage, label);
            });
        });
        ui.add_space(5.);
        if self.manage {
            ui.horizontal(|ui| {
                let input = ui.add(
                    TextEdit::singleline(&mut self.new_name)
                        .desired_width(128.)
                        .hint_text("新标签名称"),
                );
                if ui.button("添加").clicked()
                    || (input.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)))
                {
                    self.add_name(ui.ctx());
                }
            });
            hint(ui, "删除列表项会保留稿内已有标记");
        } else {
            hint(ui, "选中文字后，点击标签或使用右键");
        }
        ui.separator();
        let ready = self.doc.as_ref().is_some_and(|d| !d.selected.is_empty());
        let mut apply = None;
        let mut remove = None;
        egui::ScrollArea::vertical()
            .id_salt("script-tag-list")
            .show(ui, |ui| {
                for (i, name) in self.names.iter().enumerate() {
                    ui.horizontal(|ui| {
                        let (r, _) =
                            ui.allocate_exact_size(egui::vec2(9., 25.), egui::Sense::hover());
                        ui.painter().circle_filled(r.center(), 4., color(i));
                        if ui
                            .add_enabled(
                                ready,
                                egui::Button::new(RichText::new(name).color(color(i))).frame(false),
                            )
                            .clicked()
                        {
                            apply = Some(i);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if self.manage {
                                if ui.small_button("删除").clicked() {
                                    remove = Some(i);
                                }
                            } else if i < DIGITS.len() {
                                hint(ui, &shortcut_name(ui.ctx(), DIGITS[i]));
                            }
                        });
                    });
                }
                if !self.manage {
                    ui.separator();
                    if ui
                        .add_enabled(ready, egui::Button::new("选择标签…"))
                        .on_hover_text(shortcut_name(ui.ctx(), Key::T))
                        .clicked()
                    {
                        self.picker = true;
                        self.picker_index = 0;
                    }
                    if ui
                        .add_enabled(
                            self.doc.as_ref().and_then(|d| d.current_mark()).is_some(),
                            egui::Button::new("移除此处标记"),
                        )
                        .clicked()
                    {
                        self.remove_mark(ui.ctx());
                    }
                }
            });
        if let Some(i) = apply {
            self.apply(ui.ctx(), i);
        }
        if let Some(i) = remove {
            let mut names = self.names.clone();
            names.remove(i);
            self.set_names(ui.ctx(), names);
        }
    }

    fn shortcuts(&mut self, ctx: &Context) {
        if self.picker || !ctx.memory(|m| m.has_focus(editor_id())) {
            return;
        }
        for (n, &key) in DIGITS.iter().take(self.names.len()).enumerate() {
            if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, key)) {
                self.apply(ctx, n);
                return;
            }
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::T)) {
            self.picker = self.doc.as_ref().is_some_and(|d| !d.selected.is_empty());
            self.picker_index = 0;
        }
        let redo = ctx.input_mut(|i| {
            i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z)
                || i.consume_key(Modifiers::COMMAND, Key::Y)
        });
        if (redo || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)))
            && let Some(doc) = &mut self.doc
        {
            doc.undo(redo);
            self.rebuild(ctx, true);
        }
    }

    fn editor(&mut self, ui: &mut Ui) {
        let Some(doc) = &mut self.doc else {
            ui.add_space(ui.available_height() * 0.29);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("导入播客讲稿").size(29.).strong());
                hint(ui, "将 Markdown 讲稿拖到这里，或从本机选择文件");
                ui.add_space(10.);
                if ui
                    .add(
                        egui::Button::new(RichText::new("选择 Markdown 文件…").color(PLOT))
                            .fill(ACCENT),
                    )
                    .clicked()
                {
                    self.pick(ui.ctx());
                }
            });
            return;
        };
        let mut toggled = false;
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                toggled = ui.checkbox(&mut self.raw, "源码模式").changed();
                ui.separator();
                ui.add(
                    egui::Label::new(
                        RichText::new(doc.path.file_name().unwrap_or_default().to_string_lossy())
                            .color(MUTED),
                    )
                    .truncate(),
                )
                .on_hover_text(doc.path.display().to_string());
            });
        });
        ui.separator();
        if toggled {
            self.rebuild(ui.ctx(), true);
        }
        let doc = self.doc.as_mut().unwrap();
        let layout = doc.view.job.clone();
        let mut layouter = |ui: &Ui, text: &str, width: f32| {
            let mut job = layout.borrow().clone();
            debug_assert_eq!(job.text, text);
            job.wrap.max_width = width;
            ui.fonts(|f| f.layout_job(job))
        };
        let old_selection = doc.selected.clone();
        let mut buffer = Buffer::new(&mut doc.view, &mut doc.source, doc.newline);
        let mut apply = None;
        let mut remove_mark = false;
        let mut updated = false;
        egui::ScrollArea::vertical()
            .id_salt(("script-pages", self.generation))
            .auto_shrink([false, false])
            .show(ui, |ui| {
                let mut out = TextEdit::multiline(&mut buffer)
                    .id(editor_id())
                    .frame(false)
                    .cursor_at_end(false)
                    .desired_width(f32::INFINITY)
                    .desired_rows(12)
                    .layouter(&mut layouter)
                    .show(ui);
                if toggled && let Some(cursors) = out.state.cursor.range(&out.galley) {
                    let caret = out
                        .galley
                        .pos_from_cursor(&cursors.primary)
                        .translate(out.galley_pos.to_vec2());
                    ui.scroll_to_rect(caret, None);
                }
                // TextEdit moves the caret on any mouse-button press. Keep the selection
                // anchored while a secondary click opens its tag menu.
                if !old_selection.is_empty()
                    && out.response.hovered()
                    && ui.input(|i| {
                        i.pointer.secondary_down()
                            || i.pointer.secondary_pressed()
                            || i.pointer.secondary_released()
                    })
                {
                    let a = buffer.view.cursor(old_selection.start);
                    let b = buffer.view.cursor(old_selection.end);
                    out.state
                        .cursor
                        .set_char_range(Some(CCursorRange::two(CCursor::new(a), CCursor::new(b))));
                    out.state.clone().store(ui.ctx(), editor_id());
                }
                if let Some(cursors) = out.state.cursor.char_range() {
                    let a = cursors.primary.index.min(cursors.secondary.index);
                    let b = cursors.primary.index.max(cursors.secondary.index);
                    let pos = buffer.view.source_pos(a);
                    doc.selected = buffer.view.selection(a..b).unwrap_or(pos..pos);
                }
                if !doc.selected.is_empty() {
                    out.response.context_menu(|ui| {
                        egui::ScrollArea::vertical()
                            .max_height(340.)
                            .show(ui, |ui| {
                                for (i, name) in self.names.iter().enumerate() {
                                    if ui.button(RichText::new(name).color(color(i))).clicked() {
                                        apply = Some(i);
                                        ui.close_menu();
                                    }
                                }
                                if buffer.view.marks.iter().any(|m| {
                                    doc.selected.start >= m.outer.start
                                        && doc.selected.end <= m.outer.end
                                }) {
                                    ui.separator();
                                    if ui.button("移除此处标记").clicked() {
                                        remove_mark = true;
                                        ui.close_menu();
                                    }
                                }
                            });
                    });
                }
                updated = out.response.changed();
                if !self.picker && !updated && !out.response.context_menu_opened() {
                    hint_label::show(ui, &out, buffer.view, &mut self.keyboard_hint);
                }
            });
        if let Some(source) = buffer.before {
            doc.record(Snapshot {
                source,
                selection: old_selection,
            });
            self.message.clear();
            self.error = false;
            self.rebuild(ui.ctx(), false);
        } else if updated {
            ui.ctx().request_repaint();
        }
        if let Some(i) = apply {
            self.apply(ui.ctx(), i);
        }
        if remove_mark {
            self.remove_mark(ui.ctx());
        }
    }

    pub fn show(&mut self, ctx: &Context) {
        let save = ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::S));
        self.shortcuts(ctx);
        if self.picker {
            self.tag_picker(ctx);
        }
        egui::TopBottomPanel::bottom("script-status")
            .frame(panel(PLOT, 8))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if let Some(doc) = &self.doc {
                        ui.label(
                            RichText::new(if doc.dirty() {
                                "● 未保存"
                            } else {
                                "已保存"
                            })
                            .color(if doc.dirty() {
                                CUT
                            } else {
                                MUTED
                            }),
                        );
                        ui.separator();
                    }
                    ui.label(RichText::new(&self.message).color(if self.error {
                        CUT
                    } else {
                        MUTED
                    }));
                });
            });
        egui::SidePanel::right("script-tags")
            .resizable(false)
            .exact_width(222.)
            .frame(panel(PANEL, 14))
            .show(ctx, |ui| self.sidebar(ui));
        egui::CentralPanel::default()
            .frame(panel(BASE, 20))
            .show(ctx, |ui| self.editor(ui));
        // Commit text and IME events delivered in the same frame before writing.
        if save {
            self.save();
            ctx.request_repaint();
        }
    }

    fn tag_picker(&mut self, ctx: &Context) {
        let moved = ctx.input_mut(|i| {
            if i.consume_key(Modifiers::NONE, Key::ArrowUp) {
                self.picker_index = self.picker_index.saturating_sub(1);
                true
            } else if i.consume_key(Modifiers::NONE, Key::ArrowDown) {
                self.picker_index = (self.picker_index + 1).min(self.names.len().saturating_sub(1));
                true
            } else {
                false
            }
        });
        let mut chosen = None;
        let modal = egui::Modal::new(Id::new("script-tag-picker")).show(ctx, |ui| {
            title(ui, "选择标签");
            hint(ui, "上下键选择，回车添加");
            egui::ScrollArea::vertical()
                .max_height(350.)
                .show(ui, |ui| {
                    for (i, name) in self.names.iter().enumerate() {
                        let active = i == self.picker_index;
                        let label =
                            ui.selectable_label(active, RichText::new(name).color(color(i)));
                        if moved && active {
                            label.scroll_to_me(None);
                        }
                        if label.clicked() {
                            chosen = Some(i);
                        }
                    }
                });
        });
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Enter)) && !self.names.is_empty() {
            chosen = Some(self.picker_index);
        }
        if let Some(i) = chosen {
            self.picker = false;
            self.apply(ctx, i);
        } else if modal.should_close() {
            self.picker = false;
            self.restore_selection(ctx, true);
        }
    }
}

fn shortcut_name(ctx: &Context, key: Key) -> String {
    ctx.format_shortcut(&egui::KeyboardShortcut::new(Modifiers::COMMAND, key))
}
