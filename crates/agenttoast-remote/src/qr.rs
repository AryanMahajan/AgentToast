//! QR rendering for the pairing link.
//!
//! An SVG string rather than an image file: it goes straight into the
//! dashboard's markup, scales to whatever size the panel gives it, and needs no
//! temporary file on disk for something that is valid for five minutes.

use anyhow::{Context, Result};
use qrcode::QrCode;
use qrcode::render::svg;

/// Render `text` as a QR code, as a standalone `<svg>` element.
///
/// Both colours are given explicitly. The dashboard has a dark mode, and a QR
/// drawn in the page's foreground colour on a transparent ground is unreadable
/// to a phone camera when inverted — the scanner expects dark-on-light.
pub fn svg_for(text: &str) -> Result<String> {
    let code = QrCode::new(text.as_bytes()).context("Could not encode the pairing link as a QR")?;

    let rendered = code
        .render()
        .min_dimensions(240, 240)
        .quiet_zone(true)
        .dark_color(svg::Color("#16171a"))
        .light_color(svg::Color("#ffffff"))
        .build();

    // The renderer prefixes an XML declaration, which is valid for a standalone
    // `.svg` file and invalid inside an HTML document — a browser parses it as a
    // bogus comment and the markup after it lands in the wrong place. This is
    // going straight into the dashboard's DOM, so it has to go.
    Ok(match rendered.find("<svg") {
        Some(start) => rendered[start..].to_string(),
        None => rendered,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_an_svg_element() {
        let out = svg_for("http://192.168.1.40:8787/pair?c=abcdef").expect("should render");
        assert!(out.contains("</svg>"));
    }

    /// It goes into an HTML document, where an XML declaration is not markup.
    #[test]
    fn the_xml_declaration_is_stripped() {
        let out = svg_for("http://192.168.1.40:8787/pair?c=abcdef").expect("should render");
        assert!(out.starts_with("<svg"), "started with: {}", &out[..40.min(out.len())]);
        assert!(!out.contains("<?xml"));
    }
}
