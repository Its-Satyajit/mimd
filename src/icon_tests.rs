//! Tests for the window mark.

use super::*;
use crate::testutil::write_png;

#[test]
fn wayland_is_detected_and_never_given_an_icon() {
    // The guard is the same signal minifb itself branches on.
    let has = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    assert_eq!(
        has,
        wayland_session(),
        "detection disagrees with the variable"
    );
    if has {
        assert!(
            icon_from_logo().is_ok(),
            "the icon should still rasterize; only the set_icon call is skipped"
        );
    }
}

/// The icon must actually be the logo, not a blob. Checks the geometry
/// against the SVG at 256 units, where each feature is large enough to
/// probe unambiguously.

#[test]
fn the_icon_reproduces_the_svg_mark() {
    let size = 256u32;
    let px = logo_icon(size);
    assert_eq!(
        px.len() as u32,
        size * size,
        "icon buffer must be size*size pixels"
    );

    let at = |x: u32, y: u32| (px[(y * size + x) as usize] >> 24) as u32;
    // Sampling the middle of a 9x9 block keeps antialiasing out of it.
    let solid =
        |x: u32, y: u32| -> bool { (0..9).all(|dy| (0..9).all(|dx| at(x + dx, y + dy) > 240)) };
    // Knockouts are only 10 units wide, so probe them with a 5x5 block:
    // a 9x9 one would span the whole stroke and catch its inked sides.
    let clear =
        |x: u32, y: u32| -> bool { (0..5).all(|dy| (0..5).all(|dx| at(x + dx, y + dy) < 15)) };

    // Body of the page: solid ink.
    assert!(solid(75, 90), "the page body should be inked");
    assert!(
        solid(175, 200),
        "the lower right of the page should be inked"
    );
    // The folded corner cuts the top-right away, and the fold line knocks out.
    // (182,50) rather than nearer the diagonal: the page's cut edge runs
    // (152,40)->(188,76), so x=170 at y=50 straddles it.
    assert!(clear(182, 50), "outside the fold should be empty");
    // Well clear of the fold's 5-unit half-width, so a 9x9 block fits.
    assert!(solid(138, 55), "left of the fold line should be inked");
    // The fold line itself is a knockout, running down then across.
    assert!(clear(152, 60), "the vertical fold should be knocked out");
    assert!(clear(170, 76), "the horizontal fold should be knocked out");
    // Bullet + bar sit on a knocked-out line.
    assert!(clear(95, 111), "the bullet should be knocked out");
    assert!(clear(150, 111), "the bar should be knocked out");
    // The arrow stems and heads are knocked out of the lower page.
    assert!(clear(87, 168), "the first arrow stem should be knocked out");
    assert!(
        clear(123, 168),
        "the middle arrow stem should be knocked out"
    );
    assert!(
        clear(159, 168),
        "the third arrow stem should be knocked out"
    );
    assert!(clear(104, 165), "an arrow head should be knocked out");
    // Everything outside the page silhouette is empty.
    for (x, y) in [(10u32, 10u32), (250, 10), (10, 250), (250, 250), (60, 30)] {
        assert!(solid(x, y) == false, "({x},{y}) should be outside the mark");
    }
    // Knocked-out regions must not exceed the silhouette, or the arrows
    // would be punched through into the background.
    // Clear of the stems' 5-unit half-widths on both sides.
    assert!(solid(70, 170), "between the arrows the page is still inked");

    // Alpha is what carries the shape: RGB must be premultiplied with it.
    for &v in px.iter().take(size as usize * 4) {
        let a = (v >> 24) as u32;
        let r_ = ((v >> 16) & 0xff) as u32;
        assert_eq!(r_, a, "RGB must equal alpha for premultiplied ARGB");
    }

    // Opt-in dump so the mark can be eyeballed against the SVG.
    if let Ok(path) = std::env::var("MIMD_ICON_PNG") {
        let n = 256u32;
        let ic = logo_icon(n);
        // Ink black on white, so the knockouts read as paper the way the
        // SVG does. (The icon itself is alpha-only; this is just for review.)
        let mut rgba = Vec::with_capacity((n * n) as usize);
        for &v in &ic {
            let a = (v >> 24) as u32;
            let ink = 255 - (a * 255 / 255) as u32;
            rgba.push(0xff000000u32 | (ink << 16) | (ink << 8) | ink);
        }
        write_png(&path, &rgba, n, n);
        eprintln!("wrote {path}");
    }

    // Small sizes must still resolve the counters, not turn to mush.
    let mut cov: Vec<(u32, f64)> = Vec::new();
    for s in [16u32, 24, 32, 64, 128, 256] {
        let small = logo_icon(s);
        assert_eq!(
            small.len() as u32,
            s * s,
            "{s}px icon has the wrong pixel count"
        );
        // Coverage has to be alpha-weighted, not a count of fully opaque
        // pixels: the silhouette covers 31% of the design box, but at 16px
        // most edge pixels are partially covered and would read as empty.
        let total = (s as f64) * (s as f64);
        let sum: f64 = small.iter().map(|&v| (v >> 24) as f64).sum();
        let pct = 100.0 * sum / (255.0 * total);
        cov.push((s, pct));
    }
    let ref_pct = cov
        .iter()
        .find(|(s, _)| *s == 256)
        .map(|(_, p)| *p)
        .unwrap();
    for (s, pct) in &cov {
        assert!(
            (ref_pct - 8.0..=ref_pct + 3.0).contains(pct),
            "{s}px ink coverage {pct:.1}% drifts from the 256px reference {ref_pct:.1}%"
        );
    }
}
