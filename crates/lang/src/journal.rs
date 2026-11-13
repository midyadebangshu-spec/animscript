//! The run journal: what happened during a run, for tools that show it (an editor's timeline, hierarchy and
//! console). It is a side table: nothing in it is part of a snapshot or of its hash.
use animscript_core::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemKind {
    /// A timed action (`s.move_by(...) over D`).
    Action,
    /// A procedural modifier (`wobble`, `noise`, `drive`).
    Modifier,
    /// A `keyframes` statement or the keys of a `timeline:` for one target.
    Keys,
}

/// Something that occupies a stretch of the timeline. Ticks are project ticks (indices into the snapshots).
#[derive(Clone, Debug)]
pub struct Item {
    pub kind: ItemKind,
    pub scene: String,
    /// The shape it acts on.
    pub target: String,
    pub label: String,
    pub start: i64,
    /// Exclusive for actions and modifiers; the last key for `Keys`. Only `None` while the scene is running.
    pub end: Option<i64>,
    /// The key ticks of a `Keys` item, ascending, without duplicates.
    pub keys: Vec<i64>,
    /// The statement that made it.
    pub span: Option<Span>,
    ids: Vec<u32>,
    made_at: i64,
}

#[derive(Clone, Debug)]
pub struct SceneSpan {
    pub name: String,
    pub start: i64,
    /// Exclusive: the first project tick after the scene.
    pub end: i64,
    pub span: Span,
}

#[derive(Clone, Debug)]
pub struct Note {
    /// `None` for a note made while building (before any scene).
    pub tick: Option<i64>,
    pub text: String,
    pub span: Option<Span>,
}

#[derive(Clone, Debug)]
pub struct Decl {
    pub name: String,
    /// `shape`, `prop`, `character`, `emitter`, `fluid`, `field` or `armature`.
    pub kind: String,
    pub span: Span,
}

#[derive(Clone, Debug, Default)]
pub struct Journal {
    pub scenes: Vec<SceneSpan>,
    pub items: Vec<Item>,
    pub notes: Vec<Note>,
    pub decls: Vec<Decl>,
    /// Project tick at which the running scene started, and whether a scene is running.
    base: i64,
    in_scene: bool,
    first_item: usize,
}

impl Journal {
    pub(crate) fn begin_scene(&mut self, name: &str, span: Span, start: i64) {
        self.base = start;
        self.in_scene = true;
        self.first_item = self.items.len();
        self.scenes.push(SceneSpan {
            name: name.to_string(),
            start,
            end: start,
            span,
        });
    }

    /// The scene is over at project tick `end`: whatever is still running stops there.
    pub(crate) fn end_scene(&mut self, end: i64) {
        self.in_scene = false;
        if let Some(s) = self.scenes.last_mut() {
            s.end = end;
        }
        for it in &mut self.items[self.first_item..] {
            if it.end.is_none_or(|e| e > end) {
                it.end = Some(end);
            }
            it.keys.retain(|k| *k < end.max(it.start + 1));
        }
    }

    pub(crate) fn decl(&mut self, name: &str, kind: &str, span: Span) {
        if !self.decls.iter().any(|d| d.name == name) {
            self.decls.push(Decl {
                name: name.to_string(),
                kind: kind.to_string(),
                span,
            });
        }
    }

    /// `tick` is the scene tick.
    pub(crate) fn note(&mut self, tick: i64, text: &str, span: Option<Span>) {
        self.notes.push(Note {
            tick: self.in_scene.then_some(self.base + tick),
            text: text.to_string(),
            span,
        });
    }

    /// Record a timed item. `start`, `end` and `keys` are scene ticks. The key sets one statement makes for
    /// one target in one tick (one per scalar in the runtime) are a single item.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn item(
        &mut self,
        kind: ItemKind,
        id: u32,
        now: i64,
        target: &str,
        label: &str,
        start: i64,
        end: Option<i64>,
        keys: &[i64],
        span: Option<Span>,
    ) {
        let b = self.base;
        let scene = self
            .scenes
            .last()
            .map(|s| s.name.clone())
            .unwrap_or_default();
        if kind == ItemKind::Keys {
            let same = self.items[self.first_item..].iter_mut().find(|i| {
                i.kind == ItemKind::Keys && i.made_at == now && i.target == target && i.span == span
            });
            if let Some(i) = same {
                i.ids.push(id);
                i.start = i.start.min(b + start);
                i.end = match (i.end, end) {
                    (Some(a), Some(e)) => Some(a.max(b + e)),
                    _ => None,
                };
                i.keys.extend(keys.iter().map(|k| b + k));
                i.keys.sort();
                i.keys.dedup();
                return;
            }
        }
        self.items.push(Item {
            kind,
            scene,
            target: target.to_string(),
            label: label.to_string(),
            start: b + start,
            end: end.map(|e| b + e),
            keys: keys.iter().map(|k| b + k).collect(),
            span,
            ids: vec![id],
            made_at: now,
        });
    }

    /// An action or modifier was cancelled or interrupted at scene tick `tick`.
    pub(crate) fn stop(&mut self, id: u32, tick: i64) {
        let at = self.base + tick;
        if let Some(i) = self.items[self.first_item..]
            .iter_mut()
            .find(|i| i.ids.contains(&id))
        {
            i.end = Some(i.end.map_or(at, |e| e.min(at)).max(i.start));
        }
    }
}
