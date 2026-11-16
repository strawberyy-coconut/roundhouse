# Migrating to roundhouse 0.4

0.4 moves OpenAPI generation out of the running application and into a build-time contract
compiler. Handlers stay plain axum: they return `Response` / `impl IntoResponse`, and the
document is inferred from their return paths. There is no runtime document assembly to keep in
sync, and a production build does not compile `utoipa`.

This guide is deliberately a hard break. The 0.3 line remains available for applications that
are not ready to migrate.

## Breaking changes at a glance

| 0.3 | 0.4 |
|---|---|
| Document assembled at startup | `.roundhouse/contract.json` + `.roundhouse/openapi.json`, written by `cargo roundhouse prepare` |
| `ApiResponse` / `ApiError` outcome enums | Plain `impl IntoResponse`; `Problem` is the universal error body |
| Success probes and `DocSource` at runtime | source inference at build time + explicit prose tags |
| `utoipa` in the runtime dependency graph | `utoipa` only with the `contract` feature |
| `App::document(..)`, `App::docs()` always available | Behind `contract` (build/docs) or `contract-runtime` (serve/validate) |
| Response validation in `App` | Test-only contract response verification |

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
- `contract` — schema derivation and the document builder, and implies `contract-runtime`.
- The harness enables `contract` for the build only; a production application keeps
  `contract-runtime`.

The default features are still `schema-engine`, `serve` and `swagger`; documentation and contract
building are opt-in.

## 2. Declare the contract once

Replace any Cargo metadata or runtime `App::document` configuration with one `api!` block at the
crate root:

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

`register` is for a response body the analyzer can infer but the contract harness cannot
name (for example a private type returned only through a helper). Everything else should be
inferred.

`root` points at the `MODULE` constant exported by the `routes!` table.

## 3. Replace outcome enums with plain handlers

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
#[handler]
async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> impl IntoResponse {
    match app.get(id) {
        Some(note) => Json(note).into_response(),
        None => Problem::not_found("Note not found").into_response(),
    }
}
```

The analyzer reads the branches and derives `200 application/json` and
`404 application/problem+json`. `Error`-typed `?` paths add a conservative `500`.

- `Problem::not_found(..)`, `bad_request(..)`, `conflict(..)`, `internal(..)`,
  `new(status, title)`.
- `From<E> for Problem` maps infrastructure errors to `500`.
- `Problem` no longer implements `std::error::Error`; keep domain error enums inside the handler
  and convert them at the boundary.
- `Created<T>` and `NoContent` remain available when a status is easier to name explicitly.

## 4. Move response prose to doc tags

Statuses and bodies are inferred; prose is still authored:

```rust
/// @summary: Read a note
/// @response 200: The stored note.
/// @response 404: The requested note does not exist.
/// @response 500: The note store failed.
#[handler]
async fn show(...) -> impl IntoResponse { ... }
```

Precedence:

1. Handler `@response`.
2. Route/module default (when introduced).
3. `api! response_defaults`.
4. Body type description.
5. Literal `Problem` title.
6. Canonical HTTP reason.

Use `#[handler(response_description(404, r#"..."#))]` for long prose.

A prose tag for a status the analyzer did not infer declares that response: 2xx/3xx with no
body, 4xx/5xx as `Problem`, unless a typed `response(status, Type)` declaration supplies a body.

## 5. Register types the analyzer cannot name

The analyzer sees handler source, not resolved types. A private body returned through a helper
still needs a component:

```rust
roundhouse::api! {
    // ...
    register = [PrivateRow],
}
```

For a type that only the builder can construct, implement `ApiSchema` manually or use the
`#[roundhouse::schema]` annotation and list it in `register`.

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

`prepare` runs the harness twice under `target/roundhouse`: a discovery build, then source
inference, then a strict build. Nothing is written outside `target/` and `.roundhouse/`. The
generated `build.rs` calls the same library, so `cargo build` does this for you locally. It skips
in CI by default so `cargo roundhouse check` still sees the committed artifact; set
`ROUNDHOUSE_AUTO_CONTRACT=1` to force it, or `ROUNDHOUSE_AUTO_CONTRACT=0` to disable it locally. Commit `.roundhouse/contract.json` and `.roundhouse/openapi.json`.

`contract.json` records `contract_version`; the runtime accepts only the layout it understands
and fails `App::build` with `BuildError::InvalidContract` when the artifact is stale or from a
different major layout. Rerun `cargo roundhouse prepare` after upgrading roundhouse.

`explain` shows each response with its provenance:

```text
GET /v1/notes/1 (show)
  200     application/json             #/components/schemas/Note      inferred  The stored note.
  404     application/problem+json     (inline)                       inferred  missing
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

An outcome the analyzer cannot classify is written as a `default Problem` with
`provenance: "unknown"`. Strict builds reject it unless the handler adds `@response default: ...`
(or a typed `response(default, ..)`), or the route acknowledges it with
`unverified = "reason"`.

`verify_responses()` checks the actual status, content type and JSON body against the contract
response table. `cargo roundhouse verify` runs `cargo test` with `ROUNDHOUSE_VERIFY=1`, which
enables the same check without changing the test. A mismatch becomes a `500 Problem` naming the
operation and the violated schema.

## 9. Update extractor extension code

The runtime and documentation halves are now separate traits:

- `ExtractArg` — `IS_BODY`, `SHIM_VALIDATES`, route-level `validate`.
- `ExtractDoc` — the argument's documented contribution (`contract` only).
- `Params` — runtime capture names and wire value.
- `ParamsDoc` — parameter schemas (`contract` only).
- `MultipartForm` / `MultipartDoc` — the same split for multipart forms.
- `ValidateHook` — custom validation hooks, available in every build.

A custom extractor that used to implement one `ExtractArg::document` method now implements
`ExtractArg` (usually empty) and `ExtractDoc::document`.

## 10. Remove the old runtime documentation path

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
- `#[roundhouse(..)]` constraints; they still drive the emitted schema and hooks.

## Migration checklist

- [ ] Add `contract-runtime` to the runtime dependency (no generated feature or bin), then run
  `cargo roundhouse init` for the build-dependency and build script.
- [ ] Add one `api!` block at the crate root.
- [ ] Rewrite response/error enums as `impl IntoResponse` + `Problem`.
- [ ] Move response prose to `@response` tags and `response_defaults`.
- [ ] Add `register = [..]` for private inferred bodies.
- [ ] Run `cargo roundhouse prepare` and commit `.roundhouse/`.
- [ ] Build with `.contract(include_contract!())`.
- [ ] Add `cargo roundhouse check` to CI and `cargo roundhouse verify` to the test job.
- [ ] Delete runtime document assembly and utoipa-only imports.
- [ ] Confirm `cargo tree -e normal` has no `utoipa` in the production build.
