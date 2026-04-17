use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=RENDER_GIT_COMMIT");
    println!("cargo:rerun-if-env-changed=GIT_SHA_OVERRIDE");

    // Docker build contexts don't include .git, so `git rev-parse` fails on Render.
    // Render injects RENDER_GIT_COMMIT as a build ARG — prefer that, fall back to
    // GIT_SHA_OVERRIDE for other CI, then to a local git command for dev builds.
    let sha = std::env::var("GIT_SHA_OVERRIDE")
        .ok()
        .or_else(|| std::env::var("RENDER_GIT_COMMIT").ok())
        .map(|s| s.chars().take(12).collect::<String>())
        .or_else(|| git_output(&["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".into());
    let dirty = match git_output(&["status", "--porcelain"]) {
        Some(s) if !s.trim().is_empty() => "-dirty",
        _ => "",
    };
    println!("cargo:rustc-env=BUILD_GIT_SHA={sha}{dirty}");

    let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    println!("cargo:rustc-env=BUILD_TIME_UNIX={now}");
}

fn git_output(args: &[&str]) -> Option<String> {
    let out = Command::new("git").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
