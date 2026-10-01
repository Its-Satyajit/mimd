//! Tests for layout: wrapping, the reading measure and line positions.

use super::*;
use crate::document::{App, Mode};
use crate::fonts::Fonts;
use crate::nav::max_scroll_px;
use crate::paint::Canvas;
use crate::painter::render;
use crate::testutil::{canvas, scratch};
use crate::theme::{text_column, Row, BG, BODY_PT, H, MAX_MEASURE, MEASURE_CH, W};

#[test]
fn scrolling_in_view_mode_actually_moves_the_page() {
    let mut fonts = match Fonts::new(None, None, BODY_PT) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    let md: String = (0..80)
        .map(|i| format!("Paragraph number {i} with enough text to fill a line.\n\n"))
        .collect();
    let (w, h) = (W as u32, H as u32);
    let mut none = Vec::new();
    let row = text_column(&mut scratch(&mut none, &mut fonts), w as i32, 0);

    let k = 12usize;
    // One helper, several roles: a frame at a given scroll, and the layout
    // it came from. Separate closures would each hold `fonts` mutably.
    let mut shot = |src: &str, scroll: usize| -> (Vec<u32>, ViewLayout) {
        let mut buf = vec![BG; (w * h) as usize];
        let mut app = App::open(src.to_string(), None, Mode::View);
        app.scroll = scroll;
        let view = {
            let mut scratch = Canvas {
                buf: &mut Vec::new(),
                w: 0,
                h: 0,
                fonts: &mut fonts,
            };
            build_view_layout(&mut scratch, &app.text(), row)
        };
        {
            let mut canvas = Canvas {
                buf: &mut buf,
                w,
                h,
                fonts: &mut fonts,
            };
            render(&app, &mut canvas, &view);
        }
        (buf, view)
    };

    let (top, _) = shot(&md, 0);
    let (down, probe) = shot(&md, k);
    assert_ne!(
        top, down,
        "scrolling changed nothing: view mode is not scrolling"
    );

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
    let short = [28i32, 50, 72];
    assert_eq!(
        max_scroll_px(&short, 600),
        0,
        "short documents must not scroll"
    );

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

/// Wayland must never receive an icon: minifb panics there, and mimd would
/// die on launch for every Wayland user.

#[test]
fn a_narrow_window_wraps_instead_of_overflowing() {
    let mut fonts = match Fonts::new(None, None, BODY_PT) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    let md = "This is a long paragraph that must wrap onto several lines when the \
          window is too narrow to hold eighty characters of measure at once."
        .to_string();
    let bh = 400u32;
    for bw in [200u32, 300, 420, 560, 800] {
        let mut buf = vec![BG; (bw * bh) as usize];
        let (x, width) = {
            let mut none = Vec::new();
            let row = text_column(&mut scratch(&mut none, &mut fonts), bw as i32, 0);
            let (x, width) = (row.x, row.w);
            let app = App::open(md.clone(), None, Mode::View);
            let mut none = Vec::new();
            let view = build_view_layout(&mut scratch(&mut none, &mut fonts), &app.text(), row);
            let mut canvas = Canvas {
                buf: &mut buf,
                w: bw,
                h: bh,
                fonts: &mut fonts,
            };
            render(&app, &mut canvas, &view);
            (x, width)
        };
        let right = (x + width) as i32;
        assert!(
            x >= 0 && right <= bw as i32,
            "{bw}px: column [{x},{right}) does not fit"
        );
        // No ink may fall outside the column, at any window width. The
        // footer is chrome and legitimately spans the window, so the
        // scan stops above it.
        for y in 0..(bh - 40) {
            for px in 0..bw {
                let inked = buf[y as usize * bw as usize + px as usize] != BG;
                let px = px as i32;
                if inked && !(px >= x && px < right) {
                    panic!("{bw}px window: ink at x={px} outside column [{x},{right})");
                }
            }
        }
    }
}

/// A resized window must re-layout and fill the whole buffer: the buffer is
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
    for (bw, bh) in [
        (W as u32, H as u32),
        (400, 300),
        (1200, 900),
        (200, 120),
        (320, 700),
    ] {
        let buf_len = (bw * bh) as usize;
        let mut buf = vec![0xdead_beefu32; buf_len];
        let mut none = Vec::new();
        let row = text_column(&mut scratch(&mut none, &mut fonts), bw as i32, 0);
        let app = App::open(md.clone(), None, Mode::View);
        {
            let mut none = Vec::new();
            let view = build_view_layout(&mut scratch(&mut none, &mut fonts), &app.text(), row);
            let mut canvas = Canvas {
                buf: &mut buf,
                w: bw,
                h: bh,
                fonts: &mut fonts,
            };
            render(&app, &mut canvas, &view);
        }
        // Nothing outside the visible window area may be left unwritten.
        let body = (bh as usize).saturating_sub(40) * bw as usize; // exclude the footer band
        let untouched = buf[..body.min(buf_len)]
            .iter()
            .filter(|&&v| v == 0xdead_beef)
            .count();
        assert_eq!(
            untouched, 0,
            "{bw}x{bh}: {untouched} pixels were never written"
        );
        // The last row must be reachable, i.e. no off-by-one in the fill.
        let last_row_start = ((bh as usize) - 1) * bw as usize;
        assert!(
            buf[last_row_start..last_row_start + bw as usize]
                .iter()
                .all(|&v| v != 0xdead_beef),
            "{bw}x{bh}: the final row was not written"
        );
    }
}

