# roundhouse

An OpenAPI-first framework for [axum](https://github.com/tokio-rs/axum). You declare routes,
parameters, schemas, auth and errors once; roundhouse builds the `axum::Router` and the OpenAPI 3.1
document from the same declarations. The module tree is the complete route set.

```rust
use roundhouse::prelude::*;

#[derive(serde::Serialize, ToSchema)]
pub struct Hello {
    pub message: String,
}

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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    roundhouse::app!(())
        .openapi(|| roundhouse::openapi::Info::new("Hello", "0.1.0"))
        .serve("0.0.0.0:3000")
        .await?;
    Ok(())
}
```

`GET /v1/hello/world` answers JSON, and `GET /openapi.json` documents the same operation,
including the `name` path parameter that `Path<String>` cannot name by itself.

## How it works

1. **Declare.** A `routes!` table emits a module containing its prefix, tags, layers, security
   guards and routes, plus the child modules composed with `...module`. `#[handler]` reads the
   handler signature and produces the request, response and security documentation for it.
2. **Build.** `App::build()` walks the module tree and produces a `Built`: the `axum::Router`, the
   resolved route table and the OpenAPI document. `App::try_build()` performs the same checks but
   reports failures as a `BuildError`.
3. **Enforce.** The same schema that is published is executed for request validation. Declared
   security installs the matching guard. Rejections use the `Problem` body.

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

    "/admin" [security = [Bearer]] {
        "/audit" => {
            get(audit),
        },
    },

    ...shared,
}
```

The path-group form is the only entry form. The supported methods are `get`, `post`, `put`,
`patch`, `delete`, `head`, `options`, `trace` and `any`. Paths use `{capture}` and `{*rest}`
syntax.

Operation prose and response declarations come from doc tags on the handler:

```rust
/// @summary: Create a note.
/// @description: Persists the note and returns the stored representation.
/// @responses:
///   201: Note
///   409: Problem
#[handler]
pub async fn create(...) -> ... { ... }
```

- Untagged doc prose is ignored for OpenAPI; it remains rustdoc-only.
- `@summary:` is valid on handlers. It ends at a blank line.
- `@description:` is valid on handlers, response variants, response body types and query/path
  fields. It keeps blank lines as paragraphs.
- `@responses:` declares `status: Type`, bare `status`, or `default: Type` lines. It ends at a
  blank line. A bare status asserts a success response; if the handler already inferred a
  response for that status, the inferred shape is kept. That is the cheap way to satisfy the
  compile-time success check for a `Result` alias.
- `@response-descriptions:` overrides the description of an individual documented response, for
  example `200: A stored thing.`
- `@responses(override):` replaces an inferred response.
- `@path: name: description` describes a path parameter from the handler; it is useful for a
  scalar `Path<T>` that cannot carry a field description. A `PathParams` field description wins.
- `Json<T>` in `@responses:` is unwrapped to `T` for the body schema.

The full tag grammar and precedence rules are in
[`docs/design/doc-tags.md`](docs/design/doc-tags.md).

### Entry options

| Option | Effect |
|---|---|
| `operation_id = "..."` | Choose the operation id. Without it, an id is derived from the method, module path and handler name. |
| `tags = ["..."]` | Add tags to the operation. |
| `security = [Scheme]` | Require a scheme for this operation. The scheme is enforced by the route shim. |
| `path = "/full/path"` | Assert the final resolved path, including module prefixes. |
| `layer = expression` | Apply a tower layer to this route. |
| `doc = expression` | Run a hook over the operation's `OperationDoc` before document checks. |
| `prose = expression` | Take the summary and description from a `&'static str`, usually `include_str!(...)`. The first line is the summary. |
| `hidden` | Serve the route without documenting it. |
| `absolute` | Use the path as the full URL, ignoring inherited prefixes, tags, layers and guards. |
| `deprecated` | Mark the operation deprecated. |
| `public` | Add the `x-roundhouse-public` extension to the operation. |
| `validate = false` | Skip schema constraint validation for the whole route. The request still has to parse. |
| `validate_body = false` | Keep path/query/form validation, but treat the request body as opaque. Malformed JSON still fails. |
| `unverified = "reason"` | Acknowledge an intentional unverified body/response/extractor so `strict()` passes. |

