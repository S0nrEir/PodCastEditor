use super::tags::{Mark, color, entity, escape_value, marks};
use eframe::egui::{Color32, FontFamily, FontId, Stroke, TextBuffer, TextFormat, text::LayoutJob};
use pulldown_cmark::{Event, Options, Parser, Tag};
use std::{cell::RefCell, ops::Range, rc::Rc};

#[derive(Clone, Copy, Default, PartialEq)]
struct Style {
    tag: Option<usize>,
    heading: u8,
    bold: bool,
    italic: bool,
    strike: bool,
    quote: bool,
    code: bool,
    link: bool,
}

impl Style {
    fn format(self, raw: bool) -> TextFormat {
        let ink = self.tag.map(color).unwrap_or_else(|| {
            if self.link {
                Color32::from_rgb(140, 200, 255)
            } else if self.quote {
                super::super::MUTED
            } else {
                Color32::from_rgb(231, 237, 240)
            }
        });
        let size = match self.heading {
            1 => 26.,
            2 => 23.,
            3.. => 19.,
            _ if raw => 15.,
            _ => 17.,
        };
        let family = if raw || self.code {
            FontFamily::Monospace
        } else if self.bold || self.heading > 0 {
            FontFamily::Name("script-bold".into())
        } else {
            FontFamily::Proportional
        };
        TextFormat {
            font_id: FontId::new(size, family),
            color: ink,
            italics: self.italic,
            strikethrough: if self.strike {
                Stroke::new(1.0_f32, ink)
            } else {
                Stroke::NONE
            },
            underline: if self.link {
                Stroke::new(1.0_f32, ink)
            } else {
                Stroke::NONE
            },
            background: if self.code {
                Color32::from_rgb(36, 52, 62)
            } else {
                Color32::TRANSPARENT
            },
            ..Default::default()
        }
    }
}

// One selectable character can represent several source bytes (CJK, CRLF or an entity).
#[derive(Clone)]
struct Glyph {
    ch: char,
    raw: Range<usize>,
    style: Style,
    visible: bool,
    attribute: bool,
}

fn indices(offsets: &[usize], r: Range<usize>) -> Range<usize> {
    offsets.partition_point(|&p| p < r.start)..offsets.partition_point(|&p| p < r.end)
}

fn show(cells: &mut [Glyph]) {
    for c in cells {
        c.visible = true;
    }
}

