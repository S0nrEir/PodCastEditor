use super::*;
use egui::TextBuffer;
use std::ops::Range;
use tempfile::tempdir;

fn names() -> Vec<String> {
    DEFAULTS.map(str::to_owned).to_vec()
}

#[cfg(target_os = "macos")]
#[test]
fn mac_system_fonts_render_chinese_body_and_heading_text() {
    let ctx = Context::default();
    super::super::theme(&ctx);
    let _ = ctx.run(Default::default(), |ctx| {
        for family in [
            egui::FontFamily::Proportional,
            egui::FontFamily::Name("script-bold".into()),
        ] {
            ctx.fonts(|f| {
                assert!(
                    f.lock()
                        .fonts
                        .font(&egui::FontId::new(16., family))
                        .has_glyphs("声剪讲稿背景介绍验证")
                )
            });
        }
        assert!(super::super::cjk_font(true).is_some());
    });
}

#[test]
fn pointer_and_caret_hints_follow_the_visible_character() {
    use egui::{Event, FullOutput, Shape};
    let dir = tempdir().unwrap();
    let path = dir.path().join("提示.md");
    let real = std::env::var_os("PODCAST_TEST_SCRIPT")
        .map(|p| fs::read_to_string(p).unwrap())
        .unwrap_or_default();
    fs::write(
        &path,
        format!("前<自定义>甲<验证>乙</验证></自定义>后<补充=\"&quot;\">。\n{real}"),
    )
    .unwrap();
    let ctx = Context::default();
    super::super::theme(&ctx);
    let mut script = Script {
        names: names(),
        ..Default::default()
    };
    script.open(&ctx, path);
    let frame = |s: &mut Script, events| {
        ctx.run(
            egui::RawInput {
                events,
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000., 620.),
                )),
                ..Default::default()
            },
            |ctx| s.show(ctx),
        )
    };
    let label = |out: &FullOutput, name: &str| {
        out.shapes.iter().any(|s| matches!(&s.shape,
        Shape::Text(t) if t.galley.text() == name && t.galley.job.sections[0].format.font_id.size == 12.))
    };
    let arrow = || Event::Key {
        key: Key::ArrowRight,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers::NONE,
    };
    for raw in [false, true] {
        script.raw = raw;
        script.doc.as_mut().unwrap().selected = 0..0;
        script.rebuild(&ctx, true);
        frame(&mut script, vec![]);
        let out = frame(&mut script, vec![]);
        let view = &script.doc.as_ref().unwrap().view;
        let text = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                Shape::Text(t) if t.galley.text() == view.text => Some(t),
                _ => None,
            })
            .unwrap();
        let glyph_rect = |ch| {
            let i = view.text.chars().position(|c| c == ch).unwrap();
            text.galley.rows[0].glyphs[i]
                .logical_rect()
                .translate(text.pos.to_vec2())
        };
        let tagged = glyph_rect('甲');
        let nested = glyph_rect('乙').center();
        let plain = glyph_rect('后').center();
        for x in [tagged.left() + 1., tagged.right() - 1.] {
            let out = frame(
                &mut script,
                vec![Event::PointerMoved(egui::pos2(x, tagged.center().y))],
            );
            assert!(
                label(&out, "自定义"),
                "both halves of tagged glyph, raw={raw}"
            );
        }
        assert!(label(
            &frame(&mut script, vec![Event::PointerMoved(nested)]),
            "自定义 · 验证"
        ));
        let out = frame(&mut script, vec![Event::PointerMoved(plain)]);
        assert!(!label(&out, "自定义") && !label(&out, "自定义 · 验证"));
        let end =
            text.galley.rows[0].rect.right_center() + text.pos.to_vec2() + egui::vec2(14., 0.);
        assert!(!label(
            &frame(&mut script, vec![Event::PointerMoved(end)]),
            "自定义"
        ));
        assert!(label(&frame(&mut script, vec![arrow()]), "自定义"));
        assert!(!script.doc.as_ref().unwrap().dirty());
    }
}

fn locate(s: &str, needle: &str) -> Range<usize> {
    let start = s.find(needle).unwrap();
    start..start + needle.len()
}

