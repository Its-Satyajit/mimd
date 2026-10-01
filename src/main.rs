//! mimd — tiny Markdown viewer/editor.
//!
//! The parser is a pure function over text. The renderer speaks only in
//! typography [`Role`]s and never names a font family; concrete fonts are
//! resolved from the OS by `fonts`.

mod fonts;

use fonts::{Fonts, Role};
use minifb::{Key, KeyRepeat, MouseButton, MouseMode, Window, WindowOptions};
use std::path::PathBuf;

/// Window width. Sized to the reading column plus generous margins rather
/// than to the display: a document reader should not open a 1000px window to
/// show 68 characters of text.
const W: usize = 576;
const H: usize = 660;
/// Page margin. Deliberately generous: the text column is capped at
/// MAX_MEASURE below, and this is the space left over on a wide window.
const MARGIN: u32 = 28;

/// Cap on the reading column, in pixels. Roughly 68 characters at the body
/// size — past ~75 the eye loses the return sweep, which is the whole point
/// of a document reader.
const MAX_MEASURE: i32 = 520;
const BODY_PT: u16 = 15;

// ---------- colours (0RGB) ----------
//
// mimd reads prose, so the ground is paper rather than a code editor's void:
// a cool light grey that stays neutral beside black ink. Exactly one accent
// carries meaning — interactive things — and structure stays recessive.
const BG: u32 = 0xe9eaec; // page
const PANEL: u32 = 0xdcdee2; // status bar, one step down the page
const RULE: u32 = 0xb4b9bf; // the hairline separating bar from page
const FG: u32 = 0x1a1c1e; // body ink
const DIM: u32 = 0x5e6368; // rules, quotes, status text
const ACCENT: u32 = 0x2f5d8a; // links and the caret: the only accent
const HEAD: u32 = 0x000000; // headings carry full ink
const SEL: u32 = 0xc3d2e0; // selection, a wash of the accent
const CURSOR: u32 = 0x2f5d8a; // caret

// ---------- markdown ----------
#[derive(Clone, Copy, PartialEq, Debug)]
enum S {
    Text,
    Bold,
    Italic,
    Code,
    Link,
    /// Both emphases at once; the parser never emits it directly.
    BoldItalic,
}

impl S {
    /// How this span's emphasis composites with an enclosing one.
    ///
    /// Code and link win outright; otherwise the innermost emphasis wins,
    /// and bold+italic is not a distinct style so the pair collapses to bold.
    fn merge(self, outer: S) -> S {
        match (self, outer) {
            (S::Code, _) | (S::Link, _) => self,
            (_, S::Code) | (_, S::Link) => outer,
            (S::Bold, S::Italic) | (S::Italic, S::Bold) => S::BoldItalic,
            (S::BoldItalic, _) | (_, S::BoldItalic) => S::BoldItalic,
            (a, _) => a,
        }
    }

    /// Promote any emphasis to bold.
    ///
    /// A heading line is already distinguished by size and ink; forcing its
    /// inline emphasis to bold keeps `**strong**` inside a heading visible
    /// instead of collapsing back to the heading's own weight.
    fn emphasised(self) -> S {
        match self {
            S::Bold | S::Italic | S::BoldItalic => S::Bold,
            other => other,
        }
    }
}

type Run = Vec<(char, S)>;

/// A semantic block. The renderer decides fonts and sizes; the parser
/// decides only structure. No font knowledge lives here.
#[derive(Debug, PartialEq)]
enum Block {
    Heading(u8, Run),
    Para(Run),
    Bullet(usize, Run),
    Number(usize, Run),
    Quote(Run),
    Rule,
    Code(Run),
}

/// Markdown source -> semantic blocks.
///
/// Deliberately a useful subset, not CommonMark.
fn parse(text: &str) -> Vec<Block> {
    let mut blocks: Vec<Block> = Vec::new();
    let mut para: Run = Vec::new();
    let mut fence: Option<Run> = None;

    let flush = |para: &mut Run, blocks: &mut Vec<Block>| {
        if !para.is_empty() {
            blocks.push(Block::Para(std::mem::take(para)));
        }
    };

    for raw in text.lines() {
        // Fenced code block
        if raw.trim_start().starts_with("```") {
            flush(&mut para, &mut blocks);
            if fence.is_some() {
                blocks.push(Block::Code(fence.take().unwrap()));
            } else {
                fence = Some(Run::new());
            }
            continue;
        }
        if let Some(acc) = fence.as_mut() {
            // Inside a fence nothing is markup.
            if !acc.is_empty() {
                acc.push(('\n', S::Code));
            }
            acc.extend(raw.chars().map(|c| (c, S::Code)));
            continue;
        }

        let indent = raw.len() - raw.trim_start().len();
        let t = raw.trim_start();

        if t.is_empty() {
            flush(&mut para, &mut blocks);
            continue;
        }

        if is_rule(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Rule);
            continue;
        }

        let hashes = t.chars().take_while(|&c| c == '#').count();
        if (1..=6).contains(&hashes) && t[hashes..].starts_with(' ') {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Heading(hashes as u8, inline(t[hashes..].trim(), S::Text)));
            continue;
        }

        // A list item ends any open paragraph; keep nesting via indent.
        if let Some(body) = bullet_body(t) {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Bullet(indent / 2, inline(body, S::Text)));
            continue;
        }
        if let Some((n, body)) = numbered(t) {
            flush(&mut para, &mut blocks);
            let mut run = inline(&n.to_string(), S::Text);
            run.push(('.', S::Text));
            run.push((' ', S::Text));
            run.extend(inline(body, S::Text));
            blocks.push(Block::Number(indent / 2, run));
            continue;
        }

        if let Some(body) = t.strip_prefix('>') {
            flush(&mut para, &mut blocks);
            blocks.push(Block::Quote(inline(body.trim(), S::Text)));
            continue;
        }

        // Lazy continuation of the current paragraph.
        let mut run = inline(t, S::Text);
        if !para.is_empty() {
            para.push((' ', S::Text));
            para.append(&mut run);
        } else {
            para = run;
        }
    }
    if let Some(acc) = fence {
        // Unterminated fence: still show what we have.
        blocks.push(Block::Code(acc));
    }
    flush(&mut para, &mut blocks);
    blocks
}

fn is_rule(s: &str) -> bool {
    let t: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    t.len() >= 3
        && (t.chars().all(|c| c == '-')
            || t.chars().all(|c| c == '*')
            || t.chars().all(|c| c == '_'))
}

fn bullet_body(s: &str) -> Option<&str> {
    for m in ["- ", "* ", "+ "] {
        if let Some(b) = s.strip_prefix(m) {
            return Some(b);
        }
    }
    None
}

/// "1. text" -> (1, "text")
fn numbered(t: &str) -> Option<(u32, &str)> {
    let n = t.chars().take_while(|c| c.is_ascii_digit()).count();
    if n == 0 {
        return None;
    }
    Some((t.get(..n)?.parse().ok()?, t.get(n..)?.strip_prefix(". ")?))
}

/// Inline scan: **bold**, *italic*, `code`, [link](url), backslash escapes.
fn inline(text: &str, base: S) -> Run {
    let c: Vec<char> = text.chars().collect();
    let mut out = Run::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '\\' && i + 1 < c.len() {
            out.push((c[i + 1], base));
            i += 2;
            continue;
        }
        if c[i] == '`' {
            if let Some(e) = (i + 1..c.len()).find(|&j| c[j] == '`') {
                out.extend(c[i + 1..e].iter().map(|&ch| (ch, S::Code)));
                i = e + 1;
                continue;
            }
        }
        if c[i] == '*' || c[i] == '_' {
            let dbl = i + 1 < c.len() && c[i + 1] == c[i];
            let open = if dbl { 2 } else { 1 };
            let style = if dbl { S::Bold } else { S::Italic };
            let from = i + open;
            if let Some(e) = find_close(&c, from, c[i], dbl) {
                if e > from {
                    let inner: String = c[from..e].iter().collect();
                    for (ch, st) in inline(&inner, style) {
                        out.push((ch, st.merge(base)));
                    }
                    i = e + open;
                    continue;
                }
            }
        }
        if c[i] == '[' {
            if let Some(cb) = (i + 1..c.len()).find(|&j| c[j] == ']') {
                if c.get(cb + 1) == Some(&'(') {
                    if let Some(p) = (cb + 2..c.len()).find(|&j| c[j] == ')') {
                        out.extend(c[i + 1..cb].iter().map(|&ch| (ch, S::Link)));
                        i = p + 1;
                        continue;
                    }
                }
            }
        }
        out.push((c[i], base));
        i += 1;
    }
    out
}

fn find_close(c: &[char], from: usize, target: char, pair: bool) -> Option<usize> {
    (from..c.len()).find(|&i| c[i] == target && (!pair || c.get(i + 1) == Some(&target)))
}

// ---------- state ----------
#[derive(Clone, Copy, PartialEq, Debug)]
enum Mode {
    View,
    Edit,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Debug)]
struct Pos {
    l: usize,
    c: usize,
}

struct App {
    lines: Vec<String>,
    cur: Pos,
    anchor: Option<Pos>,
    mode: Mode,
    scroll: usize,
    /// Horizontal scroll in pixels (edit mode only).
    hscroll: u32,
    /// True while a left-button drag is selecting.
    dragging: bool,
    /// Path being typed at the open prompt; `Some` means the prompt is up.
    prompt: Option<String>,
    /// Where the current mouse press began, before any drag.
    press: Option<Pos>,
    /// Edit history. `None` records a fresh undo point.
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    /// Tag identifying the edit that produced the current state, so a run of
    /// similar keystrokes collapses into one undo step.
    last_edit: Option<EditKind>,
    /// Whether the document ended with a newline. Text files conventionally
    /// do, and dropping it would make open-then-save rewrite every file.
    trailing_newline: bool,
    dirty: bool,
    path: Option<PathBuf>,
    status: String,
    clip: String,
    quit: bool,
}

/// A restorable document state.
#[derive(Clone, PartialEq, Debug)]
struct Snapshot {
    lines: Vec<String>,
    cur: Pos,
    path: Option<PathBuf>,
}

