//! The code panel: the program text with syntax colours, line numbers and underlined diagnostics.
use crate::app::EditorApp;
use crate::highlight::{line_start_char, scan, span_bytes, Tok};
use egui::text::{CCursor, CCursorRange, LayoutJob};
use egui::{Color32, FontId, Stroke, TextFormat};
use std::hash::{Hash, Hasher};
use std::ops::Range;

#[derive(Default)]
pub struct State {
    /// A line to show and select on the next frame.
    pub jump: Option<u32>,
    /// The line last jumped to, tinted until the text is clicked or changed.
    pub mark: Option<u32>,
    job: Option<(u64, LayoutJob)>,
}

fn colour(tok: Tok, dark: bool) -> Color32 {
    let c = |d: u32, l: u32| {
        let v = if dark { d } else { l };
        Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    };
    match tok {
        Tok::Plain => c(0xd4d4d4, 0x24292e),
        Tok::Comment => c(0x6a9955, 0x6a737d),
        Tok::Keyword => c(0xc586c0, 0xa626a4),
        Tok::Number => c(0xb5cea8, 0x986801),
        Tok::Str => c(0xce9178, 0x50a14f),
        Tok::Color => c(0x4ec9b0, 0x0184bc),
        Tok::Call => c(0xdcdcaa, 0x4078f2),
        Tok::Op => c(0x9aa0a6, 0x586069),
    }
}

/// The coloured text. `marks` are byte ranges to underline (errors red, warnings amber).
fn layout(src: &str, marks: &[(Range<usize>, Color32)], font: &FontId, dark: bool) -> LayoutJob {
    let mut job = LayoutJob::default();
    let mut cuts: Vec<usize> = marks
        .iter()
        .flat_map(|(r, _)| [r.start, r.end])
        .filter(|c| src.is_char_boundary(*c))
        .collect();
    cuts.sort();
    for (range, tok) in scan(src) {
        let mut a = range.start;
        let inner = cuts
            .iter()
            .copied()
            .filter(|c| *c > range.start && *c < range.end);
        for b in inner.chain(std::iter::once(range.end)) {
            if b <= a {
                continue;
            }
            let mark = marks.iter().find(|(r, _)| r.start <= a && b <= r.end);
            let format = TextFormat {
                font_id: font.clone(),
                color: colour(tok, dark),
                underline: mark.map_or(Stroke::NONE, |(_, c)| Stroke::new(1.5, *c)),
                ..Default::default()
            };
            job.append(&src[a..b], 0.0, format);
            a = b;
        }
    }
    job
}

pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let dark = ui.visuals().dark_mode;
    // diagnostics belong to the text that was run: underline only while that is still what is shown
    let marks: Vec<(Range<usize>, Color32)> = app
        .out
        .as_ref()
        .filter(|o| o.source == app.source)
        .map(|o| {
            o.diagnostics
                .iter()
                .filter_map(|d| {
                    let s = d.span?;
                    let colour = if d.is_error() {
                        super::ERROR
                    } else {
                        super::WARNING
                    };
                    Some((span_bytes(&app.source, s.line, s.col, s.end_col)?, colour))
                })
                .collect()
        })
        .unwrap_or_default();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (&app.source, dark, font.size.to_bits()).hash(&mut hasher);
    for (r, c) in &marks {
        (r.start, r.end, c.to_array()).hash(&mut hasher);
    }
    let key = hasher.finish();
    if app.code.job.as_ref().is_none_or(|(k, _)| *k != key) {
        app.code.job = Some((key, layout(&app.source, &marks, &font, dark)));
    }
    let job = app
        .code
        .job
        .as_ref()
        .map(|(_, j)| j.clone())
        .unwrap_or_default();

    let lines = app.source.split('\n').count().max(1);
    let digits = lines.to_string().len().max(2);
    let gutter = ui.fonts_mut(|f| f.glyph_width(&font, '0')) * (digits as f32 + 1.5);
    let id = egui::Id::new("code_editor");

    if let Some(line) = app.code.jump {
        let line = line.clamp(1, lines as u32);
        let a = line_start_char(&app.source, line);
        let len = app
            .source
            .split('\n')
            .nth(line as usize - 1)
            .map_or(0, |l| l.chars().count());
        let mut state = egui::TextEdit::load_state(ui.ctx(), id).unwrap_or_default();
        state.cursor.set_char_range(Some(CCursorRange::two(
            CCursor::new(a),
            CCursor::new(a + len),
        )));
        state.store(ui.ctx(), id);
        ui.memory_mut(|m| m.request_focus(id));
        app.code.mark = Some(line);
    }

    egui::ScrollArea::both()
        .id_salt("code_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.add_space(gutter);
                let mut layouter = |ui: &egui::Ui, _buf: &dyn egui::TextBuffer, _wrap: f32| {
                    let mut job = job.clone();
                    job.wrap.max_width = f32::INFINITY;
                    ui.fonts_mut(|f| f.layout_job(job))
                };
                let out = egui::TextEdit::multiline(&mut app.source)
                    .id(id)
                    .code_editor()
                    .frame(egui::Frame::NONE)
                    .desired_width(f32::INFINITY)
                    .desired_rows(lines.max(30))
                    .lock_focus(true)
                    .layouter(&mut layouter)
                    .show(ui);
                if out.response.changed() {
                    let now = ui.input(|i| i.time);
                    app.edited(now);
                }
                if out.response.changed() || out.response.clicked() {
                    app.code.mark = None;
                }
                if let Some(row) = app
                    .code
                    .mark
                    .and_then(|m| out.galley.rows.get(m as usize - 1))
                {
                    let y = out.galley_pos.y + row.pos.y;
                    let clip = ui.clip_rect();
                    let band = egui::Rect::from_min_max(
                        egui::pos2(clip.left(), y),
                        egui::pos2(clip.right(), y + row.row.size.y),
                    );
                    ui.painter()
                        .rect_filled(band, 0.0, super::ACCENT.gamma_multiply(0.22));
                }
                // line numbers, beside the rows of the laid-out text
                let painter = ui.painter();
                let x = out.galley_pos.x - gutter * 0.3;
                let clip = ui.clip_rect();
                let weak = ui.visuals().weak_text_color();
                let cursor_line = out.cursor_range.map(|c| {
                    let i: usize = c.primary.index.into();
                    app.source.chars().take(i).filter(|c| *c == '\n').count()
                });
                for (n, row) in out.galley.rows.iter().enumerate() {
                    let y = out.galley_pos.y + row.pos.y;
                    if y > clip.bottom() || y + row.row.size.y < clip.top() {
                        continue;
                    }
                    let colour = if cursor_line == Some(n) {
                        ui.visuals().text_color()
                    } else {
                        weak
                    };
                    painter.text(
                        egui::pos2(x, y),
                        egui::Align2::RIGHT_TOP,
                        n + 1,
                        font.clone(),
                        colour,
                    );
                }
                if let Some(line) = app.code.jump.take() {
                    if let Some(row) = out.galley.rows.get(line.saturating_sub(1) as usize) {
                        let top = out.galley_pos + row.pos.to_vec2();
                        let rect = egui::Rect::from_min_size(top, egui::vec2(1.0, row.row.size.y));
                        ui.scroll_to_rect(rect.expand2(egui::vec2(0.0, 60.0)), None);
                    }
                }
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layout_keeps_the_text_and_underlines_marks() {
        let src = "shape s = sqare(size: 100) # é\n";
        let a = src.find("sqare").unwrap();
        let job = layout(
            src,
            &[(a..a + 5, super::super::ERROR)],
            &FontId::monospace(12.0),
            true,
        );
        assert_eq!(job.text, src);
        let marked: Vec<&str> = job
            .sections
            .iter()
            .filter(|s| s.format.underline != Stroke::NONE)
            .map(|s| &src[usize::from(s.byte_range.start)..usize::from(s.byte_range.end)])
            .collect();
        assert_eq!(marked, ["sqare"]);
    }
}
