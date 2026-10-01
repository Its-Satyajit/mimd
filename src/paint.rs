//! Drawing: a framebuffer, the font set, and the primitives that put glyphs
//! into pixels.
//!
//! Nothing here knows what a document is. `Canvas` is a surface — blend a
//! pixel, draw a run, fill a rect — and the layout and painter modules decide
//! what to ask it to draw.

#[cfg(test)]
#[path = "paint_tests.rs"]
mod tests;
use crate::fonts::{Fonts, Role};
use crate::markdown::S;

use super::theme::{color_for, role_for};

/// A canvas of 0RGB pixels plus the font set it draws with.
pub struct Canvas<'a> {
    pub buf: &'a mut [u32],
    pub w: u32,
    pub h: u32,
    pub fonts: &'a mut Fonts,
}

impl Canvas<'_> {
    pub fn body_size(&self) -> u16 {
        self.fonts.body.size
    }
}

impl<'a> Canvas<'a> {
    pub fn px(&mut self, x: i32, y: i32, c: u32) {
        if x >= 0 && y >= 0 && (x as u32) < self.w && (y as u32) < self.h {
            self.buf[y as usize * self.w as usize + x as usize] = c;
        }
    }

    pub fn fill(&mut self, x: i32, y: i32, w: u32, h: u32, c: u32) {
        for dy in 0..h as i32 {
            for dx in 0..w as i32 {
                self.px(x + dx, y + dy, c);
            }
        }
    }

    pub fn clear(&mut self, c: u32) {
        for p in self.buf.iter_mut() {
            *p = c;
        }
    }

    /// Blend one coverage sample.
    pub fn blend(&mut self, x: i32, y: i32, color: u32, coverage: u8) {
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
    pub fn draw_str(&mut self, x: i32, baseline: i32, s: &str, role: Role, color: u32) -> i32 {
        let size = self.body_size();
        self.draw_str_sized(x, baseline, s, role, color, size)
    }

    /// Draw `s` at an explicit point size.
    ///
    /// Every glyph is placed at the running pen position; blending at the
    /// starting `x` instead stacks every letter on the same spot.
    pub fn draw_str_sized(
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
    pub fn draw_run_sized(
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
    pub fn measure_at(&mut self, s: &str, role: Role, size: u16) -> i32 {
        self.fonts.measure_at(role, size, s)
    }

    pub fn line_height_for(&mut self, size: u16) -> u32 {
        self.fonts.line_height_at(Role::Body, size)
    }

    pub fn ascent_for(&mut self, size: u16) -> i32 {
        self.fonts.ascent_at(Role::Body, size)
    }

    pub fn measure(&mut self, s: &str, role: Role) -> i32 {
        self.fonts.for_role(role).measure(s)
    }

    pub fn line_height(&self, role: Role) -> u32 {
        self.fonts.line_height_of(role)
    }

    pub fn ascent(&self, role: Role) -> i32 {
        self.fonts.ascent_of(role)
    }
}
