//! Renders `assets/apple-touch-icon.png` from `assets/favicon.svg` with
//! `resvg`; needs no browser.
//!
//! The PNG is a committed artefact — `src/assets.rs` embeds it at compile time
//! and the root crate builds without ever running this. Run it after editing
//! the SVG:
//!
//! ```text
//! cd e2e && cargo run --bin icons
//! ```

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;
use resvg::{tiny_skia, usvg};

/// The size iOS asks for; it downscales for smaller slots.
const SIZE: u32 = 180;

fn main() -> Result<()> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("e2e/ always has a parent")?;
    let source = root.join("assets/favicon.svg");
    let target = root.join("assets/apple-touch-icon.png");

    let svg = std::fs::read_to_string(&source)
        .with_context(|| format!("reading {}", source.display()))?;
    let tree = usvg::Tree::from_str(&square_off_frame(&svg), &usvg::Options::default())
        .with_context(|| format!("parsing {}", source.display()))?;

    let mut pixmap = tiny_skia::Pixmap::new(SIZE, SIZE).context("allocating the output pixmap")?;
    let scale = SIZE as f32 / tree.size().width();
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    pixmap
        .save_png(&target)
        .with_context(|| format!("writing {}", target.display()))?;

    println!("wrote assets/apple-touch-icon.png ({SIZE}x{SIZE})");
    Ok(())
}

/// Drops the corner radius from the `data-frame` backing rect.
///
/// iOS masks the icon with its own superellipse, and a source radius under that
/// mask reads as a double-rounded edge. resvg reads the presentation
/// attribute, so the attribute is what gets removed; the inner quadrants keep
/// their own radius, which is wanted.
fn square_off_frame(svg: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let radius = RE.get_or_init(|| {
        Regex::new(r#"\s+(?:rx|ry)="[^"]*""#).expect("the radius pattern compiles")
    });
    svg.lines()
        .map(|line| {
            if line.contains("data-frame") {
                radius.replace_all(line, "").into_owned()
            } else {
                line.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}
