# roundhouse

An OpenAPI-first framework for [axum](https://github.com/tokio-rs/axum). You declare routes,
parameters, schemas, auth and errors once; roundhouse builds the `axum::Router` at runtime and the
OpenAPI 3.1 contract at build time from the same declarations. The module tree is the complete
route set.

## Install

The `cargo roundhouse` commands come from the CLI:

```sh
cargo install roundhouse-cli
# or, from a checkout:
cargo install --path roundhouse-cli
```

Contract generation itself runs through the `roundhouse-build` build-dependency that the
scaffolding adds to the application. The default `slice` harness generates a documentation-only
package and does not depend on the application; `ROUNDHOUSE_HARNESS=linked` restores the mode
that compiles the application crate directly.

`cargo roundhouse new` creates a package with a library target, a thin binary, the runtime and
build dependencies, `.roundhouse/` and a guarded `build.rs`:

```sh
cargo roundhouse new hello-api
cd hello-api
cargo run
```

Run `cargo roundhouse init` instead to adopt an existing package in the current directory. It uses
`cargo init --lib` when there is no manifest, so `init` and `new` follow Cargo's conventions.
Existing packages with only a binary target are supported: the contract harness compiles a
generated documentation slice and does not depend on the application crate.

`contract` is the production feature: it embeds the committed contract, validates requests
and serves `openapi.json` without utoipa.

Rust 1.88 or newer is required. The default features are `build`, `validation`, `server` and `swagger-ui`.

## Quick start

Contract generation starts at the crate root. A library (`src/lib.rs`) with a thin `src/main.rs`
is the recommended layout because the binary can reuse the app; a package with only `src/main.rs`
also works, because the harness compiles a generated documentation slice instead of depending on
the application.

```rust
// src/lib.rs
use roundhouse::prelude::*;

#[derive(Clone, Default)]
pub struct AppState;

roundhouse::app_state!(AppState);

roundhouse::api! {
    state = AppState,
    title = "Hello",
    version = env!("CARGO_PKG_VERSION"),
    root = routes::MODULE,
}

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Hello {
    pub message: String,
}

pub mod routes {
    use super::*;

    routes! {
        prefix = "/v1",
        tags = ["hello"],
        "/hello/{name}" => {
            get(hello),
        },
    }

    /// @summary: Say hello.
    #[handler]
    pub async fn hello(Path(name): Path<String>) -> Json<Hello> {
        Json(Hello { message: format!("hello {name}") })
    }
}

pub fn app(state: AppState) -> roundhouse::App<AppState> {
    roundhouse::app!(state, routes)
}
```

```rust
// src/main.rs
use hello_api::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    hello_api::app(AppState)
        .contract(roundhouse::include_contract!())
        .docs()
        .serve("0.0.0.0:3000")
        .await?;
    Ok(())
}
```

By default, a package created with `cargo roundhouse new`, or adopted with
`cargo roundhouse init`, has `roundhouse-build` as a build-dependency and a guarded `build.rs`. From then on, `cargo build` and
`cargo test` refresh `.roundhouse/` automatically before `include_contract!` expands. Missing
artifacts are seeded with empty placeholders during the contract build, so a new package builds
before its first contract exists. `cargo roundhouse prepare` remains available for an explicit run,
and `cargo roundhouse check` verifies the committed artifacts in CI. Automatic generation is
skipped in CI unless `ROUNDHOUSE_AUTO_CONTRACT=1`, so CI checks the committed artifact rather than
regenerating it. No `src/bin` file is generated; the contract harness lives under
`target/roundhouse`.

The generated `.roundhouse/openapi.json` documents `GET /v1/hello/world`, including the `name`
path parameter that `Path<String>` cannot name by itself, and is served verbatim at
`GET /openapi.json`. The contract also supplies request validation, so the runtime never builds a
document of its own.

## How it works

1. **Declare.** A `routes!` table emits a module containing its prefix, tags, layers, security
   guards and routes, plus the child modules composed with `...module`. `#[handler]` reads the
   handler signature and produces the request, response and security contribution for that
   operation.
2. **Compile the contract.** The generated `build.rs` calls `roundhouse_build::prepare()` before
   rustc reads `include_contract!`. One strict build evaluates the response traits and explicit
   declarations, and writes `.roundhouse/contract.json` plus `openapi.json`.
3. **Build and enforce.** `App::build()` walks the module tree and produces a `Built` with the
   `axum::Router`. The committed contract supplies `openapi.json` serving and request validation;
   declared security installs the matching guard. Rejections use the `Problem` body.

## Route tables

A table groups methods under a path and composes other tables:

```rust
routes! {
    prefix = "/api",
    tags = ["api"],
    layer = tower_http::trace::TraceLayer::new_for_http(),

    "/health" => {
        get(health),
    },

    "/notes" => {
        get(list),
        post(create),
    },

    "/notes/{id}" => {
        get(show),
        delete(destroy),
    },

    prefix "/admin" [security = [Bearer]] {
        "/audit" => {
            get(audit),
        },
    },

    ...shared,
}
```

Every route entry lives inside a path group, `"/path" => { .. }`; a `prefix "/path" [..] { .. }`
group nests another table. The supported methods are `get`, `post`, `put`, `patch`, `delete`,
`head`, `options`, `trace` and `any`. Paths use `{capture}` and `{*rest}` syntax. A table can set
`operation_prefix = "page"` to shorten derived ids (`get_page_list`); a nested group's prefix is
concatenated after its parent's.

Operation prose comes from doc tags. Structured response and path metadata lives on the handler's
attribute:

```rust
/// @summary: Create a note.
/// @description: Persists the note and returns the stored representation.
#[handler(
    response(201, Json<Note>),
    response(409, Problem),
    response_description(201, "The stored note."),
)]
pub async fn create(...) -> ... { ... }
```

- `@summary:` and `@description:` are prose. `@summary:` ends at a blank line;
  `@description:` keeps blank lines as paragraphs. Untagged doc prose is ignored for OpenAPI.
- `response(status, Type)` calls `Type`'s `DocumentedResponse` impl and overrides the status.
  `response(status)` declares an empty response, `default_response(Type)` uses OpenAPI's
  `default`, and `override_response(status, Type)` replaces the return type's documented shape.
- `response_description(status, "...")` overrides one response description.
- `path(name, "...")` describes a path parameter with no field to carry the description. A
  `PathParams` field description still wins.
- `Type` must implement `DocumentedResponse`. Use `Json<T>` for a JSON body, `Problem` for
  problem+json, or a custom response type; `response(200, Note)` is not an implicit JSON wrapper.
The grammar is in [Declarations](docs/design/declarations.md); response-description precedence is
in [The document](docs/design/document.md).

### Entry options

| Option | Effect |
|---|---|
| `operation_id = "..."` | Choose the operation id. Without it, an id is derived from the method and handler, using the table's `operation_prefix` or the module path. |
| `tags = ["..."]` | Add tags to the operation. |
| `security = [Scheme]` | Require a scheme for this operation. The scheme is enforced by the route shim. |
| `path = "/full/path"` | Assert the final resolved path, including module prefixes. |
| `layer = expression` | Apply a tower layer to this route. |
| `doc = expression` | Run a hook over the operation's `OperationDoc` before document checks. |
| `prose = expression` | Take the summary and description from a `&'static str`, usually `include_str!(...)`. The first non-empty line is the summary. |
| `hidden` | Serve the route without documenting it. |
| `absolute` | Use the path as the full URL, ignoring inherited prefixes, tags, layers and guards. |
| `deprecated` | Mark the operation deprecated. |
| `anonymous` | Clear inherited `security` guards for this entry and document `security: []`. A handler's own `Auth<A>` still applies. |
| `validate = false` | Skip schema constraint validation for the whole route. The request still has to parse. |
| `validate_body = false` | Keep path/query/form validation, but treat the request body as opaque. Malformed JSON still fails. |
| `unverified = "reason"` | Acknowledge an intentional unverified body/response/extractor so the strict build passes. |

### Module options

`prefix` is declared before a table's first entry. A path group, `"/path" => { .. }`, declares routes directly; a `prefix "/path" [..] { .. }` group nests another table. Every route entry lives inside a path group.
`state`, `tags`, `security`, `layer`, `layer_mode`, `hidden`, `anonymous` and `doc` can be declared
before a table's first entry or in the brackets of a `prefix` group. `layer_mode = full` applies a
module layer to the whole subtree; `layer_mode = route` uses `route_layer`.

Prefer `roundhouse::app!(state, routes)` so the root table is explicit. `app!(state)` is shorthand
when the table is declared in the caller's own module. `App::include` and `App::fragment` compose
roots directly.

### State and `FromRef` substates

The app state is exact at the `App::new` boundary. Declare it once at the crate root with
`app_state!`; every table without `state = ...` uses that type. Handlers may take substates through
axum's `FromRef`:

```rust
roundhouse::app_state!(AppState);

#[derive(Clone)]
pub struct AppState { db: Db }

#[derive(Clone)]
pub struct Db;

impl FromRef<AppState> for Db {
    fn from_ref(state: &AppState) -> Self {
        state.db.clone()
    }
}

routes! {
    prefix = "/notes",
    "/" => { get(list) },
}

#[handler]
pub async fn list(State(db): State<Db>) -> Json<Vec<Note>> { ... }
```

Inline path groups inherit the table's state, and a spread module resolves the same crate-level
alias, so neither needs to repeat it. `state = OtherState` overrides one table;
`state = infer` restores per-handler inference (the first `State<T>`, the auth scheme's state, or
`()`). `Auth<A>` remains exact: `AuthScheme::State` must equal the table state.

## Schemas

`#[roundhouse::schema]` is the one-annotation form: it derives the schema and the roundhouse
adapter, and constraints use the `#[roundhouse(..)]` namespace. The explicit
`#[derive(ToSchema, ApiSchema)]` form is still supported for projects that already share utoipa
types; with `build` enabled, import `ToSchema` and `PartialSchema` from
`roundhouse::interop`. `#[derive(ApiSchema)]` adds:

- a named component (`$ref`) for the type;
- `#[roundhouse(unique_items)]`;
- the `url` to `uri` format mapping;
- `#[roundhouse(validate_with = path)]` hooks.

```rust
#[roundhouse::schema]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[roundhouse(min_length = 3, max_length = 32)]
    pub user_name: String,

    #[roundhouse(format = "email")]
    pub email: String,

    #[roundhouse(validate_with = validate_age)]
    pub age: u8,
}
```

`axum::Json<T>`, `Form<T>` and a JSON response require `T: ApiSchema` when the framework documents
them automatically. A type that only implements utoipa's `ToSchema` can be adapted with
`roundhouse::interop::impl_from_utoipa!`, or documented by an explicit `DocumentedResponse` impl;
`doc = |docs| …` remains the escape hatch for metadata the declarations cannot express.

For generated types that cannot carry the derive, use the interop adapter:

```rust
roundhouse::interop::impl_from_utoipa!(generated::Row, generated::OtherRow);
```

It requires `utoipa::ToSchema` on the type and publishes the component plus everything reachable
through `ToSchema::schemas()`. A blanket `impl<T: ToSchema> ApiSchema for T` is not possible; it
would overlap the derive and the primitive implementations.

`QueryParams` and `PathParams` derive the parameter documentation for query and path structs.
A field's `@description:` tag supplies the parameter description; `#[roundhouse(description = "…")]`
takes priority. `RepeatedQuery<T>` reads repeated query keys (`?tag=a&tag=b`); axum's `Query<T>`
cannot represent them.

### Extending the document

Extractors stay axum's. Implement `roundhouse::ExtractArg` for route-level validation and, under
`build`, `roundhouse::doc::ExtractDoc` to document the argument. A custom success type
implements `roundhouse::doc::DocumentedResponse`; a custom error type implements
`roundhouse::doc::DocumentedError`. Implementing `IntoResponse` alone leaves the outcome unknown.

An `impl IntoResponse` handler is documented from its body: the build-time analyzer resolves the
tail and `return` expressions, and the macro emits typed trait calls that the compiler verifies.
An expression it cannot resolve marks the operation `x-roundhouse-unverified-response` and the
strict build rejects it unless `#[handler(response(..))]` supplies the missing status.
`.into_response()` keeps the receiver's documented type (`Json<Note>.into_response()` still
documents `Json<Note>`); only a receiver that is itself erased — a bare `Response`, a trait object,
or a helper returning `Response` — stays opaque. `.into_documented_response()` additionally erases
the value to `Documented`, so branches with different documented response types can share one
`impl IntoResponse` return type. An explicit declaration remains the escape hatch.