fn select(doc: &mut Document, needle: &str) {
    let span = locate(&doc.view.text, needle);
    let start = doc.view.text[..span.start].chars().count();
    doc.selected = doc
        .view
        .selection(start..start + needle.chars().count())
        .unwrap();
}

#[test]
fn page_undo_follows_text_and_sidebar_edits_on_windows_and_mac() {
    use egui::{Event, PointerButton, os::OperatingSystem};

    let dir = tempdir().unwrap();
    let path = dir.path().join("撤销.md");
    let original = "开场。";
    fs::write(&path, original).unwrap();
    for os in [OperatingSystem::Windows, OperatingSystem::Mac] {
        let ctx = Context::default();
        ctx.set_os(os);
        super::super::theme(&ctx);
        let command = Modifiers {
            command: true,
            ctrl: os == OperatingSystem::Windows,
            mac_cmd: os == OperatingSystem::Mac,
            ..Default::default()
        };
        let key = |key, modifiers| Event::Key {
            key,
            physical_key: Some(key),
            pressed: true,
            repeat: false,
            modifiers,
        };
        let frame = |s: &mut Script, events| {
            let mut input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1100., 720.),
                )),
                events,
                ..Default::default()
            };
            let mut queued = Vec::new();
            loop {
                order_history_input(&mut input, &mut queued);
                let output = ctx.run(input.take(), |ctx| s.show(ctx));
                if queued.is_empty() {
                    break output;
                }
            }
        };
        let mut script = Script {
            names: names(),
            ..Default::default()
        };
        script.open(&ctx, path.clone());
        frame(&mut script, vec![]);
        frame(&mut script, vec![key(Key::A, command)]);
        let edited = "改过的正文🎧";
        frame(
            &mut script,
            vec![
                Event::Ime(egui::ImeEvent::Enabled),
                Event::Ime(egui::ImeEvent::Preedit("g".into())),
            ],
        );
        frame(
            &mut script,
            vec![Event::Ime(egui::ImeEvent::Preedit("gai".into()))],
        );
        frame(
            &mut script,
            vec![Event::Ime(egui::ImeEvent::Commit(edited.into()))],
        );
        frame(&mut script, vec![key(Key::A, command)]);
        assert_eq!(script.doc.as_ref().unwrap().selected, 0..edited.len());
        let out = frame(&mut script, vec![]);
        let pos = out
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == "验证" => {
                    Some(text.visual_bounding_rect().center())
                }
                _ => None,
            })
            .unwrap();
        for pressed in [true, false] {
            frame(
                &mut script,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        }
        let tagged = format!("<验证>{edited}</验证>");
        assert_eq!(
            script.doc.as_ref().unwrap().source,
            tagged,
            "{os:?} click {pos:?}: {}",
            script.message
        );

        // A click elsewhere on the page can leave the manuscript without focus.
        ctx.memory_mut(|m| m.surrender_focus(editor_id()));
        frame(&mut script, vec![key(Key::Z, command)]);
        let doc = script.doc.as_ref().unwrap();
        assert_eq!(doc.source, edited);
        assert_eq!(doc.selected, 0..edited.len());
        assert!(doc.view.marks.is_empty());
        frame(&mut script, vec![key(Key::Z, command)]);
        assert_eq!(script.doc.as_ref().unwrap().source, original);
        assert!(!script.doc.as_ref().unwrap().dirty());

        let redo = if os == OperatingSystem::Mac {
            key(Key::Z, command | Modifiers::SHIFT)
        } else {
            key(Key::Y, command)
        };
        frame(&mut script, vec![redo.clone(), redo.clone()]);
        assert_eq!(script.doc.as_ref().unwrap().source, tagged);
        frame(
            &mut script,
            vec![Event::Paste("另一个版本".into()), key(Key::Z, command)],
        );
        assert_eq!(script.doc.as_ref().unwrap().source, tagged);
        frame(
            &mut script,
            vec![key(Key::Z, command), key(Key::Z, command)],
        );
        assert_eq!(script.doc.as_ref().unwrap().source, original);
        frame(&mut script, vec![Event::Text("新版本".into()), redo]);
        assert_eq!(script.doc.as_ref().unwrap().source, "新版本");

        script.manage = true;
        ctx.memory_mut(|m| m.request_focus(tag_name_id()));
        frame(&mut script, vec![Event::Text("临时名称".into())]);
        assert_eq!(script.new_name, "临时名称");
        frame(&mut script, vec![key(Key::Z, command)]);
        assert!(script.new_name.is_empty());
        assert_eq!(script.doc.as_ref().unwrap().source, "新版本");
    }
}