fn markdown(cells: &mut Vec<Glyph>, groups: &mut Vec<Range<usize>>) {
    let clean: String = cells.iter().map(|c| c.ch).collect();
    let offsets: Vec<_> = clean.char_indices().map(|(p, _)| p).collect();
    // Chinese prose often puts emphasis flush against both a word and a quotation
    // mark. Classify the quotation mark as a letter for delimiter matching only.
    // These replacements all have the same UTF-8 width. Display and editing use clean.
    let mut parse_text = clean.clone();
    for (i, c) in cells.iter().enumerate() {
        if "“”‘’《》「」『』【】（）".contains(c.ch)
            && (i.checked_sub(1).is_some_and(|j| cells[j].ch == '*')
                || cells.get(i + 1).is_some_and(|next| next.ch == '*'))
        {
            parse_text.replace_range(offsets[i]..offsets[i] + c.ch.len_utf8(), "字");
        }
    }
    for cell in cells.iter_mut() {
        cell.visible = cell.ch == '\n';
    }
    let options =
        Options::ENABLE_STRIKETHROUGH | Options::ENABLE_TASKLISTS | Options::ENABLE_TABLES;
    for (event, raw) in Parser::new_ext(&parse_text, options).into_offset_iter() {
        let slice = &mut cells[indices(&offsets, raw.clone())];
        if slice.is_empty() {
            continue;
        }
        match event {
            Event::Start(tag) => {
                groups.push(slice[0].raw.start..slice.last().unwrap().raw.end);
                match tag {
                    Tag::Item => {
                        let marker = clean[raw.clone()].trim_start_matches(' ');
                        let len = marker.find(char::is_whitespace).unwrap_or(0);
                        let end = clean[raw].len() - marker.len() + len;
                        let count = slice.iter().take_while(|c| c.ch != '\n').count();
                        if len > 0
                            && (marker.starts_with(['-', '*', '+'])
                                || marker.as_bytes()[0].is_ascii_digit())
                        {
                            let n = (end + 1).min(count);
                            show(&mut slice[..n]);
                            if let Some(bullet) = slice[..n]
                                .iter_mut()
                                .find(|c| matches!(c.ch, '-' | '*' | '+'))
                            {
                                bullet.ch = '•';
                            }
                        }
                    }
                    Tag::Table(_) => {
                        for c in slice {
                            if c.ch == '|' {
                                c.ch = '│';
                                c.visible = true;
                            }
                        }
                    }
                    tag => {
                        for c in slice {
                            match tag {
                                Tag::Heading { level, .. } => c.style.heading = level as u8,
                                Tag::Strong | Tag::TableHead => c.style.bold = true,
                                Tag::Emphasis => c.style.italic = true,
                                Tag::Strikethrough => c.style.strike = true,
                                Tag::BlockQuote(_) => c.style.quote = true,
                                Tag::CodeBlock(_) => c.style.code = true,
                                Tag::Link { .. } => c.style.link = true,
                                _ => break,
                            }
                        }
                    }
                }
            }
            Event::Text(value) => {
                if !slice[0].style.code
                    && value.chars().count() == 1
                    && clean[raw.clone()].starts_with('&')
                    && clean[raw].ends_with(';')
                {
                    slice[0].raw.end = slice.last().unwrap().raw.end;
                    slice[0].ch = value.chars().next().unwrap();
                    slice[0].visible = true;
                } else {
                    show(slice);
                }
            }
            Event::Html(_) | Event::InlineHtml(_) => show(slice),
            Event::Code(value) => {
                let ticks = slice.iter().take_while(|c| c.ch == '`').count();
                let end = slice.len().saturating_sub(ticks);
                if ticks <= end {
                    let body = &mut slice[ticks..end];
                    let trim = body.len() >= 2
                        && body[0].ch.is_whitespace()
                        && body.last().unwrap().ch.is_whitespace()
                        && !value.starts_with(' ');
                    let n = usize::from(trim);
                    let end = body.len() - n;
                    for c in &mut body[n..end] {
                        c.visible = true;
                        c.style.code = true;
                    }
                }
            }
            Event::Rule => {
                for c in slice {
                    if c.ch != '\n' {
                        c.ch = '─';
                        c.visible = true;
                        c.style.quote = true;
                    }
                }
            }
            Event::TaskListMarker(checked) => {
                slice[0].raw.end = slice.last().unwrap().raw.end;
                slice[0].ch = if checked { '☑' } else { '☐' };
                slice[0].visible = true;
            }
            _ => {}
        }
    }
    cells.retain(|c| c.visible);
}

#[derive(Default)]
pub struct View {
    pub text: String,
    cells: Vec<Glyph>,
    pub marks: Vec<Mark>,
    pub passages: Vec<Passage>,
    groups: Vec<Range<usize>>,
    pub job: Rc<RefCell<LayoutJob>>,
    raw: bool,
}

pub struct Passage {
    pub text: String,
    pub color: Color32,
}

impl View {
    pub fn new(source: &str, names: &[String], raw: bool) -> Self {
        let mut cells = Vec::new();
        let mut iter = source.char_indices().peekable();
        while let Some((start, mut ch)) = iter.next() {
            if ch == '\r' && iter.next_if(|&(_, next)| next == '\n').is_some() {
                ch = '\n';
            }
            let end = iter.peek().map_or(source.len(), |&(p, _)| p);
            cells.push(Glyph {
                ch,
                raw: start..end,
                style: Style::default(),
                visible: true,
                attribute: false,
            });
        }
        let offsets: Vec<_> = cells.iter().map(|c| c.raw.start).collect();
        let marks = marks(source, names);
        let mut groups: Vec<_> = marks.iter().map(|m| m.outer.clone()).collect();
        let mut palette = names.to_vec();
        let mut colors = Vec::with_capacity(marks.len());
        for mark in &marks {
            let index = palette
                .iter()
                .position(|n| n == &mark.name)
                .unwrap_or_else(|| {
                    palette.push(mark.name.clone());
                    palette.len() - 1
                });
            for c in &mut cells[indices(&offsets, mark.outer.clone())] {
                c.style.tag = Some(index);
            }
            colors.push(color(index));
        }
        // The outline always uses the rendered wording, including in source mode.
        let mut passages = Vec::new();
        if !raw || !marks.is_empty() {
            let mut rendered = if raw {
                cells.clone()
            } else {
                std::mem::take(&mut cells)
            };
            for mark in &marks {
                for range in [
                    mark.outer.start..mark.body.start,
                    mark.body.end..mark.outer.end,
                ] {
                    for c in &mut rendered[indices(&offsets, range)] {
                        c.visible = false;
                    }
                }
                if !mark.attribute {
                    continue;
                }
                let mut skip_to = 0;
                for c in &mut rendered[indices(&offsets, mark.body.clone())] {
                    c.attribute = true;
                    if c.raw.start < skip_to {
                        c.visible = false;
                    } else if c.ch == '&'
                        && let Some((encoded, ch)) = entity(&source[c.raw.start..mark.body.end])
                    {
                        skip_to = c.raw.start + encoded.len();
                        c.raw.end = skip_to;
                        c.ch = ch;
                    }
                }
            }
            rendered.retain(|c| c.visible);
            let mut rendered_groups = Vec::new();
            markdown(&mut rendered, &mut rendered_groups);
            for (mark, color) in marks.iter().zip(colors) {
                let first = rendered.partition_point(|c| c.raw.start < mark.body.start);
                let end = rendered.partition_point(|c| c.raw.start < mark.body.end);
                let body: String = rendered[first..end].iter().map(|c| c.ch).collect();
                let text = body.split_whitespace().collect::<Vec<_>>().join(" ");
                passages.push(Passage { text, color });
            }
            if !raw {
                cells = rendered;
                groups.extend(rendered_groups);
            }
        }
        let mut view = Self {
            text: cells.iter().map(|c| c.ch).collect(),
            cells,
            marks,
            passages,
            groups,
            job: Default::default(),
            raw,
        };
        view.layout();
        view
    }

