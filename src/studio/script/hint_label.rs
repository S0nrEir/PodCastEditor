use super::view::View;
use eframe::egui::{self, Event, FontId, Pos2, Rect, Ui, text_edit::TextEditOutput, vec2};

// TextEdit returns the nearest insertion position, which can lie to the right of
// the glyph. Hit-test the actual glyph to keep both halves and line ends accurate.
fn hovered_char(out: &TextEditOutput, pos: Pos2) -> Option<usize> {
    let local = (pos - out.galley_pos).to_pos2();
    let cursor = out.galley.cursor_from_pos(local.to_vec2());
    let row = out.galley.rows.get(cursor.rcursor.row)?;
    let column = row.glyphs.partition_point(|g| g.max_x() <= local.x);
    let glyph = row.glyphs.get(column)?;
    glyph
        .logical_rect()
        .contains(local)
        .then_some(cursor.ccursor.index - cursor.rcursor.column + column)
}

pub fn show(ui: &Ui, out: &TextEditOutput, view: &View, keyboard: &mut bool) {
    let focused = out.response.has_focus();
    ui.input(|input| {
        for event in &input.events {
            match event {
                Event::Key { pressed: true, .. } | Event::Text(_) if focused => *keyboard = true,
                Event::PointerMoved(_)
                | Event::PointerButton { .. }
                | Event::PointerGone
                | Event::MouseWheel { .. } => *keyboard = false,
                _ => {}
            }
        }
    });
    if ui.input(|i| i.pointer.any_down()) {
        return;
    }
    let clip = ui.clip_rect().intersect(out.text_clip_rect);
    let hit = if *keyboard && focused {
        out.cursor_range.and_then(|range| {
            let cursor = range.primary;
            let rect = out
                .galley
                .pos_from_cursor(&cursor)
                .translate(out.galley_pos.to_vec2());
            let row = &out.galley.rows[cursor.rcursor.row];
            let index = if cursor.rcursor.column == row.glyphs.len() && !row.glyphs.is_empty() {
                cursor.ccursor.index.checked_sub(1)?
            } else {
                cursor.ccursor.index
            };
            clip.intersects(rect.expand(0.5))
                .then_some((index, rect.left_top()))
        })
    } else if out.response.hovered() {
        ui.input(|i| i.pointer.hover_pos())
            .filter(|pos| clip.contains(*pos))
            .and_then(|pos| Some((hovered_char(out, pos)?, pos)))
    } else {
        None
    };
    let Some((index, anchor)) = hit else { return };
    let Some(name) = view.tag_label(index) else {
        return;
    };

    let ink = ui.visuals().strong_text_color();
    let painter = ui.painter().with_clip_rect(clip);
    let galley = painter.layout_no_wrap(name, FontId::proportional(12.), ink);
    let pad = vec2(6., 3.);
    let size = galley.size() + pad * 2.;
    let x = (anchor.x + 3.).clamp(clip.left(), (clip.right() - size.x).max(clip.left()));
    let above = anchor.y - size.y - 5.;
    let y = if above >= clip.top() {
        above
    } else {
        (anchor.y + 17.).min(clip.bottom() - size.y)
    };
    let rect = Rect::from_min_size(egui::pos2(x, y.max(clip.top())), size);
    painter.rect_filled(rect, 4., super::super::LINE);
    painter.galley(rect.min + pad, galley, ink);
}
