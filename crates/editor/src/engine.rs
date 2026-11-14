//! Running a program for the editor: check it, run it, keep whatever frames it made. Runs happen on a worker
//! thread so typing never waits for a simulation.
use animscript_core::Diagnostic;
use animscript_lang::journal::Journal;
use animscript_lang::{check_in, run_partial_in, RunOptions};
use serde_json::Value as J;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

/// Everything one run produced.
pub struct RunOutput {
    /// Which edit of the source this belongs to (see `Runner::submit`).
    pub generation: u64,
    pub source: String,
    pub snapshots: Vec<J>,
    pub journal: Journal,
    /// Errors and warnings of the check and of the run, without repeats, in source order.
    pub diagnostics: Vec<Diagnostic>,
    pub trace: Vec<String>,
    pub fps: f64,
    pub canvas: (u32, u32),
    pub millis: u128,
}

impl RunOutput {
    pub fn errors(&self) -> usize {
        self.diagnostics.iter().filter(|d| d.is_error()).count()
    }
    pub fn warnings(&self) -> usize {
        self.diagnostics.len() - self.errors()
    }
    pub fn last_tick(&self) -> usize {
        self.snapshots.len().saturating_sub(1)
    }
}

fn same(a: &Diagnostic, b: &Diagnostic) -> bool {
    a.code == b.code
        && a.message == b.message
        && a.span == b.span
        && a.file.as_ref().map(|f| &f.0) == b.file.as_ref().map(|f| &f.0)
}

fn project_of(src: &str, base: &Path) -> (f64, (u32, u32)) {
    let parsed = animscript_lang::imports::expand_imports(src, base)
        .and_then(|s| animscript_lang::parser::parse(&s));
    match parsed.ok().and_then(|p| p.project) {
        Some(p) if p.fps.0 > 0 && p.fps.1 > 0 => (p.fps.0 as f64 / p.fps.1 as f64, p.canvas),
        Some(p) => (24.0, p.canvas),
        None => (24.0, (800, 600)),
    }
}

/// Check and run `src` (relative asset paths resolve from `base`). Never fails: a program that does not
/// build gives no frames and its diagnostics.
pub fn run_source(src: &str, base: &Path, trace: bool, generation: u64) -> RunOutput {
    let started = std::time::Instant::now();
    let mut diagnostics = check_in(src, base);
    let add = |d: Diagnostic, all: &mut Vec<Diagnostic>| {
        if !all.iter().any(|x| same(x, &d)) {
            all.push(d);
        }
    };
    let (fps, canvas) = project_of(src, base);
    let opts = RunOptions {
        trace,
        ..Default::default()
    };
    let (snapshots, journal, trace) = match run_partial_in(src, &opts, base) {
        Ok((r, err)) => {
            for d in r.diagnostics {
                add(d, &mut diagnostics);
            }
            if let Some(e) = err {
                add(e, &mut diagnostics);
            }
            (r.snapshots, r.journal, r.trace)
        }
        Err(e) => {
            add(e, &mut diagnostics);
            (vec![], Journal::default(), vec![])
        }
    };
    // imported text is appended after the program, and "end of file" errors point there: show them on the
    // last line of what the user sees
    // a problem inside a part file has no line in this one: it is listed (with its file and line) in the console
    for d in &mut diagnostics {
        if let Some((file, _)) = d.file.take() {
            let line = d.span.take().map_or(0, |s| s.line);
            d.message = format!("{} (in {file}, line {line})", d.message);
        }
    }
    let lines: Vec<&str> = src.lines().collect();
    let last = lines.len().max(1) as u32;
    for d in &mut diagnostics {
        if let Some(sp) = d.span.filter(|s| s.line > last) {
            let width = lines.last().map_or(0, |l| l.chars().count()) as u32;
            d.span = Some(animscript_core::Span::new(last, 1, width + 1));
            d.notes.push(format!(
                "reported at line {} of the program with its imports",
                sp.line
            ));
        }
    }
    diagnostics.sort_by_key(|d| (d.span.map_or(u32::MAX, |s| s.line), !d.is_error()));
    RunOutput {
        generation,
        source: src.to_string(),
        snapshots,
        journal,
        diagnostics,
        trace,
        fps,
        canvas,
        millis: started.elapsed().as_millis(),
    }
}

struct Job {
    generation: u64,
    source: String,
    base: PathBuf,
    trace: bool,
}

