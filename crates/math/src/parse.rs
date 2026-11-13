//! The string frontend: `sin(x / 20 + t * 3) * 8`, with the usual precedence (`^` binds tighter than unary minus
//! and is right associative), comparisons and `&&` / `||` (giving 0 or 1), `c ? a : b` and `if(c, a, b)`.
use crate::ir::*;
use std::collections::HashMap;

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub msg: String,
    /// Byte offset into the source.
    pub at: usize,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} (at {})", self.msg, self.at)
    }
}

/// The names an equation may use: variables (inputs, parameters or earlier results) and the seed of `noise`.
#[derive(Clone, Debug, Default)]
pub struct Env {
    pub names: HashMap<String, Id>,
    pub noise_seed: u32,
}

impl Env {
    pub fn new() -> Env {
        Env::default()
    }
    pub fn with(mut self, name: &str, id: Id) -> Env {
        self.names.insert(name.to_string(), id);
        self
    }
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
    Num(f64),
    Id(String),
    Op(&'static str),
    End,
}

fn lex(src: &str) -> Result<Vec<(Tok, usize)>, ParseError> {
    let b = src.as_bytes();
    let mut out = vec![];
    let mut i = 0;
    while i < b.len() {
        let c = b[i] as char;
        if c.is_whitespace() {
            i += 1;
        } else if c.is_ascii_digit()
            || (c == '.' && b.get(i + 1).is_some_and(|d| d.is_ascii_digit()))
        {
            let s = i;
            while i < b.len() && ((b[i] as char).is_ascii_digit() || b[i] == b'.') {
                i += 1;
            }
            if i < b.len() && (b[i] == b'e' || b[i] == b'E') {
                let mut j = i + 1;
                if j < b.len() && (b[j] == b'+' || b[j] == b'-') {
                    j += 1;
                }
                if j < b.len() && b[j].is_ascii_digit() {
                    while j < b.len() && b[j].is_ascii_digit() {
                        j += 1;
                    }
                    i = j;
                }
            }
            let v = src[s..i].parse::<f64>().map_err(|_| ParseError {
                msg: format!("bad number `{}`", &src[s..i]),
                at: s,
            })?;
            out.push((Tok::Num(v), s));
        } else if c.is_ascii_alphabetic() || c == '_' {
            let s = i;
            while i < b.len() && ((b[i] as char).is_ascii_alphanumeric() || b[i] == b'_') {
                i += 1;
            }
            out.push((Tok::Id(src[s..i].to_string()), s));
        } else {
            let two = src.get(i..i + 2).unwrap_or("");
            let op: &'static str = match two {
                "<=" => "<=",
                ">=" => ">=",
                "==" => "==",
                "!=" => "!=",
                "&&" => "&&",
                "||" => "||",
                "**" => "^",
                _ => "",
            };
            if !op.is_empty() {
                out.push((Tok::Op(op), i));
                i += 2;
                continue;
            }
            let op: &'static str = match c {
                '+' => "+",
                '-' => "-",
                '*' => "*",
                '/' => "/",
                '%' => "%",
                '^' => "^",
                '(' => "(",
                ')' => ")",
                ',' => ",",
                '<' => "<",
                '>' => ">",
                '!' => "!",
                '?' => "?",
                ':' => ":",
                _ => {
                    return Err(ParseError {
                        msg: format!("unexpected `{c}`"),
                        at: i,
                    })
                }
            };
            out.push((Tok::Op(op), i));
            i += 1;
        }
    }
    out.push((Tok::End, src.len()));
    Ok(out)
}

struct P<'a> {
    toks: Vec<(Tok, usize)>,
    i: usize,
    b: &'a mut Builder,
    env: &'a Env,
}

