//! Laying the document out into positioned lines.
//!
//! Markdown blocks in, visual lines out: wrapped at the measure, with heading
//! sizes and block spacing resolved. Layout knows about type and columns; it
//! does not draw, and it does not know what mode the app is in.

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
use crate::fonts::Role;
use crate::markdown::{parse, Block, Run, S};
use crate::paint::Canvas;

use super::document::App;

use super::theme::{heading_size, role_for, Row, DIM, FG, HEAD, MARGIN};

/// One laid-out display line ready to draw.
pub struct VisualLine {
    pub cells: Run,
    /// Left indent in pixels.
    pub indent: i32,
    /// Point size to render at.
    pub size: u16,
    /// Base text colour for this line (headings use a brighter one).
    pub color: u32,
    /// Blank lines to leave after this one.
    pub gap: u32,
    /// Draw a hairline of this width instead of glyphs (horizontal rule).
    pub rule: Option<i32>,
    /// Draw a vertical rail in the left margin (blockquote).
    pub quote: bool,
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
        return vec![VisualLine {
            cells,
            indent,
            size,
            color,
            gap: first_gap,
            ..Default::default()
        }];
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
        out.push(VisualLine {
            cells: line,
            indent,
            size,
            color,
            gap: g,
            ..Default::default()
        });
    }
    if out.is_empty() {
        out.push(VisualLine {
            cells,
            indent,
            size,
            color,
            gap: first_gap,
            ..Default::default()
        });
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
pub fn layout_view(c: &mut Canvas, text: &str, width: i32) -> Vec<VisualLine> {
    let base = c.body_size();
    let mut out: Vec<VisualLine> = Vec::new();
    for block in parse(text) {
        let lines = match block {
            Block::Heading(level, run) => {
                let size = heading_size(level, base);
                // Headings are bold: hierarchy is weight first, size second.
                let run: Run = run
                    .into_iter()
                    .map(|(ch, st)| (ch, st.emphasised()))
                    .collect();
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
                rule: Some(width),
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
pub struct ViewLayout {
    pub lines: Vec<VisualLine>,
    /// Pixel y of each visual line.
    #[allow(dead_code)]
    pub ys: Vec<i32>,
    /// Where the column sits and how wide it is, plus horizontal scroll.
    pub row: Row,
}

/// Build the view layout plus its pixel positions.
pub fn build_view_layout(c: &mut Canvas, text: &str, row: Row) -> ViewLayout {
    let lines = layout_view(c, text, row.w);
    let mut ys = Vec::with_capacity(lines.len());
    let mut y = MARGIN as i32;
    for vl in &lines {
        ys.push(y);
        y += c.line_height_for(vl.size) as i32;
        for _ in 0..vl.gap {
            y += c.line_height_for(vl.size) as i32 / 2;
        }
    }
    ViewLayout { lines, ys, row }
}

/// Character index in `text` nearest to pixel `x`, using real font advances.
pub fn char_index_at_x(c: &mut Canvas, text: &str, x: i32, role: Role, size: u16) -> usize {
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

/// Widest line in the document, in pixels, for the horizontal scroll bound.
///
/// A measurement rather than state, which is why it lives with the other
/// measurements instead of on `App`.
pub fn widest(app: &App, c: &mut Canvas) -> i32 {
    app.lines
        .iter()
        .map(|l| c.measure(l, Role::Body))
        .max()
        .unwrap_or(0)
}
