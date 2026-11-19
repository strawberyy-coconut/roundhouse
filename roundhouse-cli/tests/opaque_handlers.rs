//! End-to-end: opaque `impl IntoResponse` handlers are documented from the body analyzer.
//!
//! The test writes a temporary crate, runs the real CLI, and checks that direct responses,
//! helper calls, documented error types, `.into_documented_response()` and the transparent
//! `.into_response()` receiver all reach the contract.

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
use roundhouse::status::{Empty, NotFound};

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
        "/documented-branches/{id}" => { get(documented_branches) },
        "/opaque" => { get(opaque) },
        "/named-not-found" => { get(named_not_found) },
        "/not-found" => { get(not_found) },
        "/status" => { get(status) },
        "/empty" => { get(empty) },
        "/branching/{id}" => { get(branching) },
        "/branching-helper/{id}" => { get(branching_helper) },
        "/alias-helper/{id}" => { get(alias_helper) },
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

    /// The marker erases to one concrete type, so unlike `.into_response()` it can unify two
    /// different documented branches.
    #[handler(path(id, "The note id."))]
    pub async fn documented_branches(Path(id): Path<u64>) -> impl IntoResponse {
        if id == 0 {
            return NotFound(()).into_documented_response();
        }
        Json(Note { id }).into_documented_response()
    }

    #[handler(response(200))]
    pub async fn opaque() -> impl IntoResponse {
        (StatusCode::CREATED, "x").into_response()
    }

    #[handler]
    pub async fn named_not_found() -> Result<Json<Note>, NotFound<Problem>> {
        Err(NotFound(Problem::not_found("missing")))
    }

    #[handler(response(200))]
    pub async fn not_found() -> impl IntoResponse {
        NotFound(Problem::not_found("missing"))
    }

    #[handler]
    pub async fn status() -> impl IntoResponse {
        Status::<207, _>::new(Note { id: 1 })
    }

    #[handler]
    pub async fn empty() -> impl IntoResponse {
        Empty::<204>
    }

    fn find_optional(id: u64) -> Option<Note> {
        (id != 0).then_some(Note { id })
    }

    /// The original opaque-branch pattern: a normal-path 404 and a normal-path 200, both erased
    /// through `.into_response()`, with the 404 in a `let ... else` divergent branch.
    #[handler(path(id, "The page id."))]
    pub async fn branching(Path(id): Path<u64>) -> Result<impl IntoResponse, DbError> {
        let Some(note) = find_optional(id) else {
            return Ok(NotFound(()).into_response());
        };
        Ok(Json(note).into_response())
    }

    async fn page_state(id: u64) -> Result<Note, DbError> {
        Ok(Note { id })
    }

    /// The same pattern with a fallible helper and `?` inside the erased response.
    #[handler(path(id, "The page id."))]
    pub async fn branching_helper(Path(id): Path<u64>) -> Result<impl IntoResponse, DbError> {
        if id == 0 {
            return Ok(NotFound(()).into_response());
        }
        Ok(Json(page_state(id).await?).into_response())
    }

    type ApiResult<T> = Result<T, DbError>;

    async fn alias_page_state(id: u64) -> ApiResult<Note> {
        Ok(Note { id })
    }

    /// A generic result alias must substitute its argument instead of leaking the alias's `T`.
    #[handler(path(id, "The note id."))]
    pub async fn alias_helper(Path(id): Path<u64>) -> Result<impl IntoResponse, DbError> {
        if id == 0 {
            return Ok(NotFound(()).into_response());
        }
        Ok(Json(alias_page_state(id).await?).into_response())
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
    assert!(statuses("/documented-branches/{id}").contains(&"200".to_string()));
    assert!(statuses("/documented-branches/{id}").contains(&"404".to_string()));
    assert!(statuses("/opaque").contains(&"200".to_string()));
    assert!(statuses("/opaque").contains(&"default".to_string()));
    assert!(statuses("/named-not-found").contains(&"404".to_string()));
    assert!(statuses("/not-found").contains(&"404".to_string()));
    assert!(statuses("/status").contains(&"207".to_string()));
    assert!(statuses("/empty").contains(&"204".to_string()));
    assert!(statuses("/branching/{id}").contains(&"200".to_string()));
    assert!(statuses("/branching/{id}").contains(&"404".to_string()));
    assert!(statuses("/branching-helper/{id}").contains(&"200".to_string()));
    assert!(statuses("/branching-helper/{id}").contains(&"404".to_string()));
    assert!(statuses("/alias-helper/{id}").contains(&"200".to_string()));
    assert!(statuses("/alias-helper/{id}").contains(&"404".to_string()));
    assert!(
        operation("/branching/{id}")
            .get("x-roundhouse-unverified-response")
            .is_none(),
        "both `.into_response()` branches are documented"
    );
    assert!(
        operation("/branching-helper/{id}")
            .get("x-roundhouse-unverified-response")
            .is_none(),
        "a fallible helper behind `.into_response()` is documented"
    );
    assert!(
        operation("/alias-helper/{id}")
            .get("x-roundhouse-unverified-response")
            .is_none(),
        "a generic result alias is substituted, not left as `T`"
    );

    let direct = &operation("/direct")["responses"]["200"]["content"]["application/json"]["schema"];
    assert_eq!(direct["$ref"], "#/components/schemas/Note");

    assert!(
        operation("/opaque")
            .get("x-roundhouse-unverified-response")
            .is_none(),
        "a resolvable `.into_response()` receiver is documented"
    );

    fs::remove_dir_all(&dir).expect("clean the temporary project");
}
