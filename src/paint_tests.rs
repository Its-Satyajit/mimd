//! Tests for painting: real glyphs, the design tokens and a full document.

use super::*;
use crate::document::{App, Mode};
use crate::fonts::Fonts;
use crate::layout::{build_view_layout, layout_view};
use crate::paint::Canvas;
use crate::painter::render;
use crate::testutil::{scratch, write_png};
use crate::theme::text_column;
use crate::theme::{
    color_for, heading_size, role_for, Row, ACCENT, BG, BODY_PT, DIM, FG, H, HEAD, MARGIN,
    MAX_MEASURE, PANEL, RULE, W,
};

#[test]
fn renders_system_font_glyphs_into_pixels() {
    let mut fonts = match Fonts::new(None, None, 20) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    eprintln!(
        "body font = {:?}, code font = {:?}",
        fonts.body.family, fonts.code.family
    );

    let mut buf = vec![BG; 300 * 60];
    {
        let mut c = Canvas {
            buf: &mut buf,
            w: 300,
            h: 60,
            fonts: &mut fonts,
        };
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

/// Regression: view mode must actually scroll. Drawing the layout from
/// index 0 regardless of `scroll` left the top of the document on screen
/// no matter how far the user scrolled.

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
        let mut scratch = Canvas {
            buf: &mut Vec::new(),
            w: 0,
            h: 0,
            fonts: &mut fonts,
        };
        let row = text_column(&mut scratch, w as i32, 0);
        let view = build_view_layout(&mut scratch, &app.text(), row);
        let mut canvas = Canvas {
            buf: &mut buf,
            w,
            h,
            fonts: &mut fonts,
        };
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
                buf[y as usize * w as usize + x as usize],
                BG,
                "text or rule escaped the measure at {x},{y}"
            );
        }
    }
    // Measure the extent of the text: the leftmost and rightmost inked
    // pixel, and the widest unbroken run of them. The column must be
    // centred, so the gaps either side must match.
    let mut leftmost = w;
    let mut rightmost = 0i32;
    let mut widest = 0u32;
    for y in 0..(h - 40) {
        let mut run = 0;
        let mut best = 0;
        for x in 0..w {
            if buf[y as usize * w as usize + x as usize] != BG {
                leftmost = leftmost.min(x);
                rightmost = rightmost.max(x as i32);
                run += 1;
                best = best.max(run);
            } else {
                run = 0;
            }
        }
        widest = widest.max(best);
    }
    let mut none = Vec::new();
    let measure_px = text_column(&mut scratch(&mut none, &mut fonts), w as i32, 0).w;
    eprintln!(
        "text spans x={leftmost}..={rightmost} ({widest}px widest run) in a {w}px window, \
     measure {measure_px}px"
    );
    // Nothing may spill outside the window.
    assert!(
        rightmost < w as i32,
        "text ran to x={rightmost} in a {w}px window"
    );
    // The column is centred: ink starts at the column and stops at its end,
    // leaving equal margins.
    let left_gap = leftmost as i32 - (w as i32 - measure_px - leftmost as i32);
    assert!(
        left_gap.abs() <= 2,
        "column not centred: ink starts at {leftmost}, right edge {rightmost}, \
     expected symmetric about {}",
        w / 2
    );
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
    for ch in "Handgloves".chars() {
        bw += fonts.measure_at(Role::BodyBold, BODY_PT, &ch.to_string());
        rw += fonts.measure_at(Role::Body, BODY_PT, &ch.to_string());
    }
    assert!(
        bw > rw,
        "bold face is not wider than regular ({bw} vs {rw})"
    );
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
        let mut c = Canvas {
            buf: &mut buf,
            w,
            h,
            fonts: &mut fonts,
        };
        let vls = layout_view(&mut c, md, w as i32 - 20);
        lines = vls.len();
        assert!(
            lines >= 9,
            "expected wrapping to produce lines, got {lines}"
        );
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
        let vl = build_view_layout(
            &mut c,
            md,
            Row {
                x: 0,
                w: w as i32 - 20,
                hscroll: 0,
            },
        );
        render(&app, &mut c, &vl);
    }
    let ink = buf.iter().filter(|p| **p != BG).count();
    assert!(ink > 500, "expected substantial ink, got {ink}");
    eprintln!("visual lines = {lines}, ink px = {ink}");

    for y in (0..h as usize).step_by(3) {
        let row: String = (0..w as usize)
            .step_by(2)
            .map(|x| {
                if buf[y * w as usize + x] != BG {
                    '#'
                } else {
                    '.'
                }
            })
            .collect();
        println!("{row}");
    }
}

#[test]
fn the_heading_size_ramp_is_restrained_and_monotonic() {
    let base = BODY_PT;
    let sizes: Vec<u16> = (1..=6).map(|l| heading_size(l, base)).collect();
    for w in sizes.windows(2) {
        assert!(w[0] >= w[1], "heading sizes must not increase: {sizes:?}");
    }
    assert_eq!(sizes[0] - sizes[1], 4, "H1 to H2 should be a clear step");
    assert_eq!(
        sizes[4], base,
        "H5-H6 sit at body size and differ by weight"
    );
    assert!(sizes[0] > base, "H1 must be larger than body");
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