#[test]
fn outline_shows_rendered_nested_and_attribute_text_in_both_modes() {
    let source = "<自定义>**甲**\r\n<补充=\"乙&amp;丙\"></自定义> <验证></验证>";
    for raw in [false, true] {
        let view = View::new(source, &names(), raw);
        let entries: Vec<_> = view
            .passages
            .iter()
            .map(|p| (p.text.as_str(), p.color))
            .collect();
        assert_eq!(
            entries,
            [("甲 乙&丙", color(5)), ("乙&丙", color(3)), ("", color(4))]
        );
    }
}

#[test]
fn outline_abbreviates_to_one_line_in_document_order_after_reverse_tagging() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("顺序.md");
    let opening = "开场先介绍本期话题和嘉宾，接着回顾上一期节目，然后开始讨论。".repeat(3);
    fs::write(&path, format!("{opening}\n\n中段\n转场\n\n收尾")).unwrap();
    let ctx = Context::default();
    super::super::theme(&ctx);
    let mut script = Script {
        names: names(),
        ..Default::default()
    };
    script.open(&ctx, path.clone());
    for (body, tag) in [("收尾", 0), ("中段\n转场", 3), (opening.as_str(), 4)] {
        select(script.doc.as_mut().unwrap(), body);
        script.apply(&ctx, tag);
    }
    assert!(script.save());
    for (raw, width) in [(false, 840.), (true, 1200.)] {
        script.raw = raw;
        if raw {
            script.open(&ctx, path.clone());
        }
        let frame = |s: &mut Script, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 620.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| s.show(ctx),
            )
        };
        frame(&mut script, vec![]);
        let out = frame(&mut script, vec![]);
        let mut rows: Vec<_> = out
            .shapes
            .iter()
            .filter_map(|s| match &s.shape {
                egui::Shape::Text(t)
                    if s.clip_rect.right() < width * 0.25 && t.galley.job.sections.len() == 2 =>
                {
                    Some(t)
                }
                _ => None,
            })
            .collect();
        rows.sort_by(|a, b| a.pos.y.total_cmp(&b.pos.y));
        assert_eq!(rows.len(), 3);
        assert!(rows.iter().all(|t| t.galley.rows.len() == 1));
        let labels: Vec<_> = rows.iter().map(|t| t.galley.text()).collect();
        assert!(labels[0].starts_with("验证开场") && labels[0].ends_with("..."));
        assert!(labels[0].len() < opening.len());
        assert_eq!(labels[1..], ["补充中段 转场", "背景介绍收尾"]);

        let pos = rows[0].visual_bounding_rect().center();
        for pressed in [true, false] {
            frame(
                &mut script,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Modifiers::NONE,
                    },
                ],
            );
        }
        let doc = script.doc.as_ref().unwrap();
        assert_eq!(&doc.source[doc.selected.clone()], opening);
    }
}

