# Migrating to roundhouse 0.4

0.4 moves OpenAPI generation out of the running application and into a build-time contract
compiler. The document comes from the response types you return and the declarations you write;
there is no runtime document assembly, and a production build does not compile `utoipa`. Request
validation stays, with `jsonschema` as the only engine.

This guide is deliberately a hard break. The 0.3 line remains available for applications that are
not ready to migrate.

## Breaking changes at a glance

| 0.3 | 0.4 |
|---|---|
| Document assembled at startup | `.roundhouse/contract.json` + `.roundhouse/openapi.json`, written by `cargo roundhouse prepare` |
| `ApiResponse` / `ApiError` outcome enums | Named response types + `roundhouse::doc::DocumentedResponse`; `Problem` is the universal error body |
| Success probes and `DocSource` at runtime | `DocumentedResponse` impls at build time + explicit `response(..)` declarations |
| `utoipa` in the runtime dependency graph | `utoipa` only with the `contract` feature |
| `App::document(..)`, `App::docs()` always available | Behind `contract` (build/docs) or `contract-runtime` (serve/validate) |
| Response validation in `App` | Test-only contract response verification |
| `response(200, Note)` as a JSON body | `response(200, Json<Note>)`; the declared type must implement `DocumentedResponse` |

## 1. Install the feature split

A production application consumes a committed contract:

```toml
[dependencies]
roundhouse = { version = "0.4", features = ["contract-runtime"] }
```

The build-time tooling does not require a generated feature or bin. `cargo roundhouse init` adds
`roundhouse-cli` as a build-dependency and generates a guarded `build.rs`; the build script and
the CLI share the same library. Contract generation runs a harness under `target/roundhouse` and
never writes `src/bin`.

- `contract-runtime` — `ContractBundle`, `include_contract!`, `App::contract`,
  `contract.json` request validation, response verification, and verbatim `openapi.json`
  serving. No `utoipa`.
- `contract` — schema derivation, the document builder and the public response/documentation
  traits; implies `contract-runtime`.
- The harness enables `contract` for the build only; a production application keeps
  `contract-runtime`.

The default features are still `schema-engine`, `serve` and `swagger`; documentation and contract
building are opt-in.

## 2. Declare the contract once

Replace Cargo metadata or runtime `App::document` configuration with one `api!` block at the crate
root:

```rust
roundhouse::api! {
    state = AppState,                 // optional; defaults to crate::__RoundhouseAppState
    title = "Notes API",
    version = env!("CARGO_PKG_VERSION"),
    root = routes::MODULE,
    servers = ["https://api.example.com"],
    register = [PrivateResponseBody],
    response_defaults = {
        401 => "Authentication is required.",
        500 => "An unexpected server error occurred.",
    },
}
```

`register` names a private request or response body the harness cannot otherwise reach. `root`
points at the `MODULE` constant exported by the `routes!` table.

## 3. Replace outcome enums with response types

Before:

```rust
#[derive(ApiResponse)]
enum ShowResponse {
    #[status(200)]
    Found(#[body] Json<Note>),
    #[status(404)]
    Missing,
}

#[handler]
async fn show(...) -> ShowResponse {
    match app.get(id) {
        Some(note) => ShowResponse::Found(Json(note)),
        None => ShowResponse::Missing,
    }
}
```

After:

```rust
#[handler(response(404, Problem))]
async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> Result<Json<Note>, Problem> {
    match app.get(id) {
        Some(note) => Ok(Json(note)),
        None => Err(Problem::not_found("Note not found")),
    }
}
```

- `Json<Note>` documents `200 application/json` through its `DocumentedResponse` impl.
- `Problem` documents `default` with problem+json.
- `response(404, Problem)` adds the exact 404 status; the type still supplies the body.
- `Created<T>` and `NoContent` remain available when a status is easier to name explicitly.

A built-in is not special because of its name: `Json`, `Created`, `Html` and the rest implement the
same public trait as a custom response. There is no inferred status table.

### `impl IntoResponse` handlers

`impl IntoResponse` has no nameable concrete type, so the macro cannot evaluate its trait. Declare
the responses explicitly:

