# Changelog

## Unreleased

### Breaking

- Feature names are capability-first: `build` (build the contract and OpenAPI document),
  `contract` (consume the committed contract at runtime), `validation` (execute JSON Schema),
  `server` (the axum runtime) and `swagger-ui` (vendored assets). The defaults are `build`,
  `validation`, `server` and `swagger-ui`. The old `schema-engine` is `validation`; the old
  `contract-runtime` is `contract`; the old build-time `contract` is `build`.
  `#[roundhouse::contract_only]` is now `#[roundhouse::build_only]`.
- The contract generator moved out of `roundhouse-cli` into a new `roundhouse-build` crate;
  `roundhouse-cli` is now only the `cargo-roundhouse` binary. `roundhouse-build` exposes only
  `prepare`/`prepare_to` for `build.rs`; `check`, `explain`, `verify`, `diff` and `init` live in
  the CLI. A `build.rs` should depend on `roundhouse-build = "0.4"` and call
  `roundhouse_build::prepare()`. `cargo roundhouse init` migrates an existing generated
  `build.rs` and build-dependency; a hand-written build script needs the same two-line edit.
- Source-level response inference is no longer the source of response statuses or schemas. A
  named return type is documented by its `DocumentedResponse` impl. An opaque
  `impl IntoResponse` return is resolved by a build-time analyzer that only selects the concrete
  type at each tail/`return` site; the macro emits typed trait calls and the compiler verifies
  them. A site the analyzer cannot resolve is `x-roundhouse-unverified-response`, and an
  operation with no success is still a strict-build failure.
- `Result<T, E>` now requires `E: DocumentedError`, not `E: DocumentedResponse`. `Problem`
  implements both, but a custom error type needs a `DocumentedError` impl. The impl is
  build-only, like `DocumentedResponse`.
- `.into_response()` now keeps the receiver's documented type (`Json<Note>.into_response()` still
  documents `Json<Note>`); only a receiver that is itself erased (`Response`, a trait object, or a
  helper returning `Response`) stays opaque. `.into_documented_response()` erases the value to
  `Documented`, so branches with different documented types can share one `impl IntoResponse`
  return type. `DocumentedIntoResponse::into_documented_response` therefore returns `Documented`
  instead of `Self`. Explicit `#[handler(response(..))]` declarations remain the escape hatch.

### Added

- `Status<CODE, T>` and `Empty<CODE>` document and serve any valid HTTP status code with the
  status in the type, so the document is exact instead of `default`. The vendored IANA registry
  also generates named wrappers in `roundhouse::status` (`Created`, `NoContent`, `NotFound`,
  `Conflict`, …); `tools/status-codes` refreshes the shared JSON catalog, and a proc macro turns
  it into internal `macro_rules!` invocations in the core crate.
- `DocumentedError` documents a failure type's statuses, content types, bodies and headers.
  `Result<T, E>` combines it with `DocumentedResponse`; `Problem` is one implementor rather than a
  mandatory error type. `?` documents the handler's error type, not the source of the conversion.
- Opaque `impl IntoResponse` and `impl DocumentedResponse` handlers are documented from their
  bodies. `roundhouse-build::prepare` analyzes the library before the harness, writes
  `target/roundhouse/handlers.json`, and `#[handler]` reads that index at expansion time to emit
  typed `DocumentedResponse` / `DocumentedError` calls. Helper and method return types are
  inferred through a witness, so the document does not depend on a type being imported in the
  handler's module.
- `.into_documented_response()` is a marker for an opaque return site. It asks the analyzer to
  document the receiver's type and erases the value to `Documented`, so different documented
  branches can share one return type.
- `#[handler(response(status, Type))]` accepts a `DocumentedError`-only type: the declaration
  probe tries `DocumentedResponse` first and falls back to `DocumentedError`.
