# CLAUDE.md

Project guidance for Claude Code in this repository.

## About
AnimScript is a domain-specific language for describing animated scenes. The language
specification lives in `AnimScript_Language_Spec_v0.1.pdf`; the compiler/runtime is
implemented in **Rust**.

## Working agreement

### Plan first for any multistep process (mandatory)
For any task that involves more than one step (multiple files, several operations, a
refactor, a new feature, a migration, etc.):

1. **Submit a plan first.** List the steps, the files/modules affected, the approach,
   and anything you are assuming. Do not start implementing yet.
2. **Ask questions** about anything ambiguous or any decision that is the user's to make.
3. **Wait for explicit approval** before writing code or changing files.
4. Only then go forward, following the approved plan. If the plan has to change
   materially mid-way, stop and re-submit it for approval.

Trivial single-step edits (e.g. fixing a typo) do not need a plan.

### Other rules
- Record every ambiguity resolved while implementing the spec in `docs/DECISIONS.md`.
- Tests are written first; every spec rule should be expressible as a test.
- Do not commit, push, or open pull requests unless asked.

## Tech stack
- Language: Rust (stable toolchain). Compiler, runtime and CLI are Rust crates in a Cargo workspace.
- Prefer the standard library and a small number of well-known crates; justify each new dependency in the plan.
- Determinism is a core requirement (see spec §1.2, §6.11): no hidden inputs, no unseeded randomness.

## Layout
- `crates/core`: diagnostics and the editable shape model (`geometry.rs`). No parsing.
- `crates/lang`: lexer, parser, interpreter, tick loop, snapshots, `check`.
- `crates/render`: snapshot to PNG (`tiny-skia`). Pure function of the snapshot.
- `crates/anim`: the `anim` CLI (`check`, `run`, `render`).
- `docs/DECISIONS.md`: every spec ambiguity resolved so far. `DOCUMENT.md`: the user-facing syntax guide.
- Golden snapshot: `UPDATE_GOLDEN=1 cargo test -p animscript-lang --test flower` regenerates it.

## Commands
- Build: `cargo build`
- Test: `cargo test`
- Lint/format: `cargo clippy --all-targets` and `cargo fmt`
