//! Retrying assertions.
//!
//! Playwright's `expect(...)` polls until the assertion holds or a timeout
//! expires, which is what let the old steps write
//! `await expect(page).toHaveURL('/')` straight after a click. `WebDriver` has
//! no such layer: a `find` that runs before the server's redirect has landed
//! simply reports the old page.
//!
//! thirtyfour's `ElementQuery` filters cover the cases that are really "wait
//! for an element matching X", and [`crate::dom`] uses them. These helpers
//! cover the rest — a value that has to settle, like a check's status after a
//! ping or the URL after a form post.

use std::fmt::Debug;
use std::future::Future;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use thirtyfour::error::{WebDriverError, WebDriverErrorInner};

use crate::browser::{WAIT_INTERVAL, WAIT_TIMEOUT};

/// Is this the DOM having moved under the probe, rather than a real fault?
///
/// The check page swaps its pings and notifications sections in place (the
/// fragment endpoints, and the live tail), so an element found on one poll can
/// be detached before the next line reads it. To a poll that is "not yet" — the
/// answer it is there to wait for — and treating it as an error turns every
/// assertion that spans a swap into a flake.
///
/// Only the stale-reference error is forgiven. A missing element, a bad
/// selector or a dead session still fail immediately, which is what keeps a
/// genuinely broken page from being waited on for the full timeout.
fn is_stale(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause.downcast_ref::<WebDriverError>().is_some_and(|error| {
            matches!(
                error.as_inner(),
                WebDriverErrorInner::StaleElementReference(_)
            )
        })
    })
}

/// Polls `probe` until it reports the expected value.
///
/// On timeout the failure names the last value seen, not merely that a wait
/// expired — that is the difference between "the status never reached down" and
/// a message you have to reproduce by hand to understand.
///
/// # Errors
///
/// Fails when `probe` errors, or when the value has still not matched by
/// [`WAIT_TIMEOUT`].
pub async fn eventually_eq<T, E, F, Fut>(what: &str, expected: E, mut probe: F) -> Result<()>
where
    T: Debug,
    E: Debug + PartialEq<T>,
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<T>>,
{
    let deadline = Instant::now() + WAIT_TIMEOUT;
    let mut last = None;
    loop {
        match probe().await {
            Ok(value) => {
                if expected == value {
                    return Ok(());
                }
                last = Some(value);
            }
            Err(error) if is_stale(&error) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            bail!("{what}: expected {expected:?}, last saw {last:?} after {WAIT_TIMEOUT:?}");
        }
        tokio::time::sleep(WAIT_INTERVAL).await;
    }
}

/// Polls `probe` until it reports `true`.
///
/// # Errors
///
/// Fails when `probe` errors, or when it has still not held by
/// [`WAIT_TIMEOUT`].
pub async fn eventually<F, Fut>(what: &str, probe: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    eventually_within(WAIT_TIMEOUT, what, probe).await
}

/// [`eventually`] with a deadline of its own.
///
/// For the handful of waits that are not "the page is catching up" but "a
/// background loop is getting to it" — chiefly the scan loop, which only
/// transitions an overdue check on its next pass.
///
/// # Errors
///
/// Fails when `probe` errors, or when it has still not held by `timeout`.
pub async fn eventually_within<F, Fut>(timeout: Duration, what: &str, mut probe: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        match probe().await {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(error) if is_stale(&error) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            bail!("{what}: still not true after {timeout:?}");
        }
        tokio::time::sleep(WAIT_INTERVAL).await;
    }
}

/// Polls `probe` until it reports a value, handing it back.
///
/// The shape for "read something once it exists" — Playwright's
/// `expect(locator).toBeVisible()` followed by a read, in one step.
///
/// # Errors
///
/// Fails when `probe` errors, or when it has still reported `None` by
/// [`WAIT_TIMEOUT`].
pub async fn eventually_some<T, F, Fut>(what: &str, mut probe: F) -> Result<T>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<Option<T>>>,
{
    let deadline = Instant::now() + WAIT_TIMEOUT;
    loop {
        match probe().await {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(error) if is_stale(&error) => {}
            Err(error) => return Err(error),
        }
        if Instant::now() >= deadline {
            bail!("{what}: never appeared within {WAIT_TIMEOUT:?}");
        }
        tokio::time::sleep(WAIT_INTERVAL).await;
    }
}
