//! End-to-end: opaque `impl IntoResponse` handlers are documented from the body analyzer.
//!
//! The test writes a temporary crate, runs the real CLI, and checks that direct responses,
//! helper calls, documented error types, `.into_documented_response()` and the opaque
//! `.into_response()` opt-out all reach the contract.

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
fn opaque_handlers_are_documented_end_to_end() {
    let dir = temp_project("opaque");
    let root = workspace_root();
    fs::write(
        dir.join("Cargo.toml"),
        format!(
            r#"[package]
name = "opaque-app"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
roundhouse = {{ path = {root:?}, default-features = false, features = ["build"] }}
serde = {{ version = "1", features = ["derive"] }}
serde_json = "1"
"#,
            root = root.display().to_string()
        ),
    )
    .expect("write manifest");
    fs::write(
        dir.join("src/lib.rs"),
        r#"use roundhouse::doc::{DocumentedError, ResponseDoc, SchemaContext};
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct AppState;

roundhouse::app_state!(AppState);

roundhouse::api! {
    state = AppState,
    title = "Opaque App",
    version = "0.1.0",
    root = routes::MODULE,
    register = [Note],
}

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Note {
    pub id: u64,
}

#[derive(Debug)]
pub struct DbError;

impl IntoResponse for DbError {
    fn into_response(self) -> Response {
        (StatusCode::NOT_FOUND, "missing").into_response()
    }
}

impl DocumentedError for DbError {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::problem(404, "Not found")]
    }
}

pub mod routes {
    use super::*;

    routes! {
        "/direct" => { get(direct) },
        "/helper" => { get(helper) },
        "/fallible" => { get(fallible) },
        "/documented" => { get(documented) },
        "/opaque" => { get(opaque) },
    }

    #[handler]
    pub async fn direct() -> impl IntoResponse {
        Json(Note { id: 1 })
    }

    fn load() -> Json<Note> {
        Json(Note { id: 1 })
    }

    #[handler]
    pub async fn helper() -> impl IntoResponse {
        load()
    }

    fn find(id: u64) -> Result<Note, DbError> {
        if id == 0 {
            Err(DbError)
        } else {
            Ok(Note { id })
        }
    }

    #[handler(response(404, DbError))]
    pub async fn fallible(Path(id): Path<u64>) -> Result<impl IntoResponse, DbError> {
        let note = find(id)?;
        Ok(Json(note))
    }

    #[handler]
    pub async fn documented() -> impl IntoResponse {
        Json(Note { id: 1 }).into_documented_response()
    }

    #[handler(response(200))]
    pub async fn opaque() -> impl IntoResponse {
        (StatusCode::CREATED, "x").into_response()
    }
}
"#,
    )
    .expect("write lib");

    run(&dir, &["init"]);
    cargo(&dir, &["check"]);
    run(&dir, &["prepare"]);
    run(&dir, &["check"]);

    let openapi: serde_json::Value = serde_json::from_str(
        &fs::read_to_string(dir.join(".roundhouse/openapi.json")).expect("read openapi"),
    )
    .expect("openapi is JSON");

    let operation = |path: &str| {
        openapi["paths"][path]["get"]
            .as_object()
            .unwrap_or_else(|| panic!("missing operation {path}"))
    };
    let statuses = |path: &str| {
        operation(path)["responses"]
            .as_object()
            .expect("responses")
            .keys()
            .cloned()
            .collect::<Vec<_>>()
    };

    assert!(statuses("/direct").contains(&"200".to_string()));
    assert!(statuses("/helper").contains(&"200".to_string()));
    assert!(statuses("/fallible").contains(&"200".to_string()));
    assert!(statuses("/fallible").contains(&"404".to_string()));
    assert!(statuses("/documented").contains(&"200".to_string()));
    assert!(statuses("/opaque").contains(&"200".to_string()));

    let direct = &operation("/direct")["responses"]["200"]["content"]["application/json"]["schema"];
    assert_eq!(direct["$ref"], "#/components/schemas/Note");

    assert_eq!(
        operation("/opaque")["x-roundhouse-unverified-response"],
        true
    );

    fs::remove_dir_all(&dir).expect("clean the temporary project");
}