```rust
#[handler(response(200, Json<Note>), response(404, Problem))]
async fn show(...) -> impl IntoResponse {
    match app.get(id) {
        Some(note) => Json(note).into_response(),
        None => Problem::not_found("Note not found").into_response(),
    }
}
```

Without a declaration, an `impl IntoResponse` operation has no documented success and the strict
build fails with a fix suggestion.

## 4. Move response prose to doc tags

Response structure comes from the type or declaration; prose is still authored:

```rust
/// @summary: Read a note
/// @response 200: The stored note.
/// @response 404: The requested note does not exist.
/// @response 500: The note store failed.
#[handler(response(404, Problem))]
async fn show(...) -> Result<Json<Note>, Problem> { ... }
```

Precedence:

1. Handler `@response`.
2. `api! response_defaults`.
3. Body type description.
4. Literal `Problem` title.
5. Canonical HTTP reason.

Use `#[handler(response_description(404, r#"..."#))]` for long prose.

`@response` describes an existing status. It does not declare one; use `response(status)` or
`response(status, Type)` for that.

## 5. Register private bodies

A private body that the harness cannot name still needs a component:

```rust
roundhouse::api! {
    // ...
    register = [PrivateRow],
}
```

Use `#[roundhouse::schema]` or implement `ApiSchema` manually. A response type that only exists
for the OpenAPI shape can implement `DocumentedResponse` directly.

## 6. Build and commit the artifacts

```sh
cargo roundhouse init      # .roundhouse/, build.rs and the api! snippet
cargo build                # build.rs refreshes the contract automatically
cargo roundhouse prepare   # explicit regeneration (also used by CI)
cargo roundhouse check     # CI: artifacts are current
cargo roundhouse explain "GET /v1/notes/{id}"
cargo roundhouse verify    # run tests with response verification
cargo roundhouse diff --check   # fail CI on breaking contract changes
```

`prepare` runs one strict build under `target/roundhouse`. Nothing is written outside `target/`
and `.roundhouse/`. The generated `build.rs` calls the same library, so `cargo build` does this for
you locally. It skips in CI by default so `cargo roundhouse check` still sees the committed
artifact; set `ROUNDHOUSE_AUTO_CONTRACT=1` to force it, or `ROUNDHOUSE_AUTO_CONTRACT=0` to disable
it locally. Commit `.roundhouse/contract.json` and `.roundhouse/openapi.json`.

`contract.json` records `contract_version`; the runtime accepts only the layout it understands
and fails `App::build` with `BuildError::InvalidContract` when the artifact is stale or from a
different major layout. Rerun `cargo roundhouse prepare` after upgrading roundhouse.

`explain` shows each response:

```text
GET /v1/notes/1 (show)
  200     application/json             #/components/schemas/Note      The stored note.
  404     application/problem+json     (inline)                       The requested note does not exist.
```

## 7. Consume the contract at runtime

```rust
const CONTRACT: roundhouse::ContractBundle = roundhouse::include_contract!();

roundhouse::app!(AppState, routes)
    .contract(CONTRACT)
    .openapi_route("/openapi.json")
    .serve("0.0.0.0:3000")
    .await?;
```

- Request bodies and path/query/form values are validated from `contract.json`; no runtime
  document is built. A validation-enabled build without a contract fails with
  `BuildError::MissingContract`; for a router-only build use
  `App::validation(Validation::Off)`.
- `openapi_route` serves the committed `openapi.json` verbatim.
- `.docs()` and `.openapi(info)` require the `contract` feature because they build a document.
  A production `contract-runtime` app should serve the artifact instead.
- `Built::document()` / `openapi_30()` also require `contract`; `openapi_json()` returns the
  committed artifact in a runtime-only build.

## 8. Verify responses in tests

```rust
#[tokio::test]
async fn the_response_matches_the_contract() {
    let built = roundhouse::app!(AppState, routes)
        .contract(roundhouse::include_contract!())
        .verify_responses()
        .build();

    assert_eq!(built.oneshot_get("/v1/notes/1").await.status(), 200);
}
```

`verify_responses()` checks the actual status, content type and JSON body against the contract
response table. `cargo roundhouse verify` runs `cargo test` with `ROUNDHOUSE_VERIFY=1`, which
enables the same check without changing the test. A mismatch becomes a `500 Problem` naming the
operation and the violated schema.

