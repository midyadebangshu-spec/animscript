//! The editor's state and frame: a menu bar, a status bar and the docked panels.
use crate::engine::{RunOutput, Runner};
use crate::model::Selection;
use crate::panels;
use egui_dock::{DockArea, DockState, NodeIndex, Style};
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Code,
    Preview,
    Hierarchy,
    Inspector,
    Timeline,
    Console,
}

impl Tab {
    pub const ALL: [Tab; 6] = [
        Tab::Code,
        Tab::Preview,
        Tab::Hierarchy,
        Tab::Inspector,
        Tab::Timeline,
        Tab::Console,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Tab::Code => "Code",
            Tab::Preview => "Preview",
            Tab::Hierarchy => "Hierarchy",
            Tab::Inspector => "Inspector",
            Tab::Timeline => "Timeline",
            Tab::Console => "Console",
        }
    }
}

/// Something that would throw away unsaved text, waiting for the user to agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pending {
    New,
    Open,
    Quit,
}

pub const TEMPLATE: &str = r#"project "Untitled":
    anim_version: "0.1"
    canvas: 800x600
    fps: 24
    background: #fdf6e3

shape box = square(size: 160, at: (250, 300))
box.fill = #f4a6c0
box.stroke = #b83a6b
box.stroke_width = 3

scene "Intro" for 3.seconds:
    main:
        note "the box slides, then spins"
        box.move_by(300, 0) over 1.second ease ease_in_out
        box.rotate_by(90.deg) over 1.second ease spring(2, 5)
"#;

/// Seconds without typing before the program is run again.
const DEBOUNCE: f64 = 0.35;

pub struct EditorApp {
    pub source: String,
    pub path: Option<PathBuf>,
    /// The text as it is on disk (or as created): the file is modified when `source` differs.
    pub saved: String,
    pub runner: Runner,
    /// The last finished run. It may be older than the text while a newer run is under way.
    pub out: Option<Arc<RunOutput>>,
    /// The run the preview, hierarchy, inspector and timeline show: the last one that made frames, so a
    /// half-typed line does not blank them. The console and the code panel always show `out`.
    pub shown: Option<Arc<RunOutput>>,
    pub auto_run: bool,
    /// When the text last changed and has not been submitted yet (seconds of egui time).
    pub edited_at: Option<f64>,
    pub tick: usize,
    pub playing: bool,
    pub looping: bool,
    play_clock: f64,
    pub selection: Selection,
    dock: Option<DockState<Tab>>,
    pub pending: Option<Pending>,
    pub message: String,
    pub code: panels::code::State,
    pub preview: panels::preview::State,
    pub hierarchy: panels::hierarchy::State,
    pub timeline: panels::timeline::State,
    pub console: panels::console::State,
    /// Set by tests: file dialogs are never opened.
    pub headless: bool,
}

fn default_dock() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Code]);
    let tree = dock.main_surface_mut();
    let [top, bottom] = tree.split_below(NodeIndex::root(), 0.66, vec![Tab::Timeline]);
    tree.split_right(bottom, 0.62, vec![Tab::Console]);
    let [code, _] = tree.split_left(top, 0.15, vec![Tab::Hierarchy]);
    let [_, preview] = tree.split_right(code, 0.42, vec![Tab::Preview]);
    tree.split_right(preview, 0.70, vec![Tab::Inspector]);
    dock
}

impl EditorApp {
    /// `wake` is called from the worker thread when a run finishes (ask for a repaint there).
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> EditorApp {
        let mut app = EditorApp {
            source: TEMPLATE.to_string(),
            path: None,
            saved: TEMPLATE.to_string(),
            runner: Runner::new(wake),
            out: None,
            shown: None,
            auto_run: true,
            edited_at: None,
            tick: 0,
            playing: false,
            looping: true,
            play_clock: 0.0,
            selection: Selection::None,
            dock: Some(default_dock()),
            pending: None,
            message: String::new(),
            code: Default::default(),
            preview: Default::default(),
            hierarchy: Default::default(),
            timeline: Default::default(),
            console: Default::default(),
            headless: false,
        };
        app.run_now();
        app
    }