### Module options

`prefix` is declared before a table's first entry; a path group uses its path as the prefix. Every route entry lives inside a path group.
`state`, `tags`, `security`, `layer`, `layer_mode`, `hidden` and `doc` can be declared before a
table's first entry or in the brackets of a path group. `layer_mode = full` applies a module layer
to the whole subtree; `layer_mode = route` uses `route_layer`.

Root an app with `roundhouse::app!(state)` from the module that declares the table, or
`roundhouse::app!(state, routes)` to start at another module's table. `App::include` and
`App::fragment` compose roots directly.

### State and `FromRef` substates

The app state is exact at the `App::new` boundary. A table names it once, and handlers may take
substates through axum's `FromRef`:

```rust
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
    state = AppState,
    prefix = "/notes",
    "/" => { get(list) },
}

#[handler]
pub async fn list(State(db): State<Db>) -> Json<Vec<Note>> { ... }
```

Inline path groups inherit the table's `state`; a group can override it. A spread module is a
separate `routes!` expansion, so it declares its own `state` when it uses substates. A table
without `state = ...` keeps the old default: a single `State<T>` argument, the auth scheme's state,
or `()`. `Auth<A>` remains exact: `AuthScheme::State` must equal the table state.

If every table in an application uses the same state, a crate-local wrapper removes the repeated
type name while keeping the declaration explicit:

```rust
macro_rules! routes {
    ($($tokens:tt)*) => {
        roundhouse::routes! { state = crate::AppState, $($tokens)* }
    };
}
```

A spread cannot inherit state because it is a separate macro expansion: the child table's route
records already contain their `MethodRouter<S>`. The wrapper is the supported way to centralize
the name.

## Schemas

`utoipa::ToSchema` defines the underlying schema shape. `#[roundhouse::schema]` derives both
`ToSchema` and `ApiSchema` with one annotation; the explicit
`#[derive(ToSchema, ApiSchema)]` form remains available. `#[derive(ApiSchema)]` adds:

- a named component (`$ref`) for the type;
- `#[roundhouse(unique_items)]`;
- the `url` to `uri` format mapping;
- `#[roundhouse(validate_with = path)]` hooks.

```rust
#[roundhouse::schema]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[schema(min_length = 3, max_length = 32)]
    pub user_name: String,

    #[schema(format = "email")]
    pub email: String,

    #[roundhouse(validate_with = validate_age)]
    pub age: u8,
}
```

`axum::Json<T>`, `Form<T>` and a JSON response require `T: ApiSchema` when the framework documents
them automatically. A type that only derives `ToSchema` can still be published with an explicit
`@responses:` declaration on the handler, or with a `doc = |docs| …` hook.

For generated types that cannot carry the derive, use the opt-in adapter:

```rust
roundhouse::impl_api_schema!(generated::Row, generated::OtherRow);
```

It requires `ToSchema` on the type and publishes the component plus everything reachable through
`ToSchema::schemas()`. A blanket `impl<T: ToSchema> ApiSchema for T` is not possible; it would
overlap the derive and the primitive implementations.

`QueryParams` and `PathParams` derive the parameter documentation for query and path structs.
A field's `@description:` tag supplies the parameter description; `#[schema(description = "…")]`
takes priority. `RepeatedQuery<T>` reads repeated query keys (`?tag=a&tag=b`); axum's `Query<T>`
cannot represent them.

### Extending the document

Extractors stay axum's. To teach roundhouse about a custom extractor, implement
`roundhouse::doc::ExtractArg` for it. The handler macro asks each argument how it documents itself
and whether route-level validation has any work to do.

```rust
impl ExtractArg for Tenant {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            params: vec![
                Param::new("x-tenant", ParamIn::Header, schema::string())
                    .description("tenant id"),
            ],
            ..ArgDoc::default()
        }
    }
}
```

A custom response type implements `roundhouse::doc::ApiResponse`; `SUCCESS` supplies the
compile-time success statuses and `responses` supplies the documented bodies.

An extractor without an `ExtractArg` impl (or a body type without `ApiSchema`) is still accepted,
but the operation is marked `x-roundhouse-unverified-extractor`/`-body` and `App::strict()` turns
that marker into a build error.

