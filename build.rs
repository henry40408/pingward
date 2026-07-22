use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Re-stamp on a new commit, and on `git add` so `--dirty` stays honest.
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-env-changed=GIT_VERSION");

    println!("cargo:rustc-env=GIT_VERSION={}", git_version());
}

/// The version string stamped into the binary and rendered in the UI footer.
///
/// Deliberately **not** `CARGO_PKG_VERSION`: releases are cut with
/// `gh release create`, so the git tag is the source of truth and
/// `Cargo.toml`'s `version` is never bumped.
///
/// An explicit `GIT_VERSION` wins — the release image builds from a context
/// with no `.git` (see `.dockerignore`), so the workflow passes the describe
/// output in as a build arg. A literal `dev` counts as unset so the
/// `Dockerfile`'s own default can fall through to a local `git describe`.
fn git_version() -> String {
    if let Ok(version) = std::env::var("GIT_VERSION")
        && !version.is_empty()
        && version != "dev"
    {
        return version;
    }

    Command::new("git")
        .args(["describe", "--tags", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map_or_else(
            || "dev".to_string(),
            |o| String::from_utf8_lossy(&o.stdout).trim().to_string(),
        )
}