- `#[roundhouse::build_only]` emits an item only when the `build` feature is compiled. It is
  the supported way to write a custom `DocumentedResponse` / `DocumentedError` impl in an app that
  serves with `contract`: the documentation impl disappears from the production build
  instead of failing on a missing trait.

## 0.4.0 — 2026-11-16

### Breaking

- Documentation is a build-time concern. Schema derivation, the document builder,
  `App::docs`/`App::openapi`, `roundhouse::openapi` and `roundhouse::interop` are behind the new
  `contract` feature and are not part of the default feature set. The lean `contract-runtime`
  feature adds `ContractBundle`, `include_contract!`, `App::contract`, `contract.json` request
  validation and verbatim `openapi.json` serving without utoipa.
- A production build (`default-features = false`, or without `contract`) has no `utoipa`
  dependency. `#[roundhouse::schema]` still registers runtime validation hooks, but the injected
  `ToSchema` derive is compiled out.
- Runtime extractor validation moved to `ExtractArg` + `ValidateHook`. Documentation is a separate
  contract-only `ExtractDoc`; path/query derives implement `Params` + `ParamsDoc`, and multipart
  derives implement `MultipartForm` + `MultipartDoc`.
- `cargo roundhouse init` no longer writes `src/bin/roundhouse-contract.rs`; the contract binary
  is generated under `target/roundhouse/`. `init --ephemeral` was removed in favor of the
  generated build script.
- `ApiResponse` and `ApiError` traits and their derives were removed. Built-in response shapes are
  documented internally; custom responses use `#[handler(response(..))]` and a `doc = |docs|`
  hook. The examples no longer use response enums.
- `App::build` without a contract bundle now builds a contract in-process under `contract`
  instead of assembling a separate runtime document. The old `DocSource`/probe assembly and the
  `TypeId` body/parts maps are gone; request validation always reads the contract plans.
- `App::validate_responses()` was removed. Use `App::verify_responses()` or
  `cargo roundhouse verify`, which check the actual response against the committed contract.
- `App::build` without `contract` builds and serves the router but has no document. If request
  validation is enabled and no contract bundle is supplied, the build now fails with
  `BuildError::MissingContract`; pass `.contract(include_contract!())` or call
  `App::validation(Validation::Off)` for a router-only build.
- An outcome the analyzer cannot classify is an explicit `unknown` response with
  `provenance: "unknown"` in `contract.json`. Strict builds reject it unless the handler declares
  `@response default: ...`, a typed `response(default, ..)`, or the route has `unverified = "..."`.
- `roundhouse-problem` and `roundhouse-multipart` are folded into `roundhouse`. Remove the direct
  dependencies and import `roundhouse::Problem` / `roundhouse::multipart::*` instead. Behaviour is
  unchanged.

### Added

- `cargo roundhouse new <path>` creates a package with `cargo new --lib`; `cargo roundhouse init`
  now follows `cargo init` and scaffolds a package in place when there is no manifest. Both add
  the runtime/build dependencies, `.roundhouse/`, a guarded `build.rs`, a starter `src/lib.rs`
  and a thin `src/main.rs`. Adopting an existing bin-only package fails early with the library
  guidance instead of failing during the first contract build.
- Contract builds seed missing `.roundhouse/contract.json` and `openapi.json` with empty
  placeholders before compiling the application, so a freshly scaffolded package builds before
  its first contract exists.
- `contract` / `contract-runtime` features, `tests/fixtures/production` (no-contract compile
  guard), and a CI dependency check that asserts the production fixture graph contains no
  `utoipa`, `multer` or `tempfile`.
- `roundhouse-schema` `ValidateHook`, and runtime `Params` implementations independent of schema
  derivation.
- `cargo roundhouse verify` plus `App::verify_responses()` / `ROUNDHOUSE_VERIFY=1`: actual
  responses are checked against the committed contract's status, content type and JSON schema.
- Semantic response inference: `roundhouse-cli/src/semantic.rs` resolves handler argument types,
  method and associated-function returns, struct fields, type aliases, `Option`/`Result` chains,
  custom `IntoResponse` implementations and `?` error paths. `Json(app.load(id))` now infers the
  body component without a helper annotation.