An extractor without an `ExtractDoc` impl (or a body type without `ApiSchema`) is still accepted,
but the operation is marked `x-roundhouse-unverified-extractor`/`-body` and the strict build
turns that marker into an error. See
[Extension points](docs/design/extension-points.md) for the trait contracts and examples.

Component names are the Rust type name by default. Two different Rust types that claim one name
are a `BuildError::ComponentCollision`; use `#[roundhouse(name = "module.Name")]` on one of them.
The document uses the dotted form (`module.Name`).

## Validation

With `validation` enabled (the default), a route validates the arguments whose types publish
constraints. The production source of those constraints is `.roundhouse/contract.json`, written
by the contract build. A `build` build without a bundle builds the contract in-process for
tests and checks. The emitted JSON Schema is both the published shape and the enforcer:

- JSON bodies are validated from the raw request bytes before extraction, so a request type does
  not need `Serialize` bounding just to be checked.
- Path, query and form values are validated by the route shim, including handlers written against
  axum's own extractors.
- `validate = false` skips constraints for one route. Malformed input is still rejected.
- `validate_body = false` keeps path, query and form validation while treating the body as
  opaque. This is the route policy for a large payload that should be parsed but not buffered and
  checked twice.
- `unverified = "reason"` acknowledges an intentional whole-route opt-out or an unverifiable
  body/response/extractor so the strict build passes.
