//! The `anim` command line: check, run, render.
use animscript_lang::{check, run, RunOptions};
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
            let diags = check(&src);
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
            match run(&src, &RunOptions { trace }) {
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
            let res = match run(&src, &RunOptions::default()) {
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
            match animscript_render::render_png_with(snap, &out, scale, debug, debug_physics) {
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
