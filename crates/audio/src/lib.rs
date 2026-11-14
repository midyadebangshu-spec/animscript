//! The score model and the notation of the audio system: notes, chords, drum patterns, keys, tempo.
//! Time is counted in integer ticks (960 per quarter note, 3840 per whole note) so that nothing
//! depends on floating point rounding until a tick is turned into seconds.
#![allow(
    clippy::needless_range_loop,
    clippy::type_complexity,
    clippy::should_implement_trait,
    clippy::derivable_impls,
    clippy::manual_is_multiple_of
)]
use std::fmt;

pub mod automation;
pub mod fx;
pub mod mixer;
pub mod render;
pub mod sampler;
pub mod sfz;
pub mod synth;
pub mod wav;

pub use render::{render_score, Bank, Mix, Stereo};
pub use synth::Instrument;

pub const PPQ: u64 = 960;
pub const WHOLE: u64 = PPQ * 4;

#[derive(Clone, Debug, PartialEq)]
pub struct ParseError {
    pub message: String,
}
impl ParseError {
    fn new(m: impl Into<String>) -> ParseError {
        ParseError { message: m.into() }
    }
}
impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(&self.message)
    }
}
type PRes<T> = Result<T, ParseError>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Meter {
    pub num: u32,
    pub den: u32,
}
impl Meter {
    pub fn new(num: u32, den: u32) -> Meter {
        Meter { num, den }
    }
    pub fn bar_ticks(&self) -> u64 {
        WHOLE * self.num as u64 / self.den as u64
    }
}

/// One note: where it starts and how long it lasts (ticks), its MIDI number and velocity (0 to 1), and how it
/// is articulated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ev {
    pub start: u64,
    pub len: u64,
    pub note: u8,
    pub vel: f64,
    /// The share of `len` the note is held before its release (1 = the full written length).
    pub gate: f64,
    /// Slurred to the next note: they overlap, and on a monophonic instrument the voice is not retriggered.
    pub legato: bool,
    /// Glide to this key over the note's length (glissando).
    pub slide: Option<u8>,
    /// Detune in cents (-100 to 100).
    pub cents: f64,
}
impl Ev {
    pub const PLAIN: Ev = Ev {
        start: 0,
        len: 0,
        note: 0,
        vel: 0.0,
        gate: 1.0,
        legato: false,
        slide: None,
        cents: 0.0,
    };
    pub fn new(start: u64, len: u64, note: u8, vel: f64) -> Ev {
        Ev {
            start,
            len,
            note,
            vel,
            ..Ev::PLAIN
        }
    }
}

// ---- pitch and key ------------------------------------------------------------------------

fn letter_pc(c: char) -> Option<i32> {
    Some(match c {
        'C' => 0,
        'D' => 2,
        'E' => 4,
        'F' => 5,
        'G' => 7,
        'A' => 9,
        'B' => 11,
        _ => return None,
    })
}

/// `C4` is middle C (MIDI 60); `#` and `b` raise and lower; octaves go from -1 to 9.
pub fn pitch(s: &str) -> Option<u8> {
    let mut ch = s.chars();
    let mut pc = letter_pc(ch.next()?)?;
    let rest: String = ch.collect();
    let rest = if let Some(r) = rest.strip_prefix('#') {
        pc += 1;
        r.to_string()
    } else if let Some(r) = rest.strip_prefix('b') {
        pc -= 1;
        r.to_string()
    } else {
        rest
    };
    let oct: i32 = rest.parse().ok()?;
    let m = (oct + 1) * 12 + pc;
    (0..=127).contains(&m).then_some(m as u8)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    pub tonic: u8,
    pub minor: bool,
}
impl Default for Key {
    fn default() -> Key {
        Key {
            tonic: 0,
            minor: false,
        }
    }
}
impl Key {
    /// `"A minor"`, `"C major"`, `"F# min"`, `"Bb"` (major).
    pub fn parse(s: &str) -> PRes<Key> {
        let mut parts = s.split_whitespace();
        let root = parts
            .next()
            .ok_or_else(|| ParseError::new("a key needs a tonic, like \"A minor\""))?;
        let mut ch = root.chars();
        let mut pc = ch
            .next()
            .and_then(letter_pc)
            .ok_or_else(|| ParseError::new(format!("`{root}` is not a key tonic (A to G)")))?;
        match ch.as_str() {
            "" => {}
            "#" => pc += 1,
            "b" => pc -= 1,
            other => {
                return Err(ParseError::new(format!(
                    "`{root}`: unexpected `{other}` after the tonic"
                )))
            }
        }
        let minor = match parts.next() {
            None | Some("major") | Some("maj") => false,
            Some("minor") | Some("min") => true,
            Some(o) => {
                return Err(ParseError::new(format!(
                    "a key is major or minor, not `{o}`"
                )))
            }
        };
        Ok(Key {
            tonic: pc.rem_euclid(12) as u8,
            minor,
        })
    }
    fn scale(&self) -> [i32; 7] {
        if self.minor {
            [0, 2, 3, 5, 7, 8, 10]
        } else {
            [0, 2, 4, 5, 7, 9, 11]
        }
    }
}

