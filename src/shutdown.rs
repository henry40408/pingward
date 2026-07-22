//! Cooperative shutdown: one broadcast flag shared by the HTTP server and the
//! background loops, plus the OS-signal listener that raises it.
//!
//! The flag is a `tokio::sync::watch` rather than a oneshot because every
//! consumer needs to observe the *same* request: axum's
//! `with_graceful_shutdown` takes one future, and each background loop selects
//! on another.

use tokio::sync::watch;

/// The sending half, held by `main`. Dropping it counts as a shutdown request —
/// a lost controller must never leave the background loops running forever.
pub struct ShutdownTx(watch::Sender<bool>);

/// The receiving half. Cheap to clone: hand one to every task that must stop.
#[derive(Clone)]
pub struct Shutdown(watch::Receiver<bool>);

/// Build a linked `(ShutdownTx, Shutdown)` pair, initially not shut down.
pub fn channel() -> (ShutdownTx, Shutdown) {
    let (tx, rx) = watch::channel(false);
    (ShutdownTx(tx), Shutdown(rx))
}

impl ShutdownTx {
    /// Raise the flag. Idempotent — the value stays observable after this
    /// handle drops, so a late `Shutdown::wait` still resolves.
    pub fn trigger(&self) {
        let _ = self.0.send(true);
    }
}

impl Shutdown {
    /// Resolve once shutdown has been requested — immediately if it already
    /// has, so a task that starts late still stops. Cancel-safe, so it serves
    /// both as a `tokio::select!` branch and as axum's
    /// `with_graceful_shutdown` future.
    pub async fn wait(&self) {
        // `wait_for` inspects the current value before parking, which is what
        // makes the already-triggered case resolve instead of hanging (plain
        // `changed()` only fires on a *new* value). `Err` means the
        // `ShutdownTx` was dropped, which counts as a request.
        let mut rx = self.0.clone();
        let _ = rx.wait_for(|down| *down).await;
    }
}

/// Resolve on the first SIGTERM (`docker stop`, systemd) or SIGINT (Ctrl-C).
///
/// Installing a handler is mandatory in the container image, not merely polite:
/// the exec-form `ENTRYPOINT` makes pingward PID 1, and Linux discards any
/// signal whose disposition is still the default for PID 1. Without a handler,
/// SIGTERM is silently ignored and `docker compose down` waits out its full 10s
/// grace period before resorting to SIGKILL.
///
/// A listener that fails to install is logged and then pends forever rather
/// than resolving, so a broken SIGINT registration cannot masquerade as a
/// shutdown request and stop the server on its own.
pub async fn os_signal() {
    let interrupt = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!("failed to install the SIGINT handler: {e}");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        use tokio::signal::unix::{SignalKind, signal};
        match signal(SignalKind::terminate()) {
            Ok(mut sigterm) => {
                sigterm.recv().await;
            }
            Err(e) => {
                tracing::error!("failed to install the SIGTERM handler: {e}");
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!("received SIGINT"),
        () = terminate => tracing::info!("received SIGTERM"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, timeout};

    /// The common case: a task parked on `wait` wakes when `main` triggers.
    #[tokio::test]
    async fn wait_resolves_after_trigger() {
        let (tx, shutdown) = channel();
        let waiter = tokio::spawn(async move { shutdown.wait().await });
        tx.trigger();
        timeout(Duration::from_secs(5), waiter)
            .await
            .expect("wait() must resolve once shutdown is triggered")
            .unwrap();
    }

    /// A receiver cloned (or first polled) *after* the trigger must still
    /// resolve — otherwise a task spawned during shutdown would run forever.
    #[tokio::test]
    async fn wait_resolves_immediately_when_already_triggered() {
        let (tx, shutdown) = channel();
        tx.trigger();
        timeout(Duration::from_secs(5), shutdown.clone().wait())
            .await
            .expect("wait() must resolve for a receiver that starts after the trigger");
    }

    /// Losing the controller is fail-closed: the loops stop rather than
    /// outliving whoever was supposed to stop them.
    #[tokio::test]
    async fn wait_resolves_when_sender_dropped() {
        let (tx, shutdown) = channel();
        drop(tx);
        timeout(Duration::from_secs(5), shutdown.wait())
            .await
            .expect("wait() must resolve when the ShutdownTx is dropped");
    }

    /// The other side of the assertion: `wait` must *not* resolve on its own,
    /// or every `select!` guarding a sleep would exit on the first poll.
    #[tokio::test]
    async fn wait_pends_until_triggered() {
        let (_tx, shutdown) = channel();
        assert!(
            timeout(Duration::from_millis(200), shutdown.wait())
                .await
                .is_err(),
            "wait() must not resolve before a trigger"
        );
    }
}
