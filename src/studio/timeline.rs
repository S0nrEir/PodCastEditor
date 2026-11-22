use super::{ACCENT, CUT, LINE, MUTED, PLOT};
use crate::{
    audio::peak_stride,
    edit::{Span, stamp},
};
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui, pos2, vec2};

#[cfg(test)]
mod tests;

pub enum Gesture {
    Seek(u64),
    Select(Span),
    Zoom(Span),
}

pub struct Timeline<'a> {
    pub peaks: &'a [(i16, i16)],
    pub frames: u64,
    pub window: &'a Span,
    pub rate: u32,
    pub marks: &'a [Span],
    pub selection: Option<Span>,
    pub cursor: u64,
    pub editable: bool,
}

fn frame_at(window: &Span, fraction: f64) -> u64 {
    window.start + ((window.end - window.start) as f64 * fraction.clamp(0.0, 1.0)).round() as u64
}

fn zoom_window(window: &Span, total: u64, rate: u32, fraction: f64, steps: f64) -> Span {
    let width = (window.end - window.start) as f64 / 1.5_f64.powf(steps.clamp(-64.0, 64.0));
    let minimum = u64::from(rate.div_ceil(100)).min(total);
    let width = (width.round() as u64).clamp(minimum, total);
    let start = frame_at(window, fraction)
        .saturating_sub((fraction * width as f64).round() as u64)
        .min(total - width);
    start..start + width
}

impl Timeline<'_> {
    pub fn draw(&self, ui: &mut Ui) -> Option<Gesture> {
        let (rect, response) =
            ui.allocate_exact_size(vec2(ui.available_width(), 188.0), Sense::click_and_drag());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 5, PLOT);
        let area = Rect::from_min_max(rect.min + vec2(8.0, 12.0), rect.max - vec2(8.0, 28.0));
        let visible = self.window.end - self.window.start;
        let x = |frame: u64| {
            area.left()
                + area.width() * ((frame - self.window.start) as f64 / visible as f64) as f32
        };
        let fraction = |p: Pos2| f64::from(((p.x - area.left()) / area.width()).clamp(0.0, 1.0));
        let ticks = (area.width() / 100.0).floor().clamp(2.0, 12.0) as usize;
        for i in 0..=ticks {
            let frame = frame_at(self.window, i as f64 / ticks as f64);
            let xx = x(frame);
            painter.vline(xx, area.y_range(), Stroke::new(1.0_f32, LINE));
            let align = if i == 0 {
                Align2::LEFT_CENTER
            } else if i == ticks {
                Align2::RIGHT_CENTER
            } else {
                Align2::CENTER_CENTER
            };
            let tick_seconds = visible as f64 / f64::from(self.rate) / ticks as f64;
            let timestamp = stamp(frame, self.rate);
            let (seconds, decimal) = timestamp.split_once('.').unwrap();
            let digits = if tick_seconds >= 10.0 {
                0
            } else if tick_seconds >= 1.0 {
                1
            } else if tick_seconds >= 0.001 {
                3
            } else {
                decimal.len()
            };
            let label = if digits == 0 {
                seconds.to_owned()
            } else {
                format!("{seconds}.{}", &decimal[..digits])
            };
            painter.text(
                pos2(xx, rect.bottom() - 13.0),
                align,
                label,
                FontId::monospace(11.0),
                MUTED,
            );
        }
        let clipped = |s: &Span| s.start.max(self.window.start)..s.end.min(self.window.end);
        for mark in self.marks.iter().map(clipped).filter(|s| !s.is_empty()) {
            let r = Rect::from_min_max(
                pos2(x(mark.start), area.top()),
                pos2(x(mark.end), area.bottom()),
            );
            painter.rect_filled(r, 0, Color32::from_rgba_unmultiplied(244, 133, 104, 43));
            painter.hline(r.x_range(), r.top(), Stroke::new(3.0_f32, CUT));
        }
        if let Some(s) = &self.selection
            && !clipped(s).is_empty()
        {
            let part = clipped(s);
            let r = Rect::from_min_max(
                pos2(x(part.start), area.top()),
                pos2(x(part.end), area.bottom()),
            );
            painter.rect_filled(r, 0, Color32::from_rgba_unmultiplied(91, 184, 241, 48));
            for edge in [s.start, s.end]
                .into_iter()
                .filter(|f| (self.window.start..=self.window.end).contains(f))
            {
                painter.vline(
                    x(edge),
                    area.y_range(),
                    Stroke::new(1.5_f32, egui::Color32::LIGHT_BLUE),
                );
            }
        }
        let mid = area.center().y;
        painter.hline(area.x_range(), mid, Stroke::new(1.0_f32, LINE));
        let cols = (area.width() as usize).max(1);
        let stride = peak_stride(visible);
        for col in 0..cols {
            let a = (visible * col as u64 / cols as u64 / stride) as usize;
            let b = (visible * (col + 1) as u64)
                .div_ceil(cols as u64)
                .div_ceil(stride) as usize;
            let (lo, hi) = self.peaks[a..b]
                .iter()
                .fold((0i16, 0i16), |(lo, hi), &(a, b)| (lo.min(a), hi.max(b)));
            let xx = area.left() + (col as f32 + 0.5) * area.width() / cols as f32;
            let scale = (area.height() * 0.47) / 32768.0;
            painter.line_segment(
                [
                    pos2(xx, mid - f32::from(hi) * scale - 0.45),
                    pos2(xx, mid - f32::from(lo) * scale + 0.45),
                ],
                Stroke::new(1.2_f32, ACCENT),
            );
        }
        if (self.window.start..=self.window.end).contains(&self.cursor) {
            let playhead = x(self.cursor);
            painter.vline(
                playhead,
                area.y_range(),
                Stroke::new(1.4_f32, Color32::WHITE),
            );
            let top = area.top();
            painter.add(egui::Shape::convex_polygon(
                vec![
                    pos2(playhead - 4.0, top),
                    pos2(playhead + 4.0, top),
                    pos2(playhead, top + 6.0),
                ],
                Color32::WHITE,
                Stroke::NONE,
            ));
        }
        if ui.is_enabled()
            && !response.dragged()
            && let Some(pos) = response.hover_pos()
        {
            let steps: f64 = ui.input(|i| {
                i.events
                    .iter()
                    .filter_map(|event| {
                        if let egui::Event::MouseWheel {
                            unit,
                            delta,
                            modifiers,
                        } = event
                            && modifiers.ctrl
                        {
                            let units_per_step = match unit {
                                egui::MouseWheelUnit::Point => 40.0,
                                egui::MouseWheelUnit::Line | egui::MouseWheelUnit::Page => 1.0,
                            };
                            Some(f64::from(delta.y) / units_per_step)
                        } else {
                            None
                        }
                    })
                    .sum()
            });
            if steps != 0.0 {
                let next = zoom_window(self.window, self.frames, self.rate, fraction(pos), steps);
                if next != *self.window {
                    return Some(Gesture::Zoom(next));
                }
            }
        }
        if self.editable
            && (response.dragged() || response.drag_stopped())
            && let (Some(start), Some(current)) = (
                ui.input(|i| i.pointer.press_origin()),
                response.interact_pointer_pos(),
            )
        {
            let (a, b) = (
                frame_at(self.window, fraction(start)),
                frame_at(self.window, fraction(current)),
            );
            return Some(Gesture::Select(a.min(b)..a.max(b)));
        }
        if response.clicked()
            && let Some(p) = response.interact_pointer_pos()
        {
            return Some(Gesture::Seek(frame_at(self.window, fraction(p))));
        }
        None
    }
}
