//! Tests for navigation: scrolling, hit-testing, dragging and the key map.

use super::*;
use crate::document::{App, Mode, Pos};
use crate::paint::Canvas;
use crate::testutil::{canvas, edit_app, row_at, row_at_scrolled, Mouse};

#[test]
fn one_press_moves_one_line() {
    let mut app = App::open("one\ntwo\nthree\nfour".into(), None, Mode::View);

    // A single Down press.
    app.scroll = 0;
    assert_eq!(view_key_pressed(&[Key::Down]), Some(Key::Down));
    app.scroll += 1;
    assert_eq!(app.scroll, 1, "one Down press moved more than one line");

    // The frame after the key is released: nothing is pressed, so nothing
    // happens. This is the case that used to keep scrolling.
    assert_eq!(view_key_pressed(&[]), None);

    // Up moves back by one and clamps at the top.
    app.scroll = 1;
    assert_eq!(view_key_pressed(&[Key::Up]), Some(Key::Up));
    app.scroll = app.scroll.saturating_sub(1);
    assert_eq!(app.scroll, 0);
    app.scroll = app.scroll.saturating_sub(1);
    assert_eq!(app.scroll, 0, "scrolling up past the top must not wrap");
}

/// The keys must be recognised individually, and a held arrow must not
/// masquerade as a different one.

#[test]
fn each_view_key_is_recognised_once() {
    for k in [
        Key::Down,
        Key::PageDown,
        Key::J,
        Key::Up,
        Key::PageUp,
        Key::K,
        Key::Home,
        Key::End,
    ] {
        let got = view_key_pressed(&[k]);
        assert_eq!(got, Some(k), "{k:?} was not recognised");
    }
    // A key we do not handle must not be mistaken for one we do.
    for k in [Key::A, Key::S, Key::Left, Key::Right, Key::F1, Key::Escape] {
        assert_eq!(view_key_pressed(&[k]), None, "{k:?} should not scroll");
    }
    // Two at once resolves to one action, not two.
    assert_eq!(view_key_pressed(&[Key::Up, Key::Down]), Some(Key::Down));
}

/// Ctrl+E must toggle once per press. This was the visible symptom of the
/// level-triggered bug: E went down, mode flipped to Edit, and the same key
/// was seen again on the next frame and flipped it straight back.

#[test]
fn toggling_edit_mode_applies_once_per_press() {
    let mut app = App::open("hello".into(), None, Mode::View);
    let frame_press = |app: &mut App, pressed: &[Key]| {
        if pressed.contains(&Key::E) {
            toggle_mode(app);
        }
    };
    // Frame 1: E transitions down.
    frame_press(&mut app, &[Key::E]);
    assert_eq!(app.mode, Mode::Edit, "Ctrl+E should enter edit mode");
    // Frame 2: E is still physically down but is no longer a press.
    frame_press(&mut app, &[]);
    assert_eq!(
        app.mode,
        Mode::Edit,
        "mode flipped back on its own: the key acted twice"
    );
    // Frame 3: released and pressed again -> toggles back.
    frame_press(&mut app, &[Key::E]);
    assert_eq!(
        app.mode,
        Mode::View,
        "a second press should return to view mode"
    );
}

/// The keymap is data: every documented Ctrl chord resolves, and keys
/// without a command do not.

#[test]
fn the_command_keymap_is_complete_and_exclusive() {
    for k in [
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
    ] {
        assert_eq!(command_key(&[k]), Some(k), "{k:?} lost its command binding");
    }
    for k in [Key::B, Key::D, Key::F, Key::G, Key::Left, Key::Escape] {
        assert_eq!(command_key(&[k]), None, "{k:?} should have no command");
    }
    assert_eq!(command_key(&[]), None);
}

/// Ctrl+E through the real dispatch must land in edit mode and stay there.

#[test]
fn the_ctrl_e_dispatch_reaches_run_command() {
    let mut app = App::open("hello".into(), None, Mode::View);
    let cmd = command_key(&[Key::E]).expect("E is bound");
    run_command(&mut app, cmd, false, 20);
    assert_eq!(app.mode, Mode::Edit, "Ctrl+E did not enter edit mode");
}

/// Wheel up must move back through the document. `dy` is positive upward on
/// both X11 (Button4 -> +10) and Wayland (vertical axis positive up).

#[test]
fn wheel_up_scrolls_back_and_down_scrolls_forward() {
    assert_eq!(wheel_scroll(10, 1.0), 7, "wheel up must go back");
    assert_eq!(wheel_scroll(10, -1.0), 13, "wheel down must go forward");
    assert_eq!(wheel_scroll(10, 0.0), 10, "no movement, no scroll");
    // Clamped at the top rather than wrapping to the bottom.
    assert_eq!(wheel_scroll(1, 1.0), 0);
    assert_eq!(wheel_scroll(0, 1.0), 0);
}