Component names come from utoipa. Two different Rust types that claim one component name are a
`BuildError::ComponentCollision`; use `#[schema(as = module::Name)]` on one of them. Utoipa formats
the name with dots (`module.Name`) in the document.

## Validation

With `schema-engine` enabled (the default), a route validates the arguments whose types publish
constraints. The emitted JSON Schema is the source of those constraints, and it is also the
enforcer:

- JSON bodies are validated from the raw request bytes before extraction, so a request type does
  not need `Serialize` bounding just to be checked.
- Path, query and form values are validated by the route shim, including handlers written against
  axum's own extractors.
- `validate = false` skips constraints for one route. Malformed input is still rejected.
- `validate_body = false` keeps path, query and form validation while treating the body as
  opaque. This is the route policy for a large payload that should be parsed but not buffered and
  checked twice.
- `unverified = "reason"` acknowledges an intentional whole-route opt-out or an unverifiable
  body/response/extractor so `strict()` passes.
- `App::validation_status(StatusCode::UNPROCESSABLE_ENTITY)` changes the status used for
  validation failures (`400` by default).
- `App::validation(Validation::Off)` disables schema execution for the app.
- `App::validate_responses()` validates JSON response bodies against the document at runtime and
  reports a mismatched response as a `500` problem.

`Strict<T>` makes enforcement part of the extractor (`Strict<axum::Json<T>>`,
`Strict<Path<T>>`, and so on). `Mapped<T>` only converts a foreign extractor's rejection into a
`Problem` and does not apply a schema.

A JSON body is buffered and parsed once by the schema engine and once by the extractor. That is
deliberate: it keeps the `Serialize` bound off the handler and validates the bytes the client
sent. The body limit bounds the memory; the default is 2 MiB, and `App::max_body` changes it.
`validate = false` is not a silent performance switch: it marks the body unverified, and a strict
build rejects it unless the entry acknowledges the decision with `unverified = "reason"`.
`validate_body = false` is the precise alternative: it records
`x-roundhouse-body-validation: false`, compiles no body validator, and does not warn.
`Strict<Json<T>>` honors both policies. A future after-extraction mode for `Serialize` body types
would avoid the second parse; streaming JSON Schema validation is not planned, because
`required`, `additionalProperties` and `oneOf` need the whole value.

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

`Problem` is the error body for malformed JSON, rejected extractors, failed constraints, unmatched
routes (`404`) and unsupported methods (`405`).

The intended shape is one outcome enum on the `Ok` side and one crate-wide infrastructure type on
the `Err` side:

- **Outcomes** are the statuses an endpoint can actually return. Model them as an
  `#[derive(ApiResponse)]` enum: `Found(Json<Note>)`, `NotFound { id }`, `Conflict`, and so on.
- **Infrastructure** is the shared failure surface: storage, network, serialization. Derive
  `ApiError` once for the crate and use it as the `Err` side of every fallible handler.
- `Result<Outcome, InternalError>` documents both sides independently, so each route advertises
  its real outcomes plus the common `500`.
- Avoid a shared *domain* error enum on every handler: it makes every route document errors it
  cannot produce. Avoid a per-endpoint error enum as well; the outcome enum is the right place
  for endpoint-specific failures.

`#[derive(ApiError)]` and `#[derive(ApiResponse)]` generate `IntoResponse` plus the documented
responses for their variants:

```rust
// The endpoint's outcomes.
#[derive(ApiResponse)]
pub enum ShowNote {
    #[status(200, description = "The stored note.")]
    Found(Json<Note>),

    #[status(404, problem = "NoteNotFound")]
    NotFound { id: i64 },
}

// The crate-wide infrastructure error.
#[derive(Debug, thiserror::Error, ApiError)]
pub enum InternalError {
    #[error("storage failed: {0}")]
    #[status(500)]
    Storage(#[from] std::io::Error),
}

#[handler]
pub async fn show(
    State(app): State<AppState>,
    Path(id): Path<NoteId>,
) -> Result<ShowNote, InternalError> {
    Ok(app
        .get(id)
        .map(Json)
        .map(ShowNote::Found)
        .unwrap_or(ShowNote::NotFound { id }))
}
```

Variant `@description:` tags become the documented response description; `#[status(description =
"…")]` remains the explicit override. Response and error enums may be generic. `Created<T>` and
`NoContent` are built-in wrappers for the common created/no-content responses.

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

