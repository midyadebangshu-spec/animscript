//! The hierarchy panel: scenes, shapes (top-most first), entities and other declarations.
use crate::app::EditorApp;
use crate::model::{self, Selection};

#[derive(Default)]
pub struct State {
    pub filter: String,
}

pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut app.hierarchy.filter)
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
    });
    ui.separator();
    let Some(out) = app.shown.clone() else {
        super::empty(ui, "Running...");
        return;
    };
    let tree = model::hierarchy(&out, app.tick);
    if tree.is_empty() {
        super::empty(ui, "Nothing declared yet.");
        return;
    }
    let needle = app.hierarchy.filter.to_lowercase();
    let current_scene = model::scene_at(&out.journal, app.tick as i64);
    egui::ScrollArea::vertical()
        .id_salt("hierarchy_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for group in &tree {
                let children: Vec<&model::Node> = group
                    .children
                    .iter()
                    .filter(|n| needle.is_empty() || n.label.to_lowercase().contains(&needle))
                    .collect();
                if children.is_empty() {
                    continue;
                }
                egui::CollapsingHeader::new(format!("{} ({})", group.label, children.len()))
                    .id_salt(&group.label)
                    .default_open(true)
                    .show(ui, |ui| {
                        for node in children {
                            let selected = app.selection == node.sel;
                            let playing_here =
                                matches!(node.sel, Selection::Scene(i) if Some(i) == current_scene);
                            let mut text = egui::RichText::new(&node.label);
                            if playing_here {
                                text = text.strong();
                            }
                            let resp = ui
                                .horizontal(|ui| {
                                    let r = ui.selectable_label(selected, text);
                                    if !node.detail.is_empty() {
                                        ui.weak(&node.detail);
                                    }
                                    r
                                })
                                .inner;
                            let resp = match node.line {
                                Some(l) => {
                                    resp.on_hover_text(format!("line {l} (double-click to show)"))
                                }
                                None => resp,
                            };
                            if resp.clicked() {
                                app.select(node.sel.clone());
                            }
                            if resp.double_clicked() {
                                if let Some(l) = node.line {
                                    app.goto_line(l);
                                }
                            }
                        }
                    });
            }
        });
}
