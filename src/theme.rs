//! The palette and the page geometry.
//!
//! Every colour and every measurement of the page lives here, so the visual
//! design can be read in one place — and changed in one place.

use crate::fonts::Role;
use crate::markdown::S;
use crate::paint::Canvas;

// ---------- colours (0RGB) ----------
//
// mimd reads prose, so the ground is paper rather than a code editor's void: a
// cool light grey that stays neutral beside black ink. Exactly one accent
// carries meaning — interactive things — and structure stays recessive.
pub const BG: u32 = 0xe9eaec; // page
pub const PANEL: u32 = 0xdcdee2; // status bar, one step down the page
pub const RULE: u32 = 0xb4b9bf; // the hairline separating bar from page
pub const FG: u32 = 0x1a1c1e; // body ink
pub const DIM: u32 = 0x5e6368; // rules, quotes, status text
pub const ACCENT: u32 = 0x2f5d8a; // links and the caret: the only accent
pub const HEAD: u32 = 0x000000; // headings carry full ink
pub const SEL: u32 = 0xc3d2e0; // selection, a wash of the accent
pub const CURSOR: u32 = 0x2f5d8a; // caret

// ---------- page geometry ----------

/// Window width. Sized to hold the 80ch column plus margins rather than to the
/// display, so a document reader does not open a huge window to show a narrow
/// page. `text_column` measures the real advance; this is only the default,
/// and widening the window adds margin rather than line length.
pub const W: usize = 800;
pub const H: usize = 660;
/// Page margin. Deliberately generous: the text column is capped below, and
/// this is the space kept on every side when the window has room to spare.
pub const MARGIN: u32 = 28;

/// The reading column, in characters. Past ~80 the eye loses its return
/// sweep, which is the whole point of a document reader. This is a count of
/// characters rather than a pixel constant so it survives a change of body
/// size or typeface.
pub const MEASURE_CH: usize = 80;
/// Narrowest column we will lay out, in pixels. Below this, wrapping produces
/// unreadably short lines and the window is simply too small.
pub const MIN_MEASURE: i32 = 120;
/// Hard ceiling on the column, so a very wide window cannot produce a
/// comically long measure if the font is unusually wide.
pub const MAX_MEASURE: i32 = 900;
/// Body type size, in points. 15pt keeps a line of body text near 20px, which
/// is comfortable to read for a long sitting.
pub const BODY_PT: u16 = 15;

/// Horizontal placement of the text column.
///
/// `x` is where the column's left edge falls and `w` how wide it is; `hscroll`
/// is how far the content has been scrolled sideways. Layout, drawing and
/// hit-testing all resolve screen x through this one value, so they cannot
/// disagree about where the text is — which is the whole reason it is a type
/// rather than a bare integer passed alongside MARGIN.
#[derive(Clone, Copy)]
pub struct Row {
    /// Left edge of the column, in pixels from the left of the window.
    pub x: i32,
    /// Width of the column, in pixels.
    pub w: i32,
    /// Horizontal scroll offset, in pixels.
    pub hscroll: u32,
}

impl Row {
    /// Screen x of a point `px` pixels into the text, scroll applied.
    pub fn at(&self, px: i32) -> i32 {
        self.x - self.hscroll as i32 + px
    }

    /// Inverse of [`Row::at`]: line-relative pixels for a screen `x`.
    pub fn local(&self, screen_x: i32) -> i32 {
        screen_x + self.hscroll as i32 - self.x
    }
}

/// The reading column for a window `window_w` pixels wide: at most
/// MEASURE_CH characters, never wider than the window, always centred.
pub fn text_column(c: &mut Canvas, window_w: i32, hscroll: u32) -> Row {
    // Measure one character in the body face and multiply, rather than
    // assuming a pixel width: the ch then tracks the real font metrics.
    let ch = (c.measure("0", Role::Body)).max(1);
    let target = (ch * MEASURE_CH as i32).min(MAX_MEASURE);
    // A narrow window wins over the target measure: text wraps instead of
    // running off the edge.
    let avail = (window_w - MARGIN as i32 * 2).max(1);
    let w = target.min(avail).max(MIN_MEASURE.min(avail));
    Row {
        x: MARGIN as i32 + (avail - w) / 2,
        w,
        hscroll,
    }
}

/// Colour for a span. Heading colour is applied per line by the renderer,
/// since heading-ness is a block property, not a span property.
pub fn color_for(style: S, base: u32) -> u32 {
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
pub fn role_for(style: S) -> Role {
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
pub fn heading_size(level: u8, base: u16) -> u16 {
    match level {
        1 => base + 8,
        2 => base + 4,
        3 => base + 1,
        _ => base,
    }
}
