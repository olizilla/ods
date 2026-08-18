use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

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