- `App::validation_status(StatusCode::UNPROCESSABLE_ENTITY)` changes the status used for
  validation failures (`400` by default).
- `App::validation(Validation::Off)` disables schema execution for the app.
- `App::verify_responses()` checks actual responses against the committed contract and reports a
  mismatch as a `500` problem; `cargo roundhouse verify` enables the same check from CI.

`Strict<T>` makes enforcement part of the extractor (`Strict<axum::Json<T>>`,
`Strict<Path<T>>`, and so on). `Mapped<T>` only converts a foreign extractor's rejection into a
`Problem` and does not apply a schema.

A JSON body is buffered and parsed once by the schema engine and once by the extractor. That keeps
the `Serialize` bound off the handler and validates the bytes the client sent; the body limit
bounds the memory (2 MiB by default, `App::max_body` changes it). `validate = false` marks the
body unverified and requires `unverified = "reason"`; `validate_body = false` is the body-only
policy and records `x-roundhouse-body-validation: false`.

The rationale and the full enforcement matrix are in
[Validation](docs/design/validation.md).

### Performance

`cargo bench -p roundhouse --bench throughput` measures the request path against plain axum.
On the development container, a small validated JSON request costs roughly 2–4 µs more than the
same handler on plain axum; `validate_body = false` still costs roughly 1.5–2 µs more, so the
layer/shim path dominates and the schema engine is a smaller share. The benchmark prints both
small and ~20 KiB bodies; run it on the target hardware before making deployment decisions.

