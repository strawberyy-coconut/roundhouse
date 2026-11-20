//! Parity: the generated documentation slice and the linked harness must agree.
//!
//! The linked mode compiles the application as a dependency. The slice mode copies the
//! documentation surface into a generated package. Both run the same contract builder, so the
//! committed artifacts must be identical for the same source.

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

fn copy_tree(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("create destination");
    for entry in fs::read_dir(source).expect("read source directory") {
        let entry = entry.expect("read entry");
        let target = destination.join(entry.file_name());
        if entry.file_type().expect("file type").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("copy file");
        }
    }
}

fn prepare(dir: &Path, mode: &str) {
    let status = Command::new(cli())
        .args(["prepare"])
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env("ROUNDHOUSE_HARNESS", mode)
        .env("ROUNDHOUSE_AUTO_CONTRACT", "1")
        .status()
        .expect("run cargo roundhouse");
    assert!(
        status.success(),
        "`cargo roundhouse prepare` ({mode}) failed"
    );
}

#[test]
fn the_slice_and_the_linked_harness_agree() {
    if std::env::var_os("ROUNDHOUSE_SKIP_SLOW_TESTS").is_some() {
        return;
    }

    let root = workspace_root();
    let dir = temp_project("slice-parity");
    copy_tree(&root.join("examples/notes/src"), &dir.join("src"));
    // Prose is a compile-time include: the slice must copy the referenced file to the same
    // relative location.
    fs::create_dir_all(dir.join("docs")).expect("create docs");
    fs::write(
        dir.join("docs/overview.md"),
        "Health and notes.\n\nThe generated document carries this prose.\n",
    )
    .expect("write prose");
    let routes_path = dir.join("src/routes/mod.rs");
    let routes = fs::read_to_string(&routes_path)
        .expect("read routes")
        .replace(
            "get(health::healthz, tags = [\"ops\"]),",
            "get(health::healthz, tags = [\"ops\"], prose = include_str!(\"../../docs/overview.md\")),",
        );
    fs::write(&routes_path, routes).expect("write routes");
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "notes-parity"
version = "0.1.0"
edition = "2024"
publish = false

[workspace]

[dependencies]
roundhouse = {{ path = {root:?}, default-features = false, features = ["build"] }}
serde = {{ version = "1", features = ["derive"] }}
tower-http = {{ version = "0.7", features = ["trace"] }}
"#,
            root = root.display().to_string()
        ),
    )
    .expect("write manifest");

    prepare(&dir, "linked");
    let linked_contract =
        fs::read_to_string(dir.join(".roundhouse/contract.json")).expect("read linked contract");
    let linked_openapi =
        fs::read_to_string(dir.join(".roundhouse/openapi.json")).expect("read linked openapi");

    prepare(&dir, "slice");
    let sliced_contract =
        fs::read_to_string(dir.join(".roundhouse/contract.json")).expect("read sliced contract");
    let sliced_openapi =
        fs::read_to_string(dir.join(".roundhouse/openapi.json")).expect("read sliced openapi");

    let linked_contract: serde_json::Value =
        serde_json::from_str(&linked_contract).expect("linked contract JSON");
    let sliced_contract: serde_json::Value =
        serde_json::from_str(&sliced_contract).expect("sliced contract JSON");
    assert_eq!(linked_contract, sliced_contract, "contracts agree");

    let linked_openapi: serde_json::Value =
        serde_json::from_str(&linked_openapi).expect("linked OpenAPI JSON");
    let sliced_openapi: serde_json::Value =
        serde_json::from_str(&sliced_openapi).expect("sliced OpenAPI JSON");
    assert_eq!(linked_openapi, sliced_openapi, "OpenAPI documents agree");

    let _ = fs::remove_dir_all(&dir);
}
