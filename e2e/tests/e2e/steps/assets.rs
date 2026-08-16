//! The static assets the templates reference — a port of `assets.steps.js`.

use anyhow::{Result, ensure};
use cucumber::then;
use pingward_e2e::dom::Dom;
use pingward_e2e::world::PingwardWorld;

#[then("the footer shows the build version")]
async fn footer_shows_version(world: &mut PingwardWorld) -> Result<()> {
    // The version string is `git describe` output, which has no single shape:
    // a release tag, a tag plus distance, or a bare short SHA before the first
    // tag exists (and from CI's shallow, tag-less checkout). So this asserts
    // the footer rendered *something* rather than matching a semver pattern
    // that would fail on a perfectly good build.
    let driver = world.driver()?;
    driver.expect_visible("app-version").await?;
    let version = driver.text_of("app-version").await?;
    let matches = regex::Regex::new(r"^pingward \S+$")
        .expect("the version pattern compiles")
        .is_match(&version);
    ensure!(matches, "the footer reads {version:?}");
    Ok(())
}

#[then(expr = "{string} is well-formed XML")]
async fn asset_is_well_formed_xml(world: &mut PingwardWorld, asset: String) -> Result<()> {
    // `DOMParser` with the `image/svg+xml` type is a real, spec-compliant XML
    // parser — it is what the browser itself uses for the asset — and it
    // reports a failure by returning a document whose root is a
    // `<parsererror>` rather than by throwing. Fetching from inside the page
    // keeps the request same-origin and exercises the served response, headers
    // and all.
    let result = world
        .driver()?
        .execute_async(
            "const [path, done] = arguments;\
             fetch(path).then(async (res) => {\
               if (!res.ok) { done({ status: res.status, error: null }); return; }\
               const doc = new DOMParser().parseFromString(await res.text(), 'image/svg+xml');\
               const failure = doc.querySelector('parsererror');\
               done({ status: res.status, error: failure && failure.textContent.trim() });\
             }).catch((e) => done({ status: 0, error: String(e) }));",
            vec![serde_json::json!(asset)],
        )
        .await?;
    let result = result.json();
    let status = result.get("status").and_then(serde_json::Value::as_u64);
    ensure!(status == Some(200), "{asset} returned HTTP {status:?}");
    let error = result.get("error").and_then(serde_json::Value::as_str);
    ensure!(
        error.is_none(),
        "{asset} is not well-formed XML, so a strict parser renders no icon at all:\n{}",
        error.unwrap_or_default()
    );
    Ok(())
}
