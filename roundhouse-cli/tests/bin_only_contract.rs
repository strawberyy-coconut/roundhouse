//! End-to-end: a package with no library target generates a contract.
//!
//! The harness must not depend on the application package, because a binary target cannot be a
//! dependency. The generated documentation slice is the only path that can work here.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn cli() -> &'static str {
    env!("CARGO_BIN_EXE_cargo-roundhouse")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the CLI lives inside the workspace")
        .to_path_buf()
}

fn temp_project(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roundhouse-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("src")).expect("create src");
    dir
}

fn run(dir: &Path, args: &[&str]) {
    let status = Command::new(cli())
        .args(args)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .status()
        .expect("run cargo roundhouse");
    assert!(status.success(), "`cargo roundhouse {args:?}` failed");
}

fn cargo(dir: &Path, args: &[&str]) {
    let status = Command::new("cargo")
        .args(args)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .status()
        .expect("run cargo");
    assert!(status.success(), "`cargo {args:?}` failed");
}

#[test]
fn a_bin_only_package_generates_a_contract_without_depending_on_itself() {
    if std::env::var_os("ROUNDHOUSE_SKIP_SLOW_TESTS").is_some() {
        return;
    }

    let dir = temp_project("bin-only-contract");
    let root = workspace_root();
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "bin-only-contract"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
roundhouse = {{ path = {root:?}, default-features = false, features = ["build"] }}

[build-dependencies]
roundhouse-build = {{ path = {build:?} }}
"#,
            root = root.display().to_string(),
            build = root.join("roundhouse-build").display().to_string()
        ),
    )
    .expect("write manifest");
    fs::write(
        dir.join("build.rs"),
        r#"fn main() {
    println!("cargo:rerun-if-changed=src");
    roundhouse_build::prepare().expect("roundhouse contract generation failed");
}
"#,
    )
    .expect("write build script");
    fs::write(
        dir.join("src/main.rs"),
        r#"use roundhouse::prelude::*;

roundhouse::app_state!(());

roundhouse::api! {
    state = (),
    title = "Bin only",
    version = "0.1.0",
    root = crate::MODULE,
}

routes! {
    "/ping" => {
        get(ping),
    },
}

#[handler]
fn ping() -> &'static str {
    "pong"
}

fn main() {
    println!("the application binary");
}
"#,
    )
    .expect("write main");

    run(&dir, &["prepare"]);

    let contract_path = dir.join(".roundhouse/contract.json");
    assert!(contract_path.is_file(), "the contract was written");
    let contract = fs::read_to_string(&contract_path).expect("read contract");
    assert!(
        contract.contains("/ping"),
        "the route is in the contract: {contract}"
    );

    let slice = dir.join("target/roundhouse/slice/Cargo.toml");
    assert!(slice.is_file(), "the slice package was generated");
    let manifest = fs::read_to_string(&slice).expect("read slice manifest");
    assert!(
        manifest.contains("roundhouse"),
        "the slice depends on roundhouse"
    );
    assert!(
        !manifest.contains("bin-only-contract ="),
        "the slice does not depend on the application: {manifest}"
    );
    assert!(
        !manifest.contains("bin_only_contract = {"),
        "the slice does not depend on the application under its crate name: {manifest}"
    );

    cargo(&dir, &["build"]);
    run(&dir, &["check"]);

    let _ = fs::remove_dir_all(&dir);
}
