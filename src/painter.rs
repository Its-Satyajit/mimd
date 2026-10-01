//! Painting a frame: choosing what to show and putting it on the canvas.
//!
//! `render` is the whole painter — clear, dispatch on mode, draw the footer.
//! The two modes share a canvas but not a layout, so each gets its own
//! painter rather than one function with a mode branch through it.

use crate::fonts::Role;
use crate::markdown::S;
use crate::paint::Canvas;

use super::document::{App, Mode};
use super::layout::ViewLayout;
use super::theme::{Row, ACCENT, BG, CURSOR, DIM, FG, MARGIN, PANEL, RULE, SEL};

pub fn render(app: &App, c: &mut Canvas, view: &ViewLayout) {
    let lh = c.line_height(Role::Body);
    let status_h = lh + 6;
    let rows_fit = (c.h.saturating_sub(status_h + MARGIN * 2) / lh).max(1) as usize;
    c.clear(BG);
    match app.mode {
        Mode::Edit => render_edit(app, c, view.row, rows_fit),
        Mode::View => render_view(app, c, view),
    }
    render_status(app, c);
}

/// Edit mode: raw lines, a selection wash and a caret.
fn render_edit(app: &App, c: &mut Canvas, row: Row, rows_fit: usize) {
    let lh = c.line_height(Role::Body);
    let top = MARGIN as i32;
    let lh_i = lh as i32;
    let (s, e) = app.sel();
    if s != e {
        for l in s.l..=e.l {
            let c0 = if l == s.l { s.c } else { 0 };
            let c1 = if l == e.l { e.c } else { usize::MAX };
            let chars: Vec<char> = app.lines[l].chars().collect();
            let n = chars.len();
            let prefix: String = chars[..c0.min(n)].iter().collect();
            let upto: String = chars[..c1.min(n)].iter().collect();
            let x0 = row.at(c.measure(&prefix, Role::Body));
            let x1 = row.at(c.measure(&upto, Role::Body));
            let y = top + (l as i32 - app.scroll as i32) * lh_i;
            c.fill(x0, y, (x1 - x0) as u32, lh, SEL);
        }
    }
    for l in app.scroll..app.lines.len().min(app.scroll + rows_fit) {
        let y = top + (l as i32 - app.scroll as i32) * lh_i;
        c.draw_str(
            row.at(0),
            y + c.ascent(Role::Body),
            &app.lines[l],
            Role::Body,
            FG,
        );
    }
    let (cl, cc) = if s == e {
        (app.cur.l, app.cur.c)
    } else {
        (s.l, s.c)
    };
    if cl >= app.scroll && cl - app.scroll < rows_fit {
        let chars: Vec<char> = app.lines[cl].chars().collect();
        let prefix: String = chars[..cc.min(chars.len())].iter().collect();
        let x = row.at(c.measure(&prefix, Role::Body));
        let y = top + (cl as i32 - app.scroll as i32) * lh_i;
        c.fill(x, y, 2, lh, CURSOR);
    }
}

/// View mode: the flowed document, scrolled.
fn render_view(app: &App, c: &mut Canvas, view: &ViewLayout) {
    let lh = c.line_height(Role::Body);
    let top = MARGIN as i32;
    let limit = c.h as i32 - (lh + 6) as i32;
    // Skip lines above the scroll offset, then draw the rest shifted up by
    // it. Without this the layout is drawn from the top and scrolling does
    // nothing at all.
    // `scroll` counts lines but `ys` holds pixels, so the pixel shift is
    // ys[scroll] - MARGIN: the distance from the first line's origin to the
    // line we are scrolling to. Subtracting the line index itself would
    // scroll by a few pixels rather than by pages.
    let scroll_px = view
        .ys
        .get(app.scroll)
        .map_or(0, |&y| y - view.ys.first().copied().unwrap_or(y));
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
            c.fill(view.row.x, ry, w.max(8) as u32, 1, DIM);
            continue;
        }
        // A blockquote's rail sits in the margin, aligned to the text.
        if vl.quote {
            c.fill(
                view.row.x + vl.indent - 12,
                y + 2,
                2,
                (lh - 4).max(2) as u32,
                DIM,
            );
        }
        let mut x = view.row.x + vl.indent;
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

/// The footer: mode and filename on the left, one contextual hint on the
/// right.
///
/// A pager's footer. A permanent list of every shortcut is noise — it is in the
/// README, and it crowds out the state that is actually telling you something.
fn render_status(app: &App, c: &mut Canvas) {
    let status_h = c.line_height(Role::Body) + 6;
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
            let mw = c.draw_str(
                x0,
                base_y,
                mode,
                Role::BodyBold,
                if editing { ACCENT } else { DIM },
            );
            let mut lx = x0 + mw + 14;
            if let Some(n) = &name {
                lx += c.draw_str(lx, base_y, n, Role::Body, FG) + 12;
            }
            if app.dirty {
                c.draw_str(lx, base_y, "unsaved", Role::BodyItalic, DIM);
            }
            // Transient status, centred, wins over the hint while it lasts.
            let (right_text, right_w) = if app.status.is_empty() {
                (
                    if editing {
                        "Ctrl+S to save"
                    } else {
                        "Ctrl+E to edit"
                    }
                    .to_string(),
                    0,
                )
            } else {
                let t = app.status.clone();
                let w = c.measure(&t, Role::Body);
                (t, w)
            };
            let rw = if app.status.is_empty() {
                c.measure(&right_text, Role::Body)
            } else {
                right_w
            };
            let rx = c.w as i32 - MARGIN as i32 - rw;
            c.draw_str(rx, base_y, &right_text, Role::Body, DIM);
        }
    }
}
