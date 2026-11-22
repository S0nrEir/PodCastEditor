use super::*;
use std::{fs, thread, time::Instant};

fn key(key: Key, pressed: bool) -> Event {
    Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Modifiers::NONE,
    }
}

fn frame(ctx: &Context, page: &mut Recording, events: Vec<Event>) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1000., 680.),
        )),
        events,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| page.show(ctx));
}

#[test]
fn manuscript_reader_scrolls_down_and_back_up() {
    let ctx = Context::default();
    super::super::theme(&ctx);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("提词稿.md");
    let source = std::env::var_os("PODCAST_TEST_SCRIPT")
        .map(|p| fs::read_to_string(p).unwrap())
        .unwrap_or_else(|| format!("{}\n全稿完", "## 试读\n\n正文。\n\n".repeat(120)));
    fs::write(&path, source).unwrap();
    let mut script = super::super::script::Script::default();
    script.open(&ctx, path);
    let mut page = Recording::default();
    assert!(page.load(script.reading_copy()));
    assert!(page.script.as_ref().unwrap().1.text.contains("全稿完"));
    frame(&ctx, &mut page, vec![]);
    let top = page.offset;
    frame(&ctx, &mut page, vec![key(Key::ArrowDown, true)]);
    assert!(page.offset > top);
    frame(
        &ctx,
        &mut page,
        vec![key(Key::ArrowDown, false), key(Key::ArrowUp, true)],
    );
    assert_eq!(page.offset, top);
}

#[test]
fn holding_shortcut_toggles_only_once() {
    let ctx = Context::default();
    let mut first = false;
    let mut second = false;
    let input = || egui::RawInput {
        events: vec![key(Key::R, true)],
        ..Default::default()
    };
    let _ = ctx.run(input(), |ctx| first = record_shortcut(ctx));
    let _ = ctx.run(input(), |ctx| second = record_shortcut(ctx));
    assert!(first);
    assert!(!second);
}

#[test]
#[ignore = "短暂使用真实默认麦克风，须手动运行"]
fn default_microphone_shortcut_resume_and_finish() {
    let ctx = Context::default();
    let mut page = Recording::default();
    frame(&ctx, &mut page, vec![key(Key::R, true)]);
    let deadline = Instant::now() + Duration::from_secs(180);
    while page.permission.is_some() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(100));
        page.poll();
    }
    assert!(page.capture.is_some(), "{}", page.status);
    let device = page.capture.as_ref().unwrap().device_name.clone();
    thread::sleep(Duration::from_millis(850));
    frame(&ctx, &mut page, vec![key(Key::R, false), key(Key::R, true)]);
    assert!(page.capture.is_none());
    let first = page.draft.as_ref().unwrap().frames();
    assert!(first > 0, "{}", page.status);
    thread::sleep(Duration::from_millis(200));
    assert_eq!(page.draft.as_ref().unwrap().frames(), first);
    frame(&ctx, &mut page, vec![key(Key::R, false), key(Key::R, true)]);
    assert!(page.capture.is_some(), "{}", page.status);
    thread::sleep(Duration::from_millis(850));
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("真实麦克风自测.wav");
    page.finish(&ctx, |_| Some((path.clone(), false)));
    assert!(page.capture.is_none());
    let draft = page.draft.as_ref().unwrap();
    let (rate, channels, frames) = (draft.rate, draft.channels, draft.frames());
    assert!(frames > first);
    let deadline = Instant::now() + Duration::from_secs(10);
    while page.exporting() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
        page.poll();
    }
    assert!(page.draft.is_none(), "{}", page.status);
    let mut wav = hound::WavReader::open(&path).unwrap();
    assert_eq!(
        (
            wav.spec().sample_rate,
            wav.spec().channels,
            u64::from(wav.duration())
        ),
        (rate, channels, frames)
    );
    let peak = wav
        .samples::<i16>()
        .map(|s| s.unwrap().unsigned_abs())
        .max()
        .unwrap();
    eprintln!("input={device}, rate={rate}, channels={channels}, frames={frames}, peak={peak}");
}
