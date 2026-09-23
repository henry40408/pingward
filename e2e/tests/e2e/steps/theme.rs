//! The three-state theme control and the palette it resolves.

use anyhow::{Result, ensure};
use cucumber::{given, then, when};
use pingward_e2e::dom::Dom;
use pingward_e2e::wait::eventually_eq;
use pingward_e2e::world::PingwardWorld;

#[then(expr = "the resolved theme is {string}")]
async fn resolved_theme_is(world: &mut PingwardWorld, theme: String) -> Result<()> {
    // Always `light` or `dark`, never `system`.
    world
        .driver()?
        .expect_attr("html", "data-theme", Some(&theme))
        .await
}

#[then(expr = "the stored theme preference is {string}")]
async fn stored_theme_is(world: &mut PingwardWorld, preference: String) -> Result<()> {
    let driver = world.driver()?;
    eventually_eq("the stored theme preference", preference, || async {
        Ok(driver
            .eval("return localStorage.getItem('pw-theme');")
            .await?
            .as_str()
            .unwrap_or_default()
            .to_owned())
    })
    .await
}

#[when("I click the theme toggle")]
async fn click_theme_toggle(world: &mut PingwardWorld) -> Result<()> {
    world.driver()?.click_css("#pw-theme-toggle").await
}

#[given(expr = "I set the theme preference to {string}")]
async fn set_theme_preference(world: &mut PingwardWorld, preference: String) -> Result<()> {
    // Reloads, so `theme-init.js` re-resolves it before first paint.
    let driver = world.driver()?;
    driver
        .execute(
            "localStorage.setItem('pw-theme', arguments[0]);",
            vec![serde_json::json!(preference)],
        )
        .await?;
    driver.refresh().await?;
    Ok(())
}

#[when("the OS prefers dark")]
async fn os_prefers_dark(world: &mut PingwardWorld) -> Result<()> {
    // `app.js`'s `matchMedia` listener re-resolves under `system`.
    world.browser()?.emulate_color_scheme("dark").await
}

#[when("the OS prefers light")]
async fn os_prefers_light(world: &mut PingwardWorld) -> Result<()> {
    world.browser()?.emulate_color_scheme("light").await
}

#[when("I hover the dashboard's primary action")]
async fn hover_primary_action(world: &mut PingwardWorld) -> Result<()> {
    world.goto("/").await?;
    let driver = world.driver()?;
    let button = driver.css(".btn-primary").await?;
    driver
        .action_chain()
        .move_to_element_center(&button)
        .perform()
        .await?;
    Ok(())
}

#[then("its label contrasts with its background")]
async fn label_contrasts(world: &mut PingwardWorld) -> Result<()> {
    // WCAG contrast of the computed colours (`filter` is not folded in). `rgb`
    // normalises Chromium's `color(srgb …)` 0–1 floats from `color-mix()`;
    // read as 0–255 they look near-black and fake a pass. No `//` in the
    // script: the `\` continuations join it onto one line.
    let button = world.driver()?.css(".btn-primary").await?;
    let ratio = world
        .driver()?
        .execute(
            "const cs = getComputedStyle(arguments[0]);\
             const rgb = (s) => {\
               const n = s.match(/[\\d.]+/g).slice(0, 3).map(Number);\
               return s.startsWith('color(') ? n.map((v) => v * 255) : n;\
             };\
             const lum = ([r, g, b]) => [r, g, b].map((v) => {\
               v /= 255;\
               return v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4);\
             }).reduce((acc, v, i) => acc + v * [0.2126, 0.7152, 0.0722][i], 0);\
             const a = lum(rgb(cs.color));\
             const b = lum(rgb(cs.backgroundColor));\
             return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);",
            vec![button.to_json()?],
        )
        .await?
        .json()
        .as_f64()
        .ok_or_else(|| anyhow::anyhow!("the contrast probe did not return a number"))?;
    ensure!(ratio >= 4.5, "the contrast ratio is {ratio:.2}, below 4.5");
    Ok(())
}
