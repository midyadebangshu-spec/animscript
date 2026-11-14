//! Music in the language: `score name:` blocks (tempo, key, tracks of notes, chords or drum patterns),
//! `play(score)` and `stop(score)` placing them on the project's timeline. The notation is parsed here with
//! `animscript-audio`; the sound itself is rendered afterwards by the `anim audio` command from the cues
//! this run collected.
use crate::ast::{ArmItem, ScoreDecl};
use crate::interp::{Interp, Mode};
use crate::value::*;
use animscript_audio as au;
use animscript_core::*;
use std::collections::BTreeMap;

type Args = Vec<(Option<String>, Value)>;

fn e1(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-AUD-001", msg)
}
fn e2(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-AUD-002", msg)
}
fn e3(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-AUD-003", msg)
}
fn e4(msg: impl Into<String>) -> Diagnostic {
    Diagnostic::new("E-AUD-004", msg)
}

#[derive(Clone, Debug)]
pub struct ScoreDef {
    pub score: au::Score,
    pub mix: au::Mix,
    /// A folder whose `<instrument>.sfz` files supply instruments that are not built in.
    pub library: Option<String>,
    /// Instruments given an SFZ file directly by a track's `sfz:`.
    pub sfz: BTreeMap<String, String>,
}

/// One `play` of a score on the project timeline.
#[derive(Clone, Debug, PartialEq)]
pub struct AudioCue {
    pub score: String,
    pub start_tick: i64,
    pub volume: f64,
    pub fade_in: f64,
    pub fade_out: f64,
    pub looping: bool,
    pub stop_tick: Option<i64>,
    pub offset: f64,
}

#[derive(Clone, Debug, Default)]
pub struct AudioOut {
    pub scores: BTreeMap<String, ScoreDef>,
    pub cues: Vec<AudioCue>,
    pub fps: f64,
    /// The number of ticks of the whole run.
    pub ticks: usize,
}

const SETTINGS: &[&str] = &[
    "tempo",
    "meter",
    "key",
    "swing",
    "reverb",
    "room",
    "delay",
    "delay_time",
    "delay_feedback",
    "master",
    "rate",
    "library",
];
const TRACK_KEYS: &[&str] = &[
    "instrument",
    "notes",
    "chords",
    "beat",
    "style",
    "rate",
    "strum",
    "hold",
    "vel",
    "steps",
    "octave",
    "volume",
    "pan",
    "repeat",
    "bars",
    "transpose",
    "sfz",
];

impl Interp {
    fn audio_num(&mut self, key: &str, e: &crate::ast::Expr, span: Span) -> Res<f64> {
        let vars = crate::interp::Vars::new();
        let v = self.eval(e, &vars, Mode::Build)?;
        self.num(&v, &format!("`{key}`"))
            .map_err(|d| e1(d.message).at(span))
    }

    fn audio_str(&mut self, key: &str, e: &crate::ast::Expr, span: Span) -> Res<String> {
        let vars = crate::interp::Vars::new();
        match self.eval(e, &vars, Mode::Build)? {
            Value::Str(s) => Ok(s),
            o => Err(e1(format!("`{key}` must be text, found {}", o.type_name())).at(span)),
        }
    }

    pub fn eval_scores(&mut self) -> Res<()> {
        let decls = self.program.scores.clone();
        for d in &decls {
            if self.scores.contains_key(&d.name) {
                return Err(e1(format!("there is already a score named `{}`", d.name)).at(d.span));
            }
            let def = self.eval_score(d)?;
            self.scores.insert(d.name.clone(), def);
        }
        Ok(())
    }

