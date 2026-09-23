//! Cooperative shutdown: one `watch` flag shared by the HTTP server and the
//! background loops (a oneshot has only one receiver), plus the OS-signal listener.

use tokio::sync::watch;

/// Held by `main`. Dropping it counts as a shutdown request.
pub struct ShutdownTx(watch::Sender<bool>);

/// Cheap to clone: hand one to every task that must stop.
#[derive(Clone)]
pub struct Shutdown(watch::Receiver<bool>);

/// A linked pair, initially not shut down.
pub fn channel() -> (ShutdownTx, Shutdown) {
    let (tx, rx) = watch::channel(false);
    (ShutdownTx(tx), Shutdown(rx))
}

impl ShutdownTx {
    /// Idempotent.
    pub fn trigger(&self) {
        let _ = self.0.send(true);
    }
}

impl Shutdown {
    /// Resolve once shutdown has been requested, immediately if it already has.
    /// Cancel-safe, so usable as a `select!` branch.
    pub async fn wait(&self) {
        // `wait_for` checks the current value first (`changed()` would miss an
        // earlier trigger). `Err` means the sender dropped: also a request.
        let mut rx = self.0.clone();
        let _ = rx.wait_for(|down| *down).await;
    }
}

/// Resolve on the first SIGTERM or SIGINT.
///
/// Mandatory in the image: the exec-form `ENTRYPOINT` makes pingward PID 1, and
/// Linux drops default-disposition signals to PID 1, so without a handler
/// `docker compose down` waits out its 10s grace period before SIGKILL.
/// A listener that fails to install pends forever rather than faking a shutdown.
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

    /// Otherwise a task spawned during shutdown would run forever.
    #[tokio::test]
    async fn wait_resolves_immediately_when_already_triggered() {
        let (tx, shutdown) = channel();
        tx.trigger();
        timeout(Duration::from_secs(5), shutdown.clone().wait())
            .await
            .expect("wait() must resolve for a receiver that starts after the trigger");
    }

    /// Fail-closed: the loops stop rather than outliving their controller.
    #[tokio::test]
    async fn wait_resolves_when_sender_dropped() {
        let (tx, shutdown) = channel();
        drop(tx);
        timeout(Duration::from_secs(5), shutdown.wait())
            .await
            .expect("wait() must resolve when the ShutdownTx is dropped");
    }

    /// Otherwise every `select!` guarding a sleep would exit on the first poll.
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
