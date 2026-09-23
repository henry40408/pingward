use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Re-stamp on checkout (HEAD), commit (the branch ref) and `git add`
    // (index). Resolved through git because in a worktree `.git` is a file.
    let mut watched = vec![git_path("HEAD"), git_path("index"), git_path("packed-refs")];
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        watched.push(git_path(&branch));
    }
    for path in watched.into_iter().flatten() {
        println!("cargo:rerun-if-changed={path}");
    }
    println!("cargo:rerun-if-env-changed=GIT_VERSION");

    println!("cargo:rustc-env=GIT_VERSION={}", git_version());
}

/// Version shown in the UI footer. Not `CARGO_PKG_VERSION`: releases are git
/// tags and `Cargo.toml`'s `version` is never bumped.
///
/// An explicit `GIT_VERSION` wins (the Docker build has no `.git`); `dev`, the
/// Dockerfile's default, counts as unset.
fn git_version() -> String {
    if let Ok(version) = std::env::var("GIT_VERSION")
        && !version.is_empty()
        && version != "dev"
    {
        return version;
    }

    git(&["describe", "--tags", "--always", "--dirty"]).unwrap_or_else(|| "dev".to_string())
}

fn git_path(name: &str) -> Option<String> {
    git(&["rev-parse", "--git-path", name])
}

fn git(args: &[&str]) -> Option<String> {
    Command::new("git")
        .args(args)
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
}