    fn eval_score(&mut self, d: &ScoreDecl) -> Res<ScoreDef> {
        let mut tempo = 120.0;
        let mut meter = au::Meter::new(4, 4);
        let mut key = au::Key::default();
        let mut mix = au::Mix::default();
        let mut swing = 0.0;
        let mut library = None;
        for (k, e, span) in &d.settings {
            if !SETTINGS.contains(&k.as_str()) {
                return Err(e1(format!("a score has no setting `{k}`"))
                    .at(*span)
                    .help(format!("settings: {}", SETTINGS.join(", "))));
            }
            match k.as_str() {
                "tempo" => {
                    tempo = self.audio_num(k, e, *span)?;
                    if !(20.0..=400.0).contains(&tempo) {
                        return Err(e1("`tempo` is beats per minute between 20 and 400").at(*span));
                    }
                }
                "meter" => {
                    let s = self.audio_str(k, e, *span)?;
                    let ok = s.split_once('/').and_then(|(n, dn)| {
                        Some((
                            n.trim().parse::<u32>().ok()?,
                            dn.trim().parse::<u32>().ok()?,
                        ))
                    });
                    match ok {
                        Some((n, dn)) if n > 0 && matches!(dn, 1 | 2 | 4 | 8 | 16) => {
                            meter = au::Meter::new(n, dn)
                        }
                        _ => {
                            return Err(
                                e1("`meter` is text such as \"4/4\", \"3/4\" or \"6/8\"").at(*span)
                            )
                        }
                    }
                }
                "key" => {
                    let s = self.audio_str(k, e, *span)?;
                    key = au::Key::parse(&s).map_err(|p| e1(p.message).at(*span))?;
                }
                "swing" => {
                    swing = self.audio_num(k, e, *span)?;
                    if !(0.0..=1.0).contains(&swing) {
                        return Err(e1("`swing` is between 0 and 1").at(*span));
                    }
                }
                "reverb" | "room" | "delay" | "delay_feedback" => {
                    let v = self.audio_num(k, e, *span)?;
                    if !(0.0..=1.0).contains(&v) {
                        return Err(e1(format!("`{k}` is between 0 and 1")).at(*span));
                    }
                    match k.as_str() {
                        "reverb" => mix.reverb = v,
                        "room" => mix.room = v,
                        "delay" => mix.delay = v,
                        _ => mix.delay_feedback = v,
                    }
                }
                "delay_time" => {
                    mix.delay_time = self.audio_num(k, e, *span)?;
                    if !(0.01..=4.0).contains(&mix.delay_time) {
                        return Err(e1("`delay_time` is seconds between 0.01 and 4").at(*span));
                    }
                }
                "master" => {
                    mix.master = self.audio_num(k, e, *span)?;
                    if !(0.0..=4.0).contains(&mix.master) {
                        return Err(e1("`master` is a gain between 0 and 4").at(*span));
                    }
                }
                "rate" => {
                    let r = self.audio_num(k, e, *span)?;
                    if ![
                        8000.0, 11025.0, 16000.0, 22050.0, 32000.0, 44100.0, 48000.0, 96000.0,
                    ]
                    .contains(&r)
                    {
                        return Err(e1("`rate` is a sample rate such as 44100 or 48000").at(*span));
                    }
                    mix.rate = r as u32;
                }
                _ => library = Some(self.audio_str(k, e, *span)?),
            }
        }
        let mut score = au::Score::new(tempo, meter);
        score.key = key;
        score.swing = swing;
        let mut sfz = BTreeMap::new();
        for t in &d.tracks {
            let (track, sfz_path) = self.eval_track(&d.name, t, &score)?;
            if let Some(p) = sfz_path {
                sfz.insert(track.instrument.clone(), p);
            } else if au::synth::builtin(&track.instrument).is_none() && library.is_none() {
                return Err(e3(format!("unknown instrument `{}` in track `{}`", track.instrument, t.name))
                    .at(t.span)
                    .help(format!("built in: {}; or give the score a `library:` folder or the track an `sfz:` file", au::synth::BUILTIN_NAMES.join(", "))));
            }
            score.tracks.push(track);
        }
        if score.tracks.is_empty() {
            return Err(e1(format!("score `{}` has no tracks", d.name))
                .at(d.span)
                .help("add `track lead: instrument: \"piano\", notes: \"C4:1/4 E4 G4\"`"));
        }
        au::apply_swing_to_score(&mut score);
        Ok(ScoreDef {
            score,
            mix,
            library,
            sfz,
        })
    }