// ---- durations and notes ------------------------------------------------------------------

fn parse_dur(s: &str) -> PRes<u64> {
    let (body, dotted, triplet) = {
        let mut b = s;
        let mut d = false;
        let mut t = false;
        loop {
            if let Some(r) = b.strip_suffix('.') {
                d = true;
                b = r;
            } else if let Some(r) = b.strip_suffix('t') {
                t = true;
                b = r;
            } else {
                break;
            }
        }
        (b, d, t)
    };
    let (n, d) = match body.split_once('/') {
        Some((n, d)) => (n.parse::<u64>().ok(), d.parse::<u64>().ok()),
        None => (body.parse::<u64>().ok(), Some(1)),
    };
    let (Some(n), Some(d)) = (n, d) else {
        return Err(ParseError::new(format!(
            "`{s}` is not a duration (like 1/4, 1/8., 1/8t or 2)"
        )));
    };
    if n == 0 || d == 0 {
        return Err(ParseError::new(format!("`{s}` is not a duration")));
    }
    let mut num = WHOLE * n;
    let mut den = d;
    if dotted {
        num *= 3;
        den *= 2;
    }
    if triplet {
        num *= 2;
        den *= 3;
    }
    if num % den != 0 {
        return Err(ParseError::new(format!("the duration {s} is not a whole number of ticks (use 1/2, 1/4, 1/8, 1/16, 1/32, with `.` or `t`)")));
    }
    Ok(num / den)
}

const DEFAULT_VEL: f64 = 0.7;
/// How much louder an accent (`^`) is than the current level.
const ACCENT_BOOST: f64 = 0.2;
/// The length of a grace note (ticks): a thirty-second note.
const GRACE: u64 = 120;

/// A dynamic mark as a velocity: `ppp pp p mp mf f ff`.
pub fn dynamic(s: &str) -> Option<f64> {
    Some(match s {
        "ppp" => 0.15,
        "pp" => 0.25,
        "p" => 0.4,
        "mp" => 0.55,
        "mf" => 0.7,
        "f" => 0.85,
        "ff" => 0.98,
        _ => return None,
    })
}

struct NoteTok {
    /// The pitch text, or `.` / `r` for a rest.
    head: String,
    slide_to: Option<String>,
    dur: Option<u64>,
    vel: Option<f64>,
    accent: bool,
    gate: Option<f64>,
    staccato: bool,
    tenuto: bool,
    cents: f64,
}

/// The pitch at the start of `s` (`C4`, `F#3`, `Bb-1`): its text and the rest of the string.
fn take_pitch(s: &str) -> Option<(&str, &str)> {
    let b = s.as_bytes();
    if b.is_empty() || !matches!(b[0], b'A'..=b'G') {
        return None;
    }
    let mut i = 1;
    if i < b.len() && (b[i] == b'#' || b[i] == b'b') {
        i += 1;
    }
    if i < b.len() && b[i] == b'-' {
        i += 1;
    }
    let d = i;
    while i < b.len() && b[i].is_ascii_digit() {
        i += 1;
    }
    (i > d).then(|| s.split_at(i))
}

