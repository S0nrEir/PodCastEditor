use super::{
    tags::{Mark, escape_value},
    view::View,
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::Write,
    ops::Range,
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;

pub fn atomic_write(path: &Path, body: &[u8], bom: bool) -> Result<()> {
    let mut file = NamedTempFile::new_in(
        path.parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new(".")),
    )?;
    if bom {
        file.write_all(b"\xef\xbb\xbf")?;
    }
    file.write_all(body)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

#[derive(Clone)]
pub struct Snapshot {
    pub source: String,
    pub selection: Range<usize>,
}

pub struct Document {
    pub path: PathBuf,
    pub source: String,
    saved: String,
    bom: bool,
    pub newline: &'static str,
    pub view: View,
    pub selected: Range<usize>, // byte offsets into the Markdown source
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    preedit: Option<Snapshot>,
}

impl Document {
    pub fn open(path: PathBuf, names: &[String], raw: bool) -> Result<Self> {
        let text = fs::read_to_string(&path)
            .with_context(|| format!("无法读取 UTF-8 讲稿 {}", path.display()))?;
        let bom = text.starts_with('\u{feff}');
        let source = text.strip_prefix('\u{feff}').unwrap_or(&text).to_owned();
        let newline = if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        };
        let view = View::new(&source, names, raw);
        Ok(Self {
            path,
            saved: source.clone(),
            source,
            bom,
            newline,
            view,
            selected: 0..0,
            undo: vec![],
            redo: vec![],
            preedit: None,
        })
    }

    pub fn dirty(&self) -> bool {
        self.saved != self.source
    }
    pub fn save(&mut self) -> Result<()> {
        atomic_write(&self.path, self.source.as_bytes(), self.bom)
            .with_context(|| format!("无法保存 {}", self.path.display()))?;
        self.saved.clone_from(&self.source);
        Ok(())
    }

    fn snapshot(&self) -> Snapshot {
        Snapshot {
            source: self.source.clone(),
            selection: self.selected.clone(),
        }
    }
    fn record(&mut self, old: Snapshot) {
        if self.source == old.source {
            return;
        }
        self.redo.clear();
        if self.undo.len() == 100 {
            self.undo.remove(0);
        }
        self.undo.push(old);
    }
    pub fn record_text(&mut self, old: Snapshot, preediting: bool) {
        let start = self.preedit.take().unwrap_or(old);
        if preediting {
            self.preedit = Some(start);
        } else {
            self.record(start);
        }
    }
    pub fn finish_preedit(&mut self) {
        if let Some(start) = self.preedit.take() {
            self.record(start);
        }
    }
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
            || self
                .preedit
                .as_ref()
                .is_some_and(|s| s.source != self.source)
    }
    pub fn undo(&mut self, redo: bool) -> bool {
        self.finish_preedit();
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        let Some(state) = from.pop() else {
            return false;
        };
        to.push(Snapshot {
            source: std::mem::replace(&mut self.source, state.source),
            selection: std::mem::replace(&mut self.selected, state.selection),
        });
        true
    }

    pub fn current_mark(&self) -> Option<&Mark> {
        self.view
            .marks
            .iter()
            .rev()
            .find(|m| self.selected.start >= m.outer.start && self.selected.end <= m.outer.end)
    }

    pub fn unwrap(&mut self) {
        let Some(mark) = self.current_mark().cloned() else {
            return;
        };
        self.finish_preedit();
        let old = self.snapshot();
        let body = &self.source[mark.body];
        let text = if mark.attribute {
            decode(body)
        } else {
            body.to_owned()
        };
        self.selected = mark.outer.start..mark.outer.start + text.len();
        self.source.replace_range(mark.outer, &text);
        self.record(old);
    }

    pub fn wrap(&mut self, name: &str) -> Result<()> {
        let range = self.selected.clone();
        if range.is_empty() {
            bail!("请先选中要标记的文字");
        }
        self.finish_preedit();
        let old = self.snapshot();
        // Split at existing delimiters. Even a selection crossing one side of an annotation
        // remains well nested, and attribute values are never written as markup.
        let mut delimiters = Vec::new();
        for m in &self.view.marks {
            if m.attribute {
                delimiters.push((m.outer.clone(), Some(m.body.clone())));
            } else {
                delimiters.push((m.outer.start..m.body.start, None));
                delimiters.push((m.body.end..m.outer.end, None));
            }
        }
        delimiters.sort_by_key(|(r, _)| r.start);
        let mut edits = Vec::new();
        let mut pos = range.start;
        let tagged = |s: &str| {
            if name == "补充" {
                format!("<补充=\"{}\">", escape_value(s))
            } else {
                format!("<{name}>{s}</{name}>")
            }
        };
        for (token, body) in delimiters {
            if token.end <= range.start || token.start >= range.end {
                continue;
            }
            if pos < token.start {
                edits.push((pos..token.start, tagged(&self.source[pos..token.start])));
            }
            if let Some(body) = body {
                let a = range.start.max(body.start);
                let b = range.end.min(body.end);
                if a < b && name != "补充" {
                    let mut replacement = String::new();
                    for (part, middle) in
                        [(body.start..a, false), (a..b, true), (b..body.end, false)]
                    {
                        if !part.is_empty() {
                            let attr = format!("<补充=\"{}\">", &self.source[part]);
                            replacement.push_str(&if middle { tagged(&attr) } else { attr });
                        }
                    }
                    edits.push((token.clone(), replacement));
                }
            }
            pos = pos.max(token.end);
        }
        if pos < range.end {
            edits.push((pos..range.end, tagged(&self.source[pos..range.end])));
        }
        if edits.is_empty() {
            bail!("选区中没有可添加此标签的正文");
        }
        // Keep the payload selected for the common single-wrap case.
        let start = edits[0].0.start;
        let end = edits.last().unwrap().0.end;
        let mut new_end = end;
        for (r, value) in edits.into_iter().rev() {
            new_end = new_end - r.len() + value.len();
            self.source.replace_range(r, &value);
        }
        let prefix = format!("<{name}{}", if name == "补充" { "=\"" } else { ">" });
        let suffix = if name == "补充" {
            "\">".into()
        } else {
            format!("</{name}>")
        };
        self.selected = if self.source[start..new_end].starts_with(&prefix)
            && self.source[start..new_end].ends_with(&suffix)
        {
            start + prefix.len()..new_end - suffix.len()
        } else {
            start..new_end
        };
        self.record(old);
        Ok(())
    }
}

fn decode(s: &str) -> String {
    s.replace("&quot;", "\"")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}
