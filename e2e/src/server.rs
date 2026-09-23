//! The server under test: one process and throwaway `SQLite` file per
//! scenario, since `POST /setup` works only once. Spawned directly, not via
//! `cargo run`, so killing the PID kills the server itself.

use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::process::{Child, Command};

const STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

/// The server environment a scenario's tags select.
#[derive(Debug, Default, Clone)]
pub struct Options {
    /// `PINGWARD_SCAN_INTERVAL`, for `@fast-scan`: on a fresh DB the scan
    /// loop's first sleep is the env default (30 s), since no per-check
    /// override exists yet.
    pub scan_interval_secs: Option<u64>,
    /// For `@smtp-env` and `@trusted-proxy`.
    pub extra_env: Vec<(String, String)>,
}

impl Options {
    pub fn from_tags(tags: &[String]) -> Self {
        let tagged = |name: &str| tags.iter().any(|tag| tag == name);
        let mut options = Self::default();
        if tagged("fast-scan") {
            options.scan_interval_secs = Some(1);
        }
        // Gives `/admin`'s Environment card an SMTP config, and a password it
        // must not print.
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
        // So `auth::client_ip` honours the scenario's `X-Forwarded-For`.
        if tagged("trusted-proxy") {
            options.extra_env.push((
                "PINGWARD_TRUSTED_PROXIES".to_owned(),
                "127.0.0.1".to_owned(),
            ));
        }
        options
    }
}

/// A running server and its database, both torn down on drop.
#[derive(Debug)]
pub struct Server {
    base_url: String,
    child: Child,
    // Dropping removes the test database's directory.
    _temp: tempfile::TempDir,
}

impl Server {
    /// Starts a server against a fresh database and waits for `/healthz`.
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
            // Pinned to silence the random-secret startup warning.
            .env("PINGWARD_SECRET", "pingward-e2e-secret-0123456789abcdef")
            // Teardown kills it mid-request; the noise would bury real failures.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            // `Drop` below issues the kill itself.
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

        // Built before the wait so a server that never answers is still killed.
        let server = Self {
            base_url,
            child,
            _temp: temp,
        };
        server.wait_until_healthy().await?;
        Ok(server)
    }

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
        // Issued before the temp directory is removed; reaped later by tokio.
        let _ = self.child.start_kill();
    }
}

/// Path to the dev-profile server binary, building it if missing. Dev, not
/// release: release uses `lto = true`/`codegen-units = 1` and shares no
/// artefacts with `cargo nextest run`.
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

/// An unused TCP port; racy, as it is released before the server binds it.
pub fn free_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0").context("probing for a free port")?;
    Ok(listener.local_addr()?.port())
}

/// The repository root (parent of `e2e/`).
pub fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("e2e/ always has a parent")
}
