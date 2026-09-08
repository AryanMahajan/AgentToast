//! The menu bar glyph.
//!
//! Tauri has a default window icon and the tray will happily use it, but on
//! macOS that is the wrong image twice over.
//!
//! **Wrong size.** `tauri-codegen` only ever looks for a `.png` when it picks
//! the default window icon on a non-Windows target. The macOS bundle config
//! lists an `.icns` and nothing else, so the search falls through to
//! `src-tauri/icons/icon.png` — the 32×32 image built for a Windows tray. The
//! menu bar wants 18pt, which is 36px on every display anyone has used in a
//! decade, so that icon arrives too small and gets scaled anyway.
//!
//! **Wrong kind.** A menu bar icon is a *template image*: macOS ignores its
//! colour entirely and paints the alpha channel, dark on a light menu bar and
//! light on a dark one, so it stays legible however the user has their Mac set
//! up and whatever wallpaper is behind it. That is why Docker's whale and
//! Tailscale's mark are single-colour. A full-colour app icon shoved into the
//! menu bar cannot do that.
//!
//! So the glyph is drawn here instead of shipped as a file: the same mark as
//! the app icon — a rounded card with two bars — at exactly the proportions
//! `macos/scripts/render-icon.py` uses.
//!
//! It is drawn as an **outlined** card with the bars inked inside it, rather
//! than the app icon's filled card with the bars cut out of it. A template
//! image has one tone and no second colour to fall back on, so a filled card
//! comes out as a solid block the size of the menu bar — legible, but a blob
//! sat next to Docker's whale and Tailscale's mark, both of which are mostly
//! negative space. Outlining keeps the same silhouette at a weight that
//! belongs up there.
//!
//! Drawing it in code rather than committing a binary keeps it readable, keeps
//! it in step with the app icon, and means any size can be asked for.

use tauri::image::Image;

/// 18pt at 2×, which is the size Apple documents for a menu bar extra.
const SIZE: usize = 36;

/// Samples per axis per pixel. The mark is all curves at 36px; without this the
/// corners stair-step badly enough to look like a different icon.
const OVERSAMPLE: usize = 4;

/// A rounded rectangle: x, y, width, height, corner radius.
type Rect = (f64, f64, f64, f64, f64);

/// The card's outer and inner edge, and the two bars, at the proportions in
/// `render-icon.py`.
///
/// Kept deliberately in step with the app icon's geometry so the thing in the
/// menu bar and the thing in Finder are recognisably the same mark. The inner
/// edge is what turns the card into an outline: the ink is whatever falls
/// between the two.
fn shapes(size: f64) -> (Rect, Rect, Vec<Rect>) {
    let pad = size * 0.06;
    let side = size - pad * 2.0;
    let radius = (size * 0.22).max(2.0);
    let outer = (pad, pad, side, side, radius);

    // The same weight as a bar, so the whole mark reads as one stroke width.
    let stroke = (size * 0.09).max(1.0);
    let inner = (
        pad + stroke,
        pad + stroke,
        side - stroke * 2.0,
        side - stroke * 2.0,
        (radius - stroke).max(1.0),
    );

    let left = size * 0.28;
    let width = size * 0.44;
    let height = stroke;

    let bars = vec![
        (left, size * 0.34, width, height, height / 2.0),
        (left, size * 0.55, width * 0.62, height, height / 2.0),
    ];

    (outer, inner, bars)
}

/// Whether a point falls inside a rounded rectangle.
///
/// Clamping to the rectangle inset by the radius gives the nearest point on the
/// "spine"; anything within one radius of that is inside, which handles the
/// straight edges and the four corner arcs in the same expression.
///
/// The bounds are ordered explicitly rather than handed straight to `clamp`. A
/// bar's radius is exactly half its height, so `y + r` and `y + h - r` are the
/// same number in exact arithmetic and can land a ULP apart in floating point —
/// and `f64::clamp` panics outright when its minimum exceeds its maximum.
fn inside(px: f64, py: f64, (x, y, w, h, r): Rect) -> bool {
    if px < x || px > x + w || py < y || py > y + h {
        return false;
    }

    let spine = |p: f64, start: f64, extent: f64| {
        let lo = start + r;
        p.clamp(lo, (start + extent - r).max(lo))
    };

    let (dx, dy) = (px - spine(px, x, w), py - spine(py, y, h));
    dx * dx + dy * dy <= r * r
}

/// How much of one pixel a shape covers, 0.0 to 1.0.
fn coverage(col: usize, row: usize, rect: Rect) -> f64 {
    let mut hits = 0;
    for sy in 0..OVERSAMPLE {
        for sx in 0..OVERSAMPLE {
            let px = col as f64 + (sx as f64 + 0.5) / OVERSAMPLE as f64;
            let py = row as f64 + (sy as f64 + 0.5) / OVERSAMPLE as f64;
            if inside(px, py, rect) {
                hits += 1;
            }
        }
    }
    hits as f64 / (OVERSAMPLE * OVERSAMPLE) as f64
}

