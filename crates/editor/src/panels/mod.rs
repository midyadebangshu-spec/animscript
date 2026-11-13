//! The six panels. Each is a `show(app, ui)` function with its own small `State`.
pub mod code;
pub mod console;
pub mod hierarchy;
pub mod inspector;
pub mod preview;
pub mod timeline;

use egui::Color32;

pub const ERROR: Color32 = Color32::from_rgb(0xe0, 0x5a, 0x4f);
pub const WARNING: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);
pub const ACCENT: Color32 = Color32::from_rgb(0x4a, 0x90, 0xd9);
pub const ACTION: Color32 = Color32::from_rgb(0x4a, 0x90, 0xd9);
pub const MODIFIER: Color32 = Color32::from_rgb(0x5a, 0xa8, 0x6b);
pub const KEYS: Color32 = Color32::from_rgb(0xd9, 0xa4, 0x41);

/// A message in the middle of an empty panel.
pub fn empty(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.weak(text);
    });
}
