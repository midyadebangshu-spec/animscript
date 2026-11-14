//! `import "file.anim"` and `import "std/name"`: textual inclusion done before parsing. The import line
//! itself is blanked (so line numbers in the importing file never change) and the imported text is appended
//! to the end of the source (definitions are available everywhere, wherever they are). Each file is
//! included once, so import cycles are harmless. `std/name` is a library bundled in the compiler (the same
//! files live in `crates/lang/lib/`); anything else is a path relative to the importing file's folder
//! (the `.anim` extension is optional, folders are fine: `import "parts/head"`).
//!
//! Parts: `import "head" as h` puts the file's top-level names (`shape`, `let`, `param`, `armature`, `field`,
//! `emitter`, `fluid`, `score`) in a namespace: inside the part they are renamed `h__name` and the importer
//! writes `h.name`. `import "arm" as left(side: "left")` instances the file with its `param`s set, so one
//! file can be used twice. An imported file's `project` and top-level `scene` blocks are dropped (they only
//! run when the file is opened directly), and its top-level statements run *before* the importer's (see
//! `arrange`). `Expanded::remap` points a diagnostic back at the part file and its real line.
use animscript_core::{Diagnostic, Res, Span};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const STD: &[(&str, &str)] = &[
    ("motion", include_str!("../lib/motion.anim")),
    ("gait", include_str!("../lib/gait.anim")),
    ("2d", include_str!("../lib/2d.anim")),
    ("waves", include_str!("../lib/waves.anim")),
    ("3d", include_str!("../lib/3d.anim")),
];

/// The text of a bundled library module (`motion`, not `std/motion`).
pub fn bundled(name: &str) -> Option<&'static str> {
    STD.iter().find(|(n, _)| *n == name).map(|(_, t)| *t)
}

/// One imported file's place in the expanded text.
#[derive(Clone, Debug)]
pub struct Segment {
    start: u32,
    len: u32,
    file: String,
    text: String,
}

/// The expanded source and where each appended part came from.
#[derive(Clone, Debug, Default)]
pub struct Expanded {
    pub text: String,
    /// Lines of the importing file itself (imported text starts on the next line).
    pub own_lines: u32,
    segments: Vec<Segment>,
}

impl Expanded {
    /// Re-point a diagnostic that lies in an imported file at that file and its real line.
    pub fn remap(&self, mut d: Diagnostic) -> Diagnostic {
        if d.file.is_some() {
            return d;
        }
        if let Some(sp) = d.span {
            if sp.line > self.own_lines {
                if let Some(seg) = self
                    .segments
                    .iter()
                    .find(|g| sp.line >= g.start && sp.line < g.start + g.len)
                {
                    d.span = Some(Span::new(sp.line - seg.start + 1, sp.col, sp.end_col));
                    d.file = Some((seg.file.clone(), seg.text.clone()));
                }
            }
        }
        d
    }
}

/// Statements that came from imported files run before the importing file's own (so a part can define
/// what the main file then uses). The order inside each group is kept.
pub fn arrange(prog: &mut crate::ast::Program, own_lines: u32) {
    let (own, imported): (Vec<_>, Vec<_>) = std::mem::take(&mut prog.top)
        .into_iter()
        .partition(|s| s.span.line <= own_lines);
    prog.top = imported;
    prog.top.extend(own);
}

pub fn expand_imports(src: &str, base: &Path) -> Res<String> {
    expand_full(src, base).map(|e| e.text)
}

pub fn expand_full(src: &str, base: &Path) -> Res<Expanded> {
    let mut cx = Cx {
        seen: BTreeSet::new(),
        appended: String::new(),
        lines: 0,
        segments: vec![],
        own_lines: src.lines().count() as u32,
        root: base.to_path_buf(),
    };
    let main = expand(&mut cx, src, base, "", &BTreeMap::new(), true)?;
    let text = if cx.appended.is_empty() {
        main
    } else {
        format!("{main}\n{}", cx.appended)
    };
    Ok(Expanded {
        text,
        own_lines: cx.own_lines,
        segments: cx.segments,
    })
}

struct Cx {
    seen: BTreeSet<String>,
    appended: String,
    lines: u32,
    segments: Vec<Segment>,
    own_lines: u32,
    root: PathBuf,
}

struct Alias {
    name: String,
    prefix: String,
    declared: BTreeSet<String>,
}

const DECLS: &[&str] = &[
    "shape", "let", "param", "armature", "field", "emitter", "fluid", "score",
];