/// `pitch[~pitch][:dur]` followed by any of `!vel`, `@gate`, `'` (staccato), `_` (tenuto), `^` (accent) and
/// `+20c` / `-20c` (cents), in any order. A rest is `.` or `r` with an optional `:dur`.
fn parse_note_tok(tok: &str) -> PRes<NoteTok> {
    let bad = |m: String| ParseError::new(format!("`{tok}`: {m}"));
    let mut t = NoteTok {
        head: String::new(),
        slide_to: None,
        dur: None,
        vel: None,
        accent: false,
        gate: None,
        staccato: false,
        tenuto: false,
        cents: 0.0,
    };
    let mut rest: &str;
    if let Some(r) = tok.strip_prefix('.') {
        t.head = ".".into();
        rest = r;
    } else if let Some(r) = tok.strip_prefix('r') {
        t.head = "r".into();
        rest = r;
    } else {
        let (p, r) = take_pitch(tok).ok_or_else(|| {
            ParseError::new(format!("`{tok}` is not a pitch (like C4, F#3, Bb2)"))
        })?;
        t.head = p.to_string();
        rest = r;
        if let Some(r2) = rest.strip_prefix('~') {
            let (p2, r3) = take_pitch(r2)
                .ok_or_else(|| bad("a glissando `~` needs a second pitch, like C4~E4".into()))?;
            t.slide_to = Some(p2.to_string());
            rest = r3;
        }
    }
    while let Some(c) = rest.chars().next() {
        let tail = &rest[1..];
        match c {
            ':' => {
                let end = tail
                    .find(['!', '@', '\'', '_', '^', '+', '-'])
                    .unwrap_or(tail.len());
                t.dur = Some(parse_dur(&tail[..end]).map_err(|e| bad(e.message))?);
                rest = &tail[end..];
            }
            '!' => {
                let end = tail
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(tail.len());
                let n: u32 = tail[..end]
                    .parse()
                    .map_err(|_| bad("velocity must be a number 1 to 127".into()))?;
                if !(1..=127).contains(&n) {
                    return Err(bad("velocity must be between 1 and 127".into()));
                }
                t.vel = Some(n as f64 / 127.0);
                rest = &tail[end..];
            }
            '@' => {
                let end = tail
                    .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                    .unwrap_or(tail.len());
                let g: f64 = tail[..end].parse().map_err(|_| {
                    bad("`@` is followed by a gate between 0 and 1, like @0.7".into())
                })?;
                if !(g > 0.0 && g <= 1.0) {
                    return Err(bad("a gate is above 0 and at most 1".into()));
                }
                t.gate = Some(g);
                rest = &tail[end..];
            }
            '\'' => {
                t.staccato = true;
                rest = tail;
            }
            '_' => {
                t.tenuto = true;
                rest = tail;
            }
            '^' => {
                t.accent = true;
                rest = tail;
            }
            '+' | '-' => {
                let end = tail
                    .find(|c: char| !(c.is_ascii_digit() || c == '.'))
                    .unwrap_or(tail.len());
                let n: f64 = tail[..end]
                    .parse()
                    .map_err(|_| bad("a detune is like +20c or -35c".into()))?;
                if !tail[end..].starts_with('c') {
                    return Err(bad("a detune ends in `c` (cents), like +20c".into()));
                }
                if n > 100.0 {
                    return Err(bad("a detune is at most 100 cents".into()));
                }
                t.cents = if c == '-' { -n } else { n };
                rest = &tail[end + 1..];
            }
            other => return Err(bad(format!("unexpected `{other}`"))),
        }
    }
    Ok(t)
}

fn key_of_pitch(p: &str) -> PRes<u8> {
    pitch(p).ok_or_else(|| ParseError::new(format!("`{p}` is not a pitch (like C4, F#3, Bb2)")))
}

