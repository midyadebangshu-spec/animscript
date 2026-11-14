//! Loading sample libraries described by SFZ files (the plain-text mapping format that VSCO 2 and the Sonatina
//! orchestra are distributed with). Supported: `<control> default_path`, `<global>`, `<group>`, `<region>` with
//! `sample`, `key`, `lokey`, `hikey`, `pitch_keycenter`, `lovel`, `hivel`, `loop_mode`, `loop_start`, `loop_end` and
//! `volume` (dB). Samples are WAV (PCM or float); a loop stored in the WAV's `smpl` chunk is used when the SFZ
//! does not give one. Anything else is ignored. Samples shared by several regions are read once.
use crate::sampler::{SampleSet, Zone};
use crate::wav::read_wav;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// `c4`, `F#3`, `db2` or a number: a MIDI key.
pub fn key_of(s: &str) -> Option<u8> {
    if let Ok(n) = s.parse::<i32>() {
        return (0..=127).contains(&n).then_some(n as u8);
    }
    let mut ch = s.chars();
    let letter = ch.next()?.to_ascii_uppercase();
    let rest: String = ch.collect();
    crate::pitch(&format!("{letter}{}", rest.replace('B', "b")))
}

#[derive(Clone, Default)]
struct Ops(BTreeMap<String, String>);

fn opcodes(line: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vec![];
    for tok in line.split_whitespace() {
        if let Some((k, v)) = tok.split_once('=') {
            out.push((k.to_string(), v.to_string()));
        } else if let Some(last) = out.last_mut() {
            last.1.push(' ');
            last.1.push_str(tok);
        }
    }
    out
}

/// Parse SFZ `text`; sample paths are resolved against `dir` (and `default_path`).
pub fn parse_sfz(text: &str, dir: &Path) -> Result<SampleSet, String> {
    let (mut control, mut global, mut group) = (Ops::default(), Ops::default(), Ops::default());
    let mut regions: Vec<Ops> = vec![];
    let mut cur: Option<(&str, Ops)> = None;
    let flush = |cur: &mut Option<(&str, Ops)>,
                 regions: &mut Vec<Ops>,
                 control: &mut Ops,
                 global: &mut Ops,
                 group: &mut Ops| {
        if let Some((h, ops)) = cur.take() {
            match h {
                "control" => control.0.extend(ops.0),
                "global" => global.0.extend(ops.0),
                "group" => *group = ops,
                _ => {
                    let mut all = global.0.clone();
                    all.extend(group.0.clone());
                    all.extend(ops.0);
                    regions.push(Ops(all));
                }
            }
        }
    };
    for raw in text.lines() {
        let line = raw.split("//").next().unwrap_or("").trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        // a line may start new headers and carry opcodes
        let mut rest = line;
        while let Some(pos) = rest.find('<') {
            let before = &rest[..pos];
            if let Some((_, ops)) = cur.as_mut() {
                ops.0.extend(opcodes(before));
            }
            let end = rest[pos..]
                .find('>')
                .ok_or("an SFZ header `<` is not closed")?
                + pos;
            let name = &rest[pos + 1..end];
            flush(
                &mut cur,
                &mut regions,
                &mut control,
                &mut global,
                &mut group,
            );
            if name == "group" {
                group = Ops::default();
            }
            cur = Some((
                match name {
                    "control" => "control",
                    "global" => "global",
                    "group" => "group",
                    "region" => "region",
                    _ => "other",
                },
                Ops::default(),
            ));
            rest = &rest[end + 1..];
        }
        if let Some((_, ops)) = cur.as_mut() {
            ops.0.extend(opcodes(rest));
        }
    }
    flush(
        &mut cur,
        &mut regions,
        &mut control,
        &mut global,
        &mut group,
    );
    let default_path = control
        .0
        .get("default_path")
        .cloned()
        .unwrap_or_default()
        .replace('\\', "/");
    let mut cache: BTreeMap<String, (Arc<Vec<f32>>, u32, Option<(usize, usize)>)> = BTreeMap::new();
    let mut zones = vec![];
    for r in &regions {
        let g = |k: &str| r.0.get(k).map(String::as_str);
        let Some(sample) = g("sample") else { continue };
        let rel = format!("{default_path}{}", sample.replace('\\', "/"));
        if !cache.contains_key(&rel) {
            let path = dir.join(&rel);
            let bytes = std::fs::read(&path)
                .map_err(|e| format!("cannot read sample {}: {e}", path.display()))?;
            let w = read_wav(&bytes).map_err(|e| format!("{}: {e}", path.display()))?;
            cache.insert(rel.clone(), (Arc::new(w.mono()), w.rate, w.loop_points));
        }
        let (data, rate, wav_loop) = cache[&rel].clone();
        let key = |k: &str| -> Result<Option<u8>, String> {
            g(k).map(|v| key_of(v).ok_or_else(|| format!("`{k}={v}` is not a key")))
                .transpose()
        };
        let (lo, hi, root) = match key("key")? {
            Some(k) => (k, k, k),
            None => {
                let root = key("pitch_keycenter")?;
                let lo = key("lokey")?.unwrap_or(0);
                let hi = key("hikey")?.unwrap_or(127);
                (lo, hi, root.unwrap_or(lo))
            }
        };
        let root = key("pitch_keycenter")?.unwrap_or(root);
        let num = |k: &str, d: f64| g(k).and_then(|v| v.parse::<f64>().ok()).unwrap_or(d);
        let looping = matches!(
            g("loop_mode"),
            Some("loop_continuous") | Some("loop_sustain")
        );
        let loop_range = if looping {
            match (
                g("loop_start").and_then(|v| v.parse().ok()),
                g("loop_end").and_then(|v| v.parse::<usize>().ok()),
            ) {
                (Some(s), Some(e)) => Some((s, e)),
                _ => wav_loop,
            }
        } else {
            None
        };
        zones.push(Zone {
            lo_key: lo,
            hi_key: hi,
            lo_vel: num("lovel", 0.0).max(0.0) as u8,
            hi_vel: num("hivel", 127.0).min(127.0) as u8,
            root,
            rate,
            data,
            loop_range,
            gain: 10f64.powf(num("volume", 0.0) / 20.0),
        });
    }
    if zones.is_empty() {
        return Err("the SFZ file has no regions with a sample".into());
    }
    Ok(SampleSet::new(zones))
}

pub fn load_sfz(path: &Path) -> Result<SampleSet, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    parse_sfz(&text, path.parent().unwrap_or(Path::new(".")))
}
