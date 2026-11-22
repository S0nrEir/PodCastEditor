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
    script.raw = true;
    script.rebuild(&ctx, true);
    frame(&mut script, vec![shortcut(Key::S)]);
    assert!(!script.doc.as_ref().unwrap().dirty());
    let restored = Document::open(path.clone(), &names(), false).unwrap();
    assert_eq!(restored.source, expected);
    assert_eq!(restored.view.text, format!("{original_preview}测试稿尾🎧"));
    frame(&mut script, vec![shortcut(Key::Z)]);
    frame(&mut script, vec![shortcut(Key::Z)]);
    frame(&mut script, vec![shortcut(Key::S)]);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