/// Render the real window at the real size and report what landed in it,
/// so the design can be inspected without a display.

#[test]
fn view_layout_records_a_pixel_y_per_line() {
    let Some((mut buf, mut fonts)) = canvas() else {
        return;
    };
    let mut c = Canvas {
        buf: &mut buf,
        w: 400,
        h: 400,
        fonts: &mut fonts,
    };
    let md = "# One\n\npara one\n\npara two\n\n## Two\n";
    let v = build_view_layout(
        &mut c,
        md,
        Row {
            x: 0,
            w: 380,
            hscroll: 0,
        },
    );
    // # One, para one, para two, ## Two
    assert_eq!(v.lines.len(), 4, "expected one visual line per block");
    assert_eq!(v.ys.len(), v.lines.len());
    // The first line starts at the top margin.
    assert_eq!(v.ys[0], MARGIN as i32);
}

#[test]
fn view_layout_positions_increase_monotonically() {
    let Some((mut buf, mut fonts)) = canvas() else {
        return;
    };
    let mut c = Canvas {
        buf: &mut buf,
        w: 400,
        h: 400,
        fonts: &mut fonts,
    };
    let md = "# H\n\n- a\n- b\n\n> q\n\n```\ncode\n```\n";
    let v = build_view_layout(
        &mut c,
        md,
        Row {
            x: 0,
            w: 380,
            hscroll: 0,
        },
    );
    for w in v.ys.windows(2) {
        assert!(w[1] > w[0], "ys not increasing: {:?}", w);
    }
}

#[test]
fn the_reading_measure_is_capped_for_comfortable_line_length() {
    assert_eq!(MEASURE_CH, 80, "the column is specified as 80 characters");
    assert!(MARGIN >= 20, "margins must be real margins, not a 10px gap");
    // The ceiling must be generous enough that 80ch is what binds, not the
    // constant: otherwise "80ch" would be a lie on a narrow font.
    assert!(
        MAX_MEASURE > 380,
        "the 80ch cap must bind before the pixel ceiling"
    );
}

/// The column is 80 characters, centred in the window, and never wider
/// than the window itself.

#[test]
fn the_column_is_eighty_characters_and_centred() {
    let mut fonts = match Fonts::new(None, None, BODY_PT) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    let mut buf = Vec::new();
    let mut c = Canvas {
        buf: &mut buf,
        w: 0,
        h: 0,
        fonts: &mut fonts,
    };
    let ch = c.measure("0", Role::Body);

    // At the default window width the 80ch target is what applies.
    let row = text_column(&mut c, W as i32, 0);
    let (x, width) = (row.x, row.w);
    assert_eq!(width, ch * MEASURE_CH as i32, "column must be exactly 80ch");

    // Centred: equal space on both sides.
    let left = x - MARGIN as i32;
    let right = W as i32 - (x + width) - MARGIN as i32;
    assert_eq!(
        left, right,
        "column is not centred: {left} left, {right} right"
    );
    assert!(left >= 0, "column overflows the window on the left");

    // Any window: centred, inside the window, and capped at 80ch.
    for w in [200i32, 320, 480, 700, 900, 1400, 3000] {
        let row = text_column(&mut c, w, 0);
        let (x, width) = (row.x, row.w);
        assert!(
            width <= ch * MEASURE_CH as i32,
            "{w}px window: {width}px is over 80ch"
        );
        assert!(x >= 0, "{w}px window: column starts off-screen at {x}");
        assert!(
            x + width <= w,
            "{w}px window: column overflows to {}\u{200b}",
            x + width
        );
        let left = x;
        let right = w - (x + width);
        assert_eq!(left, right, "{w}px window: not centred ({left} vs {right})");
    }

    // A window narrower than the measure must wrap, not overflow.
    let narrow = 200i32;
    let width = text_column(&mut c, narrow, 0).w;
    assert!(width < ch * MEASURE_CH as i32, "narrow window should wrap");
    assert!(width <= narrow, "wrapped column must fit the window");
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
