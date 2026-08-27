//! Out-of-band HTTP against the server under test.
//!
//! The only bootstrap path is the one-time `POST /setup`, CSRF-protected like
//! every other POST, so this does what a browser does: GET the page first, then
//! submit its cookie and hidden `_csrf` together.

use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use regex::Regex;

/// Which of the `/ping/*` endpoints to hit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PingKind {
    /// The bare ping URL — a successful run.
    Success,
    /// `/fail`.
    Fail,
    /// `/start`.
    Start,
    /// `/log`.
    Log,
    /// `/{code}` — an exit code, where 0 succeeds and anything else fails.
    ExitCode(i32),
}

impl PingKind {
    /// The suffix this kind appends to a check's ping URL.
    fn suffix(self) -> String {
        match self {
            Self::Success => String::new(),
            Self::Fail => "/fail".to_owned(),
            Self::Start => "/start".to_owned(),
            Self::Log => "/log".to_owned(),
            Self::ExitCode(code) => format!("/{code}"),
        }
    }

    /// The kind a feature file names.
    ///
    /// # Errors
    ///
    /// Fails on a name no `/ping/*` endpoint answers.
    pub fn parse(name: &str) -> Result<Self> {
        Ok(match name {
            "success" => Self::Success,
            "fail" => Self::Fail,
            "start" => Self::Start,
            "log" => Self::Log,
            other => match other.strip_prefix("exit ") {
                Some(code) => Self::ExitCode(code.trim().parse()?),
                None => bail!("no ping endpoint is called `{other}`"),
            },
        })
    }
}

/// The HTTP client the steps drive the server with directly.
#[derive(Debug, Clone)]
pub struct Api {
    base_url: String,
    client: reqwest::Client,
}

impl Api {
    /// A client for one server.
    ///
    /// # Errors
    ///
    /// Fails when the client cannot be built.
    pub fn new(base_url: &str) -> Result<Self> {
        Ok(Self {
            base_url: base_url.to_owned(),
            // Redirects are followed by default, so a failed `POST /setup`
            // surfaces as a non-success status rather than a 303.
            client: reqwest::Client::builder()
                .build()
                .context("building the HTTP client")?,
        })
    }

    /// Creates the first admin through `POST /setup`. Only ever run against a
    /// fresh server, so the re-render branches never fire.
    ///
    /// # Errors
    ///
    /// Fails when the setup page yields no session cookie and token, or when
    /// the submission is refused.
    pub async fn bootstrap_admin(&self, username: &str, password: &str) -> Result<()> {
        let page = self
            .client
            .get(format!("{}/setup", self.base_url))
            .send()
            .await
            .context("GET /setup")?;
        let cookie = page
            .headers()
            .get(reqwest::header::SET_COOKIE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .map(str::to_owned);
        let html = page.text().await?;
        let csrf = csrf_re()
            .captures(&html)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str().to_owned());
        let (Some(cookie), Some(csrf)) = (cookie, csrf) else {
            bail!("could not read a session cookie and _csrf token from GET /setup");
        };

        let response = self
            .client
            .post(format!("{}/setup", self.base_url))
            .header(reqwest::header::COOKIE, cookie)
            .form(&[
                ("_csrf", csrf.as_str()),
                ("username", username),
                ("password", password),
            ])
            .send()
            .await
            .context("POST /setup")?;
        let status = response.status();
        if !status.is_success() {
            bail!("bootstrapping the admin `{username}` answered {status}");
        }
        Ok(())
    }

    /// Sends a ping, which must be accepted. Every valid ping answers 200; use
    /// [`Api::ping_status`] for the scenarios that are *about* a refusal.
    ///
    /// # Errors
    ///
    /// Fails when the request cannot be sent, or answers anything but 2xx.
    pub async fn ping(&self, ping_url: &str, kind: PingKind) -> Result<()> {
        let status = self.ping_status(ping_url, kind).await?;
        if !(200..300).contains(&status) {
            bail!("a {kind:?} ping to {ping_url} answered {status}");
        }
        Ok(())
    }

    /// Sends a ping and reports only its status, without treating a refusal as
    /// an error — the unknown-uuid 404 path.
    ///
    /// # Errors
    ///
    /// Fails when the request cannot be sent at all.
    pub async fn ping_status(&self, ping_url: &str, kind: PingKind) -> Result<u16> {
        let target = format!("{ping_url}{}", kind.suffix());
        let response = self
            .client
            .get(&target)
            .send()
            .await
            .with_context(|| format!("pinging {target}"))?;
        Ok(response.status().as_u16())
    }

    /// Sends a ping whose body the check captures.
    ///
    /// # Errors
    ///
    /// Fails when the request cannot be sent, or answers anything but 2xx.
    pub async fn ping_with_body(&self, ping_url: &str, kind: PingKind, body: &str) -> Result<()> {
        let target = format!("{ping_url}{}", kind.suffix());
        let response = self
            .client
            .post(&target)
            .body(body.to_owned())
            .send()
            .await
            .with_context(|| format!("posting a body to {target}"))?;
        let status = response.status();
        if !status.is_success() {
            bail!("a {kind:?} ping with a body to {target} answered {status}");
        }
        Ok(())
    }

    /// GETs a path and reports its status, following no redirects.
    ///
    /// # Errors
    ///
    /// Fails when the request cannot be sent.
    pub async fn status_of(&self, path: &str) -> Result<u16> {
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .get(format!("{}{path}", self.base_url))
            .send()
            .await
            .with_context(|| format!("GET {path}"))?;
        Ok(response.status().as_u16())
    }

    /// POSTs a form and reports its status, following no redirects. Attaches
    /// neither a cookie nor a token — the CSRF scenarios need it that way.
    ///
    /// # Errors
    ///
    /// Fails when the request cannot be sent.
    pub async fn post_form_status(&self, path: &str, form: &[(&str, &str)]) -> Result<u16> {
        let response = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()?
            .post(format!("{}{path}", self.base_url))
            .form(form)
            .send()
            .await
            .with_context(|| format!("POST {path}"))?;
        Ok(response.status().as_u16())
    }
}

/// Matches the hidden CSRF field every pingward form renders.
fn csrf_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"name="_csrf" value="([^"]*)""#).expect("the CSRF pattern compiles")
    })
}