A response the types and declarations do not document is an `UndeclaredSuccess` build error, not a
guessed `default`. A permissive build (`App::permissive()`) publishes a `default` placeholder and
marks it `x-roundhouse-unverified-response`.

## 9. Update extractor and response extension code

The runtime and documentation halves are separate traits:

- `ExtractArg` — `IS_BODY`, `SHIM_VALIDATES`, route-level `validate`.
- `ExtractDoc` — the argument's documented contribution (`contract` only).
- `Params` — runtime capture names and wire value.
- `ParamsDoc` — parameter schemas (`contract` only).
- `MultipartForm` / `MultipartDoc` — the same split for multipart forms.
- `ValidateHook` — custom validation hooks, available in every build.
- `DocumentedResponse` — response status, content type, body schema and headers.

A custom extractor that used to implement one `ExtractArg::document` method now implements
`ExtractArg` (usually empty) and `ExtractDoc::document`.

A custom response type implements `IntoResponse` to serve and `DocumentedResponse` to document:

```rust
impl roundhouse::doc::DocumentedResponse for Csv {
    fn responses(context: &mut dyn roundhouse::doc::SchemaContext) -> Vec<roundhouse::doc::ResponseDoc> {
        vec![roundhouse::doc::ResponseDoc {
            status: 200,
            description: "CSV export".to_string(),
            content_type: Some("text/csv".to_string()),
            schema: Some(roundhouse::doc::schema::string()),
            headers: Vec::new(),
        }]
    }
}
```

## 10. Validation is still supported

The annotation is the schema, and `jsonschema` executes it:

```rust
#[roundhouse::schema]
#[derive(serde::Deserialize)]
pub struct Paginator {
    #[roundhouse(minimum = 1, maximum = 50)]
    pub page_size: usize,
    #[roundhouse(minimum = 1)]
    pub page_no: usize,
}
```

- `#[roundhouse(..)]` constraints still drive `openapi.json` and the request plans in
  `contract.json`.
- `validate_with` remains for custom rules that are not schema-expandable; those rules run but are
  not projected into OpenAPI.
- `validator`, `garde`, `axum-valid` and `validify` are not supported. Their rules are a runtime
  DSL, not a schema; a bridge would make the document a lossy projection.

## 11. Remove the old runtime documentation path

Delete:

- `ApiResponse` / `ApiError` derives and their enum types.
- `App::document(..)`, `App::server(..)`, `App::servers(..)` and `App::openapi(..)` from
  production startup; move metadata into `api!` or the contract builder.
- `roundhouse::interop` imports unless the type genuinely comes from another utoipa project.
- `impl_api_schema!` uses; replace with `#[roundhouse::schema]` or
  `roundhouse::interop::impl_from_utoipa!`.

Keep:

- `routes!`, `#[handler]`, extractors, auth, layers and `app_state!`.
- `Problem` as the error body.
- `#[roundhouse(..)]` constraints; they still drive the emitted schema and runtime validation.
- `contract.json`, `contract-runtime`, `include_contract!`, `verify`, `explain` and `diff`.

## Migration checklist

- [ ] Add `contract-runtime` to the runtime dependency (no generated feature or bin), then run
  `cargo roundhouse init` for the build-dependency and build script.
- [ ] Add one `api!` block at the crate root.
- [ ] Replace response/error enums with named response types and `Problem`.
- [ ] Convert `impl IntoResponse` handlers to named types or add `response(..)` declarations.
- [ ] Replace `response(status, Type)` with `response(status, Json<Type>)` or implement
  `DocumentedResponse` for the type.
- [ ] Implement `DocumentedResponse` for custom response types.
- [ ] Move response prose to `@response` tags and `response_defaults`.
- [ ] Add `register = [..]` for private bodies.
- [ ] Run `cargo roundhouse prepare` and commit `.roundhouse/`.
- [ ] Build with `.contract(include_contract!())`.
- [ ] Add `cargo roundhouse check` to CI and `cargo roundhouse verify` to the test job.
- [ ] Confirm `cargo tree -e normal` has no `utoipa` in the production build.
- [ ] Confirm the only validation engine is `jsonschema`.