    pub fn open_path(&mut self, path: &Path) {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                self.source = text.clone();
                self.saved = text;
                self.path = Some(path.to_path_buf());
                self.tick = 0;
                self.playing = false;
                self.shown = None;
                self.selection = Selection::None;
                self.message = format!("opened {}", path.display());
                self.run_now();
            }
            Err(e) => self.message = format!("cannot read {}: {e}", path.display()),
        }
    }

    pub fn new_file(&mut self) {
        self.source = TEMPLATE.to_string();
        self.saved = self.source.clone();
        self.path = None;
        self.tick = 0;
        self.playing = false;
        self.shown = None;
        self.selection = Selection::None;
        self.message = "new file".to_string();
        self.run_now();
    }

    /// Write the text to its file. Returns false when it has no file yet or writing failed.
    pub fn save(&mut self) -> bool {
        let Some(path) = self.path.clone() else {
            return false;
        };
        match std::fs::write(&path, &self.source) {
            Ok(()) => {
                self.saved = self.source.clone();
                self.message = format!("saved {}", path.display());
                true
            }
            Err(e) => {
                self.message = format!("cannot write {}: {e}", path.display());
                false
            }
        }
    }

    fn save_as(&mut self) {
        if self.headless {
            return;
        }
        let mut dialog = rfd::FileDialog::new().add_filter("AnimScript", &["anim"]);
        if let Some(dir) = self.path.as_ref().and_then(|p| p.parent()) {
            dialog = dialog.set_directory(dir);
        }
        if let Some(path) = dialog.set_file_name(self.file_name()).save_file() {
            self.path = Some(path);
            self.save();
        }
    }

    fn save_or_ask(&mut self) {
        if self.path.is_some() {
            self.save();
        } else {
            self.save_as();
        }
    }

    fn open_dialog(&mut self) {
        if self.headless {
            return;
        }
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("AnimScript", &["anim"])
            .pick_file()
        {
            self.open_path(&path);
        }
    }

    pub fn modified(&self) -> bool {
        self.source != self.saved
    }

    pub fn file_name(&self) -> String {
        self.path
            .as_ref()
            .and_then(|p| p.file_name())
            .map_or("untitled.anim".to_string(), |n| {
                n.to_string_lossy().into_owned()
            })
    }

    /// The folder relative asset paths are resolved from.
    pub fn base_dir(&self) -> PathBuf {
        self.path
            .as_ref()
            .and_then(|p| p.parent())
            .filter(|p| !p.as_os_str().is_empty())
            .map_or_else(|| PathBuf::from("."), |p| p.to_path_buf())
    }

    pub fn run_now(&mut self) {
        self.edited_at = None;
        let base = self.base_dir();
        self.runner
            .submit(&self.source, &base, self.console.show_trace);
    }

    /// The text was changed at egui time `now`.
    pub fn edited(&mut self, now: f64) {
        self.edited_at = Some(now);
    }

    pub fn last_tick(&self) -> usize {
        self.shown.as_ref().map_or(0, |o| o.last_tick())
    }

    /// The panels show an older run than the text: the newest one made no frames.
    pub fn stale(&self) -> bool {
        match (&self.out, &self.shown) {
            (Some(a), Some(b)) => a.generation != b.generation,
            _ => false,
        }
    }

    fn take(&mut self, out: Arc<RunOutput>) {
        if !out.snapshots.is_empty() || self.shown.is_none() {
            self.shown = Some(Arc::clone(&out));
        }
        self.out = Some(out);
        self.tick = self.tick.min(self.last_tick());
        if self.shown.as_ref().is_none_or(|o| o.snapshots.is_empty()) {
            self.playing = false;
        }
    }

    pub fn set_tick(&mut self, tick: i64) {
        self.tick = tick.clamp(0, self.last_tick() as i64) as usize;
    }

    pub fn select(&mut self, sel: Selection) {
        if let (Selection::Scene(i), Some(out)) = (&sel, &self.shown) {
            if let Some(s) = out.journal.scenes.get(*i) {
                self.tick = (s.start.max(0) as usize).min(out.last_tick());
            }
        }
        self.selection = sel;
    }

    /// Show `line` in the code panel.
    pub fn goto_line(&mut self, line: u32) {
        self.code.jump = Some(line);
        if let Some(dock) = &mut self.dock {
            if let Some(path) = dock.find_tab(&Tab::Code) {
                let _ = dock.set_active_tab(path);
            }
        }
    }

    /// Take a finished run, if there is one. Returns true when the panels have new data.
    pub fn poll(&mut self) -> bool {
        let Some(out) = self.runner.poll() else {
            return false;
        };
        self.take(out);
        true
    }

    /// Wait for the runs under way and take the result (tests).
    pub fn settle(&mut self) {
        if let Some(out) = self.runner.wait() {
            self.take(out);
        }
    }

    /// Move the playhead by `dt` seconds of playback.
    pub fn advance(&mut self, dt: f64) {
        let Some(out) = &self.shown else { return };
        if !self.playing || out.snapshots.is_empty() {
            return;
        }
        self.play_clock += dt * out.fps;
        let steps = self.play_clock.floor();
        self.play_clock -= steps;
        let (last, mut tick) = (out.last_tick(), self.tick + steps as usize);
        if tick > last {
            if self.looping {
                tick %= last + 1;
            } else {
                tick = last;
                self.playing = false;
            }
        }
        self.tick = tick;
    }

    pub fn toggle_play(&mut self) {
        self.playing =
            !self.playing && self.shown.as_ref().is_some_and(|o| !o.snapshots.is_empty());
        self.play_clock = 0.0;
        if self.playing && self.tick >= self.last_tick() {
            self.tick = 0;
        }
    }

    fn request(&mut self, what: Pending) {
        if self.modified() {
            self.pending = Some(what);
        } else {
            self.perform(what, None);
        }
    }

    fn perform(&mut self, what: Pending, ctx: Option<&egui::Context>) {
        match what {
            Pending::New => self.new_file(),
            Pending::Open => self.open_dialog(),
            Pending::Quit => {
                self.saved = self.source.clone(); // agreed to lose the changes: let the window close
                if let Some(ctx) = ctx {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        use egui::{Key, Modifiers};
        let (save, save_as, open, new, run) = ui.input_mut(|i| {
            (
                i.consume_key(Modifiers::COMMAND, Key::S),
                i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::S),
                i.consume_key(Modifiers::COMMAND, Key::O),
                i.consume_key(Modifiers::COMMAND, Key::N),
                i.consume_key(Modifiers::COMMAND, Key::Enter)
                    || i.consume_key(Modifiers::NONE, Key::F5),
            )
        });
        if save_as {
            self.save_as();
        } else if save {
            self.save_or_ask();
        }
        if open {
            self.request(Pending::Open);
        }
        if new {
            self.request(Pending::New);
        }
        if run {
            self.run_now();
        }
        // transport keys, unless something is being typed
        if ui.memory(|m| m.focused().is_none()) {
            let (space, left, right, home, end) = ui.input(|i| {
                (
                    i.key_pressed(Key::Space),
                    i.key_pressed(Key::ArrowLeft),
                    i.key_pressed(Key::ArrowRight),
                    i.key_pressed(Key::Home),
                    i.key_pressed(Key::End),
                )
            });
            if space {
                self.toggle_play();
            }
            let step = i64::from(right) - i64::from(left);
            if step != 0 {
                self.playing = false;
                self.set_tick(self.tick as i64 + step);
            }
            if home {
                self.set_tick(0);
            }
            if end {
                self.set_tick(i64::MAX);
            }
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("New").clicked() {
                    self.request(Pending::New);
                }
                if ui.button("Open...").clicked() {
                    self.request(Pending::Open);
                }
                if ui.button("Save").clicked() {
                    self.save_or_ask();
                }
                if ui.button("Save As...").clicked() {
                    self.save_as();
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    self.request(Pending::Quit);
                    if self.pending.is_none() {
                        ui.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                }
            });
            ui.menu_button("Run", |ui| {
                if ui.button("Run now").clicked() {
                    self.run_now();
                }
                ui.checkbox(&mut self.auto_run, "Run while typing");
                ui.separator();
                if ui
                    .button(if self.playing { "Pause" } else { "Play" })
                    .clicked()
                {
                    self.toggle_play();
                }
                ui.checkbox(&mut self.looping, "Loop playback");
            });
            ui.menu_button("View", |ui| {
                for tab in Tab::ALL {
                    let open = self
                        .dock
                        .as_ref()
                        .is_some_and(|d| d.find_tab(&tab).is_some());
                    if ui.selectable_label(open, tab.title()).clicked() && !open {
                        if let Some(d) = &mut self.dock {
                            d.push_to_focused_leaf(tab);
                        }
                    }
                }
                ui.separator();
                if ui.button("Reset layout").clicked() {
                    self.dock = Some(default_dock());
                }
            });
        });
    }

    fn status(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let name = self.file_name();
            ui.label(if self.modified() {
                format!("{name} (modified)")
            } else {
                name
            });
            ui.separator();
            let waiting = self.runner.busy() || self.edited_at.is_some();
            match &self.out {
                _ if waiting => {
                    ui.spinner();
                    ui.label("running");
                }
                Some(o) if o.errors() > 0 => {
                    ui.colored_label(panels::ERROR, format!("{} error(s)", o.errors()));
                }
                Some(o) => {
                    ui.label(format!("ok, {} ticks, {} ms", o.snapshots.len(), o.millis));
                }
                None => {}
            }
            if !self.message.is_empty() {
                ui.separator();
                ui.weak(&self.message);
            }
        });
    }

    fn confirm(&mut self, ctx: &egui::Context) {
        let Some(what) = self.pending else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::new("discard")).show(ctx, |ui| {
            ui.heading("Unsaved changes");
            ui.label(format!(
                "{} has changes that are not saved.",
                self.file_name()
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Discard changes").clicked() {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        match choice {
            Some(true) => {
                self.pending = None;
                self.saved = self.source.clone();
                self.perform(what, Some(ctx));
            }
            Some(false) => self.pending = None,
            None => {}
        }
    }

    /// One frame of the whole editor.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let now = ui.input(|i| i.time);
        let before = (self.tick, self.selection.clone());
        if self.poll() {
            ctx.request_repaint();
        }
        if let Some(at) = self.edited_at {
            if !self.auto_run {
                self.edited_at = None;
            } else if now - at >= DEBOUNCE {
                self.run_now();
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(
                    DEBOUNCE - (now - at),
                ));
            }
        }
        if self.playing {
            let dt = ui.input(|i| i.stable_dt) as f64;
            self.advance(dt.min(0.25));
            ctx.request_repaint();
        }
        // closing the window with unsaved text asks first
        if ui.input(|i| i.viewport().close_requested()) && self.modified() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(Pending::Quit);
        }
        self.shortcuts(ui);

        egui::Panel::top("menu").show(ui, |ui| self.menu(ui));
        egui::Panel::bottom("status").show(ui, |ui| self.status(ui));
        let mut dock = self.dock.take().unwrap_or_else(default_dock);
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ui, |ui| {
                DockArea::new(&mut dock)
                    .style(Style::from_egui(ui.style().as_ref()))
                    .show_leaf_collapse_buttons(false)
                    .show_leaf_close_all_buttons(false)
                    .show_inside(ui, &mut Viewer { app: self });
            });
        // a panel may have asked for the code tab (`goto_line`) while the dock was lent out
        if self.dock.is_none() {
            if self.code.jump.is_some() {
                if let Some(path) = dock.find_tab(&Tab::Code) {
                    let _ = dock.set_active_tab(path);
                }
            }
            self.dock = Some(dock);
        }
        self.confirm(&ctx);
        // a panel drawn early in the frame may be showing what a later one just changed: draw again
        if self.code.jump.is_some() || before != (self.tick, self.selection.clone()) {
            ctx.request_repaint();
        }
    }
}

struct Viewer<'a> {
    app: &'a mut EditorApp,
}

impl egui_dock::TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn title(&mut self, tab: &mut Tab) -> egui::WidgetText {
        tab.title().into()
    }
    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(("tab", tab.title()))
    }
    fn scroll_bars(&self, _tab: &Tab) -> [bool; 2] {
        [false, false] // every panel scrolls its own content
    }
    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        match tab {
            Tab::Code => panels::code::show(self.app, ui),
            Tab::Preview => panels::preview::show(self.app, ui),
            Tab::Hierarchy => panels::hierarchy::show(self.app, ui),
            Tab::Inspector => panels::inspector::show(self.app, ui),
            Tab::Timeline => panels::timeline::show(self.app, ui),
            Tab::Console => panels::console::show(self.app, ui),
        }
    }
}

impl eframe::App for EditorApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}
