//! Indentation-aware lexer (spec §2).
use animscript_core::{Diagnostic, Res, Span};

#[derive(Clone, Debug, PartialEq)]
pub enum T {
    Num(String),
    Str(String),
    Ident(String),
    /// Hex digits without the `#`.
    Color(String),
    Canvas(u32, u32),
    Op(&'static str),
    Newline,
    Indent,
    Dedent,
    Eof,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub t: T,
    pub span: Span,
    /// True when no whitespace separates this token from the previous one on the same line.
    pub adj: bool,
}

const OPS2: &[&str] = &["==", "!=", "<=", ">=", "??", "..", "->", "+="];
const OPS1: &[&str] = &[
    "+", "-", "*", "/", "%", "<", ">", "=", "(", ")", ",", ":", ".", "[", "]", "{", "}",
];

fn color_len(c: &[char], i: usize) -> Option<usize> {
    let mut n = 0;
    while i + 1 + n < c.len() && c[i + 1 + n].is_ascii_hexdigit() {
        n += 1;
    }
    let boundary = i + 1 + n >= c.len() || !(c[i + 1 + n].is_alphanumeric() || c[i + 1 + n] == '_');
    if matches!(n, 3 | 4 | 6 | 8) && boundary {
        Some(n + 1)
    } else {
        None
    }
}

pub fn lex(src: &str) -> Res<Vec<Token>> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    let mut toks: Vec<Token> = vec![];
    let mut indents = vec![0usize];
    let mut depth = 0i32;
    let mut last_line = 1u32;
    for (li, raw) in src.lines().enumerate() {
        let ln = li as u32 + 1;
        last_line = ln;
        let c: Vec<char> = raw.chars().collect();
        let mut i = 0usize;
        let sp = |a: usize, b: usize| Span::new(ln, a as u32 + 1, b as u32 + 1);
        if depth == 0 {
            while i < c.len() && (c[i] == ' ' || c[i] == '\t') {
                i += 1;
            }
            if i >= c.len() || (c[i] == '#' && color_len(&c, i).is_none()) {
                continue; // blank or comment-only line
            }
            if c[..i].contains(&'\t') {
                return Err(Diagnostic::new("E-LEX-001", "tab character in indentation")
                    .at(sp(0, i))
                    .help("indent with four spaces per level"));
            }
            let cur = *indents.last().unwrap();
            if i > cur {
                if i - cur != 4 {
                    return Err(Diagnostic::new(
                        "E-LEX-002",
                        "indentation must increase by exactly four spaces",
                    )
                    .at(sp(0, i)));
                }
                indents.push(i);
                toks.push(Token {
                    t: T::Indent,
                    span: sp(0, i),
                    adj: false,
                });
            } else if i < cur {
                while *indents.last().unwrap() > i {
                    indents.pop();
                    toks.push(Token {
                        t: T::Dedent,
                        span: sp(0, i),
                        adj: false,
                    });
                }
                if *indents.last().unwrap() != i {
                    return Err(Diagnostic::new(
                        "E-LEX-002",
                        "dedent does not return to an enclosing indentation level",
                    )
                    .at(sp(0, i))
                    .help("each block level is four spaces"));
                }
            }
        }
        let mut last_end = usize::MAX;
        let mut prev_operand = false; // previous token on this line ends an operand
        while i < c.len() {
            let ch = c[i];
            if ch == ' ' || ch == '\t' {
                i += 1;
                continue;
            }
            let start = i;
            let adj = start == last_end;
            let tok: T;
            if ch == '#' {
                if let Some(n) = color_len(&c, i) {
                    tok = T::Color(c[i + 1..i + n].iter().collect());
                    i += n;
                } else {
                    break; // comment
                }
            } else if ch == '"' {
                let mut s = String::new();
                i += 1;
                let mut closed = false;
                while i < c.len() {
                    match c[i] {
                        '"' => {
                            closed = true;
                            i += 1;
                            break;
                        }
                        '\\' if i + 1 < c.len() => {
                            s.push(match c[i + 1] {
                                'n' => '\n',
                                't' => '\t',
                                o => o,
                            });
                            i += 2;
                        }
                        o => {
                            s.push(o);
                            i += 1;
                        }
                    }
                }
                if !closed {
                    return Err(Diagnostic::new("E-LEX-003", "unterminated string")
                        .at(sp(start, c.len()))
                        .help("add a closing `\"`"));
                }
                tok = T::Str(s);
            } else if ch.is_ascii_digit()
                || (ch == '.'
                    && i + 1 < c.len()
                    && c[i + 1].is_ascii_digit()
                    && !(adj && prev_operand))
            {
                // canvas size such as 1280x720
                let mut j = i;
                while j < c.len() && c[j].is_ascii_digit() {
                    j += 1;
                }
                if ch != '.'
                    && j < c.len()
                    && c[j] == 'x'
                    && j + 1 < c.len()
                    && c[j + 1].is_ascii_digit()
                {
                    let mut k = j + 1;
                    while k < c.len() && c[k].is_ascii_digit() {
                        k += 1;
                    }
                    if k >= c.len() || !(c[k].is_alphanumeric() || c[k] == '_') {
                        let w: String = c[i..j].iter().collect();
                        let h: String = c[j + 1..k].iter().collect();
                        toks.push(Token {
                            t: T::Canvas(w.parse().unwrap_or(0), h.parse().unwrap_or(0)),
                            span: sp(start, k),
                            adj,
                        });
                        i = k;
                        last_end = k;
                        prev_operand = true;
                        continue;
                    }
                }
                let mut j = i;
                while j < c.len() && (c[j].is_ascii_digit() || c[j] == '_') {
                    j += 1;
                }
                if j + 1 < c.len() && c[j] == '.' && c[j + 1].is_ascii_digit() {
                    j += 1;
                    while j < c.len() && (c[j].is_ascii_digit() || c[j] == '_') {
                        j += 1;
                    }
                }
                if j < c.len() && (c[j] == 'e' || c[j] == 'E') {
                    let mut k = j + 1;
                    if k < c.len() && (c[k] == '+' || c[k] == '-') {
                        k += 1;
                    }
                    if k < c.len() && c[k].is_ascii_digit() {
                        while k < c.len() && c[k].is_ascii_digit() {
                            k += 1;
                        }
                        j = k;
                    }
                }
                tok = T::Num(c[i..j].iter().filter(|x| **x != '_').collect());
                i = j;
            } else if ch.is_ascii_alphabetic() || ch == '_' {
                let mut j = i;
                while j < c.len() && (c[j].is_ascii_alphanumeric() || c[j] == '_') {
                    j += 1;
                }
                tok = T::Ident(c[i..j].iter().collect());
                i = j;
            } else {
                let two: String = c[i..(i + 2).min(c.len())].iter().collect();
                if let Some(op) = OPS2.iter().find(|o| **o == two) {
                    tok = T::Op(op);
                    i += 2;
                } else if let Some(op) = OPS1.iter().find(|o| o.starts_with(ch)) {
                    match *op {
                        "(" | "[" | "{" => depth += 1,
                        ")" | "]" | "}" => depth = (depth - 1).max(0),
                        _ => {}
                    }
                    tok = T::Op(op);
                    i += 1;
                } else {
                    return Err(Diagnostic::new(
                        "E-SYN-001",
                        format!("unexpected character `{ch}`"),
                    )
                    .at(sp(start, start + 1)));
                }
            }
            prev_operand = matches!(
                tok,
                T::Num(_) | T::Ident(_) | T::Str(_) | T::Color(_) | T::Op(")") | T::Op("]")
            );
            toks.push(Token {
                t: tok,
                span: sp(start, i),
                adj,
            });
            last_end = i;
        }
        if depth == 0 {
            toks.push(Token {
                t: T::Newline,
                span: sp(c.len(), c.len() + 1),
                adj: false,
            });
        }
    }
    let end = Span::new(last_line, 1, 2);
    if depth > 0 {
        return Err(Diagnostic::new("E-SYN-001", "unclosed bracket at end of file").at(end));
    }
    while indents.len() > 1 {
        indents.pop();
        toks.push(Token {
            t: T::Dedent,
            span: end,
            adj: false,
        });
    }
    toks.push(Token {
        t: T::Eof,
        span: end,
        adj: false,
    });
    Ok(toks)
}
