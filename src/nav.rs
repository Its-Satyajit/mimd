//! Navigation: scrolling, hit-testing and the key map.
//!
//! Everything that answers "where is the user pointing" or "which key did they
//! just press". It sits between the window and the document, so the window loop
//! does not have to know how a click becomes a character index.

#[cfg(test)]
#[path = "nav_tests.rs"]
mod tests;
use minifb::Key;

use crate::clip;
use crate::fonts::Role;
use crate::paint::Canvas;

use super::document::{App, EditKind, Mode, Pos};
use super::layout::char_index_at_x;
use super::theme::{Row, MARGIN};

/// Largest scroll offset that keeps the end of the document on screen.
///
/// `ys` holds each visual line's y in window coordinates, starting at the top
/// margin; `limit` is the y below which the painter stops drawing (the top of
/// the footer). Both are pixels, so the cap is the shift that brings the final
/// line back inside that band.
///
/// Counting rows instead left the tail of long documents unreachable: headings
/// are taller than body text and blocks carry gaps, so a line is not a fixed
/// number of pixels.
pub fn max_scroll_px(ys: &[i32], limit: i32) -> usize {
    if ys.len() < 2 {
        return 0;
    }
    // Scroll no further than needed to bring the last line into view.
    let need = ys.last().copied().unwrap_or(0) - limit;
    if need <= 0 {
        return 0;
    }
    // The smallest line whose shift reaches `need`.
    for (i, &y) in ys.iter().enumerate().take(ys.len() - 1) {
        if y - ys[0] >= need {
            return i;
        }
    }
    ys.len() - 1
}

/// Which view-mode key was pressed this frame, if any.
///
/// View mode used to test `is_key_down`, which is a *level* query: it is true
/// on every frame the key is held, so one press of Down advanced several lines
/// and Ctrl+E toggled into edit and straight back out again. Presses are
/// edge-triggered here instead: a key acts on the frame it goes down, and only
/// then. Holding a key does not repeat, which is what a reader wants — a held
/// arrow should not run the document away.
pub fn view_key_pressed(pressed: &[Key]) -> Option<Key> {
    // Order matters only for readability; the keys are disjoint.
    [
        Key::Down,
        Key::PageDown,
        Key::J,
        Key::Up,
        Key::PageUp,
        Key::K,
        Key::Home,
        Key::End,
    ]
    .into_iter()
    .find(|k| pressed.contains(k))
}

/// Swap between reading and editing.
///
/// Edge-triggered input only: this is called once per *press*, so a key held
/// down cannot toggle twice and leave the mode unchanged.
pub fn toggle_mode(app: &mut App) {
    app.mode = if app.mode == Mode::View {
        Mode::Edit
    } else {
        Mode::View
    };
}

/// Scroll offset after a mouse-wheel event.
///
/// `dy` is the raw value minifb reports. Both backends make it positive when
/// the wheel moves *up*: X11 maps Button4 to `+10`, and Wayland's
/// `wl_pointer.axis` vertical value is positive upward. Scrolling up therefore
/// has to move *back* through the document.
///
/// `scroll` is a line index, so a wheel notch moves a few lines rather than a
/// fraction of one — sub-line precision would need a pixel scroll offset, and
/// no reader needs it.
pub fn wheel_scroll(scroll: usize, dy: f32) -> usize {
    const LINES: usize = 3;
    if dy > 0.0 {
        scroll.saturating_sub(LINES)
    } else if dy < 0.0 {
        scroll + LINES
    } else {
        scroll
    }
}

/// Result of a mouse step, kept separate from windowing so it can be tested
/// without a display.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MouseAction {
    None,
    /// Place the caret, extending the selection if a drag is in progress.
    Caret(Pos),
}

/// Apply one mouse sample to the caret state.
///
/// `view` maps pixel rows to document lines. In edit mode a press starts a
/// selection anchor and motion extends it; release ends the drag. In view
/// mode the mouse deliberately does nothing — there is no caret to place.
pub fn mouse_step(
    app: &mut App,
    x: i32,
    y: i32,
    down: bool,
    row: Row,
    canvas: &mut Canvas,
) -> MouseAction {
    if app.mode != Mode::Edit {
        app.dragging = false;
        app.press = None;
        app.anchor = None;
        return MouseAction::None;
    }
    let inside = x >= row.x && y >= MARGIN as i32;
    if down && inside {
        let lh = canvas.line_height(Role::Body) as i32;
        let line_i = ((y - MARGIN as i32) / lh.max(1)).max(0) as usize;
        let l = (app.scroll + line_i).min(app.lines.len().saturating_sub(1));
        let c = col_at_x(canvas, &app.lines[l], x, row);
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
pub fn caret_x(c: &mut Canvas, line: &str, col: usize, row: Row) -> i32 {
    let chars: Vec<char> = line.chars().collect();
    let prefix: String = chars[..col.min(chars.len())].iter().collect();
    row.at(c.measure(&prefix, Role::Body))
}

/// Character index nearest to a click at pixel `x`.
pub fn col_at_x(c: &mut Canvas, line: &str, x: i32, row: Row) -> usize {
    char_index_at_x(c, line, row.local(x), Role::Body, c.body_size())
}

/// Run the command bound to a Ctrl chord.
///
/// Returns true if a command consumed the key. The keymap is matched as data
/// rather than an if/else ladder so that adding a command is adding a match
/// arm here, not another branch in the event loop.
pub fn run_command(app: &mut App, cmd: Key, shift: bool, body_lh: usize) {
    match cmd {
        Key::Q => {
            if app.dirty {
                app.save();
            }
            app.quit = true;
        }
        Key::Z => {
            let ok = if shift { app.redo() } else { app.undo() };
            app.status = if ok {
                if shift { "redo" } else { "undo" }.into()
            } else {
                format!("nothing to {}", if shift { "redo" } else { "undo" })
            };
        }
        Key::Y => {
            app.status = if app.redo() {
                "redo".into()
            } else {
                "nothing to redo".into()
            };
        }
        Key::O => {
            let seed = app
                .path
                .as_ref()
                .and_then(|p| p.to_str())
                .map(|s| s.to_string())
                .unwrap_or_default();
            app.prompt = Some(seed);
            app.status = String::new();
        }
        Key::E => {
            toggle_mode(app);
            app.status = String::new();
            app.scroll = 0;
        }
        Key::S => {
            app.save();
        }
        Key::C => {
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
        }
        Key::X => {
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
        }
        Key::V => {
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
        }
        Key::A => {
            let last = app.lines.len() - 1;
            app.anchor = Some(Pos::default());
            app.cur = Pos {
                l: last,
                c: app.lines[last].chars().count(),
            };
        }
        _ => {}
    }
}

/// Whether `pressed` contains a key that `run_command` handles.
pub fn command_key(pressed: &[Key]) -> Option<Key> {
    [
        Key::Q,
        Key::Z,
        Key::Y,
        Key::O,
        Key::E,
        Key::S,
        Key::C,
        Key::X,
        Key::V,
        Key::A,
    ]
    .into_iter()
    .find(|k| pressed.contains(k))
}