#[test]
fn dragging_outline_divider_refits_the_single_line_entries() {
    use egui::{Event, PointerButton, Shape, containers::panel::PanelState};

    let dir = tempdir().unwrap();
    let path = dir.path().join("调整栏宽.md");
    let name = "需要在窄栏截短的标签";
    let passage = "这是随栏宽显示的讲稿摘录。";
    fs::write(
        &path,
        format!("<{name}>{passage}</{name}>\n<验证>结尾</验证>"),
    )
    .unwrap();
    let ctx = Context::default();
    super::super::theme(&ctx);
    let mut script = Script::default();
    script.open(&ctx, path);
    let mut frame = |events| {
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1110., 620.),
                )),
                events,
                ..Default::default()
            },
            |ctx| script.show(ctx),
        )
    };
    let panel = || {
        PanelState::load(&ctx, Id::new("script-outline"))
            .unwrap()
            .rect
    };
    let labels = |out: &egui::FullOutput| {
        out.shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                Shape::Text(t) if panel().contains(t.pos) && t.galley.job.sections.len() == 2 => {
                    assert_eq!(t.galley.rows.len(), 1);
                    assert!(t.pos.x + t.galley.size().x <= panel().right());
                    Some(t.galley.text().to_owned())
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    frame(vec![]);
    let initial = labels(&frame(vec![]));
    assert!(initial[0].ends_with("..."));
    let grab = panel().right_center();
    frame(vec![Event::PointerMoved(grab), button(grab, true)]);
    let wide = grab + egui::vec2(260., 0.);
    let expanded = labels(&frame(vec![Event::PointerMoved(wide)]));
    assert!((panel().right() - wide.x).abs() < 1.);
    assert_eq!(
        expanded,
        [format!("{name}{passage}"), "验证结尾".to_owned()]
    );
    let narrow = grab - egui::vec2(55., 0.);
    let shrunk = labels(&frame(vec![Event::PointerMoved(narrow)]));
    assert!((panel().right() - narrow.x).abs() < 1.);
    assert!(shrunk[0].len() < initial[0].len() && shrunk[0].ends_with("..."));
    assert_eq!(shrunk[1], "验证结尾");
    frame(vec![button(narrow, false)]);
    assert_eq!(labels(&frame(vec![])), shrunk);
}

#[test]
fn clicking_duplicate_passages_and_dragging_the_minimap_navigates_the_editor() {
    use egui::{Event, FullOutput, PointerButton, Shape, epaint::TextShape};
    let dir = tempdir().unwrap();
    let path = dir.path().join("导航.md");
    let paragraph = "一段足够长的讲稿，用于检查窗口宽度改变后的自动换行。".repeat(3) + "\r\n";
    let mark = "<验证>相同的片段</验证>\r\n";
    let source = format!(
        "{mark}{}{mark}{}结尾",
        paragraph.repeat(65),
        paragraph.repeat(50)
    );
    fs::write(&path, &source).unwrap();
    let second = source.rfind("相同的片段").unwrap();
    for (raw, width) in [(false, 1110.), (true, 840.)] {
        let ctx = Context::default();
        super::super::theme(&ctx);
        let mut script = Script {
            names: names(),
            raw,
            ..Default::default()
        };
        script.open(&ctx, path.clone());
        let text = script.doc.as_ref().unwrap().view.text.clone();
        let frame = |script: &mut Script, events| {
            ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 620.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| script.show(ctx),
            )
        };
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let find_text = |out: &FullOutput, needle: &str| -> Vec<(egui::Rect, TextShape)> {
            out.shapes
                .iter()
                .filter_map(|s| match &s.shape {
                    Shape::Text(t) if t.galley.text() == needle => Some((s.clip_rect, t.clone())),
                    _ => None,
                })
                .collect()
        };
        let thumb = |out: &FullOutput| {
            out.shapes
                .iter()
                .find_map(|s| match &s.shape {
                    Shape::Rect(r) if r.stroke.color == ACCENT.gamma_multiply(0.8) => Some(r.rect),
                    _ => None,
                })
                .unwrap()
        };
        frame(&mut script, vec![]);
        let out = frame(&mut script, vec![]);
        let document_shapes = find_text(&out, &text);
        assert_eq!(document_shapes.len(), 2);
        let (map, preview) = &document_shapes[1];
        let full_size = document_shapes[0].1.galley.size();
        let miniature = preview.galley.size();
        assert!((miniature.x / full_size.x - miniature.y / full_size.y).abs() < 0.001);
        assert!(
            miniature.y > map.height(),
            "long preview should pan without distorting glyphs"
        );
        assert!(
            preview
                .galley
                .rows
                .iter()
                .flat_map(|r| &r.visuals.mesh.vertices)
                .any(|v| v.color == color(4))
        );
        let items = find_text(&out, "验证相同的片段");
        assert_eq!(items.len(), 2);
        let pos = items[1].1.visual_bounding_rect().center();
        frame(
            &mut script,
            vec![Event::PointerMoved(pos), button(pos, true)],
        );
        frame(&mut script, vec![button(pos, false)]);
        let out = frame(&mut script, vec![]);
        let expected = second..second + "相同的片段".len();
        let doc = script.doc.as_ref().unwrap();
        assert_eq!(doc.selected, expected);
        assert!(
            ctx.memory(|m| m.has_focus(editor_id())),
            "jump should focus the tagged text"
        );
        let main = &find_text(&out, &text)[0];
        let caret = main
            .1
            .galley
            .pos_from_ccursor(CCursor::new(doc.view.cursor(second)))
            .translate(main.1.pos.to_vec2());
        assert!(
            main.0.contains(caret.center()),
            "jump must reveal the second occurrence"
        );
        let handle = thumb(&out);
        let grab = handle.left_top() + egui::vec2(3., 2.);
        let pressed = frame(
            &mut script,
            vec![Event::PointerMoved(grab), button(grab, true)],
        );
        assert!((thumb(&pressed).top() - handle.top()).abs() < 1.);
        let bottom = map.center_bottom() + egui::vec2(0., 8.);
        frame(&mut script, vec![Event::PointerMoved(bottom)]);
        let out = frame(&mut script, vec![]);
        let main = &find_text(&out, &text)[0];
        let last = main.1.galley.rows.last().unwrap().rect.center() + main.1.pos.to_vec2();
        assert!(main.0.contains(last), "drag to end");
        let top = map.center_top() - egui::vec2(0., 8.);
        frame(&mut script, vec![Event::PointerMoved(top)]);
        frame(&mut script, vec![button(top, false)]);
        let out = frame(&mut script, vec![]);
        let main = &find_text(&out, &text)[0];
        assert!(main.0.contains(main.1.pos), "drag back to beginning");
        let small = &find_text(&out, &text)[1].1;
        let click = map.center_bottom() - egui::vec2(0., 18.);
        let scale = small.galley.size().y / main.1.galley.size().y;
        let clicked_text_y = (click.y - small.pos.y) / scale;
        frame(
            &mut script,
            vec![Event::PointerMoved(click), button(click, true)],
        );
        frame(&mut script, vec![button(click, false)]);
        let out = frame(&mut script, vec![]);
        let main = &find_text(&out, &text)[0];
        assert!(
            (main.1.pos.y + clicked_text_y - main.0.center().y).abs() < 5.,
            "click must center the text actually shown at that point in the preview"
        );
        let before_wheel = thumb(&out).top();
        frame(
            &mut script,
            vec![
                Event::PointerMoved(main.0.center()),
                Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0., -300.),
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert!(thumb(&frame(&mut script, vec![])).top() > before_wheel);
        let doc = script.doc.as_ref().unwrap();
        assert_eq!(doc.selected, expected);
        assert!(!doc.dirty());
    }
}

#[test]
fn both_modes_share_source_and_all_five_tags_round_trip() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("讲稿.md");
    let source = "# 标题\r\n\r\n> **强调**\r\n- 玩🎧法\n\n设计 & \"引号\" < 符号\n\n~~验证~~\n";
    let original = format!("\u{feff}{source}");
    fs::write(&path, &original).unwrap();
    let names = names();
    let mut doc = Document::open(path.clone(), &names, false).unwrap();
    let preview = doc.view.text.clone();
    assert!(preview.contains("标题\n\n强调\n• 玩🎧法"));
    assert!(!preview.contains("~~"));
    for (name, needle) in
        DEFAULTS
            .into_iter()
            .zip(["标题", "玩🎧法", "设计", "& \"引号\" < 符号", "验证"])
    {
        select(&mut doc, needle);
        doc.wrap(name).unwrap();
        doc.view = View::new(&doc.source, &names, false);
        assert_eq!(doc.view.text, preview);
        let r = locate(&preview, needle);
        let job = doc.view.job.borrow();
        let section = job
            .sections
            .iter()
            .find(|s| s.byte_range.contains(&r.start))
            .unwrap();
        assert_eq!(
            section.format.color,
            color(names.iter().position(|n| n == name).unwrap())
        );
    }
    assert!(
        doc.source
            .contains("<补充=\"&amp; &quot;引号&quot; &lt; 符号\">")
    );
    assert_eq!(doc.view.marks.len(), 5);
    doc.save().unwrap();
    let mut reopened = Document::open(path.clone(), &names, true).unwrap();
    assert_eq!(reopened.view.text, doc.source.replace("\r\n", "\n"));
    assert_eq!(reopened.source, doc.source);
    for _ in 0..5 {
        doc.undo(false);
    }
    doc.save().unwrap();
    assert_eq!(fs::read(&path).unwrap(), original.as_bytes());
    reopened.selected = locate(&reopened.source, "引号");
    reopened.unwrap();
    assert!(reopened.source.contains("& \"引号\" < 符号"));
}

