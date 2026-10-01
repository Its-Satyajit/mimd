//! The document: its text, the cursor, and the edit history.
//!
//! Nothing here draws or measures. `App` is the mutable model, and it is the
//! only place document state changes — which is what lets undo, saving and the
//! dirty flag agree with each other.

#[cfg(test)]
#[path = "document_tests.rs"]
mod tests;
use std::path::PathBuf;

use minifb::Key;

use super::theme::Row;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    View,
    Edit,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
pub struct Pos {
    pub l: usize,
    pub c: usize,
}

pub struct App {
    pub lines: Vec<String>,
    pub cur: Pos,
    pub anchor: Option<Pos>,
    pub mode: Mode,
    pub scroll: usize,
    /// Horizontal scroll in pixels (edit mode only).
    pub hscroll: u32,
    /// True while a left-button drag is selecting.
    pub dragging: bool,
    /// Path being typed at the open prompt; `Some` means the prompt is up.
    pub prompt: Option<String>,
    /// Where the current mouse press began, before any drag.
    pub press: Option<Pos>,
    /// Edit history. `None` records a fresh undo point.
    pub undo: Vec<Snapshot>,
    pub redo: Vec<Snapshot>,
    /// Tag identifying the edit that produced the current state, so a run of
    /// similar keystrokes collapses into one undo step.
    pub last_edit: Option<EditKind>,
    /// Whether the document ended with a newline. Text files conventionally
    /// do, and dropping it would make open-then-save rewrite every file.
    pub trailing_newline: bool,
    pub dirty: bool,
    pub path: Option<PathBuf>,
    pub status: String,
    pub clip: String,
    pub quit: bool,
}

/// A restorable document state.
#[derive(Clone, PartialEq, Debug)]
pub struct Snapshot {
    lines: Vec<String>,
    cur: Pos,
    path: Option<PathBuf>,
}

/// What kind of edit just happened, for coalescing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum EditKind {
    Type,
    Other,
}

/// Consecutive undo steps retained. Enough for a real editing session,
/// small enough that snapshots cannot grow without bound.
const MAX_UNDO: usize = 200;