/// Notes separated by spaces. A note is `C4:1/4` with optional marks: `!90` velocity, `^` accent, `'` staccato,
/// `_` tenuto, `@0.7` gate, `+20c` detune, `C4~E4:1/2` glissando, `{D4}C4` grace note. `.` or `r` rests;
/// `[C4 E4 G4]:1/2` plays together; `( ... )` slurs; `|` is a bar line (checked against the meter). `pp p mp mf f ff`
/// set the dynamic level and `<` / `>` ramp the velocity of the notes between them and the next mark.
pub fn parse_notes(src: &str, meter: Meter) -> PRes<Vec<Ev>> {
    let bar = meter.bar_ticks();
    let chars: Vec<char> = src.chars().collect();
    let mut i = 0;
    let (mut pos, mut last) = (0u64, PPQ);
    let mut bars_done = 0usize;
    let mut vel_now = DEFAULT_VEL;
    let mut out: Vec<Ev> = vec![];
    // a slur in progress: the steps (first and one past the last event index) it contains
    let mut slur: Option<Vec<(usize, usize)>> = None;
    // a hairpin in progress: its starting velocity, direction and the event groups it covers
    let mut hairpin: Option<(f64, bool, Vec<Vec<usize>>)> = None;
    fn finish_hairpin(out: &mut [Ev], h: (f64, bool, Vec<Vec<usize>>), target: f64) -> PRes<()> {
        let (start, up, steps) = h;
        if (up && target < start) || (!up && target > start) {
            return Err(ParseError::new(format!(
                "a {} ends {} than it starts: check the dynamic mark after it",
                if up {
                    "crescendo `<`"
                } else {
                    "decrescendo `>`"
                },
                if up { "softer" } else { "louder" }
            )));
        }
        let n = steps.len().max(1) as f64;
        for (k, group) in steps.iter().enumerate() {
            for &ix in group {
                out[ix].vel = start + (target - start) * k as f64 / n;
            }
        }
        Ok(())
    }
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '|' {
            if pos % bar != 0 {
                return Err(ParseError::new(format!(
                    "bar {} is {} ticks long, the meter {}/{} wants {}",
                    bars_done + 1,
                    pos - bars_done as u64 * bar,
                    meter.num,
                    meter.den,
                    bar
                )));
            }
            bars_done += 1;
            i += 1;
            continue;
        }
        if c == '(' {
            if slur.is_some() {
                return Err(ParseError::new(
                    "slurs cannot be nested: a `(` is already open",
                ));
            }
            slur = Some(vec![]);
            i += 1;
            continue;
        }
        if c == ')' {
            let Some(steps) = slur.take() else {
                return Err(ParseError::new(
                    "a `)` closes a slur that was never opened with `(`",
                ));
            };
            let n = steps.len();
            for (k, (a, b)) in steps.iter().enumerate() {
                if k + 1 < n {
                    for e in &mut out[*a..*b] {
                        e.legato = true;
                    }
                }
            }
            i += 1;
            continue;
        }
        // a token: a bracketed group, a grace note and its note, or up to the next space or bar line
        let mut j = i;
        if c == '[' {
            while j < chars.len() && chars[j] != ']' {
                j += 1;
            }
            if j >= chars.len() {
                return Err(ParseError::new("a chord group `[` is not closed with `]`"));
            }
            j += 1;
            while j < chars.len()
                && !chars[j].is_whitespace()
                && !matches!(chars[j], '|' | ')' | '(')
            {
                j += 1;
            }
        } else {
            if c == '{' {
                while j < chars.len() && chars[j] != '}' {
                    j += 1;
                }
                if j >= chars.len() {
                    return Err(ParseError::new("a grace note `{` is not closed with `}`"));
                }
            }
            while j < chars.len()
                && !chars[j].is_whitespace()
                && !matches!(chars[j], '|' | ')' | '(')
            {
                j += 1;
            }
        }
        let tok: String = chars[i..j].iter().collect();
        i = j;
        // dynamics and hairpins
        if let Some(v) = dynamic(&tok) {
            if let Some(h) = hairpin.take() {
                finish_hairpin(&mut out, h, v)?;
            }
            vel_now = v;
            continue;
        }
        if tok == "<" || tok == ">" {
            if hairpin.is_some() {
                return Err(ParseError::new(format!(
                    "a hairpin `{tok}` starts inside another: end the first with a dynamic mark"
                )));
            }
            hairpin = Some((vel_now, tok == "<", vec![]));
            continue;
        }
        let first = out.len();
        let make = |t: &NoteTok, len: u64, vel_now: f64| -> (f64, f64) {
            let vel = t.vel.unwrap_or(if t.accent {
                (vel_now + ACCENT_BOOST).min(1.0)
            } else {
                vel_now
            });
            let gate = t.gate.unwrap_or(if t.staccato { 0.5 } else { 1.0 });
            let _ = len;
            (vel, gate)
        };
        if let Some(inner) = tok.strip_prefix('[') {
            let (list, after) = inner.split_once(']').expect("checked");
            let t = parse_note_tok(&format!("C4{after}"))?;
            let len = t.dur.unwrap_or(last);
            let (vel, gate) = make(&t, len, vel_now);
            let mut any = false;
            for p in list.split_whitespace() {
                let n = key_of_pitch(p)?;
                out.push(Ev {
                    start: pos,
                    len,
                    note: n,
                    vel,
                    gate,
                    cents: t.cents,
                    ..Ev::PLAIN
                });
                any = true;
            }
            if !any {
                return Err(ParseError::new("an empty chord group `[]`"));
            }
            last = len;
            pos += len;
        } else {
            let (grace, body) = match tok.strip_prefix('{') {
                Some(r) => {
                    let (g, b) = r.split_once('}').expect("checked");
                    (Some(key_of_pitch(g.trim())?), b.to_string())
                }
                None => (None, tok.clone()),
            };
            let t = parse_note_tok(&body)?;
            let len = t.dur.unwrap_or(last);
            last = len;
            if t.head == "." || t.head == "r" {
                if grace.is_some() {
                    return Err(ParseError::new(format!(
                        "`{tok}`: a rest cannot have a grace note"
                    )));
                }
                pos += len;
                continue;
            }
            let n = key_of_pitch(&t.head)?;
            let (vel, gate) = make(&t, len, vel_now);
            let (mut start, mut nlen) = (pos, len);
            if let Some(g) = grace {
                let gl = GRACE.min(len / 2);
                out.push(Ev {
                    start: pos,
                    len: gl,
                    note: g,
                    vel: (vel * 0.8).min(1.0),
                    gate: 1.0,
                    ..Ev::PLAIN
                });
                start += gl;
                nlen -= gl;
            }
            let slide = t.slide_to.as_deref().map(key_of_pitch).transpose()?;
            out.push(Ev {
                start,
                len: nlen,
                note: n,
                vel,
                gate,
                slide,
                cents: t.cents,
                ..Ev::PLAIN
            });
            pos += len;
        }
        let explicit = tok.contains('!') || tok.contains('^');
        if let Some(s) = slur.as_mut() {
            s.push((first, out.len()));
        }
        if let Some(h) = hairpin.as_mut() {
            if !explicit {
                // the grace note keeps its own velocity; the main notes of the step ramp
                let ids: Vec<usize> = (first..out.len())
                    .filter(|ix| !(tok.starts_with('{') && *ix == first))
                    .collect();
                h.2.push(ids);
            }
        }
    }
    if slur.is_some() {
        return Err(ParseError::new("a slur `(` is never closed with `)`"));
    }
    if hairpin.is_some() {
        return Err(ParseError::new(
            "a hairpin `<` or `>` needs a dynamic mark (such as `f`) after the notes it covers",
        ));
    }
    Ok(out)
}