#[test]
fn rendered_edit_targets_selected_duplicate_and_escapes_attribute() {
    let mut source = "甲重复 <补充=\"重复\"> 末尾".to_owned();
    let mut view = View::new(&source, &names(), false);
    assert_eq!(view.text, "甲重复 重复 末尾");
    let mut buffer = Buffer::new(&mut view, &mut source, "\n");
    buffer.delete_char_range(4..6);
    buffer.insert_text("乙\"&🎧", 4);
    assert_eq!(*buffer.source, "甲重复 <补充=\"乙&quot;&amp;🎧\"> 末尾");
    assert_eq!(buffer.view.job.borrow().text, "甲重复 乙\"&🎧 末尾");
    assert_eq!(
        View::new(&source, &names(), false).text,
        "甲重复 乙\"&🎧 末尾"
    );
}

#[test]
fn chinese_quotes_can_be_emphasized_mid_sentence() {
    let source = "放大了**“坐在屏幕前”**这件事。";
    let view = View::new(source, &names(), false);
    assert_eq!(view.text, "放大了“坐在屏幕前”这件事。");
    assert_eq!(View::new(source, &names(), true).text, source);
}

#[test]
fn deleting_across_paragraphs_keeps_partial_annotation_balanced() {
    let mut source = "开头\n\n<验证>甲\n\n乙</验证>丙\n\n末尾".to_string();
    let mut view = View::new(&source, &names(), false);
    let a = view.cursor(source.find('乙').unwrap());
    let mut edit = Buffer::new(&mut view, &mut source, "\n");
    edit.delete_char_range(a..a + 3);
    let result = View::new(&source, &names(), false);
    assert_eq!(result.marks.len(), 1);
    assert_eq!(result.text, "开头\n\n甲\n\n\n末尾");
}