    fn eval_track(
        &mut self,
        score_name: &str,
        t: &ArmItem,
        score: &au::Score,
    ) -> Res<(au::Track, Option<String>)> {
        let mut instrument = "piano".to_string();
        let (mut notation, mut kind): (Option<String>, Option<&str>) = (None, None);
        let (mut style, mut rate, mut strum) = ("block".to_string(), (1u64, 8u64), 30u64);
        let (mut hold, mut vel, mut steps, mut octave) = (0.95, 0.7, 16u32, 3i32);
        let (mut volume, mut pan, mut repeat, mut bars, mut transpose) =
            (1.0, 0.0, 1usize, 0u64, 0i32);
        let mut sfz = None;
        let vars = crate::interp::Vars::new();
        for (k, e, span) in &t.attrs {
            if !TRACK_KEYS.contains(&k.as_str()) {
                return Err(e1(format!("a track has no setting `{k}`"))
                    .at(*span)
                    .help(format!("settings: {}", TRACK_KEYS.join(", "))));
            }
            match k.as_str() {
                "instrument" => instrument = self.audio_str(k, e, *span)?,
                "sfz" => sfz = Some(self.audio_str(k, e, *span)?),
                "notes" | "chords" | "beat" => {
                    if kind.is_some() {
                        return Err(e1(format!(
                            "track `{}` has more than one of `notes`, `chords` and `beat`",
                            t.name
                        ))
                        .at(*span)
                        .help("make one track for each"));
                    }
                    kind = Some(match k.as_str() {
                        "notes" => "notes",
                        "chords" => "chords",
                        _ => "beat",
                    });
                    notation = Some(match self.eval(e, &vars, Mode::Build)? {
                        Value::Str(s) => s,
                        Value::List(items) => {
                            let mut parts = vec![];
                            for it in items {
                                match it {
                                    Value::Str(s) => parts.push(s),
                                    o => {
                                        return Err(e1(format!(
                                            "`{k}` lists text, found {}",
                                            o.type_name()
                                        ))
                                        .at(*span))
                                    }
                                }
                            }
                            parts.join(" | ")
                        }
                        o => {
                            return Err(e1(format!(
                                "`{k}` must be text or a list of text, found {}",
                                o.type_name()
                            ))
                            .at(*span))
                        }
                    });
                }
                "style" => {
                    style = self.audio_str(k, e, *span)?;
                    if !matches!(style.as_str(), "block" | "arp_up" | "arp_down" | "strum") {
                        return Err(e1(
                            "`style` is \"block\", \"arp_up\", \"arp_down\" or \"strum\"",
                        )
                        .at(*span));
                    }
                }
                "rate" => {
                    let s = self.audio_str(k, e, *span)?;
                    let r = s
                        .split_once('/')
                        .and_then(|(n, d)| Some((n.parse::<u64>().ok()?, d.parse::<u64>().ok()?)));
                    match r {
                        Some((n, d)) if n > 0 && d > 0 && (au::WHOLE * n).is_multiple_of(d) => {
                            rate = (n, d)
                        }
                        _ => {
                            return Err(
                                e1("`rate` is a note length such as \"1/8\" or \"1/16\"").at(*span)
                            )
                        }
                    }
                }
                _ => {
                    let v = self.audio_num(k, e, *span)?;
                    match k.as_str() {
                        "strum" => strum = v.max(0.0) as u64,
                        "hold" => hold = v.clamp(0.05, 1.0),
                        "vel" => vel = v.clamp(0.0, 1.0),
                        "steps" => {
                            if v < 1.0 {
                                return Err(e1("`steps` is the number of steps in a bar").at(*span));
                            }
                            steps = v as u32
                        }
                        "octave" => octave = v as i32,
                        "volume" => {
                            if !(0.0..=4.0).contains(&v) {
                                return Err(e1("`volume` is a gain between 0 and 4").at(*span));
                            }
                            volume = v
                        }
                        "pan" => {
                            if !(-1.0..=1.0).contains(&v) {
                                return Err(
                                    e1("`pan` is between -1 (left) and 1 (right)").at(*span)
                                );
                            }
                            pan = v
                        }
                        "repeat" => {
                            if !(1.0..=1000.0).contains(&v) {
                                return Err(e1("`repeat` is how many times to play it, 1 or more")
                                    .at(*span));
                            }
                            repeat = v as usize
                        }
                        "bars" => {
                            if v < 0.0 {
                                return Err(e1(
                                    "`bars` is how many bars to wait before the track starts",
                                )
                                .at(*span));
                            }
                            bars = v as u64
                        }
                        _ => transpose = v as i32,
                    }
                }
            }
        }
        let (Some(kind), Some(src)) = (kind, notation) else {
            return Err(e1(format!("track `{}` plays nothing", t.name))
                .at(t.span)
                .help("give it `notes:`, `chords:` or `beat:`"));
        };
        let bar = score.meter.bar_ticks();
        let wrap = |p: au::ParseError| {
            e2(format!(
                "track `{}` of score `{}`: {}",
                t.name, score_name, p.message
            ))
            .at(t.span)
        };
        let (mut events, pass) = match kind {
            "notes" => {
                let ev = au::parse_notes(&src, score.meter).map_err(wrap)?;
                let end = ev.iter().map(|e| e.start + e.len).max().unwrap_or(0);
                (ev, end.div_ceil(bar).max(1) * bar)
            }
            "chords" => {
                let st = chord_style(&style, rate, strum, hold, vel);
                let ev = au::parse_progression(&src, score.meter, &score.key, &st, octave)
                    .map_err(wrap)?;
                (ev, bars_in(&src) * bar)
            }
            _ => {
                let ev = au::parse_beat(&src, steps, score.meter).map_err(wrap)?;
                (ev, bars_in(&src) * bar)
            }
        };
        if transpose != 0 && kind != "beat" {
            for e in events.iter_mut() {
                e.note = (e.note as i32 + transpose).clamp(0, 127) as u8;
            }
        }
        let base = events.clone();
        for r in 1..repeat {
            for e in &base {
                let mut c = *e;
                c.start += r as u64 * pass;
                events.push(c);
            }
        }
        for e in events.iter_mut() {
            e.start += bars * bar;
        }
        let mut track = au::Track::new(&t.name, &instrument, events);
        track.volume = volume;
        track.pan = pan;
        Ok((track, sfz))
    }

