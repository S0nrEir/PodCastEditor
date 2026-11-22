use super::*;
use crate::{audio::Track, edit::Plan, studio::View};
use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, RawInput};
use std::path::Path;

// Exercise the same wheel and pointer events egui-winit supplies to the desktop widget.
fn draw(ctx: &egui::Context, view: &View, rate: u32, events: Vec<Event>) -> Option<Gesture> {
    let mut gesture = None;
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(416.0, 300.0))),
        events,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| {
                gesture = Timeline {
                    peaks: &view.peaks,
                    frames: view.plan.frames,
                    window: &view.window,
                    rate,
                    marks: &[0..5, view.plan.frames - 5..view.plan.frames],
                    selection: Some(0..1),
                    cursor: 0,
                    editable: true,
                }
                .draw(ui);
            });
    });
    gesture
}

fn wheel(ctrl: bool, steps: f32) -> Event {
    Event::MouseWheel {
        unit: MouseWheelUnit::Line,
        delta: vec2(0.0, steps),
        modifiers: Modifiers {
            ctrl,
            ..Default::default()
        },
    }
}

#[test]
fn wheel_zoom_and_drag_share_original_time_coordinates() {
    let track = Track::open(Path::new("samples/jfk.wav")).unwrap();
    let mut view = View::new(&track, track.original()).unwrap();
    let ctx = egui::Context::default();
    let quarter = pos2(108.0, 80.0); // Plot edges are x=8 and x=408.
    let middle = pos2(208.0, 80.0);
    let button = |pos, pressed| Event::PointerButton {
        pos,
        button: PointerButton::Primary,
        pressed,
        modifiers: Modifiers::NONE,
    };
    draw(&ctx, &view, track.rate, vec![]);
    assert!(
        draw(
            &ctx,
            &view,
            track.rate,
            vec![Event::PointerMoved(pos2(108.0, 250.0)), wheel(true, 1.0)]
        )
        .is_none()
    );
    assert!(
        draw(
            &ctx,
            &view,
            track.rate,
            vec![Event::PointerMoved(quarter), wheel(false, 1.0)]
        )
        .is_none()
    );
    let Some(Gesture::Zoom(range)) = draw(&ctx, &view, track.rate, vec![wheel(true, 10.0)]) else {
        panic!("expected zoom")
    };
    view.set_window(&track, range).unwrap();
    let anchor = track.frames / 4;
    let width = view.window.end - view.window.start;
    assert!(width < track.frames / 50);
    assert_eq!(frame_at(&view.window, 0.25), anchor);

    draw(&ctx, &view, track.rate, vec![button(quarter, true)]);
    let Some(Gesture::Select(selected)) =
        draw(&ctx, &view, track.rate, vec![Event::PointerMoved(middle)])
    else {
        panic!("expected drag")
    };
    assert_eq!(selected, anchor..view.window.start + width.div_ceil(2));
    draw(&ctx, &view, track.rate, vec![button(middle, false)]);
    draw(
        &ctx,
        &view,
        track.rate,
        vec![Event::PointerMoved(quarter), button(quarter, true)],
    );
    let Some(Gesture::Seek(frame)) = draw(&ctx, &view, track.rate, vec![button(quarter, false)])
    else {
        panic!("expected seek")
    };
    assert_eq!(frame, anchor);

    let Some(Gesture::Zoom(full)) = draw(&ctx, &view, track.rate, vec![wheel(true, -40.0)]) else {
        panic!("expected zoom out")
    };
    assert_eq!(full, 0..track.frames);
    assert_eq!(
        zoom_window(&full, track.frames, track.rate, 1.0, 60.0),
        track.frames - 160..track.frames
    );
    assert_eq!(zoom_window(&(0..1), 1, track.rate, 0.0, 60.0), 0..1);
}

#[test]
fn zoomed_result_waveform_uses_exact_samples_across_a_cut() {
    let path = Path::new("samples/jfk.wav");
    let track = Track::open(path).unwrap();
    let plan = Plan::from_marks(track.frames, std::slice::from_ref(&(32000..64000))).unwrap();
    let mut result = View::new(&track, plan).unwrap();
    result.set_window(&track, 31920..32080).unwrap();
    let mut wav = hound::WavReader::open(path).unwrap();
    let samples: Vec<_> = wav.samples::<i16>().map(Result::unwrap).collect();
    let expected: Vec<_> = samples[31920..32000]
        .iter()
        .chain(&samples[64000..64080])
        .map(|&s| (s.min(0), s.max(0)))
        .collect();
    assert_eq!(result.peaks, expected);
}
