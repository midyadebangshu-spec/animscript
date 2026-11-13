//! Syntax colouring for the code panel: a forgiving scanner (it must cope with half-typed programs, which
//! the real lexer rejects) that follows the lexer's rules for comments and colours.
use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tok {
    Plain,
    Comment,
    Keyword,
    Number,
    Str,
    Color,
    Call,
    Op,
}

const KEYWORDS: &[&str] = &[
    "project",
    "scene",
    "setup",
    "main",
    "timeline",
    "when",
    "whenever",
    "together",
    "race",
    "for",
    "in",
    "if",
    "else",
    "repeat",
    "times",
    "while",
    "let",
    "shape",
    "prop",
    "character",
    "states",
    "start",
    "cancel",
    "await",
    "wait",
    "over",
    "ease",
    "at",
    "note",
    "assert",
    "action",
    "fn",
    "return",
    "emitter",
    "fluid",
    "field",
    "armature",
    "world",
    "clip",
    "lock",
    "release",
    "attach",
    "to",
    "finally",
    "shot",
    "marker",
    "import",
    "and",
    "or",
    "not",
    "true",
    "false",
    "none",
    "becomes",
    "clears",
    "derive",
    "keyframes",
    "pass",
    "auto",
];

fn ident_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `#` starts a colour when 3, 4, 6 or 8 hex digits follow and then no identifier character (docs/DECISIONS.md 11).
fn colour_len(c: &[(usize, char)], i: usize) -> Option<usize> {
    let n = c[i + 1..]
        .iter()
        .take_while(|x| x.1.is_ascii_hexdigit())
        .count();
    let boundary = c.get(i + 1 + n).is_none_or(|x| !ident_char(x.1));
    (matches!(n, 3 | 4 | 6 | 8) && boundary).then_some(n + 1)
}

/// Split `src` into coloured pieces; the byte ranges are ascending and cover the whole text.
pub fn scan(src: &str) -> Vec<(Range<usize>, Tok)> {
    let c: Vec<(usize, char)> = src.char_indices().collect();
    let at = |i: usize| c.get(i).map_or(src.len(), |x| x.0);
    let mut out: Vec<(Range<usize>, Tok)> = vec![];
    let mut push = |a: usize, b: usize, t: Tok| {
        if a < b {
            match out.last_mut() {
                Some(last) if last.1 == t && last.0.end == a => last.0.end = b,
                _ => out.push((a..b, t)),
            }
        }
    };
    let mut i = 0;
    while i < c.len() {
        let ch = c[i].1;
        let start = i;
        let tok = if ch == '#' {
            match colour_len(&c, i) {
                Some(n) => {
                    i += n;
                    Tok::Color
                }
                None => {
                    while i < c.len() && c[i].1 != '\n' {
                        i += 1;
                    }
                    Tok::Comment
                }
            }
        } else if ch == '"' {
            i += 1;
            while i < c.len() && c[i].1 != '"' && c[i].1 != '\n' {
                i += if c[i].1 == '\\' { 2 } else { 1 };
            }
            i = (i + 1).min(c.len());
            Tok::Str
        } else if ch.is_ascii_digit() {
            while i < c.len() && c[i].1.is_ascii_digit() {
                i += 1;
            }
            // a fraction, but not the dot of a unit or a range: `0.5`, not `2.seconds` or `1..4`
            if i + 1 < c.len() && c[i].1 == '.' && c[i + 1].1.is_ascii_digit() {
                i += 1;
                while i < c.len() && c[i].1.is_ascii_digit() {
                    i += 1;
                }
            }
            Tok::Number
        } else if ch.is_alphabetic() || ch == '_' {
            while i < c.len() && ident_char(c[i].1) {
                i += 1;
            }
            let word = &src[at(start)..at(i)];
            let after_dot = start > 0 && c[start - 1].1 == '.';
            if !after_dot && KEYWORDS.contains(&word) {
                Tok::Keyword
            } else if c.get(i).is_some_and(|x| x.1 == '(') {
                Tok::Call
            } else {
                Tok::Plain
            }
        } else if "+-*/%<>=!?:,.()[]{}".contains(ch) {
            i += 1;
            Tok::Op
        } else {
            i += 1;
            Tok::Plain
        };
        push(at(start), at(i), tok);
    }
    out
}

