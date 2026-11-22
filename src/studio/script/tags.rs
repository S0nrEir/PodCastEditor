use eframe::egui::{Color32, ecolor::Hsva};
use pulldown_cmark::{Event, Parser, Tag};
use std::ops::Range;

pub const DEFAULTS: [&str; 5] = ["背景介绍", "玩法介绍", "游戏设计", "补充", "验证"];

pub fn valid_name(s: &str) -> bool {
    s.starts_with(char::is_alphabetic)
        && s.chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '_' | '-'))
}

pub fn color(index: usize) -> Color32 {
    let palette = [
        [123, 221, 191],
        [139, 192, 255],
        [201, 165, 255],
        [247, 198, 121],
        [253, 155, 167],
    ];
    if let Some(&[r, g, b]) = palette.get(index) {
        Color32::from_rgb(r, g, b)
    } else {
        Hsva::new(
            (0.19 + (index - 5) as f32 * 0.618034).fract(),
            0.53,
            0.95,
            1.0,
        )
        .into()
    }
}

#[derive(Clone, Debug)]
pub struct Mark {
    pub outer: Range<usize>,
    pub body: Range<usize>,
    pub name: String,
    pub attribute: bool,
}

pub fn escape_value(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

pub fn entity(s: &str) -> Option<(&'static str, char)> {
    [
        ("&amp;", '&'),
        ("&quot;", '"'),
        ("&lt;", '<'),
        ("&gt;", '>'),
    ]
    .into_iter()
    .find(|(key, _)| s.starts_with(key))
}

pub fn marks(source: &str, names: &[String]) -> Vec<Mark> {
    let mut result = Vec::new();
    if !source.contains('<') {
        return result;
    }
    let literal: Vec<_> = Parser::new(source)
        .into_offset_iter()
        .filter_map(|(event, range)| {
            matches!(event, Event::Code(_) | Event::Start(Tag::CodeBlock(_))).then_some(range)
        })
        .collect();
    let mut stack = Vec::new();
    let mut pos = 0;
    while let Some(offset) = source[pos..].find('<') {
        let start = pos + offset;
        pos = start + 1;
        if source.as_bytes()[..start]
            .iter()
            .rev()
            .take_while(|&&b| b == b'\\')
            .count()
            % 2
            == 1
        {
            continue;
        }
        if let Some(rest) = source[pos..].strip_prefix("补充=\"")
            && let Some(end) = rest.find('"')
            && rest[end..].starts_with("\">")
        {
            let body_start = source.len() - rest.len();
            pos = body_start + end + 2;
            result.push(Mark {
                outer: start..pos,
                body: body_start..body_start + end,
                name: "补充".into(),
                attribute: true,
            });
            continue;
        }
        let rest = &source[pos..];
        let closing = rest.starts_with('/');
        let name_start = pos + usize::from(closing);
        let rest = &source[name_start..];
        let Some(end) = rest.find(|c: char| !(c.is_alphanumeric() || matches!(c, '_' | '-')))
        else {
            continue;
        };
        let name = &rest[..end];
        if !rest[end..].starts_with('>')
            || !valid_name(name)
            || (literal.iter().any(|r| r.contains(&start)) && !names.iter().any(|n| n == name))
        {
            continue;
        }
        pos = name_start + end + 1;
        if !closing {
            stack.push((name, start..pos));
        } else if let Some(index) = stack.iter().rposition(|(open, _)| *open == name) {
            stack.truncate(index + 1);
            let (_, open) = stack.pop().unwrap();
            result.push(Mark {
                outer: open.start..pos,
                body: open.end..start,
                name: name.into(),
                attribute: false,
            });
        }
    }
    // Navigation follows the text that each entry jumps to, including nested marks.
    result.sort_by_key(|m| m.body.start);
    result
}
