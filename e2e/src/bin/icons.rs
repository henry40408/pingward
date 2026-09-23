//! Renders the committed `assets/apple-touch-icon.png` from
//! `assets/favicon.svg` with `resvg`. Rerun after editing the SVG:
//! `cd e2e && cargo run --bin icons`.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use regex::Regex;
use resvg::{tiny_skia, usvg};

/// iOS's size; it downscales for smaller slots.
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

/// Drops `rx`/`ry` from the `data-frame` backing rect: iOS applies its own
/// mask, and a rounded source reads as a double-rounded edge.
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
