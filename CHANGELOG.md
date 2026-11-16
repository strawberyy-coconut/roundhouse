# Changelog

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

### Added

- `contract` / `contract-runtime` features, `tests/fixtures/production` (no-contract compile
  guard), and a CI dependency check that asserts the production fixture graph contains no
  `utoipa`.
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
