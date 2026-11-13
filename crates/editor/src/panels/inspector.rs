//! The inspector panel: the values of the selected thing at the playhead.
use crate::app::EditorApp;
use crate::model::{self, Selection};
use egui::Color32;

pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let Some(out) = app.shown.clone() else {
        super::empty(ui, "Running...");
        return;
    };
    if app.selection == Selection::None {
        super::empty(
            ui,
            "Select something in the hierarchy,\nthe preview or the timeline.",
        );
        return;
    }
    let rows = model::inspect(&out, app.tick, &app.selection);
    if rows.is_empty() {
        super::empty(ui, "The selection does not exist at this tick.");
        return;
    }
    ui.horizontal(|ui| {
        ui.weak(format!(
            "at tick {} ({})",
            app.tick,
            model::fmt_time(app.tick as i64, out.fps)
        ));
    });
    ui.separator();
    let mut goto = None;
    egui::ScrollArea::vertical()
        .id_salt("inspector_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("inspector_grid")
                .num_columns(2)
                .striped(true)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    for r in &rows {
                        ui.horizontal(|ui| {
                            ui.add_space(r.depth as f32 * 12.0);
                            if r.heading {
                                ui.strong(&r.key);
                            } else {
                                ui.label(&r.key);
                            }
                        });
                        ui.horizontal(|ui| {
                            if let Some(c) = r.color {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(14.0, 14.0),
                                    egui::Sense::hover(),
                                );
                                ui.painter().rect_filled(
                                    rect,
                                    2.0,
                                    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]),
                                );
                                ui.painter().rect_stroke(
                                    rect,
                                    2.0,
                                    ui.visuals().widgets.noninteractive.bg_stroke,
                                    egui::StrokeKind::Inside,
                                );
                            }
                            match r.line {
                                Some(l) => {
                                    if ui
                                        .link(&r.value)
                                        .on_hover_text("Show in the code")
                                        .clicked()
                                    {
                                        goto = Some(l);
                                    }
                                }
                                None if r.heading => {
                                    ui.weak(&r.value);
                                }
                                None => {
                                    ui.monospace(&r.value);
                                }
                            }
                        });
                        ui.end_row();
                    }
                });
        });
    if let Some(l) = goto {
        app.goto_line(l);
    }
}