impl P<'_> {
    fn peek(&self) -> &Tok {
        &self.toks[self.i].0
    }
    fn at(&self) -> usize {
        self.toks[self.i].1
    }
    fn err<T>(&self, msg: impl Into<String>) -> Result<T, ParseError> {
        Err(ParseError {
            msg: msg.into(),
            at: self.at(),
        })
    }
    fn eat(&mut self, op: &str) -> bool {
        if matches!(self.peek(), Tok::Op(o) if *o == op) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn expect(&mut self, op: &str) -> Result<(), ParseError> {
        if self.eat(op) {
            Ok(())
        } else {
            self.err(format!("expected `{op}`"))
        }
    }

    fn ternary(&mut self) -> Result<Id, ParseError> {
        let c = self.or()?;
        if self.eat("?") {
            let a = self.ternary()?;
            self.expect(":")?;
            let bb = self.ternary()?;
            return Ok(self.b.select(c, a, bb));
        }
        Ok(c)
    }
    fn or(&mut self) -> Result<Id, ParseError> {
        let mut l = self.and()?;
        while self.eat("||") {
            let r = self.and()?;
            l = self.b.bin(Bin::Or, l, r);
        }
        Ok(l)
    }
    fn and(&mut self) -> Result<Id, ParseError> {
        let mut l = self.cmp()?;
        while self.eat("&&") {
            let r = self.cmp()?;
            l = self.b.bin(Bin::And, l, r);
        }
        Ok(l)
    }
    fn cmp(&mut self) -> Result<Id, ParseError> {
        let l = self.sum()?;
        for (s, op) in [
            ("<=", Bin::Le),
            (">=", Bin::Ge),
            ("==", Bin::Eq),
            ("!=", Bin::Ne),
            ("<", Bin::Lt),
            (">", Bin::Gt),
        ] {
            if self.eat(s) {
                let r = self.sum()?;
                return Ok(self.b.bin(op, l, r));
            }
        }
        Ok(l)
    }
    fn sum(&mut self) -> Result<Id, ParseError> {
        let mut l = self.prod()?;
        loop {
            if self.eat("+") {
                let r = self.prod()?;
                l = self.b.add(l, r);
            } else if self.eat("-") {
                let r = self.prod()?;
                l = self.b.sub(l, r);
            } else {
                return Ok(l);
            }
        }
    }
    fn prod(&mut self) -> Result<Id, ParseError> {
        let mut l = self.unary()?;
        loop {
            if self.eat("*") {
                let r = self.unary()?;
                l = self.b.mul(l, r);
            } else if self.eat("/") {
                let r = self.unary()?;
                l = self.b.div(l, r);
            } else if self.eat("%") {
                let r = self.unary()?;
                l = self.b.bin(Bin::Rem, l, r);
            } else {
                return Ok(l);
            }
        }
    }
    fn unary(&mut self) -> Result<Id, ParseError> {
        if self.eat("-") {
            let a = self.unary()?;
            return Ok(self.b.neg(a));
        }
        if self.eat("+") {
            return self.unary();
        }
        if self.eat("!") {
            let a = self.unary()?;
            return Ok(self.b.un(Un::Not, a));
        }
        self.power()
    }
    fn power(&mut self) -> Result<Id, ParseError> {
        let base = self.atom()?;
        if self.eat("^") {
            let e = self.unary()?; // right associative, and `2 ^ -1` works
            return Ok(self.b.bin(Bin::Pow, base, e));
        }
        Ok(base)
    }
    fn args(&mut self) -> Result<Vec<Id>, ParseError> {
        let mut v = vec![];
        if self.eat(")") {
            return Ok(v);
        }
        loop {
            v.push(self.ternary()?);
            if self.eat(")") {
                return Ok(v);
            }
            self.expect(",")?;
        }
    }
    fn atom(&mut self) -> Result<Id, ParseError> {
        let at = self.at();
        match self.peek().clone() {
            Tok::Num(v) => {
                self.i += 1;
                Ok(self.b.constant(v))
            }
            Tok::Op("(") => {
                self.i += 1;
                let e = self.ternary()?;
                self.expect(")")?;
                Ok(e)
            }
            Tok::Id(name) => {
                self.i += 1;
                if self.eat("(") {
                    let args = self.args()?;
                    return self.call(&name, &args, at);
                }
                if let Some(id) = self.env.names.get(&name) {
                    return Ok(*id);
                }
                match name.as_str() {
                    "pi" => Ok(self.b.constant(std::f64::consts::PI)),
                    "tau" => Ok(self.b.constant(std::f64::consts::TAU)),
                    "e" => Ok(self.b.constant(std::f64::consts::E)),
                    "true" => Ok(self.b.constant(1.0)),
                    "false" => Ok(self.b.constant(0.0)),
                    _ => Err(ParseError {
                        msg: format!("unknown name `{name}`"),
                        at,
                    }),
                }
            }
            other => self.err(format!(
                "expected a value, found {}",
                match other {
                    Tok::End => "the end".to_string(),
                    Tok::Op(o) => format!("`{o}`"),
                    _ => "?".into(),
                }
            )),
        }
    }
    fn call(&mut self, name: &str, a: &[Id], at: usize) -> Result<Id, ParseError> {
        let want = |n: usize| -> Result<(), ParseError> {
            if a.len() == n {
                Ok(())
            } else {
                Err(ParseError {
                    msg: format!(
                        "`{name}` takes {n} argument{}, {} given",
                        if n == 1 { "" } else { "s" },
                        a.len()
                    ),
                    at,
                })
            }
        };
        let un = |this: &mut Self, op: Un| -> Result<Id, ParseError> {
            want(1)?;
            Ok(this.b.un(op, a[0]))
        };
        let bn = |this: &mut Self, op: Bin| -> Result<Id, ParseError> {
            want(2)?;
            Ok(this.b.bin(op, a[0], a[1]))
        };
        match name {
            "sin" => un(self, Un::Sin),
            "cos" => un(self, Un::Cos),
            "tan" => un(self, Un::Tan),
            "asin" => un(self, Un::Asin),
            "acos" => un(self, Un::Acos),
            "atan" => un(self, Un::Atan),
            "sinh" => un(self, Un::Sinh),
            "cosh" => un(self, Un::Cosh),
            "tanh" => un(self, Un::Tanh),
            "exp" => un(self, Un::Exp),
            "log" | "ln" => un(self, Un::Log),
            "log2" => un(self, Un::Log2),
            "log10" => un(self, Un::Log10),
            "sqrt" => un(self, Un::Sqrt),
            "abs" => un(self, Un::Abs),
            "floor" => un(self, Un::Floor),
            "ceil" => un(self, Un::Ceil),
            "round" => un(self, Un::Round),
            "trunc" => un(self, Un::Trunc),
            "frac" | "fract" => un(self, Un::Fract),
            "sign" => un(self, Un::Sign),
            "hash" => un(self, Un::Hash),
            "atan2" => bn(self, Bin::Atan2),
            "pow" => bn(self, Bin::Pow),
            "min" => bn(self, Bin::Min),
            "max" => bn(self, Bin::Max),
            "mod" | "fmod" => bn(self, Bin::Rem),
            "clamp" => {
                want(3)?;
                Ok(self.b.clamp(a[0], a[1], a[2]))
            }
            "mix" | "lerp" => {
                want(3)?;
                Ok(self.b.mix(a[0], a[1], a[2]))
            }
            "smoothstep" => {
                want(3)?;
                Ok(self.b.smoothstep(a[0], a[1], a[2]))
            }
            "step" => {
                want(2)?;
                Ok(self.b.step(a[0], a[1]))
            }
            "if" => {
                want(3)?;
                Ok(self.b.select(a[0], a[1], a[2]))
            }
            "noise" => {
                if !(1..=3).contains(&a.len()) {
                    return Err(ParseError {
                        msg: "`noise` takes 1 to 3 coordinates".into(),
                        at,
                    });
                }
                Ok(self.b.noise(self.env.noise_seed, a))
            }
            _ => Err(ParseError {
                msg: format!("unknown function `{name}`"),
                at,
            }),
        }
    }
}

/// Parse an equation, adding its nodes to `b`; returns the id of its value.
pub fn parse(src: &str, b: &mut Builder, env: &Env) -> Result<Id, ParseError> {
    let toks = lex(src)?;
    let mut p = P { toks, i: 0, b, env };
    let id = p.ternary()?;
    if *p.peek() != Tok::End {
        return p.err("unexpected input after the expression");
    }
    Ok(id)
}
