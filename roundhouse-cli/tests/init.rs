//! `cargo roundhouse init` and `cargo roundhouse new` scaffold Rust-convention packages.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn cli() -> &'static str {
    env!("CARGO_BIN_EXE_cargo-roundhouse")
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the CLI lives inside the workspace")
        .to_path_buf()
}

fn temp_dir(label: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "roundhouse-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    ));
    let _ = fs::remove_dir_all(&dir);
    dir
}

fn run(directory: &Path, args: &[&str]) -> Output {
    Command::new(cli())
        .args(args)
        .current_dir(directory)
        .env("CARGO_TARGET_DIR", directory.join("target"))
        .output()
        .expect("run cargo roundhouse")
}

fn write_manifest(directory: &Path, name: &str) {
    fs::create_dir_all(directory.join("src")).expect("create src");
    fs::write(
        directory.join("Cargo.toml"),
        format!(
            "[package]\n\
             name = {name:?}\n\
             version = \"0.1.0\"\n\
             edition = \"2024\"\n\
             \n\
             [dependencies]\n\
             roundhouse = {{ path = {:?}, features = [\"contract\"] }}\n",
            workspace_root().display().to_string()
        ),
    )
    .expect("write manifest");
}

fn point_at_workspace(directory: &Path) {
    let manifest_path = directory.join("Cargo.toml");
    let mut manifest: toml_edit::DocumentMut = fs::read_to_string(&manifest_path)
        .expect("read manifest")
        .parse()
        .expect("parse manifest");
    let root = workspace_root();

    for (table, name) in [
        ("dependencies", "roundhouse"),
        ("dev-dependencies", "roundhouse"),
        ("build-dependencies", "roundhouse-build"),
    ] {
        let Some(dependency) = manifest
            .get_mut(table)
            .and_then(toml_edit::Item::as_table_mut)
            .and_then(|table| table.get_mut(name))
            .and_then(toml_edit::Item::as_inline_table_mut)
        else {
            continue;
        };
        dependency.remove("version");
        let path = if name == "roundhouse-build" {
            root.join("roundhouse-build")
        } else {
            root.clone()
        };
        dependency.insert("path", toml_edit::Value::from(path.display().to_string()));
    }

    fs::write(&manifest_path, manifest.to_string()).expect("write manifest");
}

#[test]
fn init_scaffolds_a_new_package() {
    let dir = temp_dir("fresh-init");
    fs::create_dir_all(&dir).expect("create temporary project");

    let output = run(&dir, &["init"]);
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(dir.join("Cargo.toml").is_file(), "manifest exists");
    assert!(dir.join("build.rs").is_file(), "build script exists");
    assert!(
        dir.join(".roundhouse").is_dir(),
        "artifact directory exists"
    );
    assert!(dir.join("src/lib.rs").is_file(), "lib target exists");
    assert!(dir.join("src/main.rs").is_file(), "bin target exists");

    let library = fs::read_to_string(dir.join("src/lib.rs")).expect("read lib");
    assert!(library.contains("roundhouse::api!"));
    assert!(library.contains("pub fn app"));
    let main = fs::read_to_string(dir.join("src/main.rs")).expect("read main");
    assert!(main.contains(".serve("));
    let manifest = fs::read_to_string(dir.join("Cargo.toml")).expect("read manifest");
    assert!(manifest.contains("roundhouse-build"));
    assert!(manifest.contains("contract"));

    // The scaffold must build against this checkout, including its first contract build, which
    // happens before `.roundhouse/` exists.
    point_at_workspace(&dir);
    let status = Command::new("cargo")
        .arg("check")
        .current_dir(&dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .status()
        .expect("run cargo check");
    assert!(status.success(), "the generated package builds");

    let contract: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join(".roundhouse/contract.json")).expect("read contract"),
    )
    .expect("contract is JSON");
    let operations = contract["operations"].as_array().expect("operations");
    assert!(
        operations
            .iter()
            .any(|operation| operation["method"] == "GET" && operation["path"] == "/hello"),
        "the template's route is documented: {operations:?}"
    );

    fs::remove_dir_all(&dir).expect("clean the temporary project");
}

#[test]
fn new_creates_the_directory_before_scaffolding() {
    let parent = temp_dir("new-parent");
    fs::create_dir_all(&parent).expect("create temporary parent");
    let target = parent.join("hello-api");

    let output = run(&parent, &["new", target.to_str().expect("utf-8 path")]);
    assert!(
        output.status.success(),
        "new failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(target.join("Cargo.toml").is_file(), "manifest exists");
    assert!(target.join("src/lib.rs").is_file(), "lib target exists");
    assert!(target.join("src/main.rs").is_file(), "bin target exists");
    assert!(target.join("build.rs").is_file(), "build script exists");
    let main = fs::read_to_string(target.join("src/main.rs")).expect("read main");
    assert!(main.contains("hello_api::app"));

    fs::remove_dir_all(&parent).expect("clean the temporary parent");
}

#[test]
fn init_accepts_a_bin_only_package() {
    let dir = temp_dir("bin-only");
    fs::create_dir_all(dir.join("src")).expect("create src");
    fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"bin-only\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )
    .expect("write manifest");
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("write main");

    let output = run(&dir, &["init"]);
    assert!(
        output.status.success(),
        "a bin-only package is accepted: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        dir.join("build.rs").is_file(),
        "the build script was written"
    );
    assert!(
        dir.join(".roundhouse").is_dir(),
        "the artifact directory was created"
    );
    let manifest = fs::read_to_string(dir.join("Cargo.toml")).expect("read manifest");
    assert!(
        manifest.contains("roundhouse-build"),
        "the build dependency was added"
    );

    fs::remove_dir_all(&dir).expect("clean the temporary project");
}

#[test]
fn init_preserves_existing_sources() {
    let dir = temp_dir("existing-sources");
    write_manifest(&dir, "existing-sources");
    fs::write(
        dir.join("src/lib.rs"),
        "pub fn app() -> &'static str { \"custom\" }\n",
    )
    .expect("write lib");
    fs::write(dir.join("src/main.rs"), "fn main() {}\n").expect("write main");

    let output = run(&dir, &["init"]);
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let library = fs::read_to_string(dir.join("src/lib.rs")).expect("read lib");
    assert_eq!(library, "pub fn app() -> &'static str { \"custom\" }\n");
    let main = fs::read_to_string(dir.join("src/main.rs")).expect("read main");
    assert_eq!(main, "fn main() {}\n");
    assert!(dir.join("build.rs").is_file(), "build script was added");

    fs::remove_dir_all(&dir).expect("clean the temporary project");
}

#[test]
fn init_without_build_script_leaves_the_build_hook_alone() {
    let dir = temp_dir("no-build-script");
    write_manifest(&dir, "no-build-script");
    fs::write(dir.join("src/lib.rs"), "").expect("write lib");

    let output = run(&dir, &["init", "--no-build-script"]);
    assert!(
        output.status.success(),
        "init failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert!(
        !dir.join("build.rs").exists(),
        "no build script was written"
    );
    let manifest = fs::read_to_string(dir.join("Cargo.toml")).expect("read manifest");
    assert!(!manifest.contains("roundhouse-build"));
    assert!(dir.join(".roundhouse").is_dir());

    fs::remove_dir_all(&dir).expect("clean the temporary project");
}