`cargo bench -p roundhouse --bench build` compares the full contract build with the production
`contract` startup on the same route table. On the development container the build path is
about 80 µs per app and the committed-bundle path about 25 µs, so the difference is the
documentation work the deployed binary does not perform.

`jsonschema` is the only validation engine; `validate_with` covers rules a schema cannot express.
See [Decisions](docs/design/decisions.md) for the tradeoffs behind the validation pipeline.

Failures carry JSON Pointers and stable codes:

```json
{
  "type": "https://roundhouse.rs/problems/validation",
  "title": "Validation failed",
  "status": 400,
  "errors": [
    { "pointer": "/email", "code": "format", "message": "must be a valid email address" }
  ]
}
```

## Auth

An auth scheme is a type that implements `AuthScheme` (name, OpenAPI kind, state and identity type)
and `Authenticate` (how to read credentials and return an identity):

```rust
pub struct BearerAuth;

impl AuthScheme for BearerAuth {
    type State = AppState;
    type Identity = User;

    const NAME: &'static str = "bearer";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::HttpBearer;
}

impl Authenticate for BearerAuth {
    async fn authenticate(parts: &mut Parts, state: &AppState) -> Result<User, Problem> {
        // Verify the request and return the identity.
        let _ = (parts, state);
        unimplemented!()
    }

    fn presents_credentials(parts: &Parts) -> bool {
        // Used by Option<Auth<BearerAuth>>.
        let _ = parts;
        false
    }
}
```