/// What kind of edit just happened, for coalescing.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum EditKind {
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
    fn mark(&mut self, kind: EditKind) {
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
    fn break_run(&mut self) {
        self.last_edit = None;
    }

    fn undo(&mut self) -> bool {
        let Some(snap) = self.undo.pop() else { return false };
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

    fn redo(&mut self) -> bool {
        let Some(snap) = self.redo.pop() else { return false };
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

    fn open(text: String, path: Option<PathBuf>, mode: Mode) -> Self {
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

    fn text(&self) -> String {
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
    fn sel(&self) -> (Pos, Pos) {
        match self.anchor {
            None => (self.cur, self.cur),
            Some(a) if a <= self.cur => (a, self.cur),
            Some(a) => (self.cur, a),
        }
    }

    fn selected(&self) -> String {
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
    fn edit_insert(&mut self, text: &str, kind: EditKind) {
        // A selection-replacing insert must restore the selection too, so it
        // gets its own undo point rather than merging with a typing run.
        let kind = if self.anchor.is_some() { EditKind::Other } else { kind };
        self.mark(kind);
        self.insert(text);
    }

    /// Delete one character backwards as one undoable edit.
    fn edit_backspace(&mut self) {
        self.mark(EditKind::Other);
        self.del_left();
    }

    /// Delete one character forwards as one undoable edit.
    fn edit_delete(&mut self) {
        self.mark(EditKind::Other);
        self.del_right();
    }

    /// Remove the selection as one undoable edit (no-op when empty).
    fn edit_delete_selection(&mut self) {
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

    fn insert(&mut self, s: &str) {
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

    fn move_to(&mut self, p: Pos, shift: bool) {
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
    fn follow_x(&mut self, caret_x: i32, width: i32) {
        let left = MARGIN as i32;
        let right = left + width;
        if caret_x < left {
            self.hscroll = self.hscroll.saturating_sub((left - caret_x) as u32);
        } else if caret_x > right - 40 {
            self.hscroll += (caret_x - (right - 40)) as u32;
        }
    }

    /// Widest line in pixels, for horizontal scroll bounds.
    fn widest(&self, c: &mut Canvas) -> i32 {
        self.lines
            .iter()
            .map(|l| c.measure(l, Role::Body))
            .max()
            .unwrap_or(0)
    }

    fn follow(&mut self, rows: usize) {
        if self.cur.l < self.scroll {
            self.scroll = self.cur.l;
        } else if self.cur.l >= self.scroll + rows {
            self.scroll = self.cur.l + 1 - rows;
        }
    }

    fn key(&mut self, k: Key, shift: bool, rows: usize) {
        let p = self.cur;
        let len = self.lines[p.l].chars().count();
        match k {
            Key::Left => self.move_to(Pos { l: p.l, c: p.c.saturating_sub(1) }, shift),
            Key::Right => self.move_to(Pos { l: p.l, c: p.c + 1 }, shift),
            Key::Up => self.move_to(Pos { l: p.l.saturating_sub(1), c: p.c }, shift),
            Key::Down => self.move_to(Pos { l: (p.l + 1).min(self.lines.len() - 1), c: p.c }, shift),
            Key::Home => self.move_to(Pos { l: p.l, c: 0 }, shift),
            Key::End => self.move_to(Pos { l: p.l, c: len }, shift),
            Key::PageUp => self.move_to(Pos { l: p.l.saturating_sub(rows.saturating_sub(1)), c: p.c }, shift),
            Key::PageDown => {
                self.move_to(Pos { l: (p.l + rows.saturating_sub(1)).min(self.lines.len() - 1), c: p.c }, shift)
            }
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
    fn load(&mut self, path: &std::path::Path, text: String) {
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

    fn save(&mut self) {
        // Never write outside .md/.markdown: Ctrl+S must not clobber a .txt
        // or a binary just because a document was opened some other way.
        if let Some(p) = self.path.clone() {
            if !is_markdown(&p) {
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

// ---------- render ----------

/// A canvas of 0RGB pixels plus the font set it draws with.
struct Canvas<'a> {
    buf: &'a mut [u32],
    w: u32,
    h: u32,
    fonts: &'a mut Fonts,
}


/// The mimd mark, rasterized for the window manager.
///
/// The artwork is vector (mimd-logo-monochrome.svg) but a window icon must be
/// pixels, and an SVG rasterizer would cost more than the whole rest of the
/// binary. The mark is simple enough to draw directly.
///
/// It is emitted as a single opaque silhouette with the interior marks knocked
/// out, which is what "monochrome" means: the knockouts show whatever is
/// behind the icon, so it reads on any desktop background. That is why the
/// buffer carries only alpha.
fn logo_icon(size: u32) -> Vec<u64> {
    // Everything below is in the SVG's 256-unit design space.
    const DOC: [(f32, f32); 5] = [(68., 40.), (152., 40.), (188., 76.), (188., 216.), (68., 216.)];
    const FOLD: [(f32, f32, f32, f32); 2] = [(152., 40., 152., 76.), (152., 76., 188., 76.)];
    const BULLET: (f32, f32, f32, f32) = (88., 104., 106., 122.);
    const BAR: (f32, f32, f32, f32) = (114., 104., 186., 122.);
    const STEMS: [(f32, f32, f32, f32); 3] = [(88., 150., 88., 184.), (124., 150., 124., 184.), (160., 150., 160., 184.)];
    const HEADS: [(f32, f32, f32, f32); 4] = [(88., 150., 106., 168.), (106., 168., 124., 150.), (124., 150., 142., 168.), (142., 168., 160., 150.)];
    const HAIRLINE: f32 = 10.0; // the SVG's stroke-width
    const RADIUS: f32 = 3.0; // the SVG's rect rx

    let unit = size as f32 / 256.0;
    let mut out = Vec::with_capacity((size * size) as usize);

    // Supersample, or the hairline strokes and the 18-unit bars turn to mush
    // at 32px, which is the size most window managers actually use.
    const SS: usize = 3;
    for py in 0..size {
        for px in 0..size {
            let mut hits = 0usize;
            for sy in 0..SS {
                for sx in 0..SS {
                    let ux = (px as f32 + (sx as f32 + 0.5) / SS as f32) / unit;
                    let uy = (py as f32 + (sy as f32 + 0.5) / SS as f32) / unit;
                    let ink = point_in_poly(ux, uy, &DOC)
                        && !near_any_seg(ux, uy, &FOLD, HAIRLINE / 2.0)
                        && !in_rounded_rect(ux, uy, BULLET, RADIUS)
                        && !in_rounded_rect(ux, uy, BAR, RADIUS)
                        && !near_any_seg(ux, uy, &STEMS, HAIRLINE / 2.0)
                        && !near_any_seg(ux, uy, &HEADS, HAIRLINE / 2.0);
                    hits += ink as usize;
                }
            }
            let alpha = (hits as f32 * 255.0 / (SS * SS) as f32).round() as u32;
            // X11 _NET_WM_ICON wants premultiplied ARGB; alpha-only is premultiplied.
            out.push(((alpha << 24) | (alpha << 16) | (alpha << 8) | alpha) as u64);
        }
    }
    out
}

/// The mark at the sizes a window manager is likely to ask for, concatenated
/// into the one buffer `_NET_WM_ICON` wants (a flat run of ARGB pixels).
#[cfg(target_os = "linux")]
fn icon_from_logo() -> Result<minifb::Icon, ()> {
    let mut buf = Vec::new();
    for size in [32u32, 64] {
        buf.extend_from_slice(&logo_icon(size));
    }
    minifb::Icon::try_from(buf.as_slice()).map_err(|_| ())
}

/// Distance from a point to a segment, for stroking without a rasterizer.
fn seg_dist(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 0.0 { 0.0 } else { (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0) };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
}

fn near_any_seg(px: f32, py: f32, segs: &[(f32, f32, f32, f32)], half: f32) -> bool {
    segs.iter().any(|&(ax, ay, bx, by)| seg_dist(px, py, (ax, ay), (bx, by)) <= half)
}

fn point_in_poly(px: f32, py: f32, poly: &[(f32, f32)]) -> bool {
    let mut inside = false;
    for i in 0..poly.len() {
        let (ax, ay) = poly[i];
        let (bx, by) = poly[(i + 1) % poly.len()];
        if (ay > py) != (by > py) && px < ax + (py - ay) * (bx - ax) / (by - ay) {
            inside = !inside;
        }
    }
    inside
}

/// A rounded rect as a distance test, matching the SVG's rx on both axes.
fn in_rounded_rect(px: f32, py: f32, (x0, y0, x1, y1): (f32, f32, f32, f32), r: f32) -> bool {
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let dx = (px - cx).abs() - (x1 - x0) / 2.0 + r;
    let dy = (py - cy).abs() - (y1 - y0) / 2.0 + r;
    (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() <= r
}

impl Canvas<'_> {
    fn body_size(&self) -> u16 {
        self.fonts.body.size
    }
}

impl<'a> Canvas<'a> {
    fn px(&mut self, x: i32, y: i32, c: u32) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            self.buf[y as usize * self.w as usize + x as usize] = c;
        }
    }

    fn fill(&mut self, x: i32, y: i32, w: u32, h: u32, c: u32) {
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                self.px(x + dx, y + dy, c);
            }
        }
    }

    fn clear(&mut self, c: u32) {
        for p in self.buf.iter_mut() {
            *p = c;
        }
    }

    /// Blend one coverage sample.
    fn blend(&mut self, x: i32, y: i32, color: u32, coverage: u8) {
        if coverage == 0 {
            return;
        }
        if x < 0 || y < 0 || x as u32 >= self.w || y as u32 >= self.h {
            return;
        }
        let idx = y as usize * self.w as usize + x as usize;
        if coverage == 255 {
            self.buf[idx] = color;
            return;
        }
        let a = coverage as u32;
        let dst = self.buf[idx];
        let r = (color & 0xff).saturating_mul(a) + (dst & 0xff) * (255 - a);
        let g = ((color >> 8) & 0xff).saturating_mul(a) + ((dst >> 8) & 0xff) * (255 - a);
        let b = ((color >> 16) & 0xff).saturating_mul(a) + ((dst >> 16) & 0xff) * (255 - a);
        self.buf[idx] = (r / 255) | ((g / 255) << 8) | ((b / 255) << 16);
    }

    /// Draw a string, returning its total advance.
    fn draw_str(&mut self, x: i32, baseline: i32, s: &str, role: Role, color: u32) -> i32 {
        let size = self.body_size();
        self.draw_str_sized(x, baseline, s, role, color, size)
    }

    /// Draw `s` at an explicit point size.
    ///
    /// Every glyph is placed at the running pen position; blending at the
    /// starting `x` instead stacks every letter on the same spot.
    fn draw_str_sized(
        &mut self,
        x: i32,
        baseline: i32,
        s: &str,
        role: Role,
        color: u32,
        size: u16,
    ) -> i32 {
        let mut pen = x;
        for ch in s.chars() {
            let g = self.fonts.glyph_at(role, size, ch);
            for (row, line) in g.mask.chunks(g.width.max(1) as usize).enumerate() {
                for (col, &cov) in line.iter().enumerate() {
                    self.blend(
                        pen + g.bearing_x + col as i32,
                        baseline - g.bearing_y + row as i32,
                        color,
                        cov,
                    );
                }
            }
            pen += g.advance_x;
        }
        pen - x
    }

    /// Draw a styled run at an explicit point size, flushing on style change.
    fn draw_run_sized(
        &mut self,
        x: i32,
        baseline: i32,
        cells: &[(char, S)],
        fg: u32,
        size: u16,
    ) -> i32 {
        let mut pen = x;
        let mut seg = String::new();
        let mut style = S::Text;
        for &(ch, st) in cells {
            if st != style {
                if !seg.is_empty() {
                    pen += self.draw_str_sized(
                        pen,
                        baseline,
                        &seg,
                        role_for(style),
                        color_for(style, fg),
                        size,
                    );
                    seg.clear();
                }
                style = st;
            }
            seg.push(ch);
        }
        if !seg.is_empty() {
            pen += self.draw_str_sized(
                pen,
                baseline,
                &seg,
                role_for(style),
                color_for(style, fg),
                size,
            );
        }
        pen - x
    }

    /// Width of `s` at an explicit point size.
    fn measure_at(&mut self, s: &str, role: Role, size: u16) -> i32 {
        self.fonts.measure_at(role, size, s)
    }

    fn line_height_for(&mut self, size: u16) -> u32 {
        self.fonts.line_height_at(Role::Body, size)
    }

    fn ascent_for(&mut self, size: u16) -> i32 {
        self.fonts.ascent_at(Role::Body, size)
    }

    fn measure(&mut self, s: &str, role: Role) -> i32 {
        self.fonts.for_role(role).measure(s)
    }

    fn line_height(&self, role: Role) -> u32 {
        self.fonts.line_height_of(role)
    }

    fn ascent(&self, role: Role) -> i32 {
        self.fonts.ascent_of(role)
    }
}

/// Colour for a span. Heading colour is applied per line by the renderer,
/// since heading-ness is a block property, not a span property.
fn color_for(style: S, base: u32) -> u32 {
    match style {
        S::Link => ACCENT,
        _ => base,
    }
}

/// Typography role for a span.
///
/// Emphasis is a weight axis, not a colour one: `**bold**` and `*italic*`
/// each resolve to a real face, so they are distinguishable at a glance and
/// in monochrome. Only a link earns the accent.
fn role_for(style: S) -> Role {
    match style {
        S::Code => Role::Code,
        S::Bold => Role::BodyBold,
        S::Italic => Role::BodyItalic,
        S::BoldItalic => Role::BodyBoldItalic,
        S::Link | S::Text => Role::Body,
    }
}

/// Point size for a heading level.
///
/// A restrained ramp: H1 and H2 clearly larger, H3 barely, and H4-H6 the
/// same size as body but bold. Hierarchy comes mostly from weight, which is
/// what a real typeface is for.
fn heading_size(level: u8, base: u16) -> u16 {
    match level {
        1 => base + 8,
        2 => base + 4,
        3 => base + 1,
        _ => base,
    }
}

/// One laid-out display line ready to draw.
struct VisualLine {
    cells: Run,
    /// Left indent in pixels.
    indent: i32,
    /// Point size to render at.
    size: u16,
    /// Base text colour for this line (headings use a brighter one).
    color: u32,
    /// Blank lines to leave after this one.
    gap: u32,
    /// Draw a hairline of this width instead of glyphs (horizontal rule).
    rule: Option<i32>,
    /// Draw a vertical rail in the left margin (blockquote).
    quote: bool,
}

impl Default for VisualLine {
    fn default() -> Self {
        VisualLine {
            cells: Vec::new(),
            indent: 0,
            size: 0,
            color: FG,
            gap: 0,
            rule: None,
            quote: false,
        }
    }
}

/// Wrap `cells` to `width` pixels, breaking on spaces where possible and
/// mid-word only when a single word is wider than the line.
#[allow(clippy::too_many_arguments)]
fn wrap_cells(
    c: &mut Canvas,
    cells: Run,
    indent: i32,
    size: u16,
    width: i32,
    first_gap: u32,
    gap: u32,
) -> Vec<VisualLine> {
    let color = FG;
    if cells.is_empty() {
        return vec![VisualLine { cells, indent, size, color, gap: first_gap, ..Default::default() }];
    }
    // Split into words on spaces, keeping the spaces attached.
    let mut words: Vec<Vec<(char, S)>> = Vec::new();
    let mut cur: Vec<(char, S)> = Vec::new();
    for &cell in &cells {
        if cell.0 == ' ' {
            cur.push(cell);
            words.push(std::mem::take(&mut cur));
        } else {
            cur.push(cell);
        }
    }
    if !cur.is_empty() {
        words.push(cur);
    }

    let mut out: Vec<VisualLine> = Vec::new();
    let mut line: Run = Vec::new();
    let mut line_w = 0i32;
    for word in words {
        let text: String = word.iter().map(|x| x.0).collect();
        let w = c.measure_at(&text, role_for(word[0].1), size);
        let lead = if line.is_empty() { indent } else { 0 };
        if !line.is_empty() && line_w + w > width - lead {
            out.push(VisualLine {
                cells: std::mem::take(&mut line),
                indent,
                size,
                color,
                gap: if out.is_empty() { first_gap } else { 0 },
                ..Default::default()
            });
            line_w = 0;
        }
        // Hard-break a single word that cannot fit on any line.
        if w > width - indent && line.is_empty() {
            let mut chunk = String::new();
            let mut chunk_w = 0i32;
            for (ch, st) in &word {
                let cw = c.measure_at(&ch.to_string(), role_for(*st), size);
                if chunk_w + cw > width - indent && !chunk.is_empty() {
                    out.push(VisualLine {
                        cells: chunk.chars().map(|c| (c, *st)).collect(),
                        indent,
                        size,
                        color,
                        gap: 0,
                        ..Default::default()
                    });
                    chunk.clear();
                    chunk_w = 0;
                }
                chunk.push(*ch);
                chunk_w += cw;
            }
            line = chunk.chars().map(|c| (c, word[0].1)).collect();
            line_w = chunk_w;
            continue;
        }
        line.extend_from_slice(&word);
        line_w += w;
    }
    if !line.is_empty() {
        let g = if out.is_empty() { first_gap } else { 0 };
        out.push(VisualLine { cells: line, indent, size, color, gap: g, ..Default::default() });
    }
    if out.is_empty() {
        out.push(VisualLine { cells, indent, size, color, gap: first_gap, ..Default::default() });
    }
    // The wrap above attaches gap to the wrong line; normalise: gap goes last.
    let last = out.len() - 1;
    let g = gap;
    out[last].gap = g;
    if let Some(f) = out.get_mut(0) {
        if f.gap != g && last != 0 {
            f.gap = 0;
        }
    }
    out
}

/// Lay out every block into visual lines for a viewport `width` pixels wide.
fn layout_view(c: &mut Canvas, text: &str, width: i32) -> Vec<VisualLine> {
    let base = c.body_size();
    let mut out: Vec<VisualLine> = Vec::new();
    for block in parse(text) {
        let lines = match block {
            Block::Heading(level, run) => {
                let size = heading_size(level, base);
                // Headings are bold: hierarchy is weight first, size second.
                let run: Run = run.into_iter().map(|(ch, st)| (ch, st.emphasised())).collect();
                // A heading needs air above it proportional to its size, and a
                // quieter rule below H1/H2 so sections separate without a box.
                let above = match level {
                    1 => 2,
                    2 => 2,
                    3 => 1,
                    _ => 1,
                };
                let below = match level {
                    1 => 1,
                    2 => 1,
                    _ => 0,
                };
                let mut lines = wrap_cells(c, run, 0, size, width, above, below);
                for l in &mut lines {
                    l.color = HEAD;
                }
                lines
            }
            Block::Para(run) => wrap_cells(c, run, 0, base, width, 0, 1),
            Block::Bullet(depth, run) => {
                let indent = depth as i32 * 20;
                let mut cells: Run = vec![('\u{2022}', S::Link), (' ', S::Text)];
                cells.extend(run);
                wrap_cells(c, cells, indent + 16, base, width, 0, 0)
            }
            Block::Number(depth, run) => {
                let indent = depth as i32 * 18;
                wrap_cells(c, run, indent + 14, base, width, 0, 0)
            }
            Block::Quote(run) => {
                // The rail is drawn as a filled bar, not a glyph, so it stays
                // one crisp pixel wide whatever the font does with U+2502.
                let run: Run = run.into_iter().map(|(ch, _)| (ch, S::Text)).collect();
                let mut lines = wrap_cells(c, run, 20, base, width - 20, 0, 0);
                for l in &mut lines {
                    l.color = DIM;
                    l.quote = true;
                }
                lines
            }
            Block::Rule => vec![VisualLine {
                size: base,
                color: DIM,
                gap: 1,
                rule: Some(width.min(MAX_MEASURE)),
                ..Default::default()
            }],
            Block::Code(run) => {
                let mut cells: Run = Vec::new();
                for (ch, _) in &run {
                    cells.push((*ch, S::Code));
                }
                wrap_cells(c, cells, 12, base, width - 12, 0, 1)
            }
        };
        out.extend(lines);
    }
    out
}

/// Where the view-mode layout put things, so mouse hit-testing and drawing
/// agree by construction. Built during layout, consumed by input and render.
struct ViewLayout {
    lines: Vec<VisualLine>,
    /// Pixel y of each visual line.
    #[allow(dead_code)]
    ys: Vec<i32>,
}

/// Build the view layout plus its pixel positions.
fn build_view_layout(c: &mut Canvas, text: &str, width: i32) -> ViewLayout {
    let lines = layout_view(c, text, width);
    let mut ys = Vec::with_capacity(lines.len());
    let mut y = MARGIN as i32;
    for vl in &lines {
        ys.push(y);
        y += c.line_height_for(vl.size) as i32;
        for _ in 0..vl.gap {
            y += c.line_height_for(vl.size) as i32 / 2;
        }
    }
    ViewLayout { lines, ys }
}

/// Character index in `text` nearest to pixel `x`, using real font advances.
fn char_index_at_x(c: &mut Canvas, text: &str, x: i32, role: Role, size: u16) -> usize {
    let mut pen = 0i32;
    for (i, ch) in text.chars().enumerate() {
        let w = c.measure_at(&ch.to_string(), role, size);
        if x < pen + w / 2 {
            return i;
        }
        pen += w;
    }
    text.chars().count()
}

/// `origin_x` is the left edge of the text column, shared by layout, drawing
/// and hit-testing so a click always lands where the glyph is.
fn render(app: &App, c: &mut Canvas, view: &ViewLayout) {
    c.clear(BG);
    let lh = c.line_height(Role::Body);
    let status_h = lh + 6;
    let rows_fit = (c.h.saturating_sub(status_h + MARGIN * 2) / lh).max(1) as usize;
    let top = MARGIN as i32;

    if app.mode == Mode::Edit {
        let lh_i = lh as i32;
        let hscroll = app.hscroll as i32;
        let (s, e) = app.sel();
        if s != e {
            for l in s.l..=e.l {
                let c0 = if l == s.l { s.c } else { 0 };
                let c1 = if l == e.l { e.c } else { usize::MAX };
                let chars: Vec<char> = app.lines[l].chars().collect();
                let n = chars.len();
                let prefix: String = chars[..c0.min(n)].iter().collect();
                let upto: String = chars[..c1.min(n)].iter().collect();
                let x0 = MARGIN as i32 - hscroll + c.measure(&prefix, Role::Body);
                let x1 = MARGIN as i32 - hscroll + c.measure(&upto, Role::Body);
                let y = top + (l as i32 - app.scroll as i32) * lh_i;
                c.fill(x0, y, (x1 - x0) as u32, lh, SEL);
            }
        }
        for l in app.scroll..app.lines.len().min(app.scroll + rows_fit) {
            let y = top + (l as i32 - app.scroll as i32) * lh_i;
            c.draw_str(
                MARGIN as i32 - hscroll,
                y + c.ascent(Role::Body),
                &app.lines[l],
                Role::Body,
                FG,
            );
        }
        let (cl, cc) = if s == e { (app.cur.l, app.cur.c) } else { (s.l, s.c) };
        if cl >= app.scroll && cl - app.scroll < rows_fit {
            let chars: Vec<char> = app.lines[cl].chars().collect();
            let prefix: String = chars[..cc.min(chars.len())].iter().collect();
            let x = MARGIN as i32 - hscroll + c.measure(&prefix, Role::Body);
            let y = top + (cl as i32 - app.scroll as i32) * lh_i;
            c.fill(x, y, 2, lh, CURSOR);
        }
    } else {
        let limit = c.h as i32 - status_h as i32;
        // Skip lines above the scroll offset, then draw the rest shifted up by
        // it. Without this the layout is drawn from the top and scrolling does
        // nothing at all.
        // `scroll` counts lines but `ys` holds pixels, so the pixel shift is
        // ys[scroll] - MARGIN: the distance from the first line's origin to the
        // line we are scrolling to. Subtracting the line index itself would
        // scroll by a few pixels rather than by pages.
        let scroll_px = view.ys.get(app.scroll).map_or(0, |&y| y - view.ys.first().copied().unwrap_or(y));
        for (i, vl) in view.lines.iter().enumerate().skip(app.scroll) {
            let y = view.ys[i] - scroll_px;
            if y + c.line_height_for(vl.size) as i32 <= top {
                continue;
            }
            if y > limit {
                break;
            }
            let baseline = y + c.ascent_for(vl.size);
            let lh = c.line_height_for(vl.size) as i32;
            // A horizontal rule is a filled hairline, not a row of glyphs: it
            // stays exactly one pixel tall whatever the typeface does.
            if let Some(w) = vl.rule {
                let ry = y + lh / 2;
                c.fill(MARGIN as i32, ry, w.max(8) as u32, 1, DIM);
                continue;
            }
            // A blockquote's rail sits in the margin, aligned to the text.
            if vl.quote {
                c.fill(MARGIN as i32 + vl.indent - 12, y + 2, 2, (lh - 4).max(2) as u32, DIM);
            }
            let mut x = MARGIN as i32 + vl.indent;
            let mut seg: Vec<(char, S)> = Vec::new();
            let mut style = S::Text;
            for &(ch, st) in &vl.cells {
                if st != style {
                    x += c.draw_run_sized(x, baseline, &seg, vl.color, vl.size);
                    seg.clear();
                    style = st;
                }
                seg.push((ch, st));
            }
            x += c.draw_run_sized(x, baseline, &seg, vl.color, vl.size);
            let _ = x;
        }
    }

    // Status bar: a pager's footer. Left is where you are, right is the one
    // key that matters right now. A permanent list of every shortcut is noise
    // — it is in the README, and it crowds out the state that is actually
    // telling you something.
    let y = c.h as i32 - status_h as i32;
    c.fill(0, y, c.w, status_h, PANEL);
    c.fill(0, y, c.w, 1, RULE);
    let base_y = y + c.ascent(Role::Body) + 1;
    let x0 = MARGIN as i32;

    let name = app.path.as_ref().map(|p| p.display().to_string());
    match app.prompt.as_ref() {
        // The prompt replaces the bar: it is the only thing actionable.
        Some(p) => {
            let line = format!("Open: {p}\u{2588}");
            let caret_x = c.measure(&format!("Open: {p}"), Role::Body);
            c.draw_str(x0, base_y, &line, Role::Body, FG);
            c.fill(x0 + caret_x, y + 4, 2, status_h - 8, ACCENT);
        }
        None => {
            // Left: mode and filename. Modified is a fact worth surfacing.
            let editing = app.mode == Mode::Edit;
            let mode = if editing { "EDIT" } else { "READ" };
            let mw = c.draw_str(x0, base_y, mode, Role::BodyBold, if editing { ACCENT } else { DIM });
            let mut lx = x0 + mw + 14;
            if let Some(n) = &name {
                lx += c.draw_str(lx, base_y, n, Role::Body, FG) + 12;
            }
            if app.dirty {
                c.draw_str(lx, base_y, "unsaved", Role::BodyItalic, DIM);
            }
            // Transient status, centred, wins over the hint while it lasts.
            let (right_text, right_w) = if app.status.is_empty() {
                (if editing { "Ctrl+S to save" } else { "Ctrl+E to edit" }.to_string(), 0)
            } else {
                let t = app.status.clone();
                let w = c.measure(&t, Role::Body);
                (t, w)
            };
            let rw = if app.status.is_empty() { c.measure(&right_text, Role::Body) } else { right_w };
            let rx = c.w as i32 - MARGIN as i32 - rw;
            c.draw_str(rx, base_y, &right_text, Role::Body, DIM);
        }
    }
}

/// Largest valid scroll offset for `total` rows in a window showing `rows`.
fn max_scroll(total: usize, rows: usize) -> usize {
    total.saturating_sub(rows.max(1))
}

/// Result of a mouse step, kept separate from windowing so it can be tested
/// without a display.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MouseAction {
    None,
    /// Place the caret, extending the selection if a drag is in progress.
    Caret(Pos),
}

/// Apply one mouse sample to the caret state.
///
/// `view` maps pixel rows to document lines. In edit mode a press starts a
/// selection anchor and motion extends it; release ends the drag. In view
/// mode the mouse deliberately does nothing — there is no caret to place.
fn mouse_step(
    app: &mut App,
    x: i32,
    y: i32,
    down: bool,
    hscroll: u32,
    col_of: &dyn Fn(&mut Canvas, &str, i32, u32) -> usize,
    canvas: &mut Canvas,
    view_rows: &[i32],
) -> MouseAction {
    if app.mode != Mode::Edit {
        app.dragging = false;
        app.press = None;
        app.anchor = None;
        let _ = view_rows;
        return MouseAction::None;
    }
    let inside = x >= 0 && y >= MARGIN as i32;
    if down && inside {
        let lh = canvas.line_height(Role::Body) as i32;
        let row = ((y - MARGIN as i32) / lh.max(1)).max(0) as usize;
        let l = (app.scroll + row).min(app.lines.len().saturating_sub(1));
        let c = col_of(canvas, &app.lines[l], x, hscroll);
        let target = Pos { l, c };
        if !app.dragging {
            // Press: the anchor is where the mouse went down, not where the
            // caret happened to be. Otherwise any click that moves the caret
            // would look like a drag.
            app.press = Some(target);
            app.dragging = true;
        }
        app.cur = target;
        MouseAction::Caret(app.cur)
    } else {
        if app.dragging && !down {
            app.dragging = false;
            let press = app.press.take().unwrap_or(app.cur);
            // A press and release in the same place is a plain click: it moves
            // the caret but must not leave a selection behind. Otherwise the
            // press point becomes the selection anchor.
            app.anchor = if app.cur == press { None } else { Some(press) };
        }
        MouseAction::None
    }
}

/// Pixel x of the caret at `col` in `line`, honouring horizontal scroll.
fn caret_x(c: &mut Canvas, line: &str, col: usize, hscroll: u32) -> i32 {
    let chars: Vec<char> = line.chars().collect();
    let prefix: String = chars[..col.min(chars.len())].iter().collect();
    MARGIN as i32 - hscroll as i32 + c.measure(&prefix, Role::Body)
}

/// Character index nearest to a click at pixel `x`.
fn col_at_x(c: &mut Canvas, line: &str, x: i32, hscroll: u32) -> usize {
    char_index_at_x(c, line, x + hscroll as i32 - MARGIN as i32, Role::Body, c.body_size())
}

// ---------- clipboard ----------
/// System clipboard access.
///
/// Tries the platform tool that exists (`wl-copy`/`wl-paste` on Wayland,
/// `xclip` on X11) and falls back to mimd's own buffer when neither is
/// installed, so copy/paste always does *something*.
mod clip {
    use std::io::Write;
    use std::process::{Command, Stdio};

    fn tool_exists(bin: &str) -> bool {
        std::path::Path::new(bin).exists()
            || which(bin)
    }

    fn which(bin: &str) -> bool {
        if let Ok(path) = std::env::var("PATH") {
            for dir in path.split(':') {
                if std::path::Path::new(dir).join(bin).is_file() {
                    return true;
                }
            }
        }
        false
    }

    fn wayland() -> bool {
        std::env::var("WAYLAND_DISPLAY").is_ok()
    }

    /// Put `text` on the system clipboard. Returns false if unavailable.
    pub fn copy(text: &str) -> bool {
        if text.is_empty() {
            return false;
        }
        let (bin, args): (&str, Vec<&str>) = if wayland() && tool_exists("wl-copy") {
            ("wl-copy", vec!["--type", "text/plain;charset=utf-8", "--"])
        } else if tool_exists("xclip") {
            ("xclip", vec!["-selection", "clipboard", "-in"])
        } else {
            return false;
        };
        let mut child = match Command::new(bin).args(&args).stdin(Stdio::piped()).spawn() {
            Ok(c) => c,
            Err(_) => return false,
        };
        if let Some(stdin) = child.stdin.as_mut() {
            if stdin.write_all(text.as_bytes()).is_err() {
                return false;
            }
        }
        // wl-copy/xclip daemonise to hold the selection; do not wait on them.
        drop(child.stdin.take());
        let _ = child.wait();
        true
    }

    /// Read the system clipboard. Returns None if unavailable.
    pub fn paste() -> Option<String> {
        let out = if wayland() && tool_exists("wl-paste") {
            Command::new("wl-paste").args(["--no-newline"]).output().ok()?
        } else if tool_exists("xclip") {
            Command::new("xclip").args(["-selection", "clipboard", "-out"]).output().ok()?
        } else {
            return None;
        };
        if !out.status.success() {
            return None;
        }
        String::from_utf8(out.stdout).ok()
    }
}

/// True for paths mimd is willing to read and write.
fn is_markdown(p: &std::path::Path) -> bool {
    matches!(
        p.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref(),
        Some("md") | Some("markdown")
    )
}

/// Resolve a typed path to a readable Markdown file.
fn open_path(typed: &str) -> Result<(PathBuf, String), String> {
    let t = typed.trim();
    if t.is_empty() {
        return Err("no path given".into());
    }
    let mut p = PathBuf::from(t);
    if t == "~" || t.starts_with("~/") {
        if let Ok(home) = std::env::var("HOME") {
            p = PathBuf::from(home).join(t.trim_start_matches("~/"));
        }
    }
    if !is_markdown(&p) {
        return Err(format!("{} is not a Markdown file", p.display()));
    }
    match std::fs::read_to_string(&p) {
        Ok(text) => Ok((p, text)),
        Err(e) => Err(e.to_string()),
    }
}

/// minifb reports physical keys; map the printable ones back to characters.
fn key_char(k: Key) -> Option<char> {
    Some(match k {
        Key::Space => ' ',
        Key::Menu => return None,
        Key::Apostrophe => '\'',
        Key::Backquote => '`',
        Key::Backslash => '\\',
        Key::Comma => ',',
        Key::Equal => '=',
        Key::LeftBracket => '[',
        Key::Minus => '-',
        Key::Period => '.',
        Key::RightBracket => ']',
        Key::Semicolon => ';',
        Key::Slash => '/',
        Key::A => 'a', Key::B => 'b', Key::C => 'c', Key::D => 'd', Key::E => 'e', Key::F => 'f',
        Key::G => 'g', Key::H => 'h', Key::I => 'i', Key::J => 'j', Key::K => 'k', Key::L => 'l',
        Key::M => 'm', Key::N => 'n', Key::O => 'o', Key::P => 'p', Key::Q => 'q', Key::R => 'r',
        Key::S => 's', Key::T => 't', Key::U => 'u', Key::V => 'v', Key::W => 'w', Key::X => 'x',
        Key::Y => 'y', Key::Z => 'z',
        Key::Key0 => '0', Key::Key1 => '1', Key::Key2 => '2', Key::Key3 => '3', Key::Key4 => '4',
        Key::Key5 => '5', Key::Key6 => '6', Key::Key7 => '7', Key::Key8 => '8', Key::Key9 => '9',
        _ => return None,
    })
}

/// What the command line asked for.
#[derive(Default)]
struct Args {
    mode: Option<Mode>,
    body_family: Option<String>,
    code_family: Option<String>,
    path: Option<PathBuf>,
}

/// Outcome of parsing: show usage, run, or fail with a message.
enum Parsed {
    Help,
    Run(Args),
    Err(String),
}

const USAGE: &str = "mimd [-v|-e] [--font FAMILY] [--mono FAMILY] FILE.md";

/// Pure argument parse. A flag that needs a value but has none is an error,
/// not a silently ignored argument.
fn parse_args<I: Iterator<Item = String>>(argv: I) -> Parsed {
    let mut a = Args::default();
    let mut args = argv.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-e" | "--edit" => a.mode = Some(Mode::Edit),
            "-v" | "--view" => a.mode = Some(Mode::View),
            "--font" | "--mono" => {
                let Some(v) = args.next() else {
                    return Parsed::Err(format!("{arg} needs a font family"));
                };
                if arg == "--font" {
                    a.body_family = Some(v);
                } else {
                    a.code_family = Some(v);
                }
            }
            "-h" | "--help" => return Parsed::Help,
            p if p.starts_with('-') && p.len() > 1 => {
                return Parsed::Err(format!("unknown option {p}"));
            }
            p => {
                if a.path.replace(PathBuf::from(p)).is_some() {
                    return Parsed::Err("only one file may be given".into());
                }
            }
        }
    }
    Parsed::Run(a)
}

fn main() {
    let a = match parse_args(std::env::args().skip(1)) {
        Parsed::Help => {
            println!("{USAGE}");
            println!("  Fonts default to the system UI and monospace fonts.");
            return;
        }
        Parsed::Err(e) => {
            eprintln!("mimd: {e}\n{USAGE}");
            std::process::exit(2);
        }
        Parsed::Run(a) => a,
    };
    let Args { mode, body_family, code_family, path } = a;
    let mode = mode.unwrap_or(Mode::View);

    // Fonts come from the OS; nothing is bundled.
    let mut fonts = match Fonts::new(body_family.as_deref(), code_family.as_deref(), BODY_PT) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("mimd: {e}");
            std::process::exit(1);
        }
    };

    let text = match path.clone() {
        // Same .md guard as the Ctrl+O prompt: the CLI must not be a way
        // around the restriction.
        Some(p) => match open_path(p.to_string_lossy().as_ref()) {
            Ok((_, t)) => t,
            Err(e) => {
                eprintln!("mimd: {e}");
                std::process::exit(1);
            }
        },
        None => {
            "# mimd\n\nA tiny Markdown viewer.\n\nPress **Ctrl+E** to edit.\n\nUnicode: বাংলা 日本語 Русский العربية\n\n```\ncode\n```\n".into()
        }
    };

    let mut app = App::open(text, path, mode);
    // Resizable: a document reader is often wider than its default window,
    // and a fixed-size window is a worse experience than a little more code.
    let opts = WindowOptions {
        resize: true,
        ..WindowOptions::default()
    };
    let mut win = match Window::new("mimd", W, H, opts) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("mimd: could not open a window: {e}");
            std::process::exit(1);
        }
    };
    // One 32px icon is what a taskbar or dock asks for; 64 keeps it crisp when
    // a window manager scales the titlebar up on a HiDPI display.
    if let Ok(icon) = icon_from_logo() {
        win.set_icon(icon);
    }
    // The buffer tracks the live window size: a resized window needs a bigger
    // buffer, and handing minifb the wrong length is a hard error.
    let (mut bw, mut bh) = (W, H);
    let mut buf = vec![0u32; bw * bh];
    let body_lh = fonts.body.line_height() as usize;

    while win.is_open() && !app.quit {
        let size = win.get_size();
        if size.0 != bw || size.1 != bh {
            bw = size.0;
            bh = size.1;
            buf = vec![0u32; bw * bh];
        }
        let shift = win.is_key_down(Key::LeftShift) || win.is_key_down(Key::RightShift);
        let ctrl = win.is_key_down(Key::LeftCtrl) || win.is_key_down(Key::RightCtrl);

        // The open prompt swallows all input until it is answered.
        if app.prompt.is_some() {
            if win.is_key_down(Key::Escape) {
                app.prompt = None;
                app.status = "open cancelled".into();
            } else if win.is_key_down(Key::Enter) {
                let typed = app.prompt.take().unwrap_or_default();
                match open_path(&typed) {
                    Ok((path, text)) => app.load(&path, text),
                    Err(e) => app.status = format!("open failed: {e}"),
                }
            } else if win.is_key_down(Key::Backspace) {
                if let Some(p) = app.prompt.as_mut() {
                    p.pop();
                }
            } else {
                for k in win.get_keys_pressed(KeyRepeat::Yes) {
                    if let Some(ch) = key_char(k) {
                        if let Some(p) = app.prompt.as_mut() {
                            p.push(ch);
                        }
                    }
                }
            }
        } else if ctrl {
            if win.is_key_down(Key::Q) {
                if app.dirty {
                    app.save();
                }
                app.quit = true;
            } else if win.is_key_down(Key::Z) {
                let ok = if shift { app.redo() } else { app.undo() };
                app.status = if ok {
                    if shift { "redo" } else { "undo" }.into()
                } else {
                    format!("nothing to {}", if shift { "redo" } else { "undo" })
                };
            } else if win.is_key_down(Key::Y) {
                app.status = if app.redo() { "redo".into() } else { "nothing to redo".into() };
            } else if win.is_key_down(Key::O) {
                let seed = app
                    .path
                    .as_ref()
                    .and_then(|p| p.to_str())
                    .map(|s| s.to_string())
                    .unwrap_or_default();
                app.prompt = Some(seed);
                app.status = String::new();
            } else if win.is_key_down(Key::E) {
                app.mode = if app.mode == Mode::View { Mode::Edit } else { Mode::View };
                app.status = String::new();
                app.scroll = 0;
            } else if win.is_key_down(Key::S) {
                app.save();
            } else if win.is_key_down(Key::C) {
                let t = app.selected();
                if t.is_empty() {
                    app.status = "nothing selected".into();
                } else {
                    app.clip = t.clone();
                    let sys = clip::copy(&t);
                    app.status = format!(
                        "copied {} chars{}",
                        t.chars().count(),
                        if sys { "" } else { " (internal)" }
                    );
                }
            } else if win.is_key_down(Key::X) {
                let t = app.selected();
                if !t.is_empty() {
                    app.clip = t.clone();
                    let sys = clip::copy(&t);
                    app.edit_delete_selection();
                    app.status = format!(
                        "cut {} chars{}",
                        t.chars().count(),
                        if sys { "" } else { " (internal)" }
                    );
                }
            } else if win.is_key_down(Key::V) {
                // Prefer the system clipboard, fall back to our own buffer.
                let t = clip::paste()
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| app.clip.clone());
                if !t.is_empty() {
                    app.clip = t.clone();
                    if app.mode == Mode::Edit {
                        app.edit_insert(&t.replace('\r', ""), EditKind::Other);
                        app.follow(body_lh);
                    }
                } else {
                    app.status = "clipboard empty".into();
                }
            } else if win.is_key_down(Key::A) {
                let last = app.lines.len() - 1;
                app.anchor = Some(Pos::default());
                app.cur = Pos { l: last, c: app.lines[last].chars().count() };
            }
        } else if app.mode == Mode::Edit {
            for k in [
                Key::Left, Key::Right, Key::Up, Key::Down, Key::Home, Key::End, Key::PageUp,
                Key::PageDown, Key::Backspace, Key::Delete, Key::Tab, Key::Escape,
            ] {
                if win.is_key_down(k) {
                    app.key(k, shift, body_lh);
                }
            }
            if win.is_key_down(Key::Enter) {
                app.insert("\n");
                app.follow(body_lh);
            }
            for k in win.get_keys_pressed(KeyRepeat::Yes) {
                if let Some(ch) = key_char(k) {
                    let ch = if shift { ch.to_ascii_uppercase() } else { ch };
                    app.edit_insert(&ch.to_string(), EditKind::Type);
                    app.follow(body_lh);
                }
            }
        } else {
            if win.is_key_down(Key::Down) || win.is_key_down(Key::PageDown) || win.is_key_down(Key::J) {
                app.scroll += 1;
            }
            if win.is_key_down(Key::Up) || win.is_key_down(Key::PageUp) || win.is_key_down(Key::K) {
                app.scroll = app.scroll.saturating_sub(1);
            }
            if win.is_key_down(Key::Home) {
                app.scroll = 0;
            }
            if win.is_key_down(Key::End) {
                app.scroll = usize::MAX;
            }
        }

        if let Some((_, dy)) = win.get_scroll_wheel() {
            let dy = dy as i32;
            app.scroll = if dy > 0 {
                app.scroll.saturating_sub(3)
            } else if dy < 0 {
                app.scroll + 3
            } else {
                app.scroll
            };
        }

        let (mouse_x, mouse_y, left_down) = {
            let pos = win.get_mouse_pos(MouseMode::Clamp).unwrap_or((-1.0, -1.0));
            (pos.0 as i32, pos.1 as i32, win.get_mouse_down(MouseButton::Left))
        };

        // Build the view layout once; input hit-testing and rendering share it.
        // The text column is capped at MAX_MEASURE and then centred: a
        // document reader should show a page of reasonable measure, not a
        // line stretched the width of the window.
        // The measure stays capped however wide the window gets; a resized
        // window gains margin, never an unreadable line length.
        let view_w = (bw as i32 - MARGIN as i32 * 2).clamp(120, MAX_MEASURE);
        let rows_fit = ((bh - MARGIN as usize * 2) / body_lh.max(1)).max(1);
        let mut scratch = Canvas { buf: &mut Vec::new(), w: 0, h: 0, fonts: &mut fonts };
        let text = app.text();
        let view = build_view_layout(&mut scratch, &text, view_w);

        let view_rows: Vec<i32> = view.ys.clone();
        let hs = app.hscroll;
        mouse_step(
            &mut app,
            mouse_x,
            mouse_y,
            left_down,
            hs,
            &|cv, line, x, hs| col_at_x(cv, line, x, hs),
            &mut scratch,
            &view_rows,
        );

        let mut canvas = Canvas { buf: &mut buf, w: bw as u32, h: bh as u32, fonts: &mut fonts };
        // Keep the caret visible horizontally in edit mode.
        if app.mode == Mode::Edit {
            let cx = caret_x(&mut canvas, &app.lines[app.cur.l], app.cur.c, app.hscroll);
            app.follow_x(cx, view_w);
            // Never scroll past the end of the longest line.
            let max = app.widest(&mut canvas) + MARGIN as i32 - view_w;
            let cap = max.max(0) as u32;
            if app.hscroll > cap {
                app.hscroll = cap;
            }
        } else {
            // Clamp vertical scroll to the laid-out document.
            let cap = max_scroll(view.lines.len(), rows_fit);
            if app.scroll > cap {
                app.scroll = cap;
            }
        }
        render(&app, &mut canvas, &view);
        drop(canvas);

        if let Err(e) = win.update_with_buffer(&buf, bw, bh) {
            eprintln!("mimd: {e}");
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(s: &str) -> std::vec::IntoIter<String> {
        s.split_whitespace().map(String::from).collect::<Vec<_>>().into_iter()
    }

    #[test]
    fn a_bare_path_defaults_to_view_mode() {
        let Parsed::Run(a) = parse_args(argv("notes.md")) else { panic!("expected a run") };
        assert_eq!(a.mode, None, "no flag means the caller picks the default");
        assert_eq!(a.path, Some(PathBuf::from("notes.md")));
    }

    #[test]
    fn edit_and_view_flags_set_the_mode() {
        let Parsed::Run(a) = parse_args(argv("-e notes.md")) else { panic!("expected a run") };
        assert_eq!(a.mode, Some(Mode::Edit));
        let Parsed::Run(b) = parse_args(argv("--view notes.md")) else { panic!("expected a run") };
        assert_eq!(b.mode, Some(Mode::View));
    }

    #[test]
    fn font_flags_consume_their_value() {
        let Parsed::Run(a) = parse_args(argv("--font Fira --mono JBMono notes.md")) else {
            panic!("expected a run")
        };
        assert_eq!(a.body_family.as_deref(), Some("Fira"));
        assert_eq!(a.code_family.as_deref(), Some("JBMono"));
        assert_eq!(a.path, Some(PathBuf::from("notes.md")));
    }

    #[test]
    fn a_font_flag_without_a_value_is_an_error() {
        assert!(matches!(parse_args(argv("--font")), Parsed::Err(_)));
        assert!(matches!(parse_args(argv("--mono")), Parsed::Err(_)));
        // A value that looks like a flag is still consumed as the family,
        // so the error is about the missing file, not a dangling flag.
        assert!(matches!(parse_args(argv("--font -e x.md")), Parsed::Run(_)));
    }

    #[test]
    fn an_unknown_flag_is_an_error_not_a_filename() {
        let Parsed::Err(e) = parse_args(argv("--nope notes.md")) else { panic!("expected an error") };
        assert!(e.contains("--nope"), "message should name the option: {e}");
    }

    #[test]
    fn two_files_are_rejected() {
        assert!(matches!(parse_args(argv("a.md b.md")), Parsed::Err(_)));
    }

    #[test]
    fn help_short_circuits() {
        assert!(matches!(parse_args(argv("--help")), Parsed::Help));
        assert!(matches!(parse_args(argv("-h")), Parsed::Help));
    }

    /// Offscreen proof that system-font glyphs actually rasterize into the
    /// framebuffer, including Unicode via fallback.
    #[test]
    fn renders_system_font_glyphs_into_pixels() {
        let mut fonts = match Fonts::new(None, None, 20) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        eprintln!("body font = {:?}, code font = {:?}", fonts.body.family, fonts.code.family);

        let mut buf = vec![BG; 300 * 60];
        {
            let mut c = Canvas { buf: &mut buf, w: 300, h: 60, fonts: &mut fonts };
            let base = 25;
            c.draw_str(4, base, "Hello Ag", Role::Body, FG);
            c.draw_str(4, 45, "日本語", Role::Body, FG);
        }
        let ink = buf.iter().filter(|p| **p != BG).count();
        assert!(ink > 100, "expected rasterized ink, got {ink} px");

        // Print a coarse view so the shape is inspectable.
        for y in (0..60).step_by(2) {
            let row: String = (0..120)
                .map(|x| if buf[y * 300 + x] != BG { '#' } else { '.' })
                .collect();
            println!("{row}");
        }
    }

    /// Minimal PNG writer (RGB8, no filtering) so a rendered frame can be
    /// opened in any image viewer. Test-only.
    fn write_png(path: &str, buf: &[u32], w: u32, h: u32) {
        fn crc(data: &[u8]) -> u32 {
            let mut c = 0xffff_ffffu32;
            for &b in data {
                c ^= b as u32;
                for _ in 0..8 {
                    c = if c & 1 != 0 { 0xedb8_8320 ^ (c >> 1) } else { c >> 1 };
                }
            }
            !c
        }
        let mut raw = Vec::with_capacity((w * h * 3 + h) as usize);
        for y in 0..h {
            raw.push(0); // filter: none
            for x in 0..w {
                // The framebuffer is 0RGB (as minifb wants), PNG is RGB.
                let p = buf[y as usize * w as usize + x as usize];
                raw.push(((p >> 16) & 0xff) as u8);
                raw.push(((p >> 8) & 0xff) as u8);
                raw.push((p & 0xff) as u8);
            }
        }
        // Stored (uncompressed) deflate blocks. The last block carries BFINAL;
        // getting that wrong yields a stream no decoder will accept.
        let mut idat = vec![0x78, 0x01];
        let blocks: Vec<&[u8]> = raw.chunks(65535).collect();
        for (i, chunk) in blocks.iter().enumerate() {
            let final_block = i + 1 == blocks.len();
            idat.push(if final_block { 1 } else { 0 });
            let n = chunk.len() as u16;
            idat.extend_from_slice(&n.to_le_bytes());
            idat.extend_from_slice(&(!n).to_le_bytes());
            idat.extend_from_slice(chunk);
        }
        let adler = {
            let (mut a, mut b) = (1u32, 0u32);
            for &x in &raw {
                a = (a + x as u32) % 65521;
                b = (b + a) % 65521;
            }
            (b << 16) | a
        };
        idat.extend_from_slice(&adler.to_be_bytes());
        let mut png = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        let mut chunk = |tag: &[u8], data: &[u8], out: &mut Vec<u8>| {
            out.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut full = tag.to_vec();
            full.extend_from_slice(data);
            out.extend_from_slice(&full);
            out.extend_from_slice(&crc(&full).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        chunk(b"IHDR", &ihdr, &mut png);
        chunk(b"IDAT", &idat, &mut png);
        chunk(b"IEND", &[], &mut png);
        let _ = std::fs::write(path, png);
    }

    /// Regression: view mode must actually scroll. Drawing the layout from
    /// index 0 regardless of `scroll` left the top of the document on screen
    /// no matter how far the user scrolled.
    #[test]
    fn scrolling_in_view_mode_actually_moves_the_page() {
        let mut fonts = match Fonts::new(None, None, BODY_PT) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let md: String = (0..80).map(|i| format!("Paragraph number {i} with enough text to fill a line.\n\n")).collect();
        let (w, h) = (W as u32, H as u32);
        let col = (w as i32 - MARGIN as i32 * 2).min(MAX_MEASURE);

        let k = 12usize;
        // One helper, several roles: a frame at a given scroll, and the layout
        // it came from. Separate closures would each hold `fonts` mutably.
        let mut shot = |src: &str, scroll: usize| -> (Vec<u32>, ViewLayout) {
            let mut buf = vec![BG; (w * h) as usize];
            let mut app = App::open(src.to_string(), None, Mode::View);
            app.scroll = scroll;
            let view = {
                let mut scratch = Canvas { buf: &mut Vec::new(), w: 0, h: 0, fonts: &mut fonts };
                build_view_layout(&mut scratch, &app.text(), col)
            };
            {
                let mut canvas = Canvas { buf: &mut buf, w, h, fonts: &mut fonts };
                render(&app, &mut canvas, &view);
            }
            (buf, view)
        };

        let (top, _) = shot(&md, 0);
        let (down, probe) = shot(&md, k);
        assert_ne!(top, down, "scrolling changed nothing: view mode is not scrolling");

        // Scrolling by N must shift the page up by roughly N lines: the text
        // that was N lines below the top is now at the top. Comparing a strip
        // of rows is the honest check, since some text is always on screen.
        let strip = |buf: &[u32], y0: usize, h: usize| -> Vec<u32> {
            buf[y0 * w as usize..(y0 + h) * w as usize].to_vec()
        };
        // Scrolling by N must place visual line N at the top of the viewport.
        // The shift is ys[N] pixels, not N*line_height: headings are taller
        // than body text and blocks carry gaps, so only the layout knows.
        // Line k is drawn at ys[k] - (ys[k] - ys[0]) == ys[0], i.e. exactly
        // where line 0 was, so the two strips must be identical.
        assert!(
            strip(&down, probe.ys[0] as usize, 60) == strip(&top, probe.ys[k] as usize, 60),
            "line {k} was not drawn at the top: the page did not scroll"
        );

        // The same invariant must hold for mixed content (headings, lists,
        // quotes, code) and for every target line, since the shift is derived
        // from ys[0] rather than a line height.
        for md in [
            "# A\n\n## B\n\ntext text text\n\n- one\n- two\n\n> quote\n\n```\ncode();\n```\n",
            "just one paragraph\n",
            "# H1\n",
            "",
        ] {
            for k in [0usize, 1, 2, 3, 5, 8] {
                let (a, _) = shot(md, 0);
                let (b, v) = shot(md, k);
                if k >= v.ys.len() {
                    continue;
                }
                let y0 = v.ys[0] as usize;
                let yk = v.ys[k] as usize;
                assert!(
                    strip(&b, y0, 24) == strip(&a, yk, 24),
                    "scrolling to line {k} of {md:?} did not put it at the top"
                );
            }
        }

        // Short documents must not scroll at all.
        assert_eq!(max_scroll(3, 20), 0, "short documents must not scroll");

        // Scrolling past the last line must render an empty page rather than
        // stale content: with no lines left to draw, nothing is painted.
        let (past, _) = shot(&md, 10_000);
        let body_ink = past
            .iter()
            .take(H.saturating_sub(MARGIN as usize) * w as usize)
            .filter(|&&v| v != BG)
            .count();
        assert_eq!(body_ink, 0, "scrolling past the end still painted content");
    }

    /// The icon must actually be the logo, not a blob. Checks the geometry
    /// against the SVG at 256 units, where each feature is large enough to
    /// probe unambiguously.
    #[test]
    fn the_icon_reproduces_the_svg_mark() {
        let size = 256u32;
        let px = logo_icon(size);
        assert_eq!(px.len() as u32, size * size, "icon buffer must be size*size pixels");

        let at = |x: u32, y: u32| (px[(y * size + x) as usize] >> 24) as u32;
        // Sampling the middle of a 9x9 block keeps antialiasing out of it.
        let solid = |x: u32, y: u32| -> bool {
            (0..9).all(|dy| (0..9).all(|dx| at(x + dx, y + dy) > 240))
        };
        // Knockouts are only 10 units wide, so probe them with a 5x5 block:
        // a 9x9 one would span the whole stroke and catch its inked sides.
        let clear = |x: u32, y: u32| -> bool {
            (0..5).all(|dy| (0..5).all(|dx| at(x + dx, y + dy) < 15))
        };

        // Body of the page: solid ink.
        assert!(solid(75, 90), "the page body should be inked");
        assert!(solid(175, 200), "the lower right of the page should be inked");
        // The folded corner cuts the top-right away, and the fold line knocks out.
        // (182,50) rather than nearer the diagonal: the page's cut edge runs
        // (152,40)->(188,76), so x=170 at y=50 straddles it.
        assert!(clear(182, 50), "outside the fold should be empty");
        // Well clear of the fold's 5-unit half-width, so a 9x9 block fits.
        assert!(solid(138, 55), "left of the fold line should be inked");
        // The fold line itself is a knockout, running down then across.
        assert!(clear(152, 60), "the vertical fold should be knocked out");
        assert!(clear(170, 76), "the horizontal fold should be knocked out");
        // Bullet + bar sit on a knocked-out line.
        assert!(clear(95, 111), "the bullet should be knocked out");
        assert!(clear(150, 111), "the bar should be knocked out");
        // The arrow stems and heads are knocked out of the lower page.
        assert!(clear(87, 168), "the first arrow stem should be knocked out");
        assert!(clear(123, 168), "the middle arrow stem should be knocked out");
        assert!(clear(159, 168), "the third arrow stem should be knocked out");
        assert!(clear(104, 165), "an arrow head should be knocked out");
        // Everything outside the page silhouette is empty.
        for (x, y) in [(10u32, 10u32), (250, 10), (10, 250), (250, 250), (60, 30)] {
            assert!(solid(x, y) == false, "({x},{y}) should be outside the mark");
        }
        // Knocked-out regions must not exceed the silhouette, or the arrows
        // would be punched through into the background.
        // Clear of the stems' 5-unit half-widths on both sides.
        assert!(solid(70, 170), "between the arrows the page is still inked");

        // Alpha is what carries the shape: RGB must be premultiplied with it.
        for &v in px.iter().take(size as usize * 4) {
            let a = (v >> 24) as u32;
            let r_ = ((v >> 16) & 0xff) as u32;
            assert_eq!(r_, a, "RGB must equal alpha for premultiplied ARGB");
        }

        // Opt-in dump so the mark can be eyeballed against the SVG.
        if let Ok(path) = std::env::var("MIMD_ICON_PNG") {
            let n = 256u32;
            let ic = logo_icon(n);
            let mut rgb = Vec::with_capacity((n * n * 3) as usize);
            for &v in &ic {
                let a = (v >> 24) as u8;
                // Composite onto white so knockouts read as paper.
                rgb.extend_from_slice(&[a, a, a]);
            }
            let mut rgba = Vec::with_capacity((n * n) as usize);
            for c in rgb.chunks(3) {
                rgba.push(0xff000000u32 | ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32);
            }
            write_png(&path, &rgba, n, n);
            eprintln!("wrote {path}");
        }

        // Small sizes must still resolve the counters, not turn to mush.
        let mut cov: Vec<(u32, f64)> = Vec::new();
        for s in [16u32, 24, 32, 64, 128, 256] {
            let small = logo_icon(s);
            assert_eq!(small.len() as u32, s * s, "{s}px icon has the wrong pixel count");
            // Coverage has to be alpha-weighted, not a count of fully opaque
            // pixels: the silhouette covers 31% of the design box, but at 16px
            // most edge pixels are partially covered and would read as empty.
            let total = (s as f64) * (s as f64);
            let sum: f64 = small.iter().map(|&v| (v >> 24) as f64).sum();
            let pct = 100.0 * sum / (255.0 * total);
            cov.push((s, pct));
        }
        let ref_pct = cov.iter().find(|(s, _)| *s == 256).map(|(_, p)| *p).unwrap();
        for (s, pct) in &cov {
            assert!(
                (ref_pct - 8.0..=ref_pct + 3.0).contains(pct),
                "{s}px ink coverage {pct:.1}% drifts from the 256px reference {ref_pct:.1}%"
            );
        }
    }

    /// A resized window must re-layout and fill the buffer: the buffer is
    /// reallocated to the live size, so a stale w/h either leaves a blank band
    /// or writes past the end.
    #[test]
    fn rendering_at_any_window_size_fills_the_buffer_exactly() {
        let mut fonts = match Fonts::new(None, None, BODY_PT) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let md = "# Title\n\nBody text that wraps across the measure nicely.\n\n- a\n- b\n".to_string();
        for (bw, bh) in [(W as u32, H as u32), (400, 300), (1200, 900), (200, 120), (320, 700)] {
            let buf_len = (bw * bh) as usize;
            let mut buf = vec![0xdead_beefu32; buf_len];
            let col = (bw as i32 - MARGIN as i32 * 2).clamp(120, MAX_MEASURE);
            let app = App::open(md.clone(), None, Mode::View);
            {
                let mut scratch = Canvas { buf: &mut Vec::new(), w: 0, h: 0, fonts: &mut fonts };
                let view = build_view_layout(&mut scratch, &app.text(), col);
                let mut canvas = Canvas { buf: &mut buf, w: bw, h: bh, fonts: &mut fonts };
                render(&app, &mut canvas, &view);
            }
            // Nothing outside the visible window area may be left unwritten.
            let body = (bh as usize).saturating_sub(40) * bw as usize; // exclude the footer band
            let untouched = buf[..body.min(buf_len)].iter().filter(|&&v| v == 0xdead_beef).count();
            assert_eq!(untouched, 0, "{bw}x{bh}: {untouched} pixels were never written");
            // The last row must be reachable, i.e. no off-by-one in the fill.
            let last_row_start = ((bh as usize) - 1) * bw as usize;
            assert!(
                buf[last_row_start..last_row_start + bw as usize].iter().all(|&v| v != 0xdead_beef),
                "{bw}x{bh}: the final row was not written"
            );
        }
    }

    /// Render the real window at the real size and report what landed in it,
    /// so the design can be inspected without a display.
    #[test]
    fn renders_the_window_with_the_design_tokens_applied() {
        let mut fonts = match Fonts::new(None, None, BODY_PT) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let md = "# mimd\n\nA tiny reader for **technical documents**, with *emphasis* and `code` \
                 that wraps across several lines so the measure can be judged.\n\n## Install\n\n\
                 - fetch the binary\n- make it executable\n  - nested item\n\n\
                 > Reads like a page, not a terminal.\n\n```\nfetch https://example.com\n```\n\n---\n\n\
                 Done.\n";
        let (w, h) = (W as u32, H as u32);
        let mut buf = vec![BG; (w * h) as usize];
        let app = App::open(md.into(), None, Mode::View);
        {
            let mut scratch = Canvas { buf: &mut Vec::new(), w: 0, h: 0, fonts: &mut fonts };
            let col = (w as i32 - MARGIN as i32 * 2).min(MAX_MEASURE);
            let view = build_view_layout(&mut scratch, &app.text(), col);
            let mut canvas = Canvas { buf: &mut buf, w, h, fonts: &mut fonts };
            render(&app, &mut canvas, &view);
        }
        // Distinct colours actually present: page, ink, muted, accent.
        let mut seen = std::collections::HashSet::new();
        for p in &buf {
            seen.insert(*p);
        }
        for c in [BG, PANEL, RULE, DIM, ACCENT, HEAD, FG] {
            assert!(seen.contains(&c), "token {c:#06x} never appeared on screen");
        }
        // The text column must not bleed into the right margin.
        let col_end = MARGIN + MAX_MEASURE as u32;
        for y in 0..h - 40 {
            for x in col_end..(w - MARGIN) {
                assert_eq!(
                    buf[y as usize * w as usize + x as usize], BG,
                    "text or rule escaped the measure at {x},{y}"
                );
            }
        }
        // Measure the longest run of non-background pixels: the line length.
        let mut widest = 0;
        for y in 0..(h - 40) {
            let mut run = 0;
            let mut best = 0;
            for x in 0..w {
                if buf[y as usize * w as usize + x as usize] != BG {
                    run += 1;
                    best = best.max(run);
                } else {
                    run = 0;
                }
            }
            widest = widest.max(best);
        }
        eprintln!("widest ink run = {widest}px of a {MAX_MEASURE}px measure");
        // MIMD_SNAPSHOT=<path.png> writes the frame so the design can be
        // inspected without a display.
        if let Ok(path) = std::env::var("MIMD_SNAPSHOT") {
            write_png(&path, &buf, w, h);
            eprintln!("wrote {path}");
        }
        assert!(
            widest <= MAX_MEASURE as u32,
            "a line ran {widest}px, past the {MAX_MEASURE}px measure"
        );
        // Bold and regular must not rasterize identically.
        let mut bw = 0;
        let mut rw = 0;
        for ch in "Handgloves" .chars() {
            bw += fonts.measure_at(Role::BodyBold, BODY_PT, &ch.to_string());
            rw += fonts.measure_at(Role::Body, BODY_PT, &ch.to_string());
        }
        assert!(bw > rw, "bold face is not wider than regular ({bw} vs {rw})");
    }

    /// Full pipeline: parse -> layout -> rasterize, offscreen.
    /// Prints the result so the visual output can be inspected.
    #[test]
    fn renders_a_full_document() {
        let mut fonts = match Fonts::new(None, None, 15) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("skip: {e}");
                return;
            }
        };
        let md = "# Title\n\nSome **bold** and *italic* and `code` text that is long \
enough to need wrapping across more than one visual line in the window.\n\n## Sub\n\n- one\n- two\n\n> quote\n\n```\nfn main() {}\n```\n";
        let (w, h) = (420u32, 420u32);
        let mut buf = vec![BG; (w * h) as usize];
        let lines;
        {
            let mut c = Canvas { buf: &mut buf, w, h, fonts: &mut fonts };
            let vls = layout_view(&mut c, md, w as i32 - 20);
            lines = vls.len();
            assert!(lines >= 9, "expected wrapping to produce lines, got {lines}");
            // No visual line may exceed the viewport width.
            for vl in &vls {
                let mut x = vl.indent;
                let mut seg = String::new();
                let mut style = S::Text;
                let mut pen = 0;
                for &(ch, st) in &vl.cells {
                    if st != style {
                        pen += c.measure_at(&seg, role_for(style), vl.size);
                        seg.clear();
                        style = st;
                    }
                    seg.push(ch);
                }
                pen += c.measure_at(&seg, role_for(style), vl.size);
                x += pen;
                assert!(x <= w as i32, "line overflows: {x} > {w}");
            }
            let app = App::open(md.to_string(), None, Mode::View);
            let vl = build_view_layout(&mut c, md, w as i32 - 20);
            render(&app, &mut c, &vl);
        }
        let ink = buf.iter().filter(|p| **p != BG).count();
        assert!(ink > 500, "expected substantial ink, got {ink}");
        eprintln!("visual lines = {lines}, ink px = {ink}");

        for y in (0..h as usize).step_by(3) {
            let row: String = (0..w as usize)
                .step_by(2)
                .map(|x| if buf[y * w as usize + x] != BG { '#' } else { '.' })
                .collect();
            println!("{row}");
        }
    }

    fn canvas() -> Option<(Vec<u32>, Fonts)> {
        let fonts = Fonts::new(None, None, 15).ok()?;
        Some((vec![BG; 400 * 400], fonts))
    }

    #[test]
    fn clicking_left_of_a_glyph_selects_the_earlier_column() {
        let Some((mut buf, mut fonts)) = canvas() else { return };
        let mut c = Canvas { buf: &mut buf, w: 400, h: 400, fonts: &mut fonts };
        let line = "hello world";
        // col_at_x measures from the text origin, which sits at MARGIN.
        let origin = MARGIN as i32;
        let w_h = c.measure_at("h", Role::Body, 15);
        // A click at the text origin must map to column 0.
        assert_eq!(col_at_x(&mut c, line, origin, 0), 0);
        // Inside the first glyph's left half -> column 0.
        assert_eq!(col_at_x(&mut c, line, origin + w_h / 3, 0), 0);
        // Past the first glyph's midpoint -> column 1.
        assert_eq!(col_at_x(&mut c, line, origin + w_h, 0), 1);
        // Monotonic across the whole line.
        let mut prev = 0;
        for x in 0..120 {
            let col = col_at_x(&mut c, line, origin + x, 0);
            assert!(col >= prev, "column went backwards at x={x}");
            prev = col;
        }
    }

    #[test]
    fn clicking_far_right_lands_at_end_of_line() {
        let Some((mut buf, mut fonts)) = canvas() else { return };
        let mut c = Canvas { buf: &mut buf, w: 400, h: 400, fonts: &mut fonts };
        let line = "hello";
        let total = c.measure(line, Role::Body);
        assert_eq!(col_at_x(&mut c, line, MARGIN as i32 + total + 500, 0), 5);
        assert_eq!(col_at_x(&mut c, line, MARGIN as i32 + total - 1, 0), 5);
    }

    #[test]
    fn horizontal_scroll_shifts_hit_testing_consistently() {
        let Some((mut buf, mut fonts)) = canvas() else { return };
        let mut c = Canvas { buf: &mut buf, w: 400, h: 400, fonts: &mut fonts };
        let line = "hello world";
        // With 40px of scroll, a click at x=0 is 40px into the line.
        let col = col_at_x(&mut c, line, 0, 40);
        assert!(col > 0, "expected scroll to move the caret right, got {col}");
    }

    #[test]
    fn caret_x_is_monotonic_across_the_line() {
        let Some((mut buf, mut fonts)) = canvas() else { return };
        let mut c = Canvas { buf: &mut buf, w: 400, h: 400, fonts: &mut fonts };
        let line = "monotonic caret positions";
        let mut prev = i32::MIN;
        for col in 0..=line.chars().count() {
            let x = caret_x(&mut c, line, col, 0);
            assert!(x >= prev, "caret went backwards at {col}: {x} < {prev}");
            prev = x;
        }
    }

    #[test]
    fn view_layout_records_a_pixel_y_per_line() {
        let Some((mut buf, mut fonts)) = canvas() else { return };
        let mut c = Canvas { buf: &mut buf, w: 400, h: 400, fonts: &mut fonts };
        let md = "# One\n\npara one\n\npara two\n\n## Two\n";
        let v = build_view_layout(&mut c, md, 380);
        // # One, para one, para two, ## Two
        assert_eq!(v.lines.len(), 4, "expected one visual line per block");
        assert_eq!(v.ys.len(), v.lines.len());
        // The first line starts at the top margin.
        assert_eq!(v.ys[0], MARGIN as i32);
    }

    #[test]
    fn view_layout_positions_increase_monotonically() {
        let Some((mut buf, mut fonts)) = canvas() else { return };
        let mut c = Canvas { buf: &mut buf, w: 400, h: 400, fonts: &mut fonts };
        let md = "# H\n\n- a\n- b\n\n> q\n\n```\ncode\n```\n";
        let v = build_view_layout(&mut c, md, 380);
        for w in v.ys.windows(2) {
            assert!(w[1] > w[0], "ys not increasing: {:?}", w);
        }
    }

    #[test]
    fn horizontal_scroll_stays_within_document() {
        let Some((mut buf, mut fonts)) = canvas() else { return };
        let mut c = Canvas { buf: &mut buf, w: 400, h: 400, fonts: &mut fonts };
        let mut a = App::open("short".into(), None, Mode::Edit);
        let view_w = 380;
        // Caret far to the right scrolls right...
        a.cur = Pos { l: 0, c: 5 };
        let cx = caret_x(&mut c, &a.lines[0], a.cur.c, a.hscroll);
        a.follow_x(cx, view_w);
        // ...then the cap pulls it back for a short document.
        let max = a.widest(&mut c) + MARGIN as i32 - view_w;
        if max < 0 {
            assert_eq!(a.hscroll, 0, "short lines must not scroll horizontally");
        }
    }

    /// Clipboard must degrade to the in-app buffer when no system tool
    /// exists, and must never claim success when it did nothing.
    #[test]
    fn clipboard_degrades_gracefully() {
        let copied = clip::copy("mimd clipboard test");
        let pasted = clip::paste();
        if copied {
            assert!(pasted.is_some(), "copy claimed success but paste found nothing");
            assert_eq!(pasted.as_deref(), Some("mimd clipboard test"));
        } else {
            assert!(pasted.is_none(), "paste worked but copy reported failure");
        }
    }

    #[test]
    fn clipboard_rejects_empty_input() {
        assert!(!clip::copy(""));
    }

    fn tmp_md(name: &str, body: &str) -> PathBuf {
        let p = std::env::temp_dir().join(format!("mimd-test-{name}"));
        std::fs::write(&p, body).unwrap();
        p
    }

    #[test]
    fn open_rejects_non_markdown_paths() {
        let bad = tmp_md("bad.txt", "hi");
        let r = open_path(bad.to_str().unwrap());
        assert!(r.is_err(), "a .txt file must be refused");
        assert!(open_path("/etc/passwd").is_err());
        assert!(open_path("/etc/shadow").is_err());
        assert!(open_path("").is_err());
        assert!(open_path("   ").is_err());
        let _ = std::fs::remove_file(bad);
    }

    #[test]
    fn open_reports_a_missing_file_without_panicking() {
        assert!(open_path("/nonexistent/definitely-not-here.md").is_err());
    }

    #[test]
    fn open_accepts_md_and_markdown() {
        let a = tmp_md("a.md", "# A\n");
        assert!(open_path(a.to_str().unwrap()).is_ok());
        let b = tmp_md("b.markdown", "# B\n");
        assert!(open_path(b.to_str().unwrap()).is_ok());
        let _ = std::fs::remove_file(a);
        let _ = std::fs::remove_file(b);
    }

    #[test]
    fn open_expands_tilde() {
        // A ~ path must not be treated as a literal directory named "~".
        std::env::set_var("HOME", "/tmp");
        let r = open_path("~/mimd-should-not-exist-xyz.md");
        assert!(r.is_err());
        assert!(
            !r.unwrap_err().contains("~"),
            "the tilde should have been expanded before reporting"
        );
    }

    #[test]
    fn load_resets_cursor_scroll_and_dirty() {
        let mut a = App::open("old\ncontent".into(), None, Mode::Edit);
        a.cur = Pos { l: 1, c: 3 };
        a.scroll = 7;
        a.hscroll = 40;
        a.dirty = true;
        let p = tmp_md("load.md", "# New\nbody\n");
        let text = std::fs::read_to_string(&p).unwrap();
        a.load(&p, text);
        assert_eq!(a.cur, Pos::default());
        assert_eq!(a.scroll, 0);
        assert_eq!(a.hscroll, 0);
        assert!(!a.dirty);
        assert_eq!(a.text(), "# New\nbody\n", "trailing newline preserved");
        assert_eq!(a.path.as_deref(), Some(p.as_path()));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn load_never_yields_an_empty_document() {
        let p = tmp_md("empty.md", "");
        let mut a = App::open("x".into(), None, Mode::View);
        a.load(&p, String::new());
        assert_eq!(a.lines.len(), 1, "an empty file must still give one editable line");
        assert!(a.text().is_empty());
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn opening_a_new_file_does_not_leak_the_old_document() {
        let p = tmp_md("fresh.md", "fresh\n");
        let mut a = App::open("stale secret content\nsecond line".into(), None, Mode::Edit);
        a.load(&p, "fresh\n".into());
        assert!(!a.text().contains("secret"));
        assert_eq!(a.selected(), "");
        let _ = std::fs::remove_file(p);
    }

    /// Round-trip: write via save(), then read back through open_path().
    #[test]
    fn save_then_open_round_trips() {
        let p = tmp_md("round.md", "old\n");
        let mut a = App::open("old\n".into(), Some(p.clone()), Mode::Edit);
        a.insert("# Fresh\n\n- item\n");
        a.save();
        assert!(!a.dirty, "save must clear the dirty flag");
        assert!(a.status.starts_with("saved"));
        let (back, text) = open_path(p.to_str().unwrap()).expect("reopen");
        assert_eq!(back, p);
        assert_eq!(text, a.text());
        assert!(text.contains("Fresh"));
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn saving_without_a_path_is_refused_not_silent() {
        let mut a = App::open("unsaved work\n".into(), None, Mode::Edit);
        a.save();
        assert!(a.status.contains("no file path"), "got: {}", a.status);
    }

    #[test]
    fn save_refuses_non_markdown_targets() {
        let bad = std::env::temp_dir().join("mimd-test-save.txt");
        let _ = std::fs::write(&bad, "x");
        let mut a = App::open("data\n".into(), Some(bad.clone()), Mode::Edit);
        a.insert("edited\n");
        assert!(a.dirty);
        a.save();
        assert!(a.status.contains("non-Markdown"), "got: {}", a.status);
        assert!(a.dirty, "a refused save must leave the buffer dirty");
        let _ = std::fs::remove_file(bad);
    }

    /// Drive `mouse_step` the way the event loop does, with no display.
    struct Mouse {
        app: App,
        buf: Vec<u32>,
        fonts: Fonts,
    }

    impl Mouse {
        fn new(app: App) -> Option<Self> {
            Some(Mouse { app, buf: vec![BG; 400 * 400], fonts: Fonts::new(None, None, 15).ok()? })
        }

        fn step(&mut self, x: i32, y: i32, down: bool) -> MouseAction {
            let mut c = Canvas {
                buf: &mut self.buf,
                w: 400,
                h: 400,
                fonts: &mut self.fonts,
            };
            let hs = self.app.hscroll;
            mouse_step(&mut self.app, x, y, down, hs, &|cv, l, xx, h| col_at_x(cv, l, xx, h), &mut c, &[])
        }

        /// Click at the Nth column of line L.
        fn click(&mut self, l: usize, col: usize) {
            let mut c = Canvas { buf: &mut self.buf, w: 400, h: 400, fonts: &mut self.fonts };
            let lh = c.line_height(Role::Body) as i32;
            let y = MARGIN as i32 + l as i32 * lh + lh / 2;
            let x = caret_x(&mut c, &self.app.lines[l], col, 0);
            drop(c);
            self.step(x, y, true);
            self.step(x, y, false);
        }
    }

    fn edit_app(text: &str) -> App {
        App::open(text.into(), None, Mode::Edit)
    }

    #[test]
    fn clicking_places_the_caret_at_that_column() {
        let Some(mut m) = Mouse::new(edit_app("hello world")) else { return };
        m.click(0, 6);
        assert_eq!(m.app.cur, Pos { l: 0, c: 6 });
        assert!(m.app.anchor.is_none(), "a plain click must not select");
        assert!(!m.app.dragging);
    }

    #[test]
    fn clicking_a_later_line_moves_down() {
        let Some(mut m) = Mouse::new(edit_app("one\ntwo\nthree\nfour")) else { return };
        m.click(2, 3);
        assert_eq!(m.app.cur.l, 2);
        assert_eq!(m.app.cur.c, 3);
    }

    #[test]
    fn clicking_past_the_last_line_clamps_to_it() {
        let Some(mut m) = Mouse::new(edit_app("one\ntwo")) else { return };
        // A y far below the content must not index out of bounds.
        m.step(20, 5000, true);
        m.step(20, 5000, false);
        assert_eq!(m.app.cur.l, 1, "should clamp to the final line");
    }

    #[test]
    fn dragging_selects_from_press_to_release_point() {
        let Some(mut m) = Mouse::new(edit_app("hello world")) else { return };
        let mut c = Canvas { buf: &mut m.buf, w: 400, h: 400, fonts: &mut m.fonts };
        let lh = c.line_height(Role::Body) as i32;
        let y = MARGIN as i32 + lh / 2;
        let x_end = caret_x(&mut c, &m.app.lines[0], 5, 0);
        drop(c);
        // press at column 0, drag to column 5, release
        m.step(MARGIN as i32, y, true);
        assert_eq!(m.app.cur, Pos { l: 0, c: 0 });
        assert!(m.app.dragging);
        m.step(x_end, y, true);
        assert_eq!(m.app.cur, Pos { l: 0, c: 5 });
        assert!(m.app.dragging, "still dragging while held");
        m.step(x_end, y, false);
        assert!(!m.app.dragging, "release ends the drag");
        assert_eq!(m.app.selected(), "hello");
    }

    #[test]
    fn dragging_backwards_selects_the_same_text() {
        let Some(mut m) = Mouse::new(edit_app("hello world")) else { return };
        let mut c = Canvas { buf: &mut m.buf, w: 400, h: 400, fonts: &mut m.fonts };
        let lh = c.line_height(Role::Body) as i32;
        let y = MARGIN as i32 + lh / 2;
        let x5 = caret_x(&mut c, &m.app.lines[0], 5, 0);
        drop(c);
        m.step(x5, y, true);
        m.step(MARGIN as i32, y, true);
        m.step(MARGIN as i32, y, false);
        assert_eq!(m.app.selected(), "hello", "reverse drag must select the same span");
    }

    #[test]
    fn dragging_across_lines_selects_multiline_text() {
        let Some(mut m) = Mouse::new(edit_app("aa\nbb\ncc")) else { return };
        let c = Canvas { buf: &mut m.buf, w: 400, h: 400, fonts: &mut m.fonts };
        let lh = c.line_height(Role::Body) as i32;
        let y0 = MARGIN as i32 + lh / 2;
        let y2 = MARGIN as i32 + 2 * lh + lh / 2;
        drop(c);
        m.step(MARGIN as i32, y0, true);
        m.step(MARGIN as i32, y2, true);
        m.step(MARGIN as i32, y2, false);
        assert_eq!(m.app.selected(), "aa
bb
");
    }

    #[test]
    fn a_second_click_starts_a_fresh_selection() {
        let Some(mut m) = Mouse::new(edit_app("hello world")) else { return };
        m.click(0, 0);
        m.step(MARGIN as i32 + 100, MARGIN as i32 + 8, true);
        m.step(MARGIN as i32 + 100, MARGIN as i32 + 8, false);
        assert!(m.app.selected().is_empty(), "clicking again must clear the old selection");
    }

    #[test]
    fn the_mouse_does_nothing_in_view_mode() {
        let Some(mut m) = Mouse::new(App::open("hello".into(), None, Mode::View)) else { return };
        let before = m.app.cur;
        m.step(50, 50, true);
        m.step(50, 50, false);
        assert_eq!(m.app.cur, before, "view mode has no caret to move");
        assert!(!m.app.dragging);
    }

    #[test]
    fn clicking_above_the_margin_ignores_the_event() {
        let Some(mut m) = Mouse::new(edit_app("hello")) else { return };
        m.step(50, 0, true);
        m.step(50, 0, false);
        assert_eq!(m.app.cur, Pos::default());
        assert!(!m.app.dragging, "a press above the text must not start a drag");
    }

    #[test]
    fn scroll_never_runs_past_the_end_of_the_document() {
        // Short document: cannot scroll at all.
        assert_eq!(max_scroll(3, 20), 0);
        // Exact fit: cannot scroll.
        assert_eq!(max_scroll(20, 20), 0);
        // Longer than the window: last page is fully filled.
        assert_eq!(max_scroll(50, 20), 30);
        // End key sets usize::MAX, and the clamp pulls it back.
        let cap = max_scroll(50, 20);
        assert!(cap < usize::MAX);
    }

    #[test]
    fn following_the_caret_keeps_it_in_view() {
        let mut a = App::open((0..100).map(|i| i.to_string()).collect::<Vec<_>>().join("\n"), None, Mode::Edit);
        a.follow(10);
        assert_eq!(a.scroll, 0);
        a.cur = Pos { l: 50, c: 0 };
        a.follow(10);
        assert!(a.cur.l >= a.scroll && a.cur.l < a.scroll + 10, "caret must be visible");
        a.cur = Pos { l: 0, c: 0 };
        a.follow(10);
        assert_eq!(a.scroll, 0, "moving back to the top must scroll up");
    }

    fn edit_app2(text: &str) -> App {
        App::open(text.into(), None, Mode::Edit)
    }

    #[test]
    fn undo_restores_previous_text() {
        let mut a = edit_app2("start");
        a.cur = Pos { l: 0, c: 5 };
        a.edit_insert("!", EditKind::Other);
        assert_eq!(a.text(), "start!");
        assert!(a.undo());
        assert_eq!(a.text(), "start");
        assert!(a.dirty, "undoing an edit leaves the document modified");
    }

    #[test]
    fn undo_with_nothing_to_undo_is_harmless() {
        let mut a = edit_app2("start");
        assert!(!a.undo(), "nothing to undo should report false");
        assert_eq!(a.text(), "start");
        assert!(!a.redo());
        assert_eq!(a.text(), "start");
    }

    #[test]
    fn redo_replays_an_undone_edit() {
        let mut a = edit_app2("start");
        a.cur = Pos { l: 0, c: 5 };
        a.edit_insert("!", EditKind::Other);
        assert!(a.undo());
        assert_eq!(a.text(), "start");
        assert!(a.redo());
        assert_eq!(a.text(), "start!");
    }

    #[test]
    fn a_fresh_edit_discards_the_redo_stack() {
        let mut a = edit_app2("start");
        a.cur = Pos { l: 0, c: 5 };
        a.edit_insert("!", EditKind::Other);
        a.undo();
        assert!(!a.redo.is_empty());
        a.edit_insert("?", EditKind::Other);
        assert!(a.redo.is_empty(), "a new edit must invalidate redo");
        assert!(!a.redo());
        assert_eq!(a.text(), "start?");
    }

    #[test]
    fn a_run_of_typing_undoes_as_one_step() {
        let mut a = edit_app2("");
        for ch in "hello".chars() {
            a.edit_insert(&ch.to_string(), EditKind::Type);
        }
        assert_eq!(a.text(), "hello");
        assert!(a.undo());
        assert_eq!(a.text(), "", "a whole word should undo in one step");
    }

    #[test]
    fn movement_breaks_the_typing_run() {
        let mut a = edit_app2("");
        a.edit_insert("ab", EditKind::Type);
        // A caret move must end the coalescing run.
        a.move_to(Pos { l: 0, c: 0 }, false);
        a.edit_insert("X", EditKind::Type);
        assert_eq!(a.text(), "Xab");
        assert!(a.undo());
        assert_eq!(a.text(), "ab", "only the post-move character undoes");
        assert!(a.undo());
        assert_eq!(a.text(), "");
    }

    #[test]
    fn backspace_deletes_undo_as_one_step() {
        let mut a = edit_app2("abc");
        a.cur = Pos { l: 0, c: 3 };
        a.edit_backspace();
        assert_eq!(a.text(), "ab");
        assert!(a.undo());
        assert_eq!(a.text(), "abc");
    }

    #[test]
    fn undo_restores_the_caret_and_selection_is_cleared() {
        let mut a = edit_app2("hello");
        a.cur = Pos { l: 0, c: 5 };
        a.edit_insert("!", EditKind::Other);
        a.anchor = Some(Pos { l: 0, c: 2 });
        a.undo();
        assert_eq!(a.cur, Pos { l: 0, c: 5 }, "caret must return to where it was");
        assert!(a.anchor.is_none(), "selection must not survive an undo");
    }

    #[test]
    fn undo_history_is_bounded() {
        let mut a = edit_app2("");
        for _ in 0..(MAX_UNDO + 50) {
            a.edit_insert("x", EditKind::Other);
        }
        assert!(a.undo.len() <= MAX_UNDO, "history must not grow without bound");
    }

    #[test]
    fn deleting_a_selection_is_one_undo_step() {
        let mut a = edit_app2("hello world");
        a.anchor = Some(Pos { l: 0, c: 0 });
        a.cur = Pos { l: 0, c: 5 };
        a.edit_delete_selection();
        assert_eq!(a.text(), " world");
        assert!(a.undo());
        assert_eq!(a.text(), "hello world");
    }

    #[test]
    fn typing_over_a_selection_undoes_both_at_once() {
        let mut a = edit_app2("hello world");
        a.anchor = Some(Pos { l: 0, c: 0 });
        a.cur = Pos { l: 0, c: 5 };
        a.edit_insert("bye", EditKind::Type);
        assert_eq!(a.text(), "bye world");
        assert!(a.undo());
        assert_eq!(a.text(), "hello world", "the replaced text must come back");
    }

    #[test]
    fn loading_a_file_clears_the_history() {
        let p = tmp_md("hist.md", "new\n");
        let mut a = edit_app2("old\n");
        a.edit_insert("!", EditKind::Other);
        a.load(&p, "new\n".into());
        assert!(!a.undo(), "history from the previous document must not apply");
        assert_eq!(a.text(), "new\n");
        let _ = std::fs::remove_file(p);
    }

    #[test]
    fn a_trailing_newline_survives_open_and_save() {
        // Files conventionally end in a newline; dropping it would rewrite
        // every file the user merely opened.
        let a = App::open("# T\n\nbody\n".into(), None, Mode::View);
        assert!(a.text().ends_with('\n'));
        assert_eq!(a.text(), "# T\n\nbody\n");
    }

    #[test]
    fn a_file_without_a_trailing_newline_keeps_its_shape() {
        let a = App::open("# T\n\nbody".into(), None, Mode::View);
        assert_eq!(a.text(), "# T\n\nbody");
        assert!(!a.text().ends_with('\n'));
    }

    #[test]
    fn round_trip_through_disk_preserves_bytes() {
        for original in ["# T\n\nbody\n", "# T\n\nbody", "one\n", "one", ""] {
            let p = tmp_md("bytes.md", original);
            let (path, text) = open_path(p.to_str().unwrap()).expect("reopen");
            let mut a = App::open(text, Some(path.clone()), Mode::Edit);
            a.save();
            let after = std::fs::read_to_string(&p).unwrap();
            assert_eq!(after, original, "round-trip changed {original:?}");
            let _ = std::fs::remove_file(p);
        }
    }

    #[test]
    fn crlf_line_endings_are_normalised_on_load() {
        let a = App::open("a\r\nb\r\n".into(), None, Mode::Edit);
        assert_eq!(a.lines, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(a.text(), "a\nb\n");
    }

    #[test]
    fn inline_styles() {
        let r = inline("a **b** `c` [d](e)", S::Text);
        assert_eq!(r[0], ('a', S::Text));
        for (want, style) in [('b', S::Bold), ('c', S::Code), ('d', S::Link)] {
            assert!(r.iter().any(|&(ch, s)| ch == want && s == style), "missing {want:?} as {style:?}");
        }
    }

    #[test]
    fn unclosed_marker_is_literal() {
        let r = inline("a * b", S::Text);
        assert_eq!(r.iter().map(|x| x.0).collect::<String>(), "a * b");
    }

    #[test]
    fn parser_emits_blocks_not_rows() {
        let b = parse("# T\n\n- item\n\n> quoted\n\n1. first\n\n---\n");
        assert_eq!(b[0], Block::Heading(1, vec![('T', S::Text)]));
        assert_eq!(b[1], Block::Bullet(0, vec![('i', S::Text), ('t', S::Text), ('e', S::Text), ('m', S::Text)]));
        assert_eq!(b[2], Block::Quote(vec![('q', S::Text), ('u', S::Text), ('o', S::Text), ('t', S::Text), ('e', S::Text), ('d', S::Text)]));
        assert!(matches!(b[3], Block::Number(0, _)));
        assert_eq!(b[4], Block::Rule);
    }

    #[test]
    fn heading_levels_survive_parsing() {
        let b = parse("# one\n## two\n### three\n#### four\n##### five\n###### six\n");
        let levels: Vec<u8> = b
            .iter()
            .filter_map(|x| match x {
                Block::Heading(l, _) => Some(*l),
                _ => None,
            })
            .collect();
        assert_eq!(levels, vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn heading_sizes_decrease_with_depth() {
        let base = 15;
        assert!(heading_size(1, base) > heading_size(2, base));
        assert!(heading_size(2, base) > heading_size(3, base));
        assert!(heading_size(3, base) > heading_size(6, base));
        assert_eq!(heading_size(6, base), base);
    }

    #[test]
    fn nested_lists_keep_their_depth() {
        let b = parse("- a\n    - b\n        - c\n");
        let depths: Vec<usize> = b
            .iter()
            .filter_map(|x| match x {
                Block::Bullet(d, _) => Some(*d),
                _ => None,
            })
            .collect();
        assert_eq!(depths, vec![0, 2, 4]);
    }

    #[test]
    fn italic_is_its_own_style_and_drops_delimiters() {
        let r = inline("*em*", S::Text);
        assert_eq!(r, vec![('e', S::Italic), ('m', S::Italic)]);
        let b = inline("**strong**", S::Text);
        assert!(b.iter().all(|&(_, s)| s == S::Bold));
        assert!(!b.iter().any(|&(c, _)| c == '*'));
    }

    #[test]
    fn fenced_code_becomes_a_code_block() {
        let b = parse("```\nlet x = 1;\n# not a heading\n```\n");
        match &b[0] {
            Block::Code(run) => {
                let t: String = run.iter().map(|x| x.0).collect();
                assert_eq!(t, "let x = 1;\n# not a heading");
                assert!(run.iter().all(|&(_, s)| s == S::Code));
            }
            other => panic!("expected Code, got {other:?}"),
        }
    }

    #[test]
    fn code_span_wins_over_surrounding_emphasis() {
        let r = inline("**a `b`**", S::Text);
        assert!(r.iter().any(|&(c, s)| c == 'b' && s == S::Code));
    }

    #[test]
    fn emphasis_maps_to_a_weight_not_a_colour() {
        assert_eq!(role_for(S::Code), Role::Code);
        assert_eq!(role_for(S::Text), Role::Body);
        // Bold and italic must resolve to distinct faces: rendering them as
        // plain text made `**bold**` invisible.
        assert_eq!(role_for(S::Bold), Role::BodyBold);
        assert_eq!(role_for(S::Italic), Role::BodyItalic);
        assert_ne!(role_for(S::Bold), role_for(S::Text));
        assert_ne!(role_for(S::Italic), role_for(S::Text));
        // Only a link earns the accent.
        assert_eq!(color_for(S::Link, FG), ACCENT);
        assert_eq!(color_for(S::Bold, FG), FG);
        assert_eq!(color_for(S::Italic, FG), FG);
        assert_eq!(color_for(S::Code, FG), FG);
    }

    #[test]
    fn the_heading_size_ramp_is_restrained_and_monotonic() {
        let base = BODY_PT;
        let sizes: Vec<u16> = (1..=6).map(|l| heading_size(l, base)).collect();
        for w in sizes.windows(2) {
            assert!(w[0] >= w[1], "heading sizes must not increase: {sizes:?}");
        }
        assert_eq!(sizes[0] - sizes[1], 4, "H1 to H2 should be a clear step");
        assert_eq!(sizes[4], base, "H5-H6 sit at body size and differ by weight");
        assert!(sizes[0] > base, "H1 must be larger than body");
    }

    #[test]
    fn the_reading_measure_is_capped_for_comfortable_line_length() {
        // The window exists to hold the column plus margins; the column
        // itself must never grow past a readable measure.
        assert!(MAX_MEASURE <= 560, "measure too wide: {MAX_MEASURE}px");
        assert!(MAX_MEASURE >= 380, "measure too narrow to be useful: {MAX_MEASURE}px");
        assert!(
            W as i32 - MARGIN as i32 * 2 <= MAX_MEASURE,
            "window is wider than the measure it is sized for"
        );
        assert!(MARGIN >= 20, "margins must be real margins, not a 10px gap");
    }

    #[test]
    fn edit_delete_and_insert() {
        let mut a = App::open("hello".into(), None, Mode::Edit);
        a.anchor = Some(Pos { l: 0, c: 0 });
        a.cur = Pos { l: 0, c: 5 };
        assert_eq!(a.selected(), "hello");
        a.del_sel();
        assert_eq!(a.text(), "");
        a.insert("hi there");
        assert_eq!(a.text(), "hi there");
        a.cur = Pos { l: 0, c: 2 };
        a.del_left();
        assert_eq!((a.text(), a.cur), ("h there".to_string(), Pos { l: 0, c: 1 }));
        a.del_right();
        assert_eq!((a.text(), a.cur), ("hthere".to_string(), Pos { l: 0, c: 1 }));
        assert!(a.dirty);
    }

    #[test]
    fn multiline_selection() {
        let mut a = App::open("ab\ncd\nef".into(), None, Mode::Edit);
        a.anchor = Some(Pos { l: 0, c: 1 });
        a.cur = Pos { l: 2, c: 1 };
        assert_eq!(a.selected(), "b\ncd\ne");
        a.del_sel();
        assert_eq!(a.text(), "af");
    }
}