#[test]
fn tagging_crosses_existing_marks_and_inline_code_without_losing_text() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("marks.md");
    fs::write(&path, "<既有>甲\n\n乙</既有>丙 `代码`").unwrap();
    let mut doc = Document::open(path, &names(), false).unwrap();
    let preview = doc.view.text.clone();
    select(&mut doc, "乙丙");
    doc.wrap("游戏设计").unwrap();
    doc.view = View::new(&doc.source, &names(), false);
    assert_eq!(doc.view.text, preview);
    assert_eq!(doc.view.marks.len(), 3);
    select(&mut doc, "代码");
    doc.wrap("玩法介绍").unwrap();
    doc.view = View::new(&doc.source, &names(), false);
    assert_eq!(doc.view.text, preview);
    assert_eq!(doc.view.marks.len(), 4);
}

#[test]
fn secondary_click_keeps_the_original_text_selection() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("menu.md");
    fs::write(&path, "一段正文，选中这里。").unwrap();
    let ctx = Context::default();
    super::super::theme(&ctx);
    let mut script = Script {
        names: names(),
        ..Default::default()
    };
    script.open(&ctx, path);
    let draw = |script: &mut Script, events| {
        let input = egui::RawInput {
            events,
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 720.),
            )),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| script.show(ctx));
    };
    draw(&mut script, vec![]);
    let selection = locate(&script.doc.as_ref().unwrap().source, "选中这里");
    script.doc.as_mut().unwrap().selected = selection.clone();
    script.restore_selection(&ctx, true);
    draw(&mut script, vec![]);
    let pos = ctx.read_response(editor_id()).unwrap().rect.left_top() + egui::vec2(120., 9.);
    for pressed in [true, false] {
        draw(
            &mut script,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Modifiers::NONE,
                },
            ],
        );
        assert_eq!(script.doc.as_ref().unwrap().selected, selection);
    }
    assert!(
        ctx.read_response(editor_id())
            .unwrap()
            .context_menu_opened()
    );
}