/// The byte range of a diagnostic span (1-based line, 1-based character columns, exclusive end) in `src`.
pub fn span_bytes(src: &str, line: u32, col: u32, end_col: u32) -> Option<Range<usize>> {
    let mut offset = 0;
    for (n, text) in src.split('\n').enumerate() {
        if n as u32 + 1 == line {
            let text = text.strip_suffix('\r').unwrap_or(text);
            let byte = |col: u32| {
                text.char_indices()
                    .nth(col.saturating_sub(1) as usize)
                    .map_or(text.len(), |x| x.0)
            };
            let (a, b) = (byte(col), byte(end_col.max(col + 1)));
            // an empty line still gets one position, so there is something to underline
            return Some(offset + a..offset + b.max(a));
        }
        offset += text.len() + 1;
    }
    None
}

/// The character index (what the text cursor counts) at which 1-based `line` starts.
pub fn line_start_char(src: &str, line: u32) -> usize {
    src.split('\n')
        .take(line.saturating_sub(1) as usize)
        .map(|l| l.chars().count() + 1)
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn toks(src: &str) -> Vec<(&str, Tok)> {
        scan(src).into_iter().map(|(r, t)| (&src[r], t)).collect()
    }

    #[test]
    fn ranges_cover_the_text_in_order() {
        let src = "shape s = square(size: 100) # a box\nlet c = #f4a6c0\n\"é\" 2.5.seconds";
        let mut end = 0;
        for (r, _) in scan(src) {
            assert_eq!(r.start, end);
            end = r.end;
        }
        assert_eq!(end, src.len());
    }

    #[test]
    fn hash_is_a_colour_or_a_comment() {
        let t = toks("fill = #fff # white\n#bad idea");
        assert!(t.contains(&("#fff", Tok::Color)));
        assert!(t.contains(&("# white", Tok::Comment)));
        // `#bad` followed by a space is three hex digits: a colour, as in the language
        assert!(t.contains(&("#bad", Tok::Color)));
        assert!(toks("#badly").contains(&("#badly", Tok::Comment)));
    }

    #[test]
    fn keywords_calls_numbers_and_strings() {
        let t = toks("scene \"A\" for 2.5.seconds:\n    s.move_by(10, 0) over 1..4");
        assert!(t.contains(&("scene", Tok::Keyword)));
        assert!(t.contains(&("\"A\"", Tok::Str)));
        assert!(t.contains(&("2.5", Tok::Number)));
        assert!(t.contains(&("move_by", Tok::Call)));
        assert!(t.contains(&("over", Tok::Keyword)));
        assert!(t.contains(&("1", Tok::Number)) && t.contains(&("4", Tok::Number)));
        // a member named like a keyword is not one
        assert!(!toks("x.start").contains(&("start", Tok::Keyword)));
    }

    #[test]
    fn half_typed_text_does_not_panic() {
        for src in ["\"open", "#", "1.", "a.", "\"esc\\", "é#12", ""] {
            scan(src);
        }
    }

    #[test]
    fn spans_map_to_bytes_and_lines_to_cursor_positions() {
        let src = "é = 1\nshape s = 3\n";
        assert_eq!(&src[span_bytes(src, 2, 7, 8).unwrap()], "s");
        assert_eq!(&src[span_bytes(src, 1, 1, 2).unwrap()], "é");
        assert!(span_bytes(src, 9, 1, 2).is_none());
        assert_eq!(line_start_char(src, 1), 0);
        assert_eq!(line_start_char(src, 2), 6);
    }
}