impl App {
    /// Record an undo point before mutating.
    ///
    /// `kind == Type` coalesces with a preceding run of typing so a word is
    /// one undo step rather than one per character.
    pub fn mark(&mut self, kind: EditKind) {
        if self.last_edit == Some(kind) && kind == EditKind::Type {
            self.redo.clear();
            return;
        }
        self.undo.push(Snapshot {
            lines: self.lines.clone(),
            cur: self.cur,
            path: self.path.clone(),
        });
        if self.undo.len() > MAX_UNDO {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.last_edit = Some(kind);
    }

    /// Break the coalescing run so the next edit starts a new undo step.
    pub fn break_run(&mut self) {
        self.last_edit = None;
    }

    pub fn undo(&mut self) -> bool {
        let Some(snap) = self.undo.pop() else {
            return false;
        };
        self.redo.push(Snapshot {
            lines: self.lines.clone(),
            cur: self.cur,
            path: self.path.clone(),
        });
        self.lines = snap.lines;
        self.cur = snap.cur;
        self.path = snap.path;
        self.anchor = None;
        self.break_run();
        self.dirty = true;
        self.follow(1);
        true
    }

    pub fn redo(&mut self) -> bool {
        let Some(snap) = self.redo.pop() else {
            return false;
        };
        self.undo.push(Snapshot {
            lines: self.lines.clone(),
            cur: self.cur,
            path: self.path.clone(),
        });
        self.lines = snap.lines;
        self.cur = snap.cur;
        self.path = snap.path;
        self.anchor = None;
        self.break_run();
        self.dirty = true;
        self.follow(1);
        true
    }

    pub fn open(text: String, path: Option<PathBuf>, mode: Mode) -> Self {
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let trailing_newline = text.ends_with('\n');
        App {
            lines,
            cur: Pos::default(),
            anchor: None,
            mode,
            scroll: 0,
            hscroll: 0,
            dragging: false,
            prompt: None,
            press: None,
            undo: Vec::new(),
            redo: Vec::new(),
            last_edit: None,
            trailing_newline,
            dirty: false,
            path,
            status: String::new(),
            clip: String::new(),
            quit: false,
        }
    }

    pub fn text(&self) -> String {
        let mut t = self.lines.join("\n");
        if self.trailing_newline && !t.ends_with('\n') {
            t.push('\n');
        }
        t
    }

    fn clamp(&mut self) {
        self.cur.l = self.cur.l.min(self.lines.len() - 1);
        self.cur.c = self.cur.c.min(self.lines[self.cur.l].chars().count());
    }

    /// Selection bounds, start <= end.
    pub fn sel(&self) -> (Pos, Pos) {
        match self.anchor {
            None => (self.cur, self.cur),
            Some(a) if a <= self.cur => (a, self.cur),
            Some(a) => (self.cur, a),
        }
    }

    pub fn selected(&self) -> String {
        let (s, e) = self.sel();
        if s == e {
            return String::new();
        }
        if s.l == e.l {
            return self.lines[s.l].chars().skip(s.c).take(e.c - s.c).collect();
        }
        let mut out: String = self.lines[s.l].chars().skip(s.c).collect();
        for l in s.l + 1..e.l {
            out.push('\n');
            out.push_str(&self.lines[l]);
        }
        out.push('\n');
        out.push_str(&self.lines[e.l].chars().take(e.c).collect::<String>());
        out
    }

    /// Insert text as one undoable edit.
    pub fn edit_insert(&mut self, text: &str, kind: EditKind) {
        // A selection-replacing insert must restore the selection too, so it
        // gets its own undo point rather than merging with a typing run.
        let kind = if self.anchor.is_some() {
            EditKind::Other
        } else {
            kind
        };
        self.mark(kind);
        self.insert(text);
    }

    /// Delete one character backwards as one undoable edit.
    pub fn edit_backspace(&mut self) {
        self.mark(EditKind::Other);
        self.del_left();
    }

    /// Delete one character forwards as one undoable edit.
    pub fn edit_delete(&mut self) {
        self.mark(EditKind::Other);
        self.del_right();
    }

    /// Remove the selection as one undoable edit (no-op when empty).
    pub fn edit_delete_selection(&mut self) {
        let (s, e) = self.sel();
        if s != e {
            self.mark(EditKind::Other);
        }
        self.del_sel();
    }

    fn del_sel(&mut self) {
        let (s, e) = self.sel();
        if s == e {
            return;
        }
        let head: String = self.lines[s.l].chars().take(s.c).collect();
        let tail: String = self.lines[e.l].chars().skip(e.c).collect();
        self.lines[s.l] = head + &tail;
        self.lines.drain(s.l + 1..=e.l);
        self.cur = s;
        self.anchor = None;
        self.dirty = true;
    }

    pub fn insert(&mut self, s: &str) {
        self.del_sel();
        let (mut l, mut c) = (self.cur.l, self.cur.c);
        for ch in s.chars() {
            let mut v: Vec<char> = self.lines[l].chars().collect();
            if ch == '\n' {
                let rest: String = v.split_off(c).into_iter().collect();
                self.lines[l] = v.into_iter().collect();
                self.lines.insert(l + 1, rest);
                l += 1;
                c = 0;
            } else {
                v.insert(c, ch);
                self.lines[l] = v.into_iter().collect();
                c += 1;
            }
        }
        self.cur = Pos { l, c };
        self.dirty = true;
    }

    fn del_left(&mut self) {
        self.del_sel();
        if self.cur.c > 0 {
            let mut v: Vec<char> = self.lines[self.cur.l].chars().collect();
            v.remove(self.cur.c - 1);
            self.lines[self.cur.l] = v.into_iter().collect();
            self.cur.c -= 1;
        } else if self.cur.l > 0 {
            let tail = self.lines.remove(self.cur.l);
            self.cur.l -= 1;
            self.cur.c = self.lines[self.cur.l].chars().count();
            self.lines[self.cur.l].push_str(&tail);
        }
        self.dirty = true;
    }

    fn del_right(&mut self) {
        self.del_sel();
        let n = self.lines[self.cur.l].chars().count();
        if self.cur.c < n {
            let mut v: Vec<char> = self.lines[self.cur.l].chars().collect();
            v.remove(self.cur.c);
            self.lines[self.cur.l] = v.into_iter().collect();
        } else if self.cur.l + 1 < self.lines.len() {
            let next = self.lines.remove(self.cur.l + 1);
            self.lines[self.cur.l].push_str(&next);
        }
        self.dirty = true;
    }

    pub fn move_to(&mut self, p: Pos, shift: bool) {
        self.break_run();
        if shift {
            self.anchor.get_or_insert(self.cur);
        } else {
            self.anchor = None;
        }
        self.cur = p;
        self.clamp();
    }

    /// Scroll horizontally so the caret stays on screen.
    pub fn follow_x(&mut self, caret_x: i32, row: Row) {
        let left = row.x;
        let right = left + row.w;
        if caret_x < left {
            self.hscroll = self.hscroll.saturating_sub((left - caret_x) as u32);
        } else if caret_x > right - 40 {
            self.hscroll += (caret_x - (right - 40)) as u32;
        }
    }

    /// Widest line in pixels, for horizontal scroll bounds.
    pub fn follow(&mut self, rows: usize) {
        if self.cur.l < self.scroll {
            self.scroll = self.cur.l;
        } else if self.cur.l >= self.scroll + rows {
            self.scroll = self.cur.l + 1 - rows;
        }
    }

    pub fn key(&mut self, k: Key, shift: bool, rows: usize) {
        let p = self.cur;
        let len = self.lines[p.l].chars().count();
        match k {
            Key::Left => self.move_to(
                Pos {
                    l: p.l,
                    c: p.c.saturating_sub(1),
                },
                shift,
            ),
            Key::Right => self.move_to(Pos { l: p.l, c: p.c + 1 }, shift),
            Key::Up => self.move_to(
                Pos {
                    l: p.l.saturating_sub(1),
                    c: p.c,
                },
                shift,
            ),
            Key::Down => self.move_to(
                Pos {
                    l: (p.l + 1).min(self.lines.len() - 1),
                    c: p.c,
                },
                shift,
            ),
            Key::Home => self.move_to(Pos { l: p.l, c: 0 }, shift),
            Key::End => self.move_to(Pos { l: p.l, c: len }, shift),
            Key::PageUp => self.move_to(
                Pos {
                    l: p.l.saturating_sub(rows.saturating_sub(1)),
                    c: p.c,
                },
                shift,
            ),
            Key::PageDown => self.move_to(
                Pos {
                    l: (p.l + rows.saturating_sub(1)).min(self.lines.len() - 1),
                    c: p.c,
                },
                shift,
            ),
            Key::Enter => self.edit_insert("\n", EditKind::Other),
            Key::Backspace => self.edit_backspace(),
            Key::Delete => self.edit_delete(),
            Key::Tab => self.edit_insert("  ", EditKind::Other),
            Key::Escape => {
                self.anchor = None;
                self.break_run();
            }
            _ => return,
        }
        self.follow(rows);
    }

    /// Replace the document. Single source of truth for both entry points.
    pub fn load(&mut self, path: &std::path::Path, text: String) {
        let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
        if lines.is_empty() {
            lines.push(String::new());
        }
        let trailing_newline = text.ends_with('\n');
        self.lines = lines;
        self.path = Some(path.to_path_buf());
        self.cur = Pos::default();
        self.anchor = None;
        self.press = None;
        self.dragging = false;
        self.scroll = 0;
        self.hscroll = 0;
        self.dirty = false;
        self.trailing_newline = trailing_newline;
        // History belongs to the document that produced it.
        self.undo.clear();
        self.redo.clear();
        self.break_run();
        self.status = format!("opened {}", path.display());
    }

    pub fn save(&mut self) {
        // Never write outside .md/.markdown: Ctrl+S must not clobber a .txt
        // or a binary just because a document was opened some other way.
        if let Some(p) = self.path.clone() {
            if !crate::window::is_markdown(&p) {
                self.status = format!("refusing to save non-Markdown {}", p.display());
                return;
            }
        }
        match self.path.clone() {
            Some(p) => match std::fs::write(&p, self.text()) {
                Ok(()) => {
                    self.dirty = false;
                    self.status = format!("saved {}", p.display());
                }
                Err(e) => self.status = format!("save failed: {e}"),
            },
            None => self.status = "no file path".into(),
        }
    }
}
