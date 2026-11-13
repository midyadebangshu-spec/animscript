//! The console panel: the run summary, diagnostics, `note` output and (on request) the runtime trace.
use crate::app::EditorApp;
use crate::model::{self, Level};

pub struct State {
    pub errors: bool,
    pub warnings: bool,
    pub notes: bool,
    /// Ask the runtime for its trace; switching it on runs the program again.
    pub show_trace: bool,
    pub filter: String,
}

impl Default for State {
    fn default() -> Self {
        State {
            errors: true,
            warnings: true,
            notes: true,
            show_trace: false,
            filter: String::new(),
        }
    }
}

pub fn show(app: &mut EditorApp, ui: &mut egui::Ui) {
    let lines = app
        .out
        .as_ref()
        .map(|o| model::console(o, app.console.show_trace))
        .unwrap_or_default();
    let count = |l: Level| {
        lines
            .iter()
            .filter(|x| x.level == l && !x.text.starts_with("    "))
            .count()
    };
    let mut rerun = false;
    ui.horizontal(|ui| {
        let st = &mut app.console;
        ui.toggle_value(&mut st.errors, format!("Errors {}", count(Level::Error)));
        ui.toggle_value(
            &mut st.warnings,
            format!("Warnings {}", count(Level::Warning)),
        );
        ui.toggle_value(&mut st.notes, format!("Notes {}", count(Level::Note)));
        if ui
            .toggle_value(&mut st.show_trace, "Trace")
            .on_hover_text("Every task, action and rule the runtime started or ended")
            .changed()
        {
            rerun = true;
        }
        ui.add(
            egui::TextEdit::singleline(&mut st.filter)
                .hint_text("Filter")
                .desired_width(f32::INFINITY),
        );
    });
    if rerun {
        app.run_now();
    }
    ui.separator();
    let needle = app.console.filter.to_lowercase();
    let st = &app.console;
    let shown: Vec<&model::Line> = lines
        .iter()
        .filter(|l| match l.level {
            Level::Error => st.errors,
            Level::Warning => st.warnings,
            Level::Note => st.notes,
            Level::Trace => st.show_trace,
            Level::Info => true,
        })
        .filter(|l| needle.is_empty() || l.text.to_lowercase().contains(&needle))
        .collect();
    let mut go: Option<(Option<u32>, Option<i64>)> = None;
    let row_height = ui.text_style_height(&egui::TextStyle::Monospace) + 2.0;
    egui::ScrollArea::both()
        .id_salt("console_scroll")
        .auto_shrink([false, false])
        .show_rows(ui, row_height, shown.len(), |ui, range| {
            for l in &shown[range] {
                let colour = match l.level {
                    Level::Error => super::ERROR,
                    Level::Warning => super::WARNING,
                    Level::Note => ui.visuals().text_color(),
                    Level::Trace | Level::Info => ui.visuals().weak_text_color(),
                };
                let text = egui::RichText::new(&l.text).monospace().color(colour);
                let clickable = l.line.is_some() || l.tick.is_some();
                let label = egui::Label::new(text).extend().sense(if clickable {
                    egui::Sense::click()
                } else {
                    egui::Sense::hover()
                });
                let resp = ui.add(label);
                if clickable
                    && resp
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .clicked()
                {
                    go = Some((l.line, l.tick));
                }
            }
        });
    if let Some((line, tick)) = go {
        if let Some(t) = tick {
            app.playing = false;
            app.set_tick(t);
        }
        if let Some(l) = line {
            app.goto_line(l);
        }
    }
}