/// Runs programs on a worker thread, one at a time. While a run is going, only the newest submitted source
/// is kept: the ones in between are never run.
pub struct Runner {
    tx: Sender<Arc<RunOutput>>,
    rx: Receiver<Arc<RunOutput>>,
    busy: bool,
    queued: Option<Job>,
    next: u64,
    /// Called from the worker when a result is ready (the editor asks for a repaint here).
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Runner {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Runner {
        let (tx, rx) = channel();
        Runner {
            tx,
            rx,
            busy: false,
            queued: None,
            next: 0,
            wake: Arc::new(wake),
        }
    }

    pub fn busy(&self) -> bool {
        self.busy
    }

    /// Ask for a run of `source`; returns its generation number.
    pub fn submit(&mut self, source: &str, base: &Path, trace: bool) -> u64 {
        self.next += 1;
        let job = Job {
            generation: self.next,
            source: source.to_string(),
            base: base.to_path_buf(),
            trace,
        };
        if self.busy {
            self.queued = Some(job);
        } else {
            self.spawn(job);
        }
        self.next
    }

    fn spawn(&mut self, job: Job) {
        self.busy = true;
        let (tx, wake) = (self.tx.clone(), Arc::clone(&self.wake));
        std::thread::spawn(move || {
            let out = run_source(&job.source, &job.base, job.trace, job.generation);
            let _ = tx.send(Arc::new(out));
            wake();
        });
    }

    /// The result of a finished run, if there is one. Starts the queued run, if any.
    pub fn poll(&mut self) -> Option<Arc<RunOutput>> {
        let out = self.rx.try_recv().ok()?;
        self.busy = false;
        if let Some(job) = self.queued.take() {
            self.spawn(job);
        }
        Some(out)
    }

    /// Wait for the run in progress (tests).
    pub fn wait(&mut self) -> Option<Arc<RunOutput>> {
        let mut last = None;
        while self.busy {
            let out = self.rx.recv().ok()?;
            self.busy = false;
            if let Some(job) = self.queued.take() {
                self.spawn(job);
            }
            last = Some(out);
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const OK: &str = "project \"t\":\n    anim_version: \"0.1\"\n    canvas: 320x200\n    fps: 12\n\nshape s = square(size: 50)\nscene \"a\" for 1.second:\n    main:\n        s.move_by(10, 0) over 6.frames\n";

    #[test]
    fn a_good_program_gives_frames_and_project_settings() {
        let out = run_source(OK, Path::new("."), false, 7);
        assert_eq!(out.generation, 7);
        assert_eq!(out.snapshots.len(), 12);
        assert_eq!(out.fps, 12.0);
        assert_eq!(out.canvas, (320, 200));
        assert_eq!(out.errors(), 0);
        assert_eq!(out.journal.items.len(), 1);
    }

    #[test]
    fn a_syntax_error_is_reported_once_with_no_frames() {
        let out = run_source("shape s = = 3\n", Path::new("."), false, 1);
        assert!(out.snapshots.is_empty());
        assert_eq!(out.errors(), 1, "{:?}", out.diagnostics);
        assert!(out.diagnostics[0].span.is_some());
    }

    #[test]
    fn an_error_past_the_end_of_the_file_lands_on_its_last_line() {
        let src = format!("{OK}import \"std/motion\"\nlet x = (1,\n");
        let out = run_source(&src, Path::new("."), false, 1);
        assert!(out.errors() >= 1);
        let lines = src.lines().count() as u32;
        assert!(
            out.diagnostics
                .iter()
                .all(|d| d.span.is_none_or(|s| s.line <= lines)),
            "{:?}",
            out.diagnostics
        );
    }

    #[test]
    fn a_runtime_error_keeps_earlier_frames() {
        let src = format!("{OK}        assert 1 == 2\n");
        let out = run_source(&src, Path::new("."), false, 1);
        assert!(out.errors() >= 1);
        assert!(!out.snapshots.is_empty() && out.snapshots.len() < 12);
    }

    #[test]
    fn the_runner_only_runs_the_newest_queued_source() {
        let mut r = Runner::new(|| {});
        r.submit(OK, Path::new("."), false);
        r.submit("shape broken = = 1\n", Path::new("."), false);
        let last = r.submit(OK, Path::new("."), false);
        let out = r.wait().unwrap();
        assert_eq!(out.generation, last);
        assert_eq!(out.snapshots.len(), 12);
        assert!(!r.busy());
    }
}
