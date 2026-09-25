//! Stamps the binary with the commit it was built from.
//!
//! `ODS_GIT_SHA` is `HEAD`'s commit. `ODS_GIT_DIRTY=true` is set when the files
//! that go into the binary differ from that commit. Neither is set outside a
//! git checkout.

use std::path::Path;
use std::process::Command;

/// Everything that goes into the binary. Changes anywhere else (docs, tests,
/// the site) neither mark the build dirty nor rerun this script.
const INPUTS: &[&str] = &["src", "build.rs", "Cargo.toml", "Cargo.lock", "data"];

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let stdout = String::from_utf8(output.stdout).ok()?;
    output.status.success().then(|| stdout.trim().to_string())
}

/// Cargo treats a missing path as always changed, so only watch paths that exist.
fn watch(path: &Path) {
    if path.exists() {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

fn main() {
    for input in INPUTS {
        println!("cargo:rerun-if-changed={input}");
    }

    // Resolve locations with git: in a worktree `.git` is a file, and refs
    // live in the common dir rather than the per-worktree one.
    let (Some(git_dir), Some(common_dir)) = (
        git(&["rev-parse", "--git-dir"]),
        git(&["rev-parse", "--git-common-dir"]),
    ) else {
        return;
    };
    watch(&Path::new(&git_dir).join("HEAD"));
    watch(&Path::new(&common_dir).join("packed-refs"));
    // The branch `HEAD` points at. If its ref is only in `packed-refs`, watch
    // its directory (`refs/heads`) so the loose file a commit writes is caught.
    if let Some(branch) = git(&["symbolic-ref", "-q", "HEAD"]) {
        let loose = Path::new(&common_dir).join(branch);
        let dir = loose.parent().unwrap_or(&loose);
        watch(if loose.exists() { &loose } else { dir });
    }

    if let Some(sha) = git(&["rev-parse", "HEAD"]).filter(|s| !s.is_empty()) {
        println!("cargo:rustc-env=ODS_GIT_SHA={sha}");
    }

    // Untracked files count: a new file under `src/` or `data/` that the build
    // picks up means the binary no longer matches the commit.
    let mut status = vec!["status", "--porcelain", "--"];
    status.extend(INPUTS);
    if git(&status).is_some_and(|s| !s.is_empty()) {
        println!("cargo:rustc-env=ODS_GIT_DIRTY=true");
    }
}
