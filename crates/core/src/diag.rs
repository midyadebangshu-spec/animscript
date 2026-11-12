//! Diagnostics with stable codes, source spans and `help:` lines (spec §13.1).
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub line: u32,
    pub col: u32,
    /// Exclusive end column on `line`.
    pub end_col: u32,
}

impl Span {
    pub fn new(line: u32, col: u32, end_col: u32) -> Self {
        Span { line, col, end_col }
    }
    /// Extend to cover `other` when both are on the same line.
    pub fn to(self, other: Span) -> Span {
        if other.line == self.line {
            Span {
                line: self.line,
                col: self.col.min(other.col),
                end_col: self.end_col.max(other.end_col),
            }
        } else {
            self
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub span: Option<Span>,
    pub helps: Vec<String>,
    pub notes: Vec<String>,
    pub label: String,
}

pub type Res<T> = Result<T, Diagnostic>;

impl Diagnostic {
    /// The same diagnostic under another code (used when an argument error belongs to a feature).
    pub fn with_code(mut self, code: &str) -> Self {
        self.code = code.to_string();
        self
    }
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        Diagnostic {
            code: code.to_string(),
            message: message.into(),
            span: None,
            helps: vec![],
            notes: vec![],
            label: String::new(),
        }
    }
    pub fn help(mut self, h: impl Into<String>) -> Self {
        self.helps.push(h.into());
        self
    }
    pub fn note(mut self, n: impl Into<String>) -> Self {
        self.notes.push(n.into());
        self
    }
    pub fn label(mut self, l: impl Into<String>) -> Self {
        self.label = l.into();
        self
    }
    pub fn at(mut self, span: Span) -> Self {
        self.span = Some(span);
        self
    }
    /// Attach a span only if none is set yet (errors raised deep inside keep the innermost span).
    pub fn or_at(mut self, span: Span) -> Self {
        if self.span.is_none() {
            self.span = Some(span);
        }
        self
    }
    pub fn severity(&self) -> Severity {
        if self.code.starts_with('W') {
            Severity::Warning
        } else {
            Severity::Error
        }
    }
    pub fn is_error(&self) -> bool {
        self.severity() == Severity::Error
    }

    /// Render like the examples in spec §13.1.
    pub fn render(&self, source: Option<&str>, filename: &str) -> String {
        let sev = if self.is_error() { "error" } else { "warning" };
        let mut out = format!("{sev}[{}]: {}\n", self.code, self.message);
        if let Some(sp) = self.span {
            out += &format!("  --> {filename}:{}:{}\n", sp.line, sp.col);
            if let Some(src) = source {
                if let Some(text) = src.lines().nth(sp.line as usize - 1) {
                    let num = sp.line.to_string();
                    let pad = " ".repeat(num.len());
                    let carets = "^".repeat(sp.end_col.saturating_sub(sp.col).max(1) as usize);
                    let lead = " ".repeat(sp.col.saturating_sub(1) as usize);
                    out += &format!(
                        "{pad} |\n{num} | {text}\n{pad} | {lead}{carets} {}\n{pad} |\n",
                        self.label
                    );
                }
            }
        }
        for n in &self.notes {
            out += &format!("note: {n}\n");
        }
        for h in &self.helps {
            out += &format!("help: {h}\n");
        }
        out
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for Diagnostic {}
