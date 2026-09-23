//! Retrying assertions for values that must settle (a status after a ping,
//! the URL after a post): `WebDriver` does not wait, so a `find` before a
//! redirect lands reads the old page.

use std::fmt::Debug;
use std::future::Future;
use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use thirtyfour::error::{WebDriverError, WebDriverErrorInner};

use crate::browser::{WAIT_INTERVAL, WAIT_TIMEOUT};

/// A stale element reference counts as "not yet": the check page swaps its
/// sections in place, detaching elements mid-poll. Every other error is fatal.
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

/// Polls `probe` until it equals `expected`; a timeout names the last value.
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
pub async fn eventually<F, Fut>(what: &str, probe: F) -> Result<()>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<bool>>,
{
    eventually_within(WAIT_TIMEOUT, what, probe).await
}

/// [`eventually`] with its own deadline, for waits on a background loop such
/// as the scan loop.
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

/// Polls `probe` until it returns `Some`, and hands the value back.
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
