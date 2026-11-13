//! `anim-editor [file.anim]`: the AnimScript editor.
use anim_editor::app::EditorApp;

fn main() -> eframe::Result {
    let file = std::env::args_os().nth(1).map(std::path::PathBuf::from);
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("AnimScript Editor")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "AnimScript Editor",
        options,
        Box::new(move |cc| {
            let ctx = cc.egui_ctx.clone();
            let mut app = EditorApp::new(move || ctx.request_repaint());
            if let Some(f) = &file {
                app.open_path(f);
            }
            Ok(Box::new(app))
        }),
    )
}