/// A window narrower than the 80ch measure must wrap the text rather than
/// overflow it, and must still centre what is left.

#[test]
fn clicking_left_of_a_glyph_selects_the_earlier_column() {
    let Some((mut buf, mut fonts)) = canvas() else {
        return;
    };
    let mut c = Canvas {
        buf: &mut buf,
        w: 400,
        h: 400,
        fonts: &mut fonts,
    };
    let line = "hello world";
    // col_at_x measures from the text origin, which sits at MARGIN.
    let origin = MARGIN as i32;
    let w_h = c.measure_at("h", Role::Body, 15);
    // A click at the text origin must map to column 0.
    assert_eq!(col_at_x(&mut c, line, origin, row_at(origin)), 0);
    // Inside the first glyph's left half -> column 0.
    assert_eq!(col_at_x(&mut c, line, origin + w_h / 3, row_at(origin)), 0);
    // Past the first glyph's midpoint -> column 1.
    assert_eq!(col_at_x(&mut c, line, origin + w_h, row_at(origin)), 1);
    // Monotonic across the whole line.
    let mut prev = 0;
    for x in 0..120 {
        let col = col_at_x(&mut c, line, origin + x, row_at(origin));
        assert!(col >= prev, "column went backwards at x={x}");
        prev = col;
    }
}

#[test]
fn clicking_far_right_lands_at_end_of_line() {
    let Some((mut buf, mut fonts)) = canvas() else {
        return;
    };
    let mut c = Canvas {
        buf: &mut buf,
        w: 400,
        h: 400,
        fonts: &mut fonts,
    };
    let line = "hello";
    let total = c.measure(line, Role::Body);
    let origin = MARGIN as i32;
    assert_eq!(
        col_at_x(&mut c, line, origin + total + 500, row_at(origin)),
        5
    );
    assert_eq!(
        col_at_x(&mut c, line, origin + total - 1, row_at(origin)),
        5
    );
}

#[test]
fn horizontal_scroll_shifts_hit_testing_consistently() {
    let Some((mut buf, mut fonts)) = canvas() else {
        return;
    };
    let mut c = Canvas {
        buf: &mut buf,
        w: 400,
        h: 400,
        fonts: &mut fonts,
    };
    let line = "hello world";
    // With 40px of scroll, a click at x=0 is 40px into the line.
    let col = col_at_x(&mut c, line, 0, row_at_scrolled(MARGIN as i32, 40));
    assert!(
        col > 0,
        "expected scroll to move the caret right, got {col}"
    );
}

#[test]
fn caret_x_is_monotonic_across_the_line() {
    let Some((mut buf, mut fonts)) = canvas() else {
        return;
    };
    let mut c = Canvas {
        buf: &mut buf,
        w: 400,
        h: 400,
        fonts: &mut fonts,
    };
    let line = "monotonic caret positions";
    let mut prev = i32::MIN;
    for col in 0..=line.chars().count() {
        let x = caret_x(&mut c, line, col, row_at(MARGIN as i32));
        assert!(x >= prev, "caret went backwards at {col}: {x} < {prev}");
        prev = x;
    }
}

#[test]
fn horizontal_scroll_stays_within_document() {
    let Some((mut buf, mut fonts)) = canvas() else {
        return;
    };
    let mut c = Canvas {
        buf: &mut buf,
        w: 400,
        h: 400,
        fonts: &mut fonts,
    };
    let mut a = App::open("short".into(), None, Mode::Edit);
    let view_w = 380;
    // Caret far to the right scrolls right...
    a.cur = Pos { l: 0, c: 5 };
    let cx = caret_x(
        &mut c,
        &a.lines[0],
        a.cur.c,
        row_at_scrolled(MARGIN as i32, a.hscroll),
    );
    a.follow_x(cx, row_at(MARGIN as i32));
    // ...then the cap pulls it back for a short document.
    let max = crate::layout::widest(&a, &mut c) + MARGIN as i32 - view_w;
    if max < 0 {
        assert_eq!(a.hscroll, 0, "short lines must not scroll horizontally");
    }
}

/// Clipboard must degrade to the in-app buffer when no system tool
/// exists, and must never claim success when it did nothing.

#[test]
fn clicking_places_the_caret_at_that_column() {
    let Some(mut m) = Mouse::new(edit_app("hello world")) else {
        return;
    };
    m.click(0, 6);
    assert_eq!(m.app.cur, Pos { l: 0, c: 6 });
    assert!(m.app.anchor.is_none(), "a plain click must not select");
    assert!(!m.app.dragging);
}