/// The names a file declares at its top level, and its `param` lines (line index, name).
fn scan_decls(text: &str) -> (BTreeSet<String>, Vec<(usize, String)>) {
    let mut names = BTreeSet::new();
    let mut params = vec![];
    for (i, line) in text.lines().enumerate() {
        for kw in DECLS {
            let Some(rest) = line.strip_prefix(kw).and_then(|r| r.strip_prefix(' ')) else {
                continue;
            };
            let name: String = rest
                .trim_start()
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
                continue;
            }
            if *kw == "param" {
                params.push((i, name.clone()));
            }
            names.insert(name);
        }
    }
    (names, params)
}

/// `import "path" [as name[(k: v, ...)]]` after the `import ` keyword.
type ImportLine = (String, Option<String>, Vec<(String, String)>);

fn parse_import(rest: &str, at: Span) -> Res<ImportLine> {
    let bad = || {
        Diagnostic::new(
            "E-SYN-001",
            "`import` needs a quoted path, such as `import \"std/motion\"` or `import \"parts/head\" as head`",
        )
        .at(at)
    };
    let rest = rest.trim();
    let r = rest.strip_prefix('"').ok_or_else(bad)?;
    let end = r.find('"').ok_or_else(bad)?;
    let path = r[..end].to_string();
    let tail = r[end + 1..].trim();
    if tail.is_empty() {
        return Ok((path, None, vec![]));
    }
    let tail = tail.strip_prefix("as").ok_or_else(bad)?.trim_start();
    let name: String = tail
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    if name.is_empty() {
        return Err(bad());
    }
    let after = tail[name.len()..].trim();
    if after.is_empty() {
        return Ok((path, Some(name), vec![]));
    }
    let inner = after
        .strip_prefix('(')
        .and_then(|a| a.strip_suffix(')'))
        .ok_or_else(bad)?;
    let mut args = vec![];
    for part in split_top(inner) {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (k, v) = part.split_once(':').ok_or_else(bad)?;
        args.push((k.trim().to_string(), v.trim().to_string()));
    }
    Ok((path, Some(name), args))
}

/// Split at commas that are not inside brackets or strings.
fn split_top(s: &str) -> Vec<String> {
    let (mut out, mut cur, mut depth, mut in_str) = (vec![], String::new(), 0i32, false);
    for c in s.chars() {
        match c {
            '"' => in_str = !in_str,
            '(' | '[' | '{' if !in_str => depth += 1,
            ')' | ']' | '}' if !in_str => depth -= 1,
            ',' if !in_str && depth == 0 => {
                out.push(std::mem::take(&mut cur));
                continue;
            }
            _ => {}
        }
        cur.push(c);
    }
    out.push(cur);
    out
}

fn is_color_at(c: &[char], i: usize) -> bool {
    let n = c[i + 1..]
        .iter()
        .take_while(|ch| ch.is_ascii_hexdigit())
        .count();
    matches!(n, 3 | 4 | 6 | 8)
        && !c
            .get(i + 1 + n)
            .is_some_and(|ch| ch.is_alphanumeric() || *ch == '_')
}

#[derive(PartialEq)]
enum Prev {
    Line,
    Open,
    Dot,
    Word(String),
    Other,
}

