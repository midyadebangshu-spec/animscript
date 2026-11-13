//! The preview panel: the frame at the playhead, drawn by the same renderer as `anim render`.
use crate::app::EditorApp;
use crate::model::{self, Selection};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

pub struct State {
    /// Extra zoom on top of "fit the panel"; 1 = fit.
    pub zoom: f32,
    pub pan: Vec2,
    pub vertices: bool,
    pub physics: bool,
    texture: Option<egui::TextureHandle>,
    /// What the texture shows: run generation, tick, overlays, render scale (in quarters).
    shown: Option<(u64, usize, bool, bool, u32)>,
    pub error: Option<String>,
}

impl Default for State {
    fn default() -> Self {
        State {
            zoom: 1.0,
            pan: Vec2::ZERO,
            vertices: false,
            physics: false,
            texture: None,
            shown: None,
            error: None,
        }
    }
}

pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        if ui
            .button("Fit")
            .on_hover_text("Fit the canvas in the panel")
            .clicked()
        {
            app.preview.zoom = 1.0;
            app.preview.pan = Vec2::ZERO;
        }
        ui.label(format!("{:.0}%", app.preview.zoom * 100.0));
        ui.separator();
        ui.checkbox(&mut app.preview.vertices, "Vertices");
        ui.checkbox(&mut app.preview.physics, "Physics");
        if let Some(o) = &app.shown {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("{} x {}", o.canvas.0, o.canvas.1));
                if app.stale() {
                    ui.colored_label(super::WARNING, "last good run")
                        .on_hover_text("The code has errors: this is the last version that ran.");
                }
            });
        }
    });
    ui.separator();
    let Some(out) = app.shown.clone() else {
        super::empty(ui, "Running...");
        return;
    };
    let Some(snap) = out.snapshots.get(app.tick) else {
        super::empty(ui, "No frames. Fix the errors listed in the console.");
        return;
    };
    let (cw, ch) = (
        snap["canvas"][0].as_f64().unwrap_or(out.canvas.0 as f64) as f32,
        snap["canvas"][1].as_f64().unwrap_or(out.canvas.1 as f64) as f32,
    );
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let area = resp.rect;
    painter.rect_filled(area, 0.0, ui.visuals().extreme_bg_color);

    // zoom about the pointer with the wheel, pan by dragging
    if resp.hovered() {
        let scroll =
            ui.input(|i| i.smooth_scroll_delta.y) + (ui.input(|i| i.zoom_delta()) - 1.0) * 200.0;
        if scroll != 0.0 {
            let old = app.preview.zoom;
            let new = (old * (scroll * 0.004).exp()).clamp(0.1, 16.0);
            if let Some(p) = resp.hover_pos() {
                let from_centre = p - area.center() - app.preview.pan;
                app.preview.pan -= from_centre * (new / old - 1.0);
            }
            app.preview.zoom = new;
        }
    }
    if resp.dragged() {
        app.preview.pan += resp.drag_delta();
    }
    let fit = ((area.width() - 16.0) / cw)
        .min((area.height() - 16.0) / ch)
        .max(0.01);
    let scale = fit * app.preview.zoom;
    let image = Rect::from_center_size(area.center() + app.preview.pan, Vec2::new(cw, ch) * scale);

    // render at the size shown (in steps, so a slow drag of the zoom does not redraw every frame)
    let ppp = ui.ctx().pixels_per_point();
    let quarters = ((scale * ppp * 4.0).ceil() as u32).clamp(1, 8);
    let key = (
        out.generation,
        app.tick,
        app.preview.vertices,
        app.preview.physics,
        quarters,
    );
    if app.preview.shown != Some(key) {
        let base = app.base_dir();
        match animscript_render::render_assets(
            snap,
            quarters as f32 / 4.0,
            app.preview.vertices,
            app.preview.physics,
            &base,
        ) {
            Ok(pm) => {
                let img = egui::ColorImage::from_rgba_premultiplied(
                    [pm.width() as usize, pm.height() as usize],
                    pm.data(),
                );
                match &mut app.preview.texture {
                    Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                    None => {
                        app.preview.texture = Some(ui.ctx().load_texture(
                            "preview",
                            img,
                            egui::TextureOptions::LINEAR,
                        ));
                    }
                }
                app.preview.error = None;
            }
            Err(e) => app.preview.error = Some(e),
        }
        app.preview.shown = Some(key);
    }
    let painter = painter.with_clip_rect(area);
    if let Some(t) = &app.preview.texture {
        let uv = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(t.id(), image, uv, Color32::WHITE);
    }
    painter.rect_stroke(
        image,
        0.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Outside,
    );
    if let Some(e) = &app.preview.error {
        painter.text(
            area.left_top() + Vec2::splat(8.0),
            egui::Align2::LEFT_TOP,
            format!("render failed: {e}"),
            egui::FontId::proportional(13.0),
            super::ERROR,
        );
    }
    let to_screen = |p: (f64, f64)| image.min + Vec2::new(p.0 as f32, p.1 as f32) * scale;
    if let Selection::Shape(name) = &app.selection {
        if let Some(corners) = model::shape_outline(snap, name) {
            let pts: Vec<Pos2> = corners.iter().map(|c| to_screen(*c)).collect();
            painter.add(egui::Shape::closed_line(
                pts.clone(),
                Stroke::new(1.5, super::ACCENT),
            ));
            for p in pts {
                painter.rect_filled(
                    Rect::from_center_size(p, Vec2::splat(6.0)),
                    1.0,
                    super::ACCENT,
                );
            }
        }
    }
    // click a shape to select it; double-click goes to its declaration
    if resp.clicked() || resp.double_clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let q = (p - image.min) / scale;
            match model::pick(snap, q.x as f64, q.y as f64) {
                Some(name) => {
                    if resp.double_clicked() {
                        let line = out
                            .journal
                            .decls
                            .iter()
                            .find(|d| d.name == name)
                            .map(|d| d.span.line);
                        if let Some(l) = line {
                            app.goto_line(l);
                        }
                    }
                    app.select(Selection::Shape(name));
                }
                None => app.select(Selection::None),
            }
        }
    }
}
