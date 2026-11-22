use super::super::{ACCENT, LINE, PLOT};
use eframe::egui::{
    self, Color32, Galley, PointerButton, Pos2, Rect, Response, Shape, Stroke, StrokeKind, Ui,
    Vec2, vec2,
};
use std::sync::Arc;

#[derive(Default)]
pub struct Minimap {
    preview: Option<Preview>,
    grab_offset: Option<f32>,
}

struct Preview {
    original: Arc<Galley>,
    scale: f32,
    thumbnail: Arc<Galley>,
}

impl Preview {
    fn new(original: &Arc<Galley>, scale: f32) -> Self {
        let mut text = Shape::galley(Pos2::ZERO, original.clone(), Color32::WHITE);
        text.scale(scale);
        let Shape::Text(small) = text else {
            unreachable!()
        };
        Self {
            original: original.clone(),
            scale,
            thumbnail: small.galley,
        }
    }
}

impl Minimap {
    pub fn show(
        &mut self,
        ui: &Ui,
        response: Response,
        galley: &Arc<Galley>,
        text_origin: Vec2,
        scroll: &egui::scroll_area::ScrollAreaOutput<()>,
    ) -> Option<f32> {
        let rect = response.rect;
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            4.,
            PLOT,
            Stroke::new(1.0_f32, LINE),
            StrokeKind::Inside,
        );
        let track = rect.shrink(4.);
        if !track.is_positive() {
            return None;
        }
        let viewport = scroll.inner_rect.size();
        let height = scroll.content_size.y.max(viewport.y).max(1.);
        let fit_width = (track.width() / viewport.x.max(1.)).min(0.5);
        // Fit the whole manuscript when possible. For long scripts, move the
        // preview window instead of flattening glyphs into sub-pixel lines.
        let scale = fit_width.min((track.height() / height).max(0.22));
        if self
            .preview
            .as_ref()
            .is_none_or(|p| !Arc::ptr_eq(&p.original, galley) || p.scale != scale)
        {
            self.preview = Some(Preview::new(galley, scale));
        }
        let max_offset = height - viewport.y;
        let track_height = (height * scale).min(track.height());
        let thumb_height = (viewport.y * scale).min(track_height);
        let travel = track_height - thumb_height;
        let mut offset = scroll.state.offset.y.clamp(0., max_offset);
        let thumb_top = |offset: f32| track.top() + travel * offset / max_offset.max(1.);
        let preview_offset =
            |offset: f32| (height * scale - track_height) * offset / max_offset.max(1.);
        let mut requested = None;
        if max_offset > 0. && travel > 0. {
            let response = response.on_hover_cursor(egui::CursorIcon::ResizeVertical);
            if let Some(pointer) = response.interact_pointer_pos() {
                if response.dragged_by(PointerButton::Primary)
                    || response.drag_stopped_by(PointerButton::Primary)
                {
                    let grab = *self.grab_offset.get_or_insert_with(|| {
                        let press = ui.input(|i| i.pointer.press_origin()).unwrap_or(pointer);
                        let relative = press.y - thumb_top(offset);
                        if (0. ..=thumb_height).contains(&relative) {
                            relative
                        } else {
                            thumb_height * 0.5
                        }
                    });
                    offset = ((pointer.y - track.top() - grab) / travel).clamp(0., 1.) * max_offset;
                    requested = Some(offset);
                } else if response.clicked_by(PointerButton::Primary)
                    && !(thumb_top(offset)..=thumb_top(offset) + thumb_height).contains(&pointer.y)
                {
                    let document_y = (pointer.y - track.top() + preview_offset(offset)) / scale;
                    offset = (document_y - viewport.y * 0.5).clamp(0., max_offset);
                    requested = Some(offset);
                }
            }
            if !response.is_pointer_button_down_on() {
                self.grab_offset = None;
            }
        } else {
            self.grab_offset = None;
        }
        let miniature = &self.preview.as_ref().unwrap().thumbnail;
        painter.with_clip_rect(track).galley(
            track.min + text_origin * scale - vec2(0., preview_offset(offset)),
            miniature.clone(),
            Color32::WHITE,
        );
        let thumb = Rect::from_min_size(
            egui::pos2(track.left(), thumb_top(offset)),
            vec2(track.width(), thumb_height),
        );
        painter.rect(
            thumb,
            2.,
            ACCENT.gamma_multiply(0.12),
            Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.8)),
            StrokeKind::Inside,
        );
        requested
    }
}