/// Humanise a track: every start moves by up to `amount * 40` ticks and every velocity by up to `amount * 0.15`,
/// from a seeded sequence (the same events and seed always give the same result). Starts never go below 0.
pub fn humanize(events: &mut [Ev], amount: f64, seed: u64) {
    if amount <= 0.0 {
        return;
    }
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut next = || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        (state >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    };
    for e in events.iter_mut() {
        let dt = (next() * amount * 40.0).round() as i64;
        e.start = (e.start as i64 + dt).max(0) as u64;
        e.vel = (e.vel + next() * amount * 0.15).clamp(0.05, 1.0);
    }
}

// ---- chords -------------------------------------------------------------------------------

fn quality(q: &str) -> Option<&'static [i32]> {
    Some(match q {
        "" | "maj" | "M" => &[0, 4, 7],
        "m" | "min" | "-" => &[0, 3, 7],
        "7" => &[0, 4, 7, 10],
        "maj7" | "M7" => &[0, 4, 7, 11],
        "m7" | "min7" => &[0, 3, 7, 10],
        "dim" => &[0, 3, 6],
        "dim7" => &[0, 3, 6, 9],
        "m7b5" => &[0, 3, 6, 10],
        "aug" | "+" => &[0, 4, 8],
        "sus2" => &[0, 2, 7],
        "sus4" | "sus" => &[0, 5, 7],
        "6" => &[0, 4, 7, 9],
        "m6" => &[0, 3, 7, 9],
        "9" => &[0, 4, 7, 10, 14],
        "maj9" => &[0, 4, 7, 11, 14],
        "m9" => &[0, 3, 7, 10, 14],
        "add9" => &[0, 4, 7, 14],
        "5" => &[0, 7],
        _ => return None,
    })
}

/// Place pitch class `pc` in `octave` (C3 = 48 for octave 3).
fn at_octave(pc: i32, octave: i32) -> i32 {
    (octave + 1) * 12 + pc.rem_euclid(12)
}

fn roman(s: &str) -> Option<(i32, usize, bool, &str)> {
    // optional accidental, a numeral I..VII (case gives the quality), the rest is the chord suffix
    let (acc, rest) = if let Some(r) = s.strip_prefix('b') {
        (-1, r)
    } else if let Some(r) = s.strip_prefix('#') {
        (1, r)
    } else {
        (0, s)
    };
    let end = rest
        .find(|c: char| !matches!(c, 'I' | 'V' | 'i' | 'v'))
        .unwrap_or(rest.len());
    let (num, suffix) = rest.split_at(end);
    if num.is_empty() {
        return None;
    }
    let upper = num.chars().all(|c| c.is_ascii_uppercase());
    let lower = num.chars().all(|c| c.is_ascii_lowercase());
    if !upper && !lower {
        return None;
    }
    let deg = match num.to_ascii_uppercase().as_str() {
        "I" => 0,
        "II" => 1,
        "III" => 2,
        "IV" => 3,
        "V" => 4,
        "VI" => 5,
        "VII" => 6,
        _ => return None,
    };
    Some((acc, deg, lower, suffix))
}

