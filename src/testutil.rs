//! Shared scaffolding for the test modules.
//!
//! Building a `Canvas` needs a `Fonts`, and a `Fonts` needs a framebuffer to
//! draw into. Several test modules want the same handful of those, so they
//! live here rather than being copied into each file.

use crate::document::{App, Mode};
use crate::fonts::{Fonts, Role};
use crate::nav::{caret_x, mouse_step, MouseAction};
use crate::paint::Canvas;
use crate::theme::{Row, BG, MARGIN};
use std::path::PathBuf;

pub struct Mouse {
    pub app: App,
    pub buf: Vec<u32>,
    pub fonts: Fonts,
}

impl Mouse {
    pub fn new(app: App) -> Option<Self> {
        Some(Mouse {
            app,
            buf: vec![BG; 400 * 400],
            fonts: Fonts::new(None, None, 15).ok()?,
        })
    }

    pub fn step(&mut self, x: i32, y: i32, down: bool) -> MouseAction {
        let mut c = Canvas {
            buf: &mut self.buf,
            w: 400,
            h: 400,
            fonts: &mut self.fonts,
        };
        let hs = self.app.hscroll;
        mouse_step(
            &mut self.app,
            x,
            y,
            down,
            Row {
                x: MARGIN as i32,
                w: 400,
                hscroll: hs,
            },
            &mut c,
        )
    }

    /// Click at the Nth column of line L.
    pub fn click(&mut self, l: usize, col: usize) {
        let mut c = Canvas {
            buf: &mut self.buf,
            w: 400,
            h: 400,
            fonts: &mut self.fonts,
        };
        let lh = c.line_height(Role::Body) as i32;
        let y = MARGIN as i32 + l as i32 * lh + lh / 2;
        let x = caret_x(&mut c, &self.app.lines[l], col, row_at(MARGIN as i32));
        drop(c);
        self.step(x, y, true);
        self.step(x, y, false);
    }
}

pub fn edit_app(text: &str) -> App {
    App::open(text.into(), None, Mode::Edit)
}

pub fn tmp_md(name: &str, body: &str) -> PathBuf {
    let p = std::env::temp_dir().join(format!("mimd-test-{name}"));
    std::fs::write(&p, body).unwrap();
    p
}

/// Minimal PNG writer (RGB8, no filtering) so a rendered frame can be
/// opened in any image viewer. Test-only.
pub fn write_png(path: &str, buf: &[u32], w: u32, h: u32) {
    fn crc(data: &[u8]) -> u32 {
        let mut c = 0xffff_ffffu32;
        for &b in data {
            c ^= b as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xedb8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
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
    let chunk = |tag: &[u8], data: &[u8], out: &mut Vec<u8>| {
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

pub fn row_at_scrolled(x: i32, hscroll: u32) -> Row {
    Row { x, w: 400, hscroll }
}

/// A `Row` at a known origin, for hit-testing tests.
pub fn row_at(x: i32) -> Row {
    Row {
        x,
        w: 400,
        hscroll: 0,
    }
}

/// A canvas that draws nowhere, for measuring and laying out.
///
/// Text measurement needs a `Canvas`, and layout needs one too, but neither
/// wants a real framebuffer. Building this inline in every test needs a
/// fresh `fonts` borrow each time, which is why it is a helper; the caller
/// owns the buffer so the two borrows can have different lifetimes.
pub fn scratch<'a>(buf: &'a mut Vec<u32>, fonts: &'a mut Fonts) -> Canvas<'a> {
    Canvas {
        buf,
        w: 0,
        h: 0,
        fonts,
    }
}

pub fn canvas() -> Option<(Vec<u32>, Fonts)> {
    let fonts = Fonts::new(None, None, 15).ok()?;
    Some((vec![BG; 400 * 400], fonts))
}

/// Split a command line the way `std::env::args` would present it.
pub fn argv(s: &str) -> std::vec::IntoIter<String> {
    s.split_whitespace()
        .map(str::to_string)
        .collect::<Vec<_>>()
        .into_iter()
}
