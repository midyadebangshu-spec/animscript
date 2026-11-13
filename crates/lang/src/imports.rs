//! `import "file.anim"` and `import "std/name"`: textual inclusion done before parsing. The import line
//! itself is blanked (so line numbers in the importing file never change) and the imported text is appended
//! to the end of the source (definitions are available everywhere, wherever they are). Each file is
//! included once, so import cycles are harmless. `std/name` is a library bundled in the compiler (the same
//! files live in `crates/lang/lib/`); anything else is a path relative to the importing file's folder.
use animscript_core::{Diagnostic, Res, Span};
use std::collections::BTreeSet;
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

pub fn expand_imports(src: &str, base: &Path) -> Res<String> {
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut appended = String::new();
    let main = expand(src, base, &mut seen, &mut appended, true)?;
    if appended.is_empty() {
        return Ok(main);
    }
    Ok(format!("{main}\n{appended}"))
}

fn expand(
    src: &str,
    dir: &Path,
    seen: &mut BTreeSet<String>,
    appended: &mut String,
    is_main: bool,
) -> Res<String> {
    let mut out = Vec::new();
    for (i, line) in src.lines().enumerate() {
        let Some(rest) = line.strip_prefix("import ") else {
            out.push(line.to_string());
            continue;
        };
        let at = Span::new(i as u32 + 1, 1, line.chars().count() as u32 + 1);
        let path = rest
            .trim()
            .strip_prefix('"')
            .and_then(|r| r.strip_suffix('"'))
            .ok_or_else(|| {
                Diagnostic::new(
                    "E-SYN-001",
                    "`import` needs a quoted path, such as `import \"std/motion\"`",
                )
                .at(at)
            })?;
        out.push(String::new()); // keep the line numbering
        let (key, text, sub_dir): (String, String, PathBuf) =
            if let Some(name) = path.strip_prefix("std/") {
                let name = name.trim_end_matches(".anim");
                match bundled(name) {
                    Some(t) => (format!("std/{name}"), t.to_string(), dir.to_path_buf()),
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
                let full = dir.join(path);
                let text = std::fs::read_to_string(&full).map_err(|_| {
                    Diagnostic::new("E-AST-001", format!("imported file `{path}` was not found"))
                        .at(at)
                        .help(format!(
                            "paths are relative to the importing file's folder ({})",
                            dir.display()
                        ))
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
                (key, text, sub)
            };
        if seen.insert(key) {
            let body = expand(&text, &sub_dir, seen, appended, false)?;
            appended.push_str(&body);
            appended.push('\n');
        }
    }
    let _ = is_main;
    Ok(out.join("\n"))
}