    /// The score's state at this tick, from its latest `play`: `playing`, `time` (seconds into it), `beat` and `bar`
    /// (phase 0 to 1), `pulse` (1 at each beat, fading), `beat_now` and `bar_now` (true for the one tick a beat or bar
    /// starts), and a level (0 to 1) for each track by its name.
    pub fn score_member(&self, sc: &str, name: &str) -> Res<Value> {
        let def = &self.scores[sc];
        let score = &def.score;
        let track = score.tracks.iter().position(|t| t.name == name);
        const FIXED: &[&str] = &[
            "playing", "time", "beat", "bar", "pulse", "beat_now", "bar_now",
        ];
        if track.is_none() && !FIXED.contains(&name) {
            let mut all: Vec<&str> = FIXED.to_vec();
            all.extend(score.tracks.iter().map(|t| t.name.as_str()));
            return Err(Diagnostic::new(
                "E-NAM-003",
                format!("score `{sc}` has no member `{name}`"),
            )
            .help(format!("it has: {}", all.join(", "))));
        }
        let fps = self.fps.to_f64();
        // the latest cue that has started and not ended
        let tick = self.project_tick;
        let cue = self.audio_cues.iter().rev().find(|c| {
            c.score == sc
                && c.start_tick <= tick
                && c.stop_tick
                    .is_none_or(|s| tick < s + (c.fade_out * fps).ceil() as i64)
        });
        let at = |t_ticks: i64| -> Option<(f64, u64)> {
            // music time in seconds and ticks for a project tick, looping where the cue loops
            let c = cue?;
            let mut secs = (t_ticks - c.start_tick) as f64 / fps + c.offset;
            if c.looping {
                let len = score.loop_seconds();
                secs = secs.rem_euclid(len);
            }
            (secs >= 0.0).then(|| (secs, au::seconds_to_ticks(secs, score.tempo)))
        };
        let Some((secs, ticks)) = at(tick) else {
            return Ok(match name {
                "playing" | "beat_now" | "bar_now" => Value::Bool(false),
                _ => Value::Num(0.0),
            });
        };
        let beat_ticks = au::PPQ;
        let bar_ticks = score.meter.bar_ticks();
        let phase = |len: u64| (ticks % len) as f64 / len as f64;
        Ok(match name {
            "playing" => Value::Bool(true),
            "time" => Value::Num(secs),
            "beat" => Value::Num(phase(beat_ticks)),
            "bar" => Value::Num(phase(bar_ticks)),
            "pulse" => Value::Num((-phase(beat_ticks) * 4.0).exp()),
            "beat_now" | "bar_now" => {
                let len = if name == "beat_now" {
                    beat_ticks
                } else {
                    bar_ticks
                };
                // a boundary was crossed since the previous tick (or the music has just begun)
                let prev = at(tick - 1).map(|p| p.1 / len);
                Value::Bool(prev != Some(ticks / len))
            }
            _ => Value::Num(score.pulse(track.expect("checked"), ticks)),
        })
    }