/// A chord symbol (`Am7`, `F#dim`, `C/E`, or a roman numeral such as `V7`, `ii`, `bVII` in `key`) as MIDI
/// notes, low to high: the root in `octave` and the chord above it.
pub fn parse_chord_symbol(sym: &str, key: &Key, octave: i32) -> PRes<Vec<u8>> {
    let (body, bass) = match sym.split_once('/') {
        Some((b, bass)) => (b, Some(bass)),
        None => (sym, None),
    };
    let (root_pc, intervals): (i32, Vec<i32>) = if body
        .chars()
        .next()
        .is_some_and(|c| matches!(c, 'A'..='G'))
        && !is_roman_start(body)
    {
        let mut ch = body.chars();
        let mut pc = letter_pc(ch.next().expect("non-empty")).expect("A to G");
        let mut rest = ch.as_str();
        if let Some(r) = rest.strip_prefix('#') {
            pc += 1;
            rest = r;
        } else if let Some(r) = rest.strip_prefix('b') {
            // `Bb` is B flat; but `Bb5`-style qualities do not exist, so a leading b is a flat
            pc -= 1;
            rest = r;
        }
        let iv = quality(rest)
            .ok_or_else(|| ParseError::new(format!("`{sym}`: unknown chord quality `{rest}`")))?;
        (pc, iv.to_vec())
    } else if let Some((acc, deg, minor, suffix)) = roman(body) {
        let pc = key.tonic as i32 + key.scale()[deg] + acc;
        let q = if suffix.is_empty() {
            if minor {
                "m"
            } else {
                ""
            }
        } else if minor && !suffix.starts_with('m') && !matches!(suffix, "dim" | "dim7" | "m7b5") {
            // `ii7` is a minor seventh, `V7` a dominant one
            match suffix {
                "7" => "m7",
                s => s,
            }
        } else {
            suffix
        };
        let iv = quality(q)
            .ok_or_else(|| ParseError::new(format!("`{sym}`: unknown chord quality `{suffix}`")))?;
        (pc, iv.to_vec())
    } else {
        return Err(ParseError::new(format!(
            "`{sym}` is not a chord (like Am7, F#dim, C/E or a numeral such as V7)"
        )));
    };
    // the root sits in the octave; the chord stacks upward
    let root = at_octave(root_pc, octave);
    let mut out: Vec<u8> = intervals.iter().map(|i| (root + i) as u8).collect();
    if let Some(b) = bass {
        let bpc = if let Some((acc, deg, _, _)) =
            roman(b).filter(|_| !b.chars().next().is_some_and(|c| matches!(c, 'A'..='G')))
        {
            key.tonic as i32 + key.scale()[deg] + acc
        } else {
            let mut ch = b.chars();
            let mut pc = ch
                .next()
                .and_then(letter_pc)
                .ok_or_else(|| ParseError::new(format!("`{sym}`: `{b}` is not a bass note")))?;
            match ch.as_str() {
                "" => {}
                "#" => pc += 1,
                "b" => pc -= 1,
                o => {
                    return Err(ParseError::new(format!(
                        "`{sym}`: unexpected `{o}` in the bass note"
                    )))
                }
            }
            pc
        };
        out.insert(0, at_octave(bpc, octave - 1) as u8);
    }
    Ok(out)
}

fn is_roman_start(s: &str) -> bool {
    // `C`, `D`, ... are letters; only I V i v (and b/# prefixed) numerals are romans, never A-G
    s.starts_with(['I', 'V', 'i', 'v'])
}

#[derive(Clone, Debug, PartialEq)]
pub enum ChordKind {
    Block,
    ArpUp,
    ArpDown,
    Strum,
}

