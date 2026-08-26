//! Retrying assertions.
//!
//! `WebDriver` has no polling layer: a `find` that runs before the server's
//! redirect has landed simply reports the old page. thirtyfour's `ElementQuery`
//! filters cover "wait for an element matching X" and [`crate::dom`] uses them;
//! these helpers cover a value that has to settle, like a check's status after
//! a ping or the URL after a form post.

use std::fmt::Debug;
use std::future::Future;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use thirtyfour::error::{WebDriverError, WebDriverErrorInner};

use crate::browser::{WAIT_INTERVAL, WAIT_TIMEOUT};

/// Is this the DOM having moved under the probe, rather than a real fault?
///
/// The check page swaps its pings and notifications sections in place, so an
/// element found on one poll can be detached before the next line reads it — to
/// a poll that is "not yet". Only the stale-reference error is forgiven; a
/// missing element, bad selector or dead session still fails immediately.
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

/// Polls `probe` until it reports the expected value. On timeout the failure
/// names the last value seen, not merely that a wait expired.
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

/// [`eventually`] with a deadline of its own, for the waits on a background
/// loop rather than on the page — chiefly the scan loop, which only transitions
/// an overdue check on its next pass.
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

/// Polls `probe` until it reports a value, handing it back — "read something
/// once it exists".
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