- `Auth<A>` extracts the identity and documents the security requirement.
- `Auth<A, Scopes>` also requires `Scopes: Scopes<A>`. The scope is documented only when an
  authorization decision exists.
- `Option<Auth<A>>` documents authentication as optional.
- `security = [A]` on a route or module installs the guard as well as publishing the requirement.
- Multiple schemes form one AND requirement.
- A handler that already extracts `Auth<A>` is not authenticated a second time.
- `401`/`403` responses are documented from the scheme kind, and HTTP auth rejections send a
  matching `WWW-Authenticate` header.

The schemes in `examples/auth` are a small reference implementation. Production authentication
should verify signatures, expiry, audience and issuer with a maintained library or gateway.

## Errors

`Problem` is the default error body for malformed JSON, rejected extractors, failed constraints,
unmatched routes (`404`) and unsupported methods (`405`). It is a helper, not a requirement:
handlers may return any error type that implements `IntoResponse` and
`roundhouse::doc::DocumentedError`.

Named response types come from their traits. The example below returns `Problem`, which documents
OpenAPI's catch-all `default` response:

```rust
/// @response 200: The stored note.
/// @response 404: The requested note does not exist.
#[handler(response(404, Problem))]
pub async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> Result<Json<Note>, Problem> {
    match app.get(id) {
        Some(note) => Ok(Json(note)),
        None => Err(Problem::not_found("Note not found")
            .type_uri("https://roundhouse.rs/problems/NoteNotFound")
            .detail(format!("note {id} was not found"))),
    }
}
```

- `Problem::not_found(..)`, `bad_request(..)`, `conflict(..)`, `internal(..)` and
  `new(status, title)` are the typed error constructors.
- A blanket `From<E> for Problem` turns infrastructure errors into `500`; `?` works without an
  `InternalError` enum.
- `Created<T>`, `NoContent`, `Status<CODE, T>` and `Empty<CODE>` cover the common and arbitrary
  status responses. The named wrappers in `roundhouse::status` (`NotFound`, `Conflict`,
  `Accepted`, …) are generated from the vendored IANA registry; refresh that snapshot with
  `cargo run -p roundhouse-status-codes --bin update-status-codes`.
