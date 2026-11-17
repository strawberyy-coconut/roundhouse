//! End-to-end: a fresh app with named response types and one explicit declaration.
//!
//! The test writes a temporary crate, runs the real CLI, and checks that the response traits fill
//! in the 200/201/default responses without response enums or inferred statuses.

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
fn plain_handlers_are_documented_end_to_end() {
    let dir = temp_project("plain");
    let root = workspace_root();
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "plain-app"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
roundhouse = {{ path = {root:?}, default-features = false, features = ["contract-runtime"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
            root = root.display().to_string()
        ),
    )
    .expect("write manifest");
    fs::write(
        dir.join("src/lib.rs"),
        r#"use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct AppState;

impl AppState {
    fn load(&self, id: u64) -> Note {
        Note { id }
    }
}

roundhouse::app_state!(AppState);

roundhouse::api! {
    state = AppState,
    title = "Plain App",
    version = "0.1.0",
    root = routes::MODULE,
    register = [Note],
}

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Note {
    pub id: u64,
}

pub mod routes {
    use super::*;

    routes! {
        "/plain" => { get(list), post(create) },
        "/plain/{id}" => { get(show) },
        "/plain/{id}/note" => { get(note) },
    }

    #[handler]
    pub async fn list() -> Json<serde_json::Value> {
        Json(serde_json::json!([]))
    }

    #[handler]
    pub async fn create(Json(body): Json<serde_json::Value>) -> Created<serde_json::Value> {
        Created(body)
    }

    #[handler(response(404, Problem))]
    pub async fn show(Path(id): Path<u64>) -> Result<Json<serde_json::Value>, Problem> {
        if id == 0 {
            Err(Problem::not_found("missing"))
        } else {
            Ok(Json(serde_json::json!({ "id": id })))
        }
    }

    #[handler]
    pub async fn note(State(app): State<AppState>, Path(id): Path<u64>) -> Json<Note> {
        Json(app.load(id))
    }
}
"#,
    )
    .expect("write lib");

    run(&dir, &["init"]);
    let manifest = fs::read_to_string(dir.join("Cargo.toml")).expect("read manifest");
    assert!(
        manifest.contains("[build-dependencies]") && manifest.contains("roundhouse-cli"),
        "init adds the build-dependency: {manifest}"
    );
    assert!(
        !dir.join("src/bin").exists(),
        "init must not create a source-tree bin"
    );
    assert!(
        dir.join("build.rs").is_file(),
        "init generates the automatic contract build script"
    );
    assert!(
        dir.join(".roundhouse").is_dir(),
        "init creates the artifact directory"
    );

    // The generated build script must produce the contract on a normal build.
    cargo(&dir, &["check"]);
    assert!(
        dir.join(".roundhouse/contract.json").is_file(),
        "the build script generated the contract"
    );

    run(&dir, &["prepare"]);
    run(&dir, &["check"]);

    let contract: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join(".roundhouse/contract.json")).expect("read contract"),
    )
    .expect("contract is JSON");
    let operations = contract["operations"].as_array().expect("operations");

    let responses = |method: &str, path: &str| {
        operations
            .iter()
            .find(|operation| operation["method"] == method && operation["path"] == path)
            .map(|operation| {
                operation["responses"]
                    .as_array()
                    .expect("responses")
                    .clone()
            })
            .unwrap_or_else(|| panic!("missing operation {method} {path}"))
    };
    let statuses = |method: &str, path: &str| {
        responses(method, path)
            .iter()
            .filter_map(|response| response["status"].as_u64())
            .collect::<Vec<_>>()
    };
    assert!(statuses("GET", "/plain").contains(&200));
    assert!(statuses("POST", "/plain").contains(&201));
    assert!(statuses("GET", "/plain/{id}").contains(&200));
    assert!(statuses("GET", "/plain/{id}").contains(&404));
    assert!(statuses("GET", "/plain/{id}/note").contains(&200));
    let note = responses("GET", "/plain/{id}/note")
        .into_iter()
        .find(|response| response["status"] == 200)
        .expect("note response");
    assert_eq!(
        note["content"]["application/json"]["schema"]["$ref"],
        "#/components/schemas/Note"
    );

    let explain = Command::new(cli())
        .args(["explain", "GET /plain/{id}"])
        .current_dir(&dir)
        .output()
        .expect("run explain");
    let text = String::from_utf8_lossy(&explain.stdout);
    assert!(text.contains("200"), "explain output: {text}");
    assert!(text.contains("404"), "explain output: {text}");

    fs::remove_dir_all(&dir).expect("clean the temporary project");
}
