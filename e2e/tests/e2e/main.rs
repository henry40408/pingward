//! The Cucumber runner.
//!
//! `harness = false`: cucumber drives the scenarios itself, so there is no
//! libtest harness collecting `#[test]` functions. Run it from `e2e/` with
//! `cargo test --test e2e`.
//!
//! What `playwright.config.js` expressed as two projects — `chromium`, and a
//! `no-js` project matching only `no_js.feature` — is expressed here as the
//! feature's `@nojs` tag, read in a `before` hook. The other three tags the
//! JavaScript fixtures read off `$tags` (`@fast-scan`, `@smtp-env`,
//! `@trusted-proxy`) select the server's environment in the same hook; see
//! [`pingward_e2e::server::Options`].
//!
//! Unlike the sibling ports, **every scenario gets its own server and its own
//! database**. That is not conservatism: `POST /setup` creates the first admin
//! once and only once, and almost every scenario here starts by walking through
//! it.

mod steps;

use cucumber::World as _;
use cucumber::gherkin;
use cucumber::writer::Stats as _;
use pingward_e2e::browser::{Browser, Scripting};
use pingward_e2e::server::Options;
use pingward_e2e::world::PingwardWorld;

const FEATURES: &str = "features";

/// The most scenarios — and so browsers, servers and databases — to run at
/// once, whatever the machine.
const CONCURRENCY_CEILING: usize = 4;

/// How many scenarios run at once, one per core up to [`CONCURRENCY_CEILING`].
///
/// A fixed four was wrong in the sibling ports: it is fine on a developer's
/// machine and too many for a two-core CI runner, where four browsers contend
/// for two cores until pages take longer to settle than the steps wait for.
/// Each scenario here also carries a whole pingward process, so the ceiling is
/// if anything more load-bearing.
fn max_concurrent_scenarios() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(CONCURRENCY_CEILING)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Before anything runs in parallel — see `Browser::prepare`. On a cold
    // driver cache concurrent sessions contend on the same download and the
    // run wedges rather than slows, and CI is cold every run.
    Browser::prepare().await?;

    let writer = PingwardWorld::cucumber()
        .max_concurrent_scenarios(max_concurrent_scenarios())
        .fail_on_skipped()
        .before(|feature, rule, scenario, world| {
            Box::pin(async move {
                let tags = tags_of(feature, rule, scenario);
                let scripting = if tags.iter().any(|tag| tag == "nojs") {
                    Scripting::Disabled
                } else {
                    Scripting::Enabled
                };
                world
                    .open(&Options::from_tags(&tags), scripting)
                    .await
                    .expect("could not start a server and browser for the scenario");
            })
        })
        .after(|_feature, _rule, _scenario, _finished, world| {
            Box::pin(async move {
                if let Some(world) = world {
                    world.close().await.expect("could not close the session");
                }
            })
        })
        .run(FEATURES)
        .await;

    let failures = writer.failed_steps() + writer.parsing_errors() + writer.hook_errors();
    anyhow::ensure!(failures == 0, "{failures} cucumber failure(s)");
    Ok(())
}

/// Every tag in scope for a scenario.
///
/// Gherkin scopes tags by inheritance — `@fast-scan` sits on the whole of
/// `time_states.feature`, `@smtp-env` on one scenario of `admin.feature` — but
/// `Scenario::tags` reports only the ones written on the scenario itself. The
/// union is what `playwright-bdd`'s `$tags` fixture handed the JavaScript
/// suite, so it is what the options are read from here.
fn tags_of(
    feature: &gherkin::Feature,
    rule: Option<&gherkin::Rule>,
    scenario: &gherkin::Scenario,
) -> Vec<String> {
    feature
        .tags
        .iter()
        .chain(rule.iter().flat_map(|rule| rule.tags.iter()))
        .chain(scenario.tags.iter())
        .cloned()
        .collect()
}
