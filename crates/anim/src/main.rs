//! The `anim` command line: check, run, render.
use animscript_lang::{check_in, run_in, RunOptions};
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "anim", about = "AnimScript compiler and runtime")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Parse, check and dry-run setup; print diagnostics. Exit code 1 on errors.
    Check { file: PathBuf },
    /// Run the simulation without rendering.
    Run {
        file: PathBuf,
        #[arg(long)]
        trace: bool,
        /// Print the snapshot hash of every tick.
        #[arg(long)]
        hash: bool,
    },
    /// Render every tick to PNGs `f0000.png`, `f0001.png`... in a directory, simulating only once
    /// (much faster than calling `render` per tick for physics scenes).
    Frames {
        file: PathBuf,
        /// Output directory (created if missing).
        #[arg(short, long)]
        out: PathBuf,
        #[arg(long, default_value_t = 1.0)]
        scale: f32,
        /// First and last tick to write (inclusive); default is all.
        #[arg(long)]
        from: Option<usize>,
        #[arg(long)]
        to: Option<usize>,
        /// Draw physics contacts, joints and soft pins.
        #[arg(long)]
        debug_physics: bool,
    },
    /// Render one tick to a PNG.
    Render {
        file: PathBuf,
        #[arg(short, long)]
        out: PathBuf,
        /// Tick to render (default: the last one).
        #[arg(long)]
        tick: Option<usize>,
        #[arg(long, default_value_t = 2.0)]
        scale: f32,
        /// Draw a dot on every vertex.
        #[arg(long)]
        debug: bool,
        /// Draw physics contacts (red, with normals) and joints (blue).
        #[arg(long)]
        debug_physics: bool,
    },
}

/// The folder a program lives in: relative asset paths (images) are resolved from here.
fn base_of(file: &std::path::Path) -> PathBuf {
    file.parent()
        .map(|p| p.to_path_buf())
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("."))
}

fn read(file: &PathBuf) -> Result<String, ExitCode> {
    std::fs::read_to_string(file).map_err(|e| {
        eprintln!("cannot read {}: {e}", file.display());
        ExitCode::from(2)
    })
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Check { file } => {
            let src = match read(&file) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let name = file.display().to_string();
            let diags = check_in(&src, &base_of(&file));
            for d in &diags {
                eprintln!("{}", d.render(Some(&src), &name));
            }
            let errs = diags.iter().filter(|d| d.is_error()).count();
            println!("{name}: {errs} error(s), {} warning(s)", diags.len() - errs);
            if errs > 0 {
                ExitCode::from(1)
            } else {
                ExitCode::SUCCESS
            }
        }
        Cmd::Run { file, trace, hash } => {
            let src = match read(&file) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let name = file.display().to_string();
            match run_in(&src, &RunOptions { trace }, &base_of(&file)) {
                Ok(r) => {
                    for l in &r.trace {
                        println!("{l}");
                    }
                    if hash {
                        for (i, h) in r.hashes.iter().enumerate() {
                            println!("tick {i:04} {h:016x}");
                        }
                    }
                    for d in &r.diagnostics {
                        eprintln!("{}", d.render(Some(&src), &name));
                    }
                    println!("{} ticks", r.snapshots.len());
                    ExitCode::SUCCESS
                }
                Err(d) => {
                    eprint!("{}", d.render(Some(&src), &name));
                    ExitCode::from(1)
                }
            }
        }
        Cmd::Frames {
            file,
            out,
            scale,
            from,
            to,
            debug_physics,
        } => {
            let src = match read(&file) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let name = file.display().to_string();
            let res = match run_in(&src, &RunOptions::default(), &base_of(&file)) {
                Ok(r) => r,
                Err(d) => {
                    eprint!("{}", d.render(Some(&src), &name));
                    return ExitCode::from(1);
                }
            };
            if let Err(e) = std::fs::create_dir_all(&out) {
                eprintln!("cannot create {}: {e}", out.display());
                return ExitCode::from(2);
            }
            let last = res.snapshots.len().saturating_sub(1);
            let (a, b) = (from.unwrap_or(0), to.unwrap_or(last).min(last));
            for t in a..=b {
                let path = out.join(format!("f{t:04}.png"));
                if let Err(e) = animscript_render::render_png_assets(
                    &res.snapshots[t],
                    &path,
                    scale,
                    false,
                    debug_physics,
                    &base_of(&file),
                ) {
                    eprintln!("render failed at tick {t}: {e}");
                    return ExitCode::from(1);
                }
            }
            println!("wrote {} frames to {}", b + 1 - a, out.display());
            ExitCode::SUCCESS
        }
        Cmd::Render {
            file,
            out,
            tick,
            scale,
            debug,
            debug_physics,
        } => {
            let src = match read(&file) {
                Ok(s) => s,
                Err(c) => return c,
            };
            let name = file.display().to_string();
            let res = match run_in(&src, &RunOptions::default(), &base_of(&file)) {
                Ok(r) => r,
                Err(d) => {
                    eprint!("{}", d.render(Some(&src), &name));
                    return ExitCode::from(1);
                }
            };
            let Some(snap) = tick.map_or(res.snapshots.last(), |t| res.snapshots.get(t)) else {
                eprintln!(
                    "the program produced no frame at that tick ({} ticks)",
                    res.snapshots.len()
                );
                return ExitCode::from(1);
            };
            match animscript_render::render_png_assets(
                snap,
                &out,
                scale,
                debug,
                debug_physics,
                &base_of(&file),
            ) {
                Ok(()) => {
                    println!("wrote {}", out.display());
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("render failed: {e}");
                    ExitCode::from(1)
                }
            }
        }
    }
}
