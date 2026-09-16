use std::fs;
use std::path::Path;
use std::process::Command;

fn track_git_refs(refs_dir: &Path) {
    if !refs_dir.exists() {
        return;
    }
    println!("cargo:rerun-if-changed={}", refs_dir.display());
    if let Ok(entries) = fs::read_dir(refs_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                track_git_refs(&path);
            } else {
                println!("cargo:rerun-if-changed={}", path.display());
            }
        }
    }
}

fn main() {
    let git_dir = Path::new(".git");
    if git_dir.exists() {
        println!("cargo:rerun-if-changed=.git/HEAD");
        println!("cargo:rerun-if-changed=.git/index");
        println!("cargo:rerun-if-changed=.git/packed-refs");
        track_git_refs(&git_dir.join("refs"));
    }
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");

    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output() {
        if output.status.success() {
            let sha = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !sha.is_empty() {
                println!("cargo:rustc-env=ODS_GIT_SHA={}", sha);
            }
        }
    }

    if let Ok(output) = Command::new("git").args(["status", "--porcelain"]).output() {
        if output.status.success() {
            let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !status.is_empty() {
                println!("cargo:rustc-env=ODS_GIT_DIRTY=true");
            }
        }
    }
}
