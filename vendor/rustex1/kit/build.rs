//! Bakes the compiler version and build stamp into the wasm binary so the page
//! can prove *which* Rust build you are currently looking at.

use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src");
    println!("cargo:rerun-if-changed=Cargo.toml");

    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let version = Command::new(&rustc)
        .arg("-V")
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_else(|| "rustc ?".into());
    println!("cargo:rustc-env=RUSTC_VERSION={version}");

    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().to_string())
        .unwrap_or_else(|_| "?".into());
    println!("cargo:rustc-env=BUILD_TIME={stamp}");
}