- `contract.json` carries `kind` and `contract_version`; `App::build` rejects unsupported layouts
  with `BuildError::InvalidContract` instead of misreading them.
- CI builds `cargo-roundhouse` and runs `cargo roundhouse check` for every example, so committed
  artifacts cannot drift from the handlers.
- `cargo bench -p roundhouse --bench build` records the contract-build versus committed-bundle
  startup cost.
- Inference now reads response-builder `.status(..)` chains, `Redirect` constructors, numeric
  statuses, text/byte literals and an `if` without an `else` as a 200 empty response.
- `cargo roundhouse` builds its contract binary under `target/roundhouse/`. `init` adds the
  `roundhouse-cli` build-dependency and a guarded `build.rs`; `cargo build` and `cargo test`
  refresh the contract before `include_contract!` expands. No `src/bin` file or generated feature
  is written. `init --no-build-script` opts out; `init --ephemeral` is removed.
- `cargo roundhouse diff [--against <contract.json>] [--check]` reports removed operations,
  responses, content types, request bodies and security changes as breaking; `--check` fails CI.
- All five examples (`notes`, `layers`, `uploads`, `auth`, `quickstart`) now consume committed
  `.roundhouse/` artifacts through `contract-runtime`; quickstart moved its app into a library
  so it can generate a contract. Their production dependency graphs contain no utoipa.
- The docs UI (`App::docs`, `App::docs_route`, `App::swagger*`) is part of `contract-runtime`
  when a contract bundle is present, so a lean app can still serve static documentation.
- Contract mode parses the embedded `openapi.json` lazily, so startup does not depend on whether
  the optional typed view round-trips.
- The notes and quickstart examples use plain `impl IntoResponse` handlers for their outcome
  paths; no `ApiResponse`/`ApiError` enums remain in the examples.

## 0.3.0

### Breaking

- Strict builds are the default. Unverified bodies, responses and extractors, undocumented
  `any` routes, empty modules and unacknowledged merged routers are build errors.
  `App::permissive()` restores warning-only builds.
- Removed the `public` route option. Use `anonymous` to clear inherited security.
- Removed the 0.2 `@responses:`, `@responses(override):`, `@response-descriptions:` and `@path:`
  doc tags. Use `#[handler(..)]`.
- Removed `App::router()` and the `HandlerDocs` alias. Use `App::into_router()` /
  `Built::router()` and `OperationDoc`.
- utoipa is now an implementation detail. The documented path is `#[roundhouse::schema]` with
  `#[roundhouse(..)]` constraints. utoipa names live in `roundhouse::interop`.
  `impl_api_schema!` is now `roundhouse::interop::impl_from_utoipa!`.
- `Info` and `OpenApi` no longer dereference to their utoipa values; use `as_utoipa()` or
  `into_utoipa()`.
- Prefix groups use `prefix "/path" [options] { .. }`. `"/path" [options] { .. }` is a compile
  error.
- `app_state!` outside the crate root is a compile error at the call site.
- `App::merge(router)` requires `App::merge_undocumented(router, reason)`.

### Added

- `#[roundhouse(..)]` constraints for lengths, patterns, numeric and collection bounds, formats,
  descriptions, examples, `unique_items`, `validate_with` and component names.
- `roundhouse::interop` for utoipa types and generated `ToSchema` adapters.
- `App::permissive()` and `App::merge_undocumented()`.
- A dependency-free throughput benchmark (`cargo bench -p roundhouse --bench throughput`).
- Documentation for the new authoring surface and the 0.3 migration path.

### Changed

- Table-local duplicate checks skip `#[cfg(..)]`-gated entries; collisions that survive `cfg`
  are caught by the resolved route table.
- `#[roundhouse(..)]` constraints are patched into the published schema, so the document and the
  validator cannot disagree.