    /// `play(score, volume: 1, fade_in: 0.s, fade_out: 0.s, loop: false, offset: 0.s)` and
    /// `stop(score, fade_out: 0.s)`; also `score.play(...)` and `score.stop(...)`.
    pub(crate) fn audio_call(
        &mut self,
        name: &str,
        recv: Option<String>,
        mut args: Args,
        mode: Mode,
    ) -> Res<Value> {
        if mode != Mode::Run {
            return Err(e4(format!("`{name}` works while a scene runs"))
                .help("use it in `main:`, a rule or an action"));
        }
        if let Some(sc) = recv {
            args.insert(0, (Some("score".into()), Value::Score(sc)));
        }
        let params: &[&str] = if name == "play" {
            &["score", "volume", "fade_in", "fade_out", "loop", "offset"]
        } else {
            &["score", "fade_out"]
        };
        let m = self.bind(name, args, params, &["score"])?;
        let Value::Score(sc) = &m["score"] else {
            return Err(e4(format!(
                "`{name}` needs a score declared with `score name:`, found {}",
                m["score"].type_name()
            )));
        };
        let secs = |v: &Value, what: &str| -> Res<f64> {
            match v {
                Value::Dur(r) => Ok(r.to_f64()),
                Value::Num(n) => Ok(*n),
                Value::Int(n) => Ok(*n as f64),
                o => Err(e4(format!(
                    "`{what}` is a duration such as 0.5.seconds, found {}",
                    o.type_name()
                ))),
            }
        };
        let fade_out = m
            .get("fade_out")
            .map(|v| secs(v, "fade_out"))
            .transpose()?
            .unwrap_or(0.0);
        if fade_out < 0.0 {
            return Err(e4("`fade_out` cannot be negative"));
        }
        if name == "stop" {
            let tick = self.project_tick;
            let Some(c) = self
                .audio_cues
                .iter_mut()
                .rev()
                .find(|c| c.score == *sc && c.stop_tick.is_none())
            else {
                return Err(e4(format!(
                    "`{sc}` is not playing, there is nothing to stop"
                )));
            };
            c.stop_tick = Some(tick);
            c.fade_out = fade_out;
            return Ok(Value::None);
        }
        let volume = match m.get("volume") {
            Some(v) => self.num(v, "`volume`").map_err(|e| e4(e.message))?,
            None => 1.0,
        };
        if !(0.0..=4.0).contains(&volume) {
            return Err(e4("`volume` is a gain between 0 and 4"));
        }
        let fade_in = m
            .get("fade_in")
            .map(|v| secs(v, "fade_in"))
            .transpose()?
            .unwrap_or(0.0);
        let offset = m
            .get("offset")
            .map(|v| secs(v, "offset"))
            .transpose()?
            .unwrap_or(0.0);
        if fade_in < 0.0 || offset < 0.0 {
            return Err(e4("`fade_in` and `offset` cannot be negative"));
        }
        let looping = match m.get("loop") {
            None => false,
            Some(Value::Bool(b)) => *b,
            Some(o) => {
                return Err(e4(format!(
                    "`loop` is true or false, found {}",
                    o.type_name()
                )))
            }
        };
        self.audio_cues.push(AudioCue {
            score: sc.clone(),
            start_tick: self.project_tick,
            volume,
            fade_in,
            fade_out,
            looping,
            stop_tick: None,
            offset,
        });
        Ok(Value::None)
    }
}