/// How a chord is played: all at once, as an arpeggio (one note every `rate`), or strummed.
#[derive(Clone, Debug, PartialEq)]
pub struct ChordStyle {
    pub kind: ChordKind,
    /// The arpeggio step as a fraction of a whole note.
    pub rate: (u64, u64),
    pub strum_ticks: u64,
    /// How much of its time a chord is held (0 to 1).
    pub hold: f64,
    pub vel: f64,
}
impl ChordStyle {
    pub fn block() -> ChordStyle {
        ChordStyle {
            kind: ChordKind::Block,
            rate: (1, 8),
            strum_ticks: 0,
            hold: 0.95,
            vel: DEFAULT_VEL,
        }
    }
    pub fn arp_up(n: u64, d: u64) -> ChordStyle {
        ChordStyle {
            kind: ChordKind::ArpUp,
            rate: (n, d),
            ..ChordStyle::block()
        }
    }
    pub fn arp_down(n: u64, d: u64) -> ChordStyle {
        ChordStyle {
            kind: ChordKind::ArpDown,
            rate: (n, d),
            ..ChordStyle::block()
        }
    }
    pub fn strum(ticks: u64) -> ChordStyle {
        ChordStyle {
            kind: ChordKind::Strum,
            strum_ticks: ticks,
            ..ChordStyle::block()
        }
    }
}

fn chord_events(notes: &[u8], start: u64, len: u64, st: &ChordStyle, out: &mut Vec<Ev>) {
    let held = ((len as f64 * st.hold) as u64).max(1);
    match st.kind {
        ChordKind::Block => {
            for &n in notes {
                out.push(Ev {
                    start,
                    len: held,
                    note: n,
                    vel: st.vel,
                    ..Ev::PLAIN
                });
            }
        }
        ChordKind::Strum => {
            for (k, &n) in notes.iter().enumerate() {
                let off = k as u64 * st.strum_ticks;
                out.push(Ev {
                    start: start + off,
                    len: held.saturating_sub(off).max(1),
                    note: n,
                    vel: st.vel,
                    ..Ev::PLAIN
                });
            }
        }
        ChordKind::ArpUp | ChordKind::ArpDown => {
            let step = (WHOLE * st.rate.0 / st.rate.1.max(1)).max(1);
            let mut seq: Vec<u8> = notes.to_vec();
            if st.kind == ChordKind::ArpDown {
                seq.reverse();
            }
            let mut k = 0usize;
            let mut t = 0;
            while t < len {
                out.push(Ev {
                    start: start + t,
                    len: (step as f64 * 0.9) as u64,
                    note: seq[k % seq.len()],
                    vel: st.vel,
                    ..Ev::PLAIN
                });
                k += 1;
                t += step;
            }
        }
    }
}

/// A chord progression: bars separated by `|`, the chords of a bar share it equally. `%` repeats the
/// previous chord and `.` is a rest. Chord symbols are letters (`Am7`) or roman numerals in `key`.
pub fn parse_progression(
    src: &str,
    meter: Meter,
    key: &Key,
    style: &ChordStyle,
    octave: i32,
) -> PRes<Vec<Ev>> {
    let bar = meter.bar_ticks();
    let mut out = vec![];
    let mut prev: Option<Vec<u8>> = None;
    for (bi, bar_src) in src.split('|').enumerate() {
        let toks: Vec<&str> = bar_src.split_whitespace().collect();
        if toks.is_empty() {
            continue;
        }
        if bar % toks.len() as u64 != 0 {
            return Err(ParseError::new(format!(
                "bar {} has {} chords, which do not divide it into whole ticks",
                bi + 1,
                toks.len()
            )));
        }
        let each = bar / toks.len() as u64;
        for (ti, t) in toks.iter().enumerate() {
            let start = bi as u64 * bar + ti as u64 * each;
            match *t {
                "." => prev = None,
                "%" => {
                    let c = prev.clone().ok_or_else(|| {
                        ParseError::new(format!("`%` in bar {} has no chord before it", bi + 1))
                    })?;
                    chord_events(&c, start, each, style, &mut out);
                }
                sym => {
                    let c = parse_chord_symbol(sym, key, octave)?;
                    chord_events(&c, start, each, style, &mut out);
                    prev = Some(c);
                }
            }
        }
    }
    Ok(out)
}

// ---- drums --------------------------------------------------------------------------------

fn drum(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        'k' => 36, // bass drum
        's' => 38, // snare
        'h' => 42, // closed hat
        'o' => 46, // open hat
        'c' => 49, // crash
        't' => 45, // tom
        'r' => 37, // rim shot
        'p' => 39, // clap
        _ => return None,
    })
}