/// Rename this file's own declarations to `prefix__name` and `alias.name` to the alias's mangled name.
/// Strings, comments, `.member` accesses and `key:` argument names are left alone.
fn rewrite(text: &str, own: Option<(&str, &BTreeSet<String>)>, aliases: &[Alias]) -> String {
    let cs: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len() + 64);
    let (mut i, mut depth) = (0usize, 0i32);
    let mut prev = Prev::Line;
    let word = |c: char| c.is_alphanumeric() || c == '_';
    while i < cs.len() {
        let c = cs[i];
        if c == '\n' {
            out.push(c);
            prev = Prev::Line;
            i += 1;
        } else if c.is_whitespace() {
            out.push(c);
            i += 1;
        } else if c == '"' {
            out.push(c);
            i += 1;
            while i < cs.len() && cs[i] != '"' && cs[i] != '\n' {
                if cs[i] == '\\' && i + 1 < cs.len() {
                    out.push(cs[i]);
                    i += 1;
                }
                out.push(cs[i]);
                i += 1;
            }
            if i < cs.len() && cs[i] == '"' {
                out.push('"');
                i += 1;
            }
            prev = Prev::Other;
        } else if c == '#' && !is_color_at(&cs, i) {
            while i < cs.len() && cs[i] != '\n' {
                out.push(cs[i]);
                i += 1;
            }
        } else if c == '#' {
            out.push(c);
            i += 1;
            while i < cs.len() && word(cs[i]) {
                out.push(cs[i]);
                i += 1;
            }
            prev = Prev::Other;
        } else if c.is_ascii_digit() {
            while i < cs.len()
                && (word(cs[i])
                    || (cs[i] == '.' && cs.get(i + 1).is_some_and(|d| d.is_ascii_digit())))
            {
                out.push(cs[i]);
                i += 1;
            }
            prev = Prev::Other;
        } else if word(c) {
            let start = i;
            while i < cs.len() && word(cs[i]) {
                i += 1;
            }
            let id: String = cs[start..i].iter().collect();
            let after = cs[i..].iter().copied().find(|ch| *ch != ' ' && *ch != '\t');
            let after2 = cs[i..]
                .iter()
                .copied()
                .filter(|ch| *ch != ' ' && *ch != '\t')
                .nth(1);
            let mut emitted = false;
            if prev != Prev::Dot {
                if let Some(a) = aliases.iter().find(|a| a.name == id) {
                    if cs.get(i) == Some(&'.') {
                        let s2 = i + 1;
                        let mut e2 = s2;
                        while e2 < cs.len() && word(cs[e2]) {
                            e2 += 1;
                        }
                        let member: String = cs[s2..e2].iter().collect();
                        if a.declared.contains(&member) {
                            out.push_str(&format!("{}__{member}", a.prefix));
                            i = e2;
                            prev = Prev::Word(member);
                            continue;
                        }
                    }
                }
                if let Some((pre, declared)) = own {
                    if declared.contains(&id) {
                        let key_pos = after == Some(':')
                            && after2 != Some(':')
                            && matches!(prev, Prev::Line | Prev::Open)
                            && !matches!(prev, Prev::Word(_));
                        if !key_pos {
                            out.push_str(&format!("{pre}__{id}"));
                            emitted = true;
                        }
                    }
                }
            }
            if !emitted {
                out.push_str(&id);
            }
            prev = Prev::Word(id);
        } else {
            out.push(c);
            i += 1;
            match c {
                '(' | '[' | '{' => {
                    depth += 1;
                    prev = Prev::Open;
                }
                ')' | ']' | '}' => {
                    depth -= 1;
                    prev = Prev::Other;
                }
                ',' => prev = Prev::Open,
                '.' => prev = Prev::Dot,
                _ => prev = Prev::Other,
            }
        }
    }
    let _ = depth;
    out
}

