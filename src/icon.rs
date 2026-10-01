//! The mimd mark, drawn for the window manager.
//!
//! The artwork is vector (mimd-logo-monochrome.svg) but a window icon must be
//! pixels, and an SVG rasterizer would cost more than the whole rest of the
//! binary. The mark is simple enough to draw directly.

#[cfg(test)]
#[path = "icon_tests.rs"]
mod tests;
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
    const DOC: [(f32, f32); 5] = [
        (68., 40.),
        (152., 40.),
        (188., 76.),
        (188., 216.),
        (68., 216.),
    ];
    const FOLD: [(f32, f32, f32, f32); 2] = [(152., 40., 152., 76.), (152., 76., 188., 76.)];
    const BULLET: (f32, f32, f32, f32) = (88., 104., 106., 122.);
    const BAR: (f32, f32, f32, f32) = (114., 104., 186., 122.);
    const STEMS: [(f32, f32, f32, f32); 3] = [
        (88., 150., 88., 184.),
        (124., 150., 124., 184.),
        (160., 150., 160., 184.),
    ];
    const HEADS: [(f32, f32, f32, f32); 4] = [
        (88., 150., 106., 168.),
        (106., 168., 124., 150.),
        (124., 150., 142., 168.),
        (142., 168., 160., 150.),
    ];
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

/// Whether this process is on a Wayland compositor. minifb picks its backend
/// from `WAYLAND_DISPLAY`, so this is the same signal it uses.
pub fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
}

/// The mark at the sizes a window manager is likely to ask for, concatenated
/// into the one buffer `_NET_WM_ICON` wants (a flat run of ARGB pixels).
#[cfg(target_os = "linux")]
pub fn icon_from_logo() -> Result<minifb::Icon, ()> {
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
    let t = if len2 <= 0.0 {
        0.0
    } else {
        (((px - a.0) * dx + (py - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (qx, qy) = (a.0 + t * dx, a.1 + t * dy);
    ((px - qx).powi(2) + (py - qy).powi(2)).sqrt()
}

fn near_any_seg(px: f32, py: f32, segs: &[(f32, f32, f32, f32)], half: f32) -> bool {
    segs.iter()
        .any(|&(ax, ay, bx, by)| seg_dist(px, py, (ax, ay), (bx, by)) <= half)
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
