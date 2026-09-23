//! The Cucumber runner (`harness = false`; run `cargo test --test e2e` from `e2e/`).
//!
//! Scenario tags pick the environment: `@nojs` disables page scripts, the rest
//! go to [`pingward_e2e::server::Options`]. Each scenario gets its own server
//! and database, since `POST /setup` succeeds only once.

mod steps;

use cucumber::World as _;
use cucumber::gherkin;
use cucumber::writer::Stats as _;
use pingward_e2e::browser::{Browser, Scripting};
use pingward_e2e::server::Options;
use pingward_e2e::world::PingwardWorld;

const FEATURES: &str = "features";

/// Upper bound on concurrent scenarios (each is a browser + server + DB).
const CONCURRENCY_CEILING: usize = 4;

/// One scenario per core up to [`CONCURRENCY_CEILING`]: a fixed four overloads
/// a two-core CI runner until pages settle slower than the steps wait.
fn max_concurrent_scenarios() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(CONCURRENCY_CEILING)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Download the driver once up front: concurrent sessions on a cold cache
    // contend on the same download and wedge.
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

/// Feature, rule and scenario tags combined: `Scenario::tags` omits inherited
/// ones (e.g. `@fast-scan` on all of `time_states.feature`).
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