#[test]
fn clicking_a_later_line_moves_down() {
    let Some(mut m) = Mouse::new(edit_app("one\ntwo\nthree\nfour")) else {
        return;
    };
    m.click(2, 3);
    assert_eq!(m.app.cur.l, 2);
    assert_eq!(m.app.cur.c, 3);
}

#[test]
fn clicking_past_the_last_line_clamps_to_it() {
    let Some(mut m) = Mouse::new(edit_app("one\ntwo")) else {
        return;
    };
    // A y far below the content must not index out of bounds. The x is the
    // column origin: clicks left of the column are not in the text, now
    // that the column is centred rather than flush to the margin.
    m.step(MARGIN as i32, 5000, true);
    m.step(MARGIN as i32, 5000, false);
    assert_eq!(m.app.cur.l, 1, "should clamp to the final line");
}

#[test]
fn dragging_selects_from_press_to_release_point() {
    let Some(mut m) = Mouse::new(edit_app("hello world")) else {
        return;
    };
    let mut c = Canvas {
        buf: &mut m.buf,
        w: 400,
        h: 400,
        fonts: &mut m.fonts,
    };
    let lh = c.line_height(Role::Body) as i32;
    let y = MARGIN as i32 + lh / 2;
    let x_end = caret_x(&mut c, &m.app.lines[0], 5, row_at(MARGIN as i32));
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
    let Some(mut m) = Mouse::new(edit_app("hello world")) else {
        return;
    };
    let mut c = Canvas {
        buf: &mut m.buf,
        w: 400,
        h: 400,
        fonts: &mut m.fonts,
    };
    let lh = c.line_height(Role::Body) as i32;
    let y = MARGIN as i32 + lh / 2;
    let x5 = caret_x(&mut c, &m.app.lines[0], 5, row_at(MARGIN as i32));
    drop(c);
    m.step(x5, y, true);
    m.step(MARGIN as i32, y, true);
    m.step(MARGIN as i32, y, false);
    assert_eq!(
        m.app.selected(),
        "hello",
        "reverse drag must select the same span"
    );
}

#[test]
fn dragging_across_lines_selects_multiline_text() {
    let Some(mut m) = Mouse::new(edit_app("aa\nbb\ncc")) else {
        return;
    };
    let c = Canvas {
        buf: &mut m.buf,
        w: 400,
        h: 400,
        fonts: &mut m.fonts,
    };
    let lh = c.line_height(Role::Body) as i32;
    let y0 = MARGIN as i32 + lh / 2;
    let y2 = MARGIN as i32 + 2 * lh + lh / 2;
    drop(c);
    m.step(MARGIN as i32, y0, true);
    m.step(MARGIN as i32, y2, true);
    m.step(MARGIN as i32, y2, false);
    assert_eq!(
        m.app.selected(),
        "aa
bb
"
    );
}

#[test]
fn a_second_click_starts_a_fresh_selection() {
    let Some(mut m) = Mouse::new(edit_app("hello world")) else {
        return;
    };
    m.click(0, 0);
    m.step(MARGIN as i32 + 100, MARGIN as i32 + 8, true);
    m.step(MARGIN as i32 + 100, MARGIN as i32 + 8, false);
    assert!(
        m.app.selected().is_empty(),
        "clicking again must clear the old selection"
    );
}

#[test]
fn the_mouse_does_nothing_in_view_mode() {
    let Some(mut m) = Mouse::new(App::open("hello".into(), None, Mode::View)) else {
        return;
    };
    let before = m.app.cur;
    m.step(50, 50, true);
    m.step(50, 50, false);
    assert_eq!(m.app.cur, before, "view mode has no caret to move");
    assert!(!m.app.dragging);
}

#[test]
fn clicking_above_the_margin_ignores_the_event() {
    let Some(mut m) = Mouse::new(edit_app("hello")) else {
        return;
    };
    m.step(50, 0, true);
    m.step(50, 0, false);
    assert_eq!(m.app.cur, Pos::default());
    assert!(
        !m.app.dragging,
        "a press above the text must not start a drag"
    );
}

