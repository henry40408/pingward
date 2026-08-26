//! The Cucumber runner.
//!
//! `harness = false`: cucumber drives the scenarios itself, so there is no
//! libtest harness collecting `#[test]` functions. Run it from `e2e/` with
//! `cargo test --test e2e`.
//!
//! A `before` hook reads the scenario's tags: `@nojs` decides whether the
//! page's scripts run, and `@fast-scan` / `@smtp-env` / `@trusted-proxy` select
//! the server's environment (see [`pingward_e2e::server::Options`]).
//!
//! Every scenario gets its own server and database: `POST /setup` creates the
//! first admin once and only once, and almost every scenario walks through it.

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
/// A fixed four is too many for a two-core CI runner, where the browsers
/// contend until pages take longer to settle than the steps wait for — and each
/// scenario here also carries a whole pingward process.
fn max_concurrent_scenarios() -> usize {
    std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .min(CONCURRENCY_CEILING)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Before anything runs in parallel: on a cold driver cache (every CI run)
    // concurrent sessions contend on the same download and the run wedges.
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
/// Gherkin scopes tags by inheritance (`@fast-scan` sits on the whole of
/// `time_states.feature`), but `Scenario::tags` reports only the ones written
/// on the scenario itself, so the options are read from the union.
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