- A custom success type implements `roundhouse::doc::DocumentedResponse`; a custom error implements
  `roundhouse::doc::DocumentedError`. Mark the impl with `#[roundhouse::build_only]` so a
  production build that compiles only `contract` drops it; otherwise the documentation
  types do not exist in that build.
- `?` documents the handler's error type `E`, not the source of the question mark. If
  `From<DbError> for Problem` drops a status, the document says `Problem`'s `default`; return
  `DbError` when its statuses matter.
- An `impl IntoResponse` handler is documented from its body. `.into_response()` preserves the
  receiver's documentation; only an already-erased `Response` stays opaque.
  `.into_documented_response()` additionally erases the value to `Documented`, so different
  documented branches can share one return type.
- `#[handler(response(..))]` remains the escape hatch for a response the analyzer cannot resolve or
  an intentionally undocumented path. `doc = |docs|` handles hand-written prose, headers or
  metadata.

`App::document_rejections(false)` disables automatic extractor rejection responses, leaving only
the responses supplied by handlers and declarations.

## Files and streams

With the `multipart` feature, derive `roundhouse::Multipart` and extract with
`Multipart<F>`:

```rust
#[derive(roundhouse::Multipart)]
pub struct Upload {
    pub file: FilePart,
    #[file(max_bytes = 1_000_000, accept = ["text/plain", "text/markdown"])]
    pub notes: Option<FilePart>,
    pub tags: Vec<String>,
}
```

`FilePart` buffers a part in memory. `TempFile` (feature `multipart-tempfile`) streams a part to
an already-unlinked temporary file. `#[file(max_bytes = ..)]` bounds one part, `accept` checks its
media type, and `#[multipart(deny_unknown)]` rejects unexpected fields. `App::max_body()` and
`App::max_parts()` set the app-wide limits.

`Attachment<T>`, `File` and axum's `Sse` document their response headers and content types.

## The document

The OpenAPI document is generated by the contract build. `.contract(include_contract!())` embeds
and serves the committed artifact; a `build` build without a bundle builds one in-process for
tests and checks. Production configuration is only routing and UI: `App::openapi_route(..)`
chooses the document path, `App::docs()` serves a Swagger UI at `/docs`, and
`App::docs_route(..)` moves it.

The document's title, version, servers and shared response prose belong in `api!`.
`App::openapi(..)` is a `build`-only build-time override for tests and tools, not a production
runtime setting.

With the default `swagger-ui` feature, the CSS, JavaScript and OAuth assets are embedded in the
binary and served by the app, so `/docs` works in an air-gapped deployment. `App::docs_cdn()` opts
back into the pinned unpkg assets. `App::swagger()` serves the same local UI at `/docs/swagger`,
and `App::swagger_route(..)` moves it; `App::docs()` and `App::swagger()` can be used together.

```rust
let built = app(state).build();
std::fs::write("target/openapi.json", built.openapi_json())?;
```

`Built::document()` returns the `OpenApi` value, `Built::openapi_30()` returns a 3.0.3-shaped
document, `Built::routes()` returns the resolved route table with source locations, and
`Built::diagnostics()` returns non-fatal observations. These document accessors require the
`build` feature. A `contract` production build serves the committed
`.roundhouse/openapi.json` instead (`App::openapi_route(..)`, `Built::openapi_json()`).
`Built::router()` and `App::into_router()` return the ready `axum::Router<()>` in every
configuration; application state is already applied.

`api! servers = [..]` or the `build`-only `App::server(..)`/`App::servers(..)` add OpenAPI
server entries. `App::document(..)`/`Contract::document(..)` run a hook over the finished document
for metadata the declarations cannot derive, such as tag descriptions.
`App::layer_documented(..)` records the responses, parameters, bodies and auth guarantees of a
tower layer that terminates requests. These are contract-build APIs; a production build does not
run them.

Use `App::try_build()` in a test or check binary so CI fails without starting a server:

```rust
#[test]
fn routes_build() {
    app(state()).try_build().expect("route declarations are consistent");
}
```