/// A `preview:` block (a part's own lights, camera and so on): dropped when the file is imported, and when the file
/// is the one being run its lines are moved out to the top level (the line `preview:` goes blank, the block loses
/// one level of indent), so line numbers never change.
fn preview_blocks(lines: &mut [String], keep: bool) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].trim_end() == "preview:" {
            lines[i].clear();
            i += 1;
            while i < lines.len()
                && (lines[i].trim().is_empty()
                    || lines[i].starts_with(' ')
                    || lines[i].starts_with('\t'))
            {
                if !keep {
                    lines[i].clear();
                } else if let Some(rest) = lines[i].strip_prefix("    ") {
                    lines[i] = rest.to_string();
                }
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

/// Blank the top-level `project` and `scene` blocks (a part's own preview).
fn strip_blocks(lines: &mut [String]) {
    let mut i = 0;
    while i < lines.len() {
        let l = &lines[i];
        let starts = |kw: &str| {
            l.strip_prefix(kw)
                .is_some_and(|r| r.starts_with(' ') || r.starts_with('"') || r.starts_with(':'))
        };
        if starts("project") || starts("scene") {
            lines[i].clear();
            i += 1;
            while i < lines.len()
                && (lines[i].trim().is_empty()
                    || lines[i].starts_with(' ')
                    || lines[i].starts_with('\t'))
            {
                lines[i].clear();
                i += 1;
            }
        } else {
            i += 1;
        }
    }
}

fn find_file(dir: &Path, path: &str) -> Option<PathBuf> {
    let full = dir.join(path);
    if full.is_file() {
        return Some(full);
    }
    let with = dir.join(format!("{path}.anim"));
    with.is_file().then_some(with)
}

fn attach(mut d: Diagnostic, file: &str, text: &str) -> Diagnostic {
    if d.file.is_none() {
        d.file = Some((file.to_string(), text.to_string()));
    }
    d
}

fn expand(
    cx: &mut Cx,
    src: &str,
    dir: &Path,
    prefix: &str,
    args: &BTreeMap<String, String>,
    is_main: bool,
) -> Res<String> {
    let (declared, params) = scan_decls(src);
    let mut aliases: Vec<Alias> = vec![];
    let mut out: Vec<String> = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let Some(rest) = line.strip_prefix("import ") else {
            out.push(line.to_string());
            continue;
        };
        let at = Span::new(i as u32 + 1, 1, line.chars().count() as u32 + 1);
        let (path, alias, raw_args) = parse_import(rest, at)?;
        out.push(String::new()); // keep the line numbering
        if alias.is_none() && !raw_args.is_empty() {
            return Err(Diagnostic::new(
                "E-SYN-001",
                "settings need a name: `import \"file\" as name(key: value)`",
            )
            .at(at));
        }
        let (key, text, sub_dir, shown): (String, String, PathBuf, String) = if let Some(name) =
            path.strip_prefix("std/")
        {
            let name = name.trim_end_matches(".anim");
            match bundled(name) {
                Some(t) => (
                    format!("std/{name}"),
                    t.to_string(),
                    dir.to_path_buf(),
                    format!("std/{name}"),
                ),
                None => {
                    let known: Vec<&str> = STD.iter().map(|(n, _)| *n).collect();
                    return Err(Diagnostic::new(
                        "E-AST-001",
                        format!("there is no bundled library `std/{name}`"),
                    )
                    .at(at)
                    .help(format!(
                        "bundled libraries: {}",
                        known
                            .iter()
                            .map(|k| format!("std/{k}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )));
                }
            }
        } else {
            let full = find_file(dir, &path).ok_or_else(|| {
                    Diagnostic::new("E-AST-001", format!("imported file `{path}` was not found"))
                        .at(at)
                        .help(format!(
                            "paths are relative to the importing file's folder ({}); `.anim` may be left out",
                            dir.display()
                        ))
                })?;
            let text = std::fs::read_to_string(&full).map_err(|_| {
                Diagnostic::new(
                    "E-AST-001",
                    format!("imported file `{path}` could not be read"),
                )
                .at(at)
            })?;
            let key = full
                .canonicalize()
                .unwrap_or_else(|_| full.clone())
                .display()
                .to_string();
            let sub = full
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| dir.to_path_buf());
            let shown = full
                .strip_prefix(&cx.root)
                .unwrap_or(&full)
                .display()
                .to_string();
            (key, text, sub, shown)
        };
        let child_prefix = match &alias {
            Some(a) if prefix.is_empty() => a.clone(),
            Some(a) => format!("{prefix}__{a}"),
            None => String::new(),
        };
        let (child_decl, child_params) = scan_decls(&text);
        if let Some(a) = &alias {
            aliases.push(Alias {
                name: a.clone(),
                prefix: child_prefix.clone(),
                declared: child_decl.clone(),
            });
        }
        let seen_key = format!("{key}\0{child_prefix}");
        if !cx.seen.insert(seen_key) {
            continue;
        }
        // The settings are written in this file's terms, so they are renamed here, not in the part.
        let own = (!prefix.is_empty()).then_some((prefix, &declared));
        let mut child_args = BTreeMap::new();
        for (k, v) in &raw_args {
            if !child_params.iter().any(|(_, n)| n == k) {
                let names: Vec<&str> = child_params.iter().map(|(_, n)| n.as_str()).collect();
                let mut d =
                    Diagnostic::new("E-AST-001", format!("`{path}` has no `param {k}`")).at(at);
                if !names.is_empty() {
                    d = d.help(format!("its params are: {}", names.join(", ")));
                }
                return Err(d);
            }
            child_args.insert(k.clone(), rewrite(v, own, &aliases));
        }
        let body = expand(cx, &text, &sub_dir, &child_prefix, &child_args, false)
            .map_err(|d| attach(d, &shown, &text))?;
        let n = (body.lines().count() as u32).max(1);
        cx.segments.push(Segment {
            start: cx.own_lines + 1 + cx.lines,
            len: n,
            file: shown,
            text,
        });
        cx.lines += n;
        cx.appended.push_str(&body);
        cx.appended.push('\n');
    }
    preview_blocks(&mut out, is_main);
    if !is_main {
        strip_blocks(&mut out);
    }
    let own = (!prefix.is_empty()).then_some((prefix, &declared));
    // `param name = default` is a `let` (renamed with the rest), unless the importer gave a value.
    for (i, _) in &params {
        if let Some(rest) = out[*i].strip_prefix("param ") {
            out[*i] = format!("let {rest}");
        }
    }
    let mut text = rewrite(&out.join("\n"), own, &aliases);
    if !args.is_empty() {
        let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
        for (i, name) in &params {
            if let Some(v) = args.get(name) {
                let n = if prefix.is_empty() {
                    name.clone()
                } else {
                    format!("{prefix}__{name}")
                };
                lines[*i] = format!("let {n} = {v}");
            }
        }
        text = lines.join("\n");
    }
    Ok(text)
}
