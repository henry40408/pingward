//! The pingward server under test: one fresh binary and one throwaway
//! `SQLite` file per scenario.
//!
//! A shared server is not available — every scenario bootstraps the first admin
//! through the one-time `POST /setup`, which a second scenario would find
//! already closed. The binary is spawned directly rather than through
//! `cargo run`, so the PID held here is the server's own; killing `cargo` would
//! leave the server it spawned holding the port.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::process::{Child, Command};

/// How long to wait for `/healthz` to answer.
const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// The environment a scenario's server starts with, as its tags select it.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// `PINGWARD_SCAN_INTERVAL`, for `@fast-scan`.
    ///
    /// The scan loop's *first* post-startup sleep is the env default (~30 s)
    /// whatever a per-check override says, so scenarios waiting for a check to
    /// go down must shorten it here.
    pub scan_interval_secs: Option<u64>,
    /// Extra environment, for `@smtp-env` and `@trusted-proxy`.
    pub extra_env: Vec<(String, String)>,
}

impl Options {
    /// The options a scenario's tags ask for.
    pub fn from_tags(tags: &[String]) -> Self {
        let tagged = |name: &str| tags.iter().any(|tag| tag == name);
        let mut options = Self::default();
        if tagged("fast-scan") {
            options.scan_interval_secs = Some(1);
        }
        // Gives the `/admin` Environment card's SMTP group something to report
        // as configured, and a password it must report without printing.
        if tagged("smtp-env") {
            options.extra_env.extend([
                ("PINGWARD_SMTP_HOST".to_owned(), "smtp.e2e.test".to_owned()),
                (
                    "PINGWARD_SMTP_FROM".to_owned(),
                    "alerts@e2e.test".to_owned(),
                ),
                (
                    "PINGWARD_SMTP_PASSWORD".to_owned(),
                    "e2e-secret-password".to_owned(),
                ),
            ]);
        }
        // Trusts the harness's loopback address, so `auth::client_ip` honours
        // the scenario's `X-Forwarded-For` instead of recording the peer.
        if tagged("trusted-proxy") {
            options.extra_env.push((
                "PINGWARD_TRUSTED_PROXIES".to_owned(),
                "127.0.0.1".to_owned(),
            ));
        }
        options
    }
}

/// A running pingward server and the database behind it. Both are torn down
/// when this is dropped.
#[derive(Debug)]
pub struct Server {
    base_url: String,
    child: Child,
    // Held for its Drop: removes the directory containing the test database.
    _temp: tempfile::TempDir,
}

impl Server {
    /// Starts a server against a fresh database and waits for `/healthz`.
    ///
    /// # Errors
    ///
    /// Fails when the binary cannot be built or spawned, or when the server
    /// does not answer within [`STARTUP_TIMEOUT`].
    pub async fn start(options: &Options) -> Result<Self> {
        let binary = ensure_binary()?;
        let temp = tempfile::Builder::new()
            .prefix("pingward-e2e-")
            .tempdir()
            .context("creating the temporary directory for the test database")?;
        let db_path = temp.path().join("test.sqlite3");

        let port = free_port()?;
        let base_url = format!("http://127.0.0.1:{port}");

        let mut command = Command::new(&binary);
        command
            .current_dir(repo_root())
            .env(
                "DATABASE_URL",
                format!("sqlite://{}?mode=rwc", db_path.display()),
            )
            .env("PINGWARD_BIND", format!("127.0.0.1:{port}"))
            .env("PINGWARD_BASE_URL", &base_url)
            .env("RUST_LOG", "warn")
            // Unset, this is randomised per process and warned about on every
            // start; pinning it keeps the output to what a failure produced.
            .env("PINGWARD_SECRET", "pingward-e2e-secret-0123456789abcdef")
            // A scenario's server is killed mid-request at teardown, and the
            // resulting noise would bury the one failure worth reading.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // Reaped by `Drop` below, which cannot await — so the child must
            // not be killed by tokio's own async reaper first.
            .kill_on_drop(false);
        if let Some(secs) = options.scan_interval_secs {
            command.env("PINGWARD_SCAN_INTERVAL", secs.to_string());
        }
        for (key, value) in &options.extra_env {
            command.env(key, value);
        }

        let child = command
            .spawn()
            .with_context(|| format!("spawning the pingward server at {}", binary.display()))?;

        // Bound before the wait, so a server that never answers is still
        // killed when the error propagates.
        let server = Self {
            base_url,
            child,
            _temp: temp,
        };
        server.wait_until_healthy().await?;
        Ok(server)
    }

    /// Where the browser and the API helper address this server.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    async fn wait_until_healthy(&self) -> Result<()> {
        let client = reqwest::Client::new();
        let healthz = format!("{}/healthz", self.base_url);
        let deadline = Instant::now() + STARTUP_TIMEOUT;
        while Instant::now() < deadline {
            if let Ok(response) = client.get(&healthz).send().await
                && response.status().is_success()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        bail!("pingward did not answer {healthz} within {STARTUP_TIMEOUT:?}")
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
        // Not awaited — `Drop` cannot — so the child is reaped on the next
        // tokio poll. What matters is that the kill is issued before the
        // temporary directory is removed underneath it.
    }
}

/// Path to the server binary, building it first when it is not there.
///
/// The dev profile: release is tuned for the Docker image (`lto = true`,
/// `codegen-units = 1`), while dev shares artefacts with `cargo nextest run`.
/// CI builds it in an earlier step, so this is the local-developer path.
fn ensure_binary() -> Result<PathBuf> {
    let binary = repo_root().join("target/debug/pingward");
    if binary.is_file() {
        return Ok(binary);
    }

    eprintln!("e2e: {} is missing — building it", binary.display());
    let status = std::process::Command::new("cargo")
        .current_dir(repo_root())
        .arg("build")
        .status()
        .context("running `cargo build`")?;
    if !status.success() {
        bail!("`cargo build` failed with {status}");
    }
    if !binary.is_file() {
        bail!("`cargo build` did not produce {}", binary.display());
    }
    Ok(binary)
}

/// An unused TCP port. Inherently a race: the port is released before the
/// server claims it.
pub fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("probing for a free port")?;
    Ok(listener.local_addr()?.port())
}

/// The repository root — the parent of this crate's directory.
pub fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("e2e/ always has a parent")
}