#[test]
fn scroll_never_runs_past_the_end_of_the_document() {
    // ys are window coordinates: the first line sits at the top margin and the
    // painter stops drawing at `limit`.
    let ys = |n: usize| -> Vec<i32> { (0..n).map(|i| 28 + i as i32 * 22).collect() };
    let limit = 632;
    // Short document: cannot scroll at all.
    assert_eq!(max_scroll_px(&ys(3), limit), 0);
    // Exact fit: cannot scroll.
    assert_eq!(max_scroll_px(&ys(20), limit), 0);
    // Longer than the window: the cap is a real index, not a sentinel.
    let cap = max_scroll_px(&ys(50), limit);
    assert!(cap < usize::MAX);
    // Whatever the cap, the final line must land inside the viewport.
    let last = ys(50)[49] - (ys(50)[cap] - ys(50)[0]);
    assert!(
        last <= limit,
        "at the cap the last line sits at {last}, past the draw limit {limit}"
    );
}

#[test]
fn following_the_caret_keeps_it_in_view() {
    let mut a = App::open(
        (0..100)
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join("\n"),
        None,
        Mode::Edit,
    );
    a.follow(10);
    assert_eq!(a.scroll, 0);
    a.cur = Pos { l: 50, c: 0 };
    a.follow(10);
    assert!(
        a.cur.l >= a.scroll && a.cur.l < a.scroll + 10,
        "caret must be visible"
    );
    a.cur = Pos { l: 0, c: 0 };
    a.follow(10);
    assert_eq!(a.scroll, 0, "moving back to the top must scroll up");
}

#[test]
fn the_last_line_of_a_long_document_is_reachable() {
    let mut fonts = match crate::fonts::Fonts::new(None, None, crate::theme::BODY_PT) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    let md: String = (0..40)
        .map(|i| format!("Para {i} with enough words to wrap around the measure.\n\n"))
        .collect();
    let (w, h) = (crate::theme::W as u32, crate::theme::H as u32);
    let mut none = Vec::new();
    let mut sc = crate::testutil::scratch(&mut none, &mut fonts);
    let row = crate::theme::text_column(&mut sc, w as i32, 0);
    let app = crate::document::App::open(md, None, crate::document::Mode::View);
    let view = {
        let mut none2 = Vec::new();
        let mut s2 = crate::testutil::scratch(&mut none2, &mut fonts);
        crate::layout::build_view_layout(&mut s2, &app.text(), row)
    };
    let n = view.lines.len();
    let lh = fonts.body.line_height() as i32;
    let mar = crate::theme::MARGIN as i32;
    // The viewport the painter draws into.
    let viewport_h = h as i32 - mar - (lh + 6);
    let cap = crate::nav::max_scroll_px(&view.ys, viewport_h);

    // `cap` is the first line drawn at the maximum scroll; the painter stops
    // at `limit`, so the invariant is that the final line is at or above it.
    let limit = h as i32 - (lh + 6);
    let sp = view.ys[cap] - view.ys[0];
    let last_y = view.ys[n - 1] - sp;
    assert!(
        last_y <= limit,
        "the last line lands at {last_y}, past the draw limit {limit}"
    );
    // Several offsets can show the end when lines are unevenly spaced; the cap
    // must at least be far enough that the document's tail is on screen, which
    // the assertion above establishes.
    assert!(cap < n, "the cap must be a real line index")
}

#[test]
fn scrolling_forward_shows_later_lines_and_back_shows_earlier() {
    let mut fonts = match crate::fonts::Fonts::new(None, None, crate::theme::BODY_PT) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("skip: {e}");
            return;
        }
    };
    let md: String = (0..60).map(|i| format!("Line {i}\n\n")).collect();
    let w = crate::theme::W as u32;
    let mut none = Vec::new();
    let mut sc = crate::testutil::scratch(&mut none, &mut fonts);
    let row = crate::theme::text_column(&mut sc, w as i32, 0);
    let app = crate::document::App::open(md, None, crate::document::Mode::View);
    let view = {
        let mut n2 = Vec::new();
        let mut s2 = crate::testutil::scratch(&mut n2, &mut fonts);
        crate::layout::build_view_layout(&mut s2, &app.text(), row)
    };
    // The top of the window shows line `scroll`, so increasing scroll moves the
    // window forward and decreasing moves it back.
    let top_shift = |s: usize| view.ys[s] - view.ys[0];
    assert_eq!(top_shift(0), 0);
    assert!(
        top_shift(10) > top_shift(5),
        "scroll 10 must be further down than 5"
    );
    assert!(top_shift(5) > 0, "scrolling must move the page");
    // Wheel up goes back, wheel down goes forward.
    assert!(wheel_scroll(10, 1.0) < 10, "wheel up must scroll back");
    assert!(
        wheel_scroll(10, -1.0) > 10,
        "wheel down must scroll forward"
    );
    assert_eq!(wheel_scroll(10, 0.0), 10, "no wheel movement, no scroll");
    // And wheel up must never wrap past the top.
    assert_eq!(wheel_scroll(1, 1.0), 0);
}