/// A drum grid: `steps` equal steps per bar, one letter each (`k` kick, `s` snare, `h` closed hat, `o` open hat,
/// `c` crash, `t` tom, `r` rim, `p` clap; a capital is accented; `.` or `-` is a rest). Spaces are ignored and `|`
/// ends a bar, which must have exactly `steps` steps.
pub fn parse_beat(src: &str, steps: u32, meter: Meter) -> PRes<Vec<Ev>> {
    let bar = meter.bar_ticks();
    if steps == 0 || bar % steps as u64 != 0 {
        return Err(ParseError::new(format!(
            "{steps} steps do not divide a bar into whole ticks"
        )));
    }
    let step = bar / steps as u64;
    let mut out = vec![];
    for (bi, bar_src) in src.split('|').enumerate() {
        let cells: Vec<char> = bar_src.chars().filter(|c| !c.is_whitespace()).collect();
        if cells.is_empty() {
            continue;
        }
        if cells.len() != steps as usize {
            return Err(ParseError::new(format!(
                "bar {} has {} steps, the pattern has {steps} per bar",
                bi + 1,
                cells.len()
            )));
        }
        for (si, c) in cells.iter().enumerate() {
            if matches!(c, '.' | '-') {
                continue;
            }
            let n = drum(*c).ok_or_else(|| {
                ParseError::new(format!(
                    "`{c}` is not a drum (k s h o c t r p, or `.` for a rest)"
                ))
            })?;
            let accent = c.is_ascii_uppercase();
            out.push(Ev {
                start: bi as u64 * bar + si as u64 * step,
                len: step.min(PPQ / 2),
                note: n,
                vel: if accent { 0.95 } else { 0.7 },
                ..Ev::PLAIN
            });
        }
    }
    Ok(out)
}

// ---- time ---------------------------------------------------------------------------------

pub fn ticks_to_seconds(ticks: u64, bpm: f64) -> f64 {
    ticks as f64 / PPQ as f64 * 60.0 / bpm
}
pub fn seconds_to_ticks(sec: f64, bpm: f64) -> u64 {
    (sec * bpm / 60.0 * PPQ as f64).round() as u64
}

/// Swing: every off-beat eighth note is delayed by `amount` (0 to 1) of a third of a quarter note
/// (0.5 is a triplet feel, 1 a heavy shuffle).
pub fn apply_swing(events: &mut [Ev], amount: f64) {
    if amount <= 0.0 {
        return;
    }
    let delay = (amount * (PPQ as f64 / 3.0)).round() as u64;
    for e in events {
        if e.start % PPQ == PPQ / 2 {
            e.start += delay;
        }
    }
}

// ---- the score ----------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub struct Track {
    pub name: String,
    pub instrument: String,
    pub events: Vec<Ev>,
    pub volume: f64,
    /// -1 (left) to 1 (right).
    pub pan: f64,
    /// Curves over time (volume, pan, filter cutoff, brightness, reverb send).
    pub auto: automation::Automation,
}
impl Track {
    pub fn new(name: &str, instrument: &str, events: Vec<Ev>) -> Track {
        Track {
            name: name.into(),
            instrument: instrument.into(),
            events,
            volume: 1.0,
            pan: 0.0,
            auto: automation::Automation::default(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Score {
    pub tempo: f64,
    pub meter: Meter,
    pub key: Key,
    pub swing: f64,
    pub tracks: Vec<Track>,
}
impl Score {
    pub fn new(tempo: f64, meter: Meter) -> Score {
        Score {
            tempo,
            meter,
            key: Key::default(),
            swing: 0.0,
            tracks: vec![],
        }
    }
    pub fn length_ticks(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| t.events.iter())
            .map(|e| e.start + e.len)
            .max()
            .unwrap_or(0)
    }
    /// The length rounded up to whole bars (seconds): the period of a looped score.
    pub fn loop_seconds(&self) -> f64 {
        let bar = self.meter.bar_ticks();
        let bars = self.length_ticks().div_ceil(bar).max(1);
        ticks_to_seconds(bars * bar, self.tempo)
    }
    /// How strongly track `track` has just sounded at `tick`: the velocities of its recent notes, each fading
    /// away exponentially (about 8 per second), summed and capped at 1.
    pub fn pulse(&self, track: usize, tick: u64) -> f64 {
        let Some(t) = self.tracks.get(track) else {
            return 0.0;
        };
        let mut sum = 0.0;
        for e in &t.events {
            if e.start > tick {
                continue;
            }
            let dt = ticks_to_seconds(tick - e.start, self.tempo);
            if dt < 1.0 {
                sum += e.vel * (-dt * 8.0).exp();
            }
        }
        sum.min(1.0)
    }
    pub fn length_seconds(&self) -> f64 {
        ticks_to_seconds(self.length_ticks(), self.tempo)
    }
}

/// Apply a score's `swing` to every track.
pub fn apply_swing_to_score(score: &mut Score) {
    let amount = score.swing;
    for t in score.tracks.iter_mut() {
        apply_swing(&mut t.events, amount);
    }
}