The OpenAPI document is always built. `App::openapi(..)` sets the info and serves it at
`/openapi.json`; `App::openapi_route(..)` chooses another path. `App::docs()` serves a Swagger UI
at `/docs`, and `App::docs_route(..)` moves it.

With the default `swagger` feature, the CSS, JavaScript and OAuth assets are embedded in the
binary and served by the app, so `/docs` works in an air-gapped deployment. `App::docs_cdn()` opts
back into the pinned unpkg assets. `App::swagger()` serves the same local UI at `/docs/swagger`,
and `App::swagger_route(..)` moves it; `App::docs()` and `App::swagger()` can be used together.

```rust
let built = app(state).build();
std::fs::write("target/openapi.json", built.openapi_json())?;
```

`Built::document()` returns the `OpenApi` value, `Built::openapi_30()` returns a 3.0.3-shaped
document, `Built::routes()` returns the resolved route table with source locations, and
`Built::diagnostics()` returns non-fatal observations. `Built::router()` and
`App::into_router()` return the ready `axum::Router<()>`; application state is already applied.

`App::server(..)` and `App::servers(..)` add OpenAPI server entries. `App::document(..)` runs a
hook over the finished document for metadata the declarations cannot derive, such as tag
descriptions. `App::layer_documented(..)` records the responses, parameters, bodies and auth
guarantees of a tower layer that terminates requests.

Use `App::try_build()` in a test or check binary so CI fails without starting a server:

```rust
#[test]
fn routes_build() {
    app(state()).try_build().expect("route declarations are consistent");
}
```

`App::strict()` promotes diagnostics into `BuildError`s. This includes undocumented response
types, unverified bodies, empty modules and routers added with `App::merge`. Merged routers are
served but cannot be discovered for the document, so the document is marked
`x-roundhouse-merged` and the merge is recorded in `Built::merges()`.

Every build also validates the finished document: path captures match parameters, operation ids
are unique, every security requirement names a declared scheme, and every operation has a
response.

## Install

Upgrading from 0.2? See [`MIGRATING-0.3.md`](MIGRATING-0.3.md) for the route-table and doc-tag
changes.

```sh
cargo add roundhouse
cargo add axum
cargo add serde --features derive
```

Rust 1.88 or newer is required. The default features are `schema-engine`, `serve` and `swagger`;
the macros are always part of the framework.

| Feature | Enables |
|---|---|
| `schema-engine` | Execute the emitted JSON Schema for request and response validation |
| `serve` | `App::serve`/`App::serve_with`, axum's `ConnectInfo<T>`, and the tokio server runtime |
| `swagger` | Vendored Swagger UI assets for local `App::docs()` and `App::swagger()` |
| `multipart` | `Multipart<F>`, `FilePart` and axum's multipart support |
| `multipart-tempfile` | `TempFile` (implies `multipart`) |
| `uuid`, `chrono`, `time`, `jiff`, `decimal` | Schema support for those types |

`roundhouse::prelude` re-exports `ToSchema` and the `utoipa` module used by the derives.
`#[derive(ApiSchema)]` needs `ToSchema` on the same type; `#[roundhouse::schema]` derives both at
once.

## Examples

Run any example from the repository root:

| Command | Port | What it demonstrates |
|---|---|---|
| `cargo notes` | 3000 | Explicit module composition, schemas and errors |
| `cargo blog` | 3001 | Nested modules, grouped paths, query parameters and auth |
| `cargo layers` | 3002 | Global, module and entry layers; fallback; embedding |
| `cargo validation` | 3003 | Constraints, JSON Pointers and `validate = false` |
| `cargo uploads` | 3004 | Multipart, `TempFile`, attachments, SSE and the docs UI |
| `cargo showcase` | 3005 | Enums, `validate_with`, utoipa interop and response validation |
| `cargo auth` | 3006 | Scopes, optional auth, two-scheme AND and the auth guard |

`examples/notes` is the smallest starting point. `examples/renamed` checks that the macros work
when the dependency is renamed.

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

The rationale behind the API lives in [`docs/design`](docs/design): the crate and feature
policy, the routing and document pipelines, validation, extensibility and macro diagnostics.

## License

MIT OR Apache-2.0.