fn chord_style(style: &str, rate: (u64, u64), strum: u64, hold: f64, vel: f64) -> au::ChordStyle {
    let mut s = match style {
        "arp_up" => au::ChordStyle::arp_up(rate.0, rate.1),
        "arp_down" => au::ChordStyle::arp_down(rate.0, rate.1),
        "strum" => au::ChordStyle::strum(strum),
        _ => au::ChordStyle::block(),
    };
    s.hold = hold;
    s.vel = vel;
    s
}

/// The number of non-empty bars in a `|`-separated pattern.
fn bars_in(src: &str) -> u64 {
    src.split('|')
        .filter(|b| !b.trim().is_empty())
        .count()
        .max(1) as u64
}

/// Render every score of a run and place the cues on the project's timeline: the whole soundtrack.
/// Sample libraries (`library:` folders and `sfz:` files) are looked up relative to `base`. `rate` overrides
/// the score's sample rate (the first score's, else 44100). Errors name the instrument or file.
pub fn render_project_audio(
    out: &AudioOut,
    base: &std::path::Path,
    rate: Option<u32>,
) -> Result<au::Stereo, String> {
    if out.cues.is_empty() {
        return Err("the program never calls `play(...)`, so there is no sound".into());
    }
    let rate = rate
        .or_else(|| out.scores.values().next().map(|s| s.mix.rate))
        .unwrap_or(44100);
    let mut names: Vec<&String> = vec![];
    for c in &out.cues {
        if !names.contains(&&c.score) {
            names.push(&c.score);
        }
    }
    let mut rendered: Vec<au::Stereo> = vec![];
    for n in &names {
        let def = &out.scores[*n];
        let mut bank = au::Bank::new();
        for (inst, path) in &def.sfz {
            bank.load_sfz(inst, &base.join(path))
                .map_err(|e| format!("instrument `{inst}`: {e}"))?;
        }
        if let Some(lib) = &def.library {
            for t in &def.score.tracks {
                if au::synth::builtin(&t.instrument).is_none()
                    && !def.sfz.contains_key(&t.instrument)
                {
                    let path = base.join(lib).join(format!("{}.sfz", t.instrument));
                    bank.load_sfz(&t.instrument, &path)
                        .map_err(|e| format!("instrument `{}`: {e}", t.instrument))?;
                }
            }
        }
        let mix = au::Mix {
            rate,
            ..def.mix.clone()
        };
        rendered.push(
            au::render_score(&def.score, &mix, &bank).map_err(|e| format!("score `{n}`: {e}"))?,
        );
    }
    let cues: Vec<au::mixer::Cue> = out
        .cues
        .iter()
        .map(|c| {
            let idx = names.iter().position(|n| **n == c.score).expect("rendered");
            let def = &out.scores[&c.score];
            au::mixer::Cue {
                source: idx,
                start: c.start_tick as f64 / out.fps,
                volume: c.volume,
                fade_in: c.fade_in,
                fade_out: c.fade_out,
                looping: c.looping,
                loop_len: def.score.loop_seconds(),
                stop: c.stop_tick.map(|t| t as f64 / out.fps),
                offset: c.offset,
            }
        })
        .collect();
    Ok(au::mixer::mix_cues(
        &rendered,
        &cues,
        out.ticks as f64 / out.fps,
        rate,
    ))
}
