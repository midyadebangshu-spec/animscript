//! The timeline panel: transport controls, a ruler with the scenes, and one row per animated shape with its
//! actions, modifiers and keyframes. Drag on the ruler to scrub; click a bar to select its shape and see its line.
use crate::app::EditorApp;
use crate::model::{self, Selection};
use animscript_lang::journal::ItemKind;
use egui::{Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

pub struct State {
    /// Width of one tick in points.
    pub px_per_tick: f32,
    /// Scrubbing started on the ruler and lasts until the button is released.
    scrubbing: bool,
}

impl Default for State {
    fn default() -> Self {
        State {
            px_per_tick: 8.0,
            scrubbing: false,
        }
    }
}

const LABELS: f32 = 120.0;
const SCENES: f32 = 18.0;
const RULER: f32 = 20.0;
const LANE: f32 = 20.0;
const HEAD: f32 = SCENES + RULER;

/// A "nice" number of ticks between ruler labels so that they are at least `min_px` apart.
pub fn ruler_step(px_per_tick: f32, min_px: f32) -> i64 {
    let need = (min_px / px_per_tick.max(1e-3)).ceil() as i64;
    let mut step = 1i64;
    for mult in std::iter::successors(Some(1i64), |m| m.checked_mul(10)) {
        for base in [1, 2, 5] {
            step = base * mult;
            if step >= need {
                return step;
            }
        }
    }
    step
}

fn transport(app: &mut EditorApp, ui: &mut egui::Ui) {
    let last = app.last_tick();
    let fps = app.shown.as_ref().map_or(24.0, |o| o.fps);
    ui.horizontal(|ui| {
        if ui.button("|<").on_hover_text("First tick (Home)").clicked() {
            app.set_tick(0);
        }
        if ui
            .button("<")
            .on_hover_text("Back one tick (Left)")
            .clicked()
        {
            app.playing = false;
            app.set_tick(app.tick as i64 - 1);
        }
        let play = if app.playing { "Pause" } else { "Play" };
        if ui
            .button(play)
            .on_hover_text("Play or pause (Space)")
            .clicked()
        {
            app.toggle_play();
        }
        if ui
            .button(">")
            .on_hover_text("Forward one tick (Right)")
            .clicked()
        {
            app.playing = false;
            app.set_tick(app.tick as i64 + 1);
        }
        if ui.button(">|").on_hover_text("Last tick (End)").clicked() {
            app.set_tick(last as i64);
        }
        ui.toggle_value(&mut app.looping, "Loop");
        ui.separator();
        ui.label("Tick");
        let mut t = app.tick;
        if ui
            .add(egui::DragValue::new(&mut t).range(0..=last))
            .changed()
        {
            app.playing = false;
            app.set_tick(t as i64);
        }
        ui.weak(format!(
            "of {last}   {} / {}   {} fps",
            model::fmt_time(app.tick as i64, fps),
            model::fmt_time(
                last as i64
                    + i64::from(app.shown.as_ref().is_some_and(|o| !o.snapshots.is_empty())),
                fps
            ),
            model::num(fps)
        ));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::Slider::new(&mut app.timeline.px_per_tick, 1.0..=40.0)
                    .logarithmic(true)
                    .show_value(false),
            )
            .on_hover_text("Zoom the timeline");
            ui.weak("Zoom");
        });
    });
}

pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    transport(app, ui);
    ui.separator();
    let Some(out) = app.shown.clone() else {
        super::empty(ui, "Running...");
        return;
    };
    if out.snapshots.is_empty() {
        super::empty(ui, "No frames to show.");
        return;
    }
    let j = &out.journal;
    let rows = model::timeline_rows(j);
    let ticks = out.snapshots.len() as i64;
    let ppt = app.timeline.px_per_tick;
    let row_tops: Vec<f32> = rows
        .iter()
        .scan(HEAD, |y, r| {
            let top = *y;
            *y += r.lanes as f32 * LANE + 4.0;
            Some(top)
        })
        .collect();
    let content_h = row_tops
        .last()
        .zip(rows.last())
        .map_or(HEAD, |(t, r)| t + r.lanes as f32 * LANE + 4.0);
    let content = Vec2::new(
        LABELS + ticks as f32 * ppt + 40.0,
        content_h.max(HEAD + LANE),
    );
    let font = FontId::proportional(11.0);
    let visuals = ui.visuals().clone();
    let weak = visuals.weak_text_color();
    let line = visuals.widgets.noninteractive.bg_stroke.color;

    egui::ScrollArea::both()
        .id_salt("timeline_scroll")
        .auto_shrink([false, false])
        .show_viewport(ui, |ui, viewport| {
            let size = content.max(ui.available_size());
            let (resp, painter) = ui.allocate_painter(size, Sense::click_and_drag());
            let origin = resp.rect.min;
            // the part of the panel on screen: the ruler and the names stay there while the tracks scroll
            let view = Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size());
            let tracks_left = view.left() + LABELS;
            let x_of = |tick: f32| origin.x + LABELS + tick * ppt;
            let tick_at = |x: f32| ((x - origin.x - LABELS) / ppt).floor() as i64;
            let y_of = |y: f32| origin.y + y; // rows scroll vertically
            let head_bottom = view.top() + HEAD;
            let tracks = painter.with_clip_rect(Rect::from_min_max(
                Pos2::new(tracks_left, head_bottom),
                view.max,
            ));
            let head = painter.with_clip_rect(Rect::from_min_max(
                Pos2::new(tracks_left, view.top()),
                Pos2::new(view.right(), head_bottom),
            ));
            let names = painter.with_clip_rect(Rect::from_min_max(
                Pos2::new(view.left(), head_bottom),
                Pos2::new(tracks_left, view.bottom()),
            ));

            // rows: a stripe, then the items
            let mut hovered: Option<usize> = None;
            let pointer = resp.hover_pos();
            for (ri, row) in rows.iter().enumerate() {
                let top = y_of(row_tops[ri]);
                let h = row.lanes as f32 * LANE + 4.0;
                let selected = app.selection == Selection::Shape(row.target.clone());
                let band =
                    Rect::from_min_size(Pos2::new(view.left(), top), Vec2::new(view.width(), h));
                if selected {
                    painter.with_clip_rect(view).rect_filled(
                        band,
                        0.0,
                        visuals.selection.bg_fill.gamma_multiply(0.25),
                    );
                } else if ri % 2 == 1 {
                    painter
                        .with_clip_rect(view)
                        .rect_filled(band, 0.0, visuals.faint_bg_color);
                }
                names.text(
                    Pos2::new(view.left() + 6.0, top + h / 2.0),
                    Align2::LEFT_CENTER,
                    &row.target,
                    FontId::proportional(12.5),
                    if selected {
                        visuals.strong_text_color()
                    } else {
                        visuals.text_color()
                    },
                );
                for (k, idx) in row.items.iter().enumerate() {
                    let it = &j.items[*idx];
                    let y = top + 2.0 + row.lane_of[k] as f32 * LANE;
                    let end = it.end.unwrap_or(ticks);
                    let colour = match it.kind {
                        ItemKind::Action => super::ACTION,
                        ItemKind::Modifier => super::MODIFIER,
                        ItemKind::Keys => super::KEYS,
                    };
                    // a key set owns its last tick; everything else ends before `end`
                    let right = if it.kind == ItemKind::Keys {
                        end as f32 + 1.0
                    } else {
                        end as f32
                    };
                    let bar = Rect::from_min_max(
                        Pos2::new(x_of(it.start as f32), y + 2.0),
                        Pos2::new(x_of(right).max(x_of(it.start as f32) + 3.0), y + LANE - 2.0),
                    );
                    let over = pointer
                        .is_some_and(|p| bar.contains(p) && p.x > tracks_left && p.y > head_bottom);
                    if over {
                        hovered = Some(*idx);
                    }
                    let active = model::item_active(it, app.tick as i64);
                    let fill = if active || over {
                        colour
                    } else {
                        colour.gamma_multiply(0.6)
                    };
                    if it.kind == ItemKind::Keys {
                        let mid = bar.center().y;
                        tracks.line_segment(
                            [Pos2::new(bar.left(), mid), Pos2::new(bar.right(), mid)],
                            Stroke::new(2.0, fill.gamma_multiply(0.7)),
                        );
                        for key in &it.keys {
                            let c = Pos2::new(x_of(*key as f32 + 0.5), mid);
                            let r = 5.0;
                            tracks.add(egui::Shape::convex_polygon(
                                vec![
                                    c + Vec2::new(0.0, -r),
                                    c + Vec2::new(r, 0.0),
                                    c + Vec2::new(0.0, r),
                                    c + Vec2::new(-r, 0.0),
                                ],
                                fill,
                                Stroke::new(1.0, visuals.extreme_bg_color),
                            ));
                        }
                    } else {
                        tracks.rect_filled(bar, 3.0, fill);
                        let text = it
                            .label
                            .strip_prefix(&format!("{}.", row.target))
                            .unwrap_or(&it.label);
                        let clip = bar
                            .shrink2(Vec2::new(4.0, 0.0))
                            .intersect(tracks.clip_rect());
                        if clip.width() > 24.0 {
                            tracks.with_clip_rect(clip).text(
                                Pos2::new(bar.left() + 4.0, bar.center().y),
                                Align2::LEFT_CENTER,
                                text,
                                font.clone(),
                                Color32::WHITE,
                            );
                        }
                    }
                }
            }
            if rows.is_empty() {
                painter.with_clip_rect(view).text(
                    Pos2::new(tracks_left + 12.0, head_bottom + 16.0),
                    Align2::LEFT_CENTER,
                    "Nothing is animated: timed actions, modifiers and keyframes appear here.",
                    FontId::proportional(12.5),
                    weak,
                );
            }

            // header: scenes over a ruler, on a solid background so rows pass underneath
            let head_bg = Rect::from_min_max(view.min, Pos2::new(view.right(), head_bottom));
            painter
                .with_clip_rect(view)
                .rect_filled(head_bg, 0.0, visuals.panel_fill);
            let current = model::scene_at(j, app.tick as i64);
            for (i, s) in j.scenes.iter().enumerate() {
                let r = Rect::from_min_max(
                    Pos2::new(x_of(s.start as f32), view.top() + 1.0),
                    Pos2::new(x_of(s.end as f32) - 1.0, view.top() + SCENES - 1.0),
                );
                let fill = if Some(i) == current {
                    super::ACCENT.gamma_multiply(0.55)
                } else {
                    visuals.widgets.inactive.bg_fill
                };
                head.rect_filled(r, 3.0, fill);
                let clip = r.shrink2(Vec2::new(4.0, 0.0)).intersect(head.clip_rect());
                if clip.width() > 16.0 {
                    head.with_clip_rect(clip).text(
                        Pos2::new(r.left().max(tracks_left) + 4.0, r.center().y),
                        Align2::LEFT_CENTER,
                        &s.name,
                        font.clone(),
                        visuals.strong_text_color(),
                    );
                }
            }
            let step = ruler_step(ppt, 64.0);
            let first = (tick_at(tracks_left).max(0) / step) * step;
            let mut t = first;
            while t <= ticks && x_of(t as f32) < view.right() {
                let x = x_of(t as f32);
                head.line_segment(
                    [
                        Pos2::new(x, view.top() + SCENES + 10.0),
                        Pos2::new(x, head_bottom),
                    ],
                    Stroke::new(1.0, weak),
                );
                head.text(
                    Pos2::new(x + 3.0, view.top() + SCENES + 1.0),
                    Align2::LEFT_TOP,
                    format!(
                        "{t} ({}s)",
                        model::num((t as f64 / out.fps * 100.0).round() / 100.0)
                    ),
                    font.clone(),
                    weak,
                );
                tracks.line_segment(
                    [Pos2::new(x, head_bottom), Pos2::new(x, view.bottom())],
                    Stroke::new(1.0, line.gamma_multiply(0.5)),
                );
                t += step;
            }
            let all = painter.with_clip_rect(view);
            all.line_segment(
                [
                    Pos2::new(view.left(), head_bottom),
                    Pos2::new(view.right(), head_bottom),
                ],
                Stroke::new(1.0, line),
            );
            all.line_segment(
                [
                    Pos2::new(tracks_left, view.top()),
                    Pos2::new(tracks_left, view.bottom()),
                ],
                Stroke::new(1.0, line),
            );
            all.text(
                Pos2::new(view.left() + 6.0, view.top() + HEAD / 2.0),
                Align2::LEFT_CENTER,
                "Shape",
                font.clone(),
                weak,
            );

            // playhead: the tick being shown, as a column one tick wide
            let px = x_of(app.tick as f32);
            let playhead = painter.with_clip_rect(Rect::from_min_max(
                Pos2::new(tracks_left, view.top()),
                view.max,
            ));
            playhead.rect_filled(
                Rect::from_min_max(
                    Pos2::new(px, view.top() + SCENES),
                    Pos2::new(px + ppt.max(2.0), head_bottom),
                ),
                2.0,
                super::ERROR,
            );
            playhead.line_segment(
                [
                    Pos2::new(px, view.top() + SCENES),
                    Pos2::new(px, view.bottom()),
                ],
                Stroke::new(1.5, super::ERROR),
            );

            // input
            let pos = resp.interact_pointer_pos();
            let on_tracks = |p: Pos2| p.x >= tracks_left;
            if resp.drag_started() {
                app.timeline.scrubbing =
                    pos.is_some_and(|p| on_tracks(p) && (p.y <= head_bottom || hovered.is_none()));
            }
            if resp.drag_stopped() {
                app.timeline.scrubbing = false;
            }
            if let Some(p) = pos {
                if (resp.dragged() && app.timeline.scrubbing)
                    || (resp.clicked() && on_tracks(p) && hovered.is_none())
                {
                    app.playing = false;
                    app.set_tick(tick_at(p.x));
                    // clicking a scene on the top band selects it
                    if resp.clicked() && p.y <= view.top() + SCENES {
                        if let Some(i) = model::scene_at(j, tick_at(p.x)) {
                            app.selection = Selection::Scene(i);
                        }
                    }
                } else if resp.clicked() && !on_tracks(p) && p.y > head_bottom {
                    let hit = rows.iter().enumerate().find(|(ri, r)| {
                        let top = y_of(row_tops[*ri]);
                        p.y >= top && p.y < top + r.lanes as f32 * LANE + 4.0
                    });
                    if let Some((_, r)) = hit {
                        app.select(Selection::Shape(r.target.clone()));
                    }
                }
            }
            if let Some(idx) = hovered {
                let it = &j.items[idx];
                if resp.clicked() {
                    app.select(Selection::Shape(it.target.clone()));
                    if let Some(sp) = it.span {
                        app.goto_line(sp.line);
                    }
                }
                resp.on_hover_text_at_pointer(model::item_tooltip(it, out.fps));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ruler_steps_are_round_and_wide_enough() {
        assert_eq!(ruler_step(8.0, 64.0), 10);
        assert_eq!(ruler_step(64.0, 64.0), 1);
        assert_eq!(ruler_step(1.0, 64.0), 100);
        assert_eq!(ruler_step(40.0, 64.0), 2);
        assert_eq!(ruler_step(13.0, 64.0), 5);
    }
}