// Pass an actual UTF-8 manuscript through PODCAST_TEST_SCRIPT to exercise the same editor
// and filesystem path with a long document. Only the temporary copy is written.
#[test]
fn manuscript_edits_and_keyboard_save_reopen_exactly() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("播客讲稿.md");
    let bytes = std::env::var_os("PODCAST_TEST_SCRIPT")
        .map(|f| fs::read(f).unwrap())
        .unwrap_or_else(|| {
            "# 中文开场\n\n> **录制参考**\n\n<已有标签>正文</已有标签>\n\n*全稿完*\n"
                .as_bytes()
                .to_vec()
        });
    fs::write(&path, &bytes).unwrap();
    let ctx = Context::default();
    super::super::theme(&ctx);
    let mut script = Script {
        names: names(),
        ..Default::default()
    };
    script.open(&ctx, path.clone());
    let source = script.doc.as_ref().unwrap().source.clone();
    let original_preview = script.doc.as_ref().unwrap().view.text.clone();
    assert!(original_preview.contains("全稿完"));
    let frame = |script: &mut Script, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1100., 720.),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| script.show(ctx));
    };
    let shortcut = |key| egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Modifiers {
            ctrl: true,
            command: true,
            ..Default::default()
        },
    };
    frame(&mut script, vec![]);
    script.doc.as_mut().unwrap().selected = source.len()..source.len();
    script.restore_selection(&ctx, true);
    frame(
        &mut script,
        vec![egui::Event::Paste("测试稿尾🎧".into()), shortcut(Key::S)],
    );
    let doc = script.doc.as_mut().unwrap();
    assert_eq!(doc.source, format!("{source}测试稿尾🎧"));
    assert!(!doc.dirty());
    select(doc, "测试稿尾🎧");
    script.restore_selection(&ctx, true);
    frame(&mut script, vec![shortcut(Key::Num4)]);
    let expected = format!("{source}<补充=\"测试稿尾🎧\">");
    assert_eq!(script.doc.as_ref().unwrap().source, expected);
    assert_eq!(
        script
            .doc
            .as_ref()
            .unwrap()
            .view
            .passages
            .last()
            .unwrap()
            .text,
        "测试稿尾🎧"
    );
    // Recording reads the unsaved, formatted manuscript even from source mode.
    let formatted = script.reading_copy().unwrap().1;
    assert_eq!(formatted.text, format!("{original_preview}测试稿尾🎧"));
    assert!(script.doc.as_ref().unwrap().dirty());
    script.raw = true;
    script.rebuild(&ctx, true);
    assert_eq!(script.reading_copy().unwrap().1, formatted);
    frame(&mut script, vec![shortcut(Key::S)]);
    assert!(!script.doc.as_ref().unwrap().dirty());
    let restored = Document::open(path.clone(), &names(), false).unwrap();
    assert_eq!(restored.source, expected);
    assert_eq!(restored.view.text, format!("{original_preview}测试稿尾🎧"));
    frame(&mut script, vec![shortcut(Key::Z)]);
    frame(&mut script, vec![shortcut(Key::Z)]);
    assert!(
        !script
            .doc
            .as_ref()
            .unwrap()
            .view
            .passages
            .iter()
            .any(|p| p.text == "测试稿尾🎧")
    );
    frame(&mut script, vec![shortcut(Key::S)]);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