Strict builds are the default: diagnostics are `BuildError`s. This includes undocumented response
types, unverified bodies, empty modules and routers added with `App::merge`. `App::permissive()`
downgrades them to warnings. Merged routers are served but cannot be discovered for the document,
so the document is marked `x-roundhouse-merged` and the merge is recorded in
`Built::merges()`. Use `App::merge_undocumented(router, reason)` to acknowledge one.

Every contract build validates the finished document: path captures match parameters, operation
ids are unique, every security requirement names a declared scheme, and every operation has a
response. The runtime build validates the route table and the embedded contract version.

## Features

| Feature | Enables |
|---|---|
| `build` | Schema derivation and the document builder (`App::openapi`, `App::document`, `roundhouse::openapi`/`interop`); implies `contract` |
| `contract` | `ContractBundle`, `include_contract!`, `App::contract`, `contract.json` request validation, response verification and verbatim `openapi.json` serving; no utoipa |
| `validation` | Execute the emitted JSON Schema for request validation |
| `server` | `App::serve`/`App::serve_with`, axum's `ConnectInfo<T>`, and the tokio server runtime |
| `swagger-ui` | Vendored Swagger UI assets for local `App::docs()` and `App::swagger()` |
| `multipart` | `Multipart<F>`, `FilePart` and axum's multipart support |
| `multipart-tempfile` | `TempFile` (implies `multipart`) |
| `uuid`, `chrono`, `time`, `jiff`, `decimal` | Schema support for those types |

`#[roundhouse::schema]` is the normal way to document a type. With `build` it injects the
schema derive; without it the type keeps its runtime validation hooks and no utoipa is compiled.
Projects that already use utoipa can import `roundhouse::interop::{ToSchema, PartialSchema,
Components, utoipa}` explicitly when `build` is enabled.

## Examples

Run any example from the repository root:

| Command | Port | What it demonstrates |
|---|---|---|
| `cargo quickstart` | 3000 | The smallest app: one library, one table, schemas and a server |
| `cargo notes` | 3001 | Multi-file composition, response inference, `?` errors and custom `DocumentedError` |
| `cargo layers` | 3002 | Global, module and entry layers; fallback; embedding |
| `cargo uploads` | 3003 | Multipart, `TempFile`, attachments, SSE and the docs UI |
| `cargo auth` | 3004 | Scopes, optional auth, two-scheme AND and the auth guard |

`examples/quickstart` is the smallest starting point; `examples/notes` shows the multi-file
layout and the full response-inference pattern: opaque `impl IntoResponse` successes, `?` on two
different error types, custom `DocumentedError` impls marked with
`#[roundhouse::build_only]`, and a build script that generates the handler index. Each example
commits its `.roundhouse/` artifacts and builds production with `contract`, so
`cargo tree -p <example>` has no utoipa; the examples that read the typed document enable
`build` in their dev-dependencies. Run `cargo roundhouse check` inside an
example after changing handlers.
Detailed feature coverage lives in the crate's integration tests. The route, schema,
validation, auth and layering contracts are exercised there without a runnable example for every
feature. `tests/fixtures/renamed` checks that the macros work when the dependency is renamed.

## Development

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features

# After an intentional document change, regenerate the golden file:
ROUNDHOUSE_BLESS=1 cargo test -p roundhouse --test spine openapi_golden
```

The dev container is available with VS Code *Reopen in Container*, or:

```sh
podman compose -f compose.dev.yaml up -d
podman compose -f compose.dev.yaml exec dev bash
```

Full API documentation:

```sh
cargo doc --workspace --no-deps --open
```

The rationale behind the API lives in [`docs/design`](docs/design): the architecture, declaration
and contract-build phases, runtime and validation behavior, document assembly, extension points
and the decision log. Start with [Architecture](docs/design/architecture.md).

## License

MIT OR Apache-2.0.