    fn layout(&mut self) {
        let mut job = LayoutJob::default();
        for run in self.cells.chunk_by(|a, b| a.style == b.style) {
            let segment: String = run.iter().map(|c| c.ch).collect();
            job.append(&segment, 0., run[0].style.format(self.raw));
        }
        *self.job.borrow_mut() = job;
    }

    pub fn source_pos(&self, cursor: usize) -> usize {
        let before = cursor.checked_sub(1).and_then(|i| self.cells.get(i));
        let after = self.cells.get(cursor);
        match (before, after) {
            (Some(a), Some(b))
                if b.ch == '\n'
                    || (a.style.tag.is_some() && b.style.tag.is_none() && a.ch != '\n') =>
            {
                a.raw.end
            }
            (_, Some(b)) => b.raw.start,
            (Some(a), None) => a.raw.end,
            _ => 0,
        }
    }

    pub fn cursor(&self, source_pos: usize) -> usize {
        self.cells.partition_point(|c| c.raw.end <= source_pos)
    }

    pub fn tag_label(&self, index: usize) -> Option<String> {
        let cell = self.cells.get(index)?;
        cell.style.tag?;
        let names: Vec<_> = self
            .marks
            .iter()
            .filter(|mark| mark.outer.contains(&cell.raw.start))
            .map(|mark| mark.name.as_str())
            .collect();
        (!names.is_empty()).then(|| names.join(" · "))
    }

    pub fn selection(&self, range: Range<usize>) -> Option<Range<usize>> {
        if range.is_empty() {
            return None;
        }
        Some(self.cells.get(range.start)?.raw.start..self.cells.get(range.end - 1)?.raw.end)
    }
}

// egui edits in displayed character coordinates. Only the selected source spans change.
// The projection is regenerated after the input event batch, including after an IME commit.
pub struct Buffer<'a> {
    pub view: &'a mut View,
    pub source: &'a mut String,
    pub newline: &'a str,
    pub before: Option<String>,
    insertion: Option<(usize, usize, bool)>,
}

impl<'a> Buffer<'a> {
    pub fn new(view: &'a mut View, source: &'a mut String, newline: &'a str) -> Self {
        Self {
            view,
            source,
            newline,
            before: None,
            insertion: None,
        }
    }
    fn checkpoint(&mut self) {
        self.before.get_or_insert_with(|| self.source.clone());
    }

    // Hidden delimiters can put two source boundaries at the same screen caret.
    // Keep an explicitly restored boundary (e.g. after the final closing tag).
    pub fn restore_caret(&mut self, pos: usize) {
        let index = self.view.cursor(pos);
        if self.view.source_pos(index) != pos
            && (pos == self.source.len()
                || self
                    .view
                    .marks
                    .iter()
                    .any(|m| m.outer.end == pos || m.outer.start == pos))
        {
            self.insertion = Some((index, pos, false));
        }
    }

