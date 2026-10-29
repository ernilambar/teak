//! Tightens every vendored file icon to its ink box.
//!
//!   cargo run --example normalize-icons
//!
//! Seti draws its art inset inside a square canvas, so at a fixed 14px box its
//! glyphs cover only 36–83% of the edge while the Material set it replaced
//! filled 94–98%. VS Code never shows this because it renders Seti as a font,
//! and font tooling normalizes each glyph to its own ink box. This does the
//! same thing to the SVGs: it measures the painted bounds with the app's own
//! renderer and rewrites `viewBox` to a square around them, leaving a small
//! margin so strokes are not clipped.
//!
//! Run it after `./scripts/vendor-seti-icons.sh`, which calls it. It is
//! idempotent: re-running on normalized art reproduces the same box.

use std::path::Path;
use std::sync::Arc;

use gpui_kit::{DevicePixels, SvgRenderer, SvgSize};

/// Raster edge used to measure the ink. The icons are vector art, so this only
/// needs to be large enough to resolve the thinnest stroke.
const RASTER: u32 = 512;

/// Extra edge around the ink, as a fraction of its longest side, so that
/// anti-aliased or stroked edges are not clipped by the new box.
const MARGIN: f32 = 0.08;

/// Alpha above which a pixel counts as painted.
const INK: u8 = 8;

/// Coverage at or above which an icon is left alone. Normalized art lands at
/// `1 / (1 + MARGIN)` ≈ 0.93, so this only catches icons that are already
/// filling their canvas.
const TIGHT_ENOUGH: f32 = 0.85;

fn main() -> std::io::Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/file-icons");
    let renderer = SvgRenderer::new(Arc::new(()));
    let mut changed = 0;
    let mut skipped = Vec::new();

    for entry in std::fs::read_dir(&root)? {
        let path = entry?.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("svg") {
            continue;
        }

        let source = std::fs::read_to_string(&path)?;
        let Some((start, end, current)) = view_box(&source) else {
            skipped.push(path.clone());
            continue;
        };

        let Some(ink) = ink_box(&renderer, &source, current) else {
            skipped.push(path.clone());
            continue;
        };

        // Already tight: the art fills its box, so re-measuring would only
        // nudge it pixel by pixel on every run. This keeps the pass idempotent.
        if ink.2.max(ink.3) / current[2] >= TIGHT_ENOUGH {
            continue;
        }

        let side = ink.2.max(ink.3) * (1.0 + MARGIN);
        let x = ink.0 + ink.2 / 2.0 - side / 2.0;
        let y = ink.1 + ink.3 / 2.0 - side / 2.0;
        let replacement = format!(
            "viewBox=\"{} {} {} {}\"",
            round(x),
            round(y),
            round(side),
            round(side)
        );

        let rewritten = format!("{}{}{}", &source[..start], replacement, &source[end..]);
        if rewritten != source {
            std::fs::write(&path, rewritten)?;
            changed += 1;
        }
    }

    println!("normalized {changed} icons");
    if !skipped.is_empty() {
        println!("skipped {} without a readable ink box:", skipped.len());
        for path in skipped {
            println!("  {}", path.display());
        }
    }
    Ok(())
}

/// The byte span of the whole `viewBox="…"` attribute, plus its four numbers.
fn view_box(source: &str) -> Option<(usize, usize, [f32; 4])> {
    let start = source.find("viewBox=\"")?;
    let value_start = start + "viewBox=\"".len();
    let value_end = value_start + source[value_start..].find('"')?;
    let numbers: Vec<f32> = source[value_start..value_end]
        .split([' ', ','])
        .filter_map(|part| part.parse().ok())
        .collect();
    (numbers.len() == 4).then(|| (start, value_end + 1, numbers.try_into().unwrap()))
}

/// Painted bounds in the SVG's own coordinates: `(x, y, width, height)`.
fn ink_box(renderer: &SvgRenderer, source: &str, view: [f32; 4]) -> Option<(f32, f32, f32, f32)> {
    let parsed = renderer.parse_svg(source.as_bytes()).ok()?;
    let edge = DevicePixels(RASTER as i32);
    let image = renderer
        .render_parsed(&parsed, SvgSize::Size(gpui_kit::Size::new(edge, edge)))
        .ok()?;

    let size = image.size(0);
    let width = size.width.0 as usize;
    let height = size.height.0 as usize;
    let (pixels, _) = image.as_bytes(0)?.as_chunks::<4>();

    let (mut left, mut top) = (width, height);
    let (mut right, mut bottom) = (0usize, 0usize);
    for y in 0..height {
        for x in 0..width {
            if pixels[y * width + x][3] > INK {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x);
                bottom = bottom.max(y);
            }
        }
    }
    if right < left || bottom < top {
        return None;
    }

    // The vendor step already made every viewBox square, so one scale serves
    // both axes.
    let scale = view[2] / width as f32;
    Some((
        view[0] + left as f32 * scale,
        view[1] + top as f32 * scale,
        (right - left + 1) as f32 * scale,
        (bottom - top + 1) as f32 * scale,
    ))
}

fn round(value: f32) -> f32 {
    (value * 100.0).round() / 100.0
}