/// The glyph as RGBA, black with the shape in the alpha channel.
///
/// Black because a template image's colour is thrown away — only the alpha is
/// read — and black is what every other template image in the tree uses.
fn render(size: usize) -> Vec<u8> {
    let (outer, inner, bars) = shapes(size as f64);
    let mut rgba = Vec::with_capacity(size * size * 4);

    for row in 0..size {
        for col in 0..size {
            // The outline is the ring between the two edges. Multiplying by the
            // inner shape's *absence* rather than subtracting it keeps the
            // antialiased inner edge from biting into the outer one.
            let ring = coverage(col, row, outer) * (1.0 - coverage(col, row, inner));

            let bar = bars
                .iter()
                .map(|bar| coverage(col, row, *bar))
                .fold(0.0f64, f64::max);

            // Ink is ink: a pixel covered by both is no darker than either.
            let alpha = ring.max(bar).clamp(0.0, 1.0);
            rgba.extend_from_slice(&[0, 0, 0, (alpha * 255.0).round() as u8]);
        }
    }

    rgba
}

/// The menu bar glyph, ready to hand to the tray.
pub fn icon() -> Image<'static> {
    Image::new_owned(render(SIZE), SIZE as u32, SIZE as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(rgba: &[u8], size: usize, col: usize, row: usize) -> u8 {
        rgba[(row * size + col) * 4 + 3]
    }

    /// Write the glyph out so it can be looked at, because no assertion about
    /// pixel values tells you whether a 36px mark actually reads.
    ///
    /// ```text
    /// GLYPH_OUT=/tmp/menubar.rgba cargo test -p agenttoast -- --ignored glyph_preview
    /// ```
    #[test]
    #[ignore = "writes a file for a human to look at"]
    fn glyph_preview() {
        let path = std::env::var("GLYPH_OUT").unwrap_or_else(|_| "menubar.rgba".into());
        std::fs::write(&path, render(SIZE)).expect("write the preview");
        println!("{SIZE}x{SIZE} RGBA written to {path}");
    }

    #[test]
    fn the_glyph_is_the_size_the_menu_bar_asks_for() {
        let image = icon();
        assert_eq!(image.width(), 36);
        assert_eq!(image.height(), 36);
        assert_eq!(image.rgba().len(), 36 * 36 * 4);
    }

    #[test]
    fn every_pixel_is_black_so_only_the_alpha_carries_the_shape() {
        // A template image's colour is discarded; anything but black here would
        // be a lie about what gets drawn.
        let rgba = render(SIZE);
        assert!(rgba.chunks(4).all(|p| p[0] == 0 && p[1] == 0 && p[2] == 0));
    }

    #[test]
    fn the_card_is_an_outline_not_a_block() {
        let rgba = render(SIZE);
        let (outer, ..) = shapes(SIZE as f64);

        // On the stroke itself, near the middle of the top edge.
        let top = (outer.1 + 1.0) as usize;
        assert_eq!(alpha_at(&rgba, SIZE, SIZE / 2, top), 255, "the stroke");

        // And hollow between the stroke and the first bar.
        assert_eq!(alpha_at(&rgba, SIZE, SIZE / 2, 8), 0, "inside the card");
    }

    #[test]
    fn the_bars_are_inked() {
        let rgba = render(SIZE);
        let (.., bars) = shapes(SIZE as f64);

        for bar in &bars {
            let col = (bar.0 + bar.2 / 2.0) as usize;
            let row = (bar.1 + bar.3 / 2.0) as usize;
            assert_eq!(alpha_at(&rgba, SIZE, col, row), 255, "the middle of a bar");
        }
    }

    /// A menu bar icon that is mostly ink is a blob. This is the property that
    /// separates an outline from the filled app icon.
    #[test]
    fn most_of_the_glyph_is_negative_space() {
        let rgba = render(SIZE);
        let inked: usize = rgba.chunks(4).filter(|p| p[3] > 127).count();
        let ratio = inked as f64 / (SIZE * SIZE) as f64;
        assert!(ratio < 0.45, "{:.0}% inked; too heavy for a menu bar", ratio * 100.0);
    }

    #[test]
    fn the_corners_are_rounded_away() {
        let rgba = render(SIZE);
        for (col, row) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
            assert_eq!(alpha_at(&rgba, SIZE, col, row), 0, "corner {col},{row}");
        }
    }

    /// A bar's corner radius is exactly half its height, so the inset bounds
    /// coincide. Rounding can order them the wrong way round, which used to
    /// panic inside `f64::clamp` for every pixel of every bar.
    #[test]
    fn a_shape_as_round_as_it_can_be_does_not_panic() {
        for r in [1.62, 2.0, 0.5] {
            let pill = (3.0, 4.0, 10.0, r * 2.0, r);
            for row in 0..12 {
                for col in 0..16 {
                    let _ = coverage(col, row, pill);
                }
            }
        }
    }

    #[test]
    fn the_two_bars_are_different_lengths() {
        // The mark is asymmetric on purpose; equal bars would read as an
        // equals sign rather than as the app icon.
        let (.., bars) = shapes(SIZE as f64);
        assert!(bars[1].2 < bars[0].2);
    }

    #[test]
    fn the_edges_are_antialiased_rather_than_stepped() {
        let rgba = render(SIZE);
        let partial = rgba
            .chunks(4)
            .filter(|p| p[3] > 0 && p[3] < 255)
            .count();
        assert!(partial > 20, "expected soft edges, found {partial} partial pixels");
    }
}