    pub fn caret_source(&self, index: usize) -> usize {
        self.insertion
            .filter(|(i, _, _)| *i == index)
            .map_or_else(|| self.view.source_pos(index), |(_, pos, _)| pos)
    }
}

impl TextBuffer for Buffer<'_> {
    fn is_mutable(&self) -> bool {
        true
    }
    fn as_str(&self) -> &str {
        &self.view.text
    }

    fn insert_text(&mut self, s: &str, index: usize) -> usize {
        if s.is_empty() {
            return 0;
        }
        self.checkpoint();
        let pos = self.view.source_pos(index);
        let attr = self
            .view
            .cells
            .get(index)
            .is_some_and(|c| c.attribute && c.raw.start == pos)
            || index
                .checked_sub(1)
                .and_then(|i| self.view.cells.get(i))
                .is_some_and(|c| c.attribute && c.raw.end == pos);
        let (pos, attr) = self
            .insertion
            .take()
            .filter(|(i, _, _)| *i == index)
            .map_or((pos, attr), |(_, p, a)| (p, a));
        let mut encoded = String::new();
        let mut new = Vec::new();
        let style = self
            .view
            .cells
            .get(index)
            .or_else(|| self.view.cells.last())
            .map(|g| g.style)
            .unwrap_or_default();
        let s = s.replace("\r\n", "\n");
        for ch in s.chars() {
            let start = pos + encoded.len();
            if ch == '\n' {
                encoded.push_str(self.newline);
            } else if attr {
                encoded.push_str(&escape_value(&ch.to_string()));
            } else {
                encoded.push(ch);
            }
            new.push(Glyph {
                ch,
                raw: start..pos + encoded.len(),
                style,
                visible: true,
                attribute: attr,
            });
        }
        self.source.insert_str(pos, &encoded);
        for c in &mut self.view.cells[index..] {
            c.raw.start += encoded.len();
            c.raw.end += encoded.len();
        }
        for g in &mut self.view.groups {
            if g.start >= pos {
                g.start += encoded.len();
            }
            if g.end >= pos {
                g.end += encoded.len();
            }
        }
        let count = new.len();
        self.view.cells.splice(index..index, new);
        self.view.text.insert_text(&s, index);
        self.view.layout();
        count
    }

    fn delete_char_range(&mut self, range: Range<usize>) {
        let range = range.start.min(self.view.cells.len())..range.end.min(self.view.cells.len());
        let Some(bounds) = self.view.selection(range.clone()) else {
            return;
        };
        self.checkpoint();
        let mut remove: Vec<_> = self.view.cells[range.clone()]
            .iter()
            .map(|c| c.raw.clone())
            .collect();
        // Markdown and annotation spans may overlap without nesting (e.g. a mark
        // starting halfway through a bold phrase). Keep delimiters for surviving spans.
        let partial: Vec<_> = self
            .view
            .groups
            .iter()
            .filter(|g| {
                (g.start < bounds.start && g.end > bounds.start && g.end < bounds.end)
                    || (g.start > bounds.start && g.start < bounds.end && g.end > bounds.end)
            })
            .collect();
        remove.extend(
            self.view
                .groups
                .iter()
                .filter(|g| {
                    g.start >= bounds.start
                        && g.end <= bounds.end
                        && !partial.iter().any(|p| {
                            (g.start < p.start && p.start < g.end)
                                || (g.start < p.end && p.end < g.end)
                        })
                })
                .cloned(),
        );
        let all = range.len() == self.view.cells.len();
        if all {
            remove.clear();
            remove.push(0..self.source.len());
        }
        remove.sort_by_key(|r| r.start);
        let mut spans: Vec<Range<usize>> = Vec::new();
        for r in remove {
            if let Some(last) = spans.last_mut()
                && last.end >= r.start
            {
                last.end = last.end.max(r.end);
            } else {
                spans.push(r);
            }
        }
        self.insertion = Some((
            range.start,
            spans[0].start,
            !all && self.view.cells[range.start].attribute,
        ));
        let removed: usize = spans.iter().map(Range::len).sum();
        for r in spans.iter().rev() {
            self.source.replace_range(r.clone(), "");
        }
        for c in &mut self.view.cells[range.end..] {
            c.raw.start -= removed;
            c.raw.end -= removed;
        }
        self.view.cells.drain(range.clone());
        self.view.text.delete_char_range(range);
        self.view.layout();
        // Remaining groups will be parsed again when this input batch finishes.
        self.view.groups.clear();
    }
}
