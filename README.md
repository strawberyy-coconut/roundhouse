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
    get("/hello/{name}", hello),
}

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

A table can declare routes directly, group them by path, and compose other tables:

```rust
routes! {
    prefix = "/api",
    tags = ["api"],
    layer = tower_http::trace::TraceLayer::new_for_http(),

    get("/health", health, summary = "Liveness probe"),

    "/notes" => {
        get(list, summary = "List notes"),
        post(create, summary = "Create a note", responses(201 = Note)),
    },

    "/notes/{id}" => {
        get(show),
        delete(destroy, responses(204)),
    },

    "/admin" [security = [Bearer]] {
        get "/audit" => audit,
    },

    ...shared,
}
```

The supported methods are `get`, `post`, `put`, `patch`, `delete`, `head`, `options`, `trace`
and `any`. Paths use `{capture}` and `{*rest}` syntax.

### Entry options

| Option | Effect |
|---|---|
| `summary = "..."`, `description = "..."` | Set the operation's OpenAPI summary and description. |
| `operation_id = "..."` | Choose the operation id. Without it, an id is derived from the method, module path and handler name. |
| `tags = ["..."]` | Add tags to the operation. |
| `security = [Scheme]` | Require a scheme for this operation. The scheme is enforced by the route shim. |
| `responses(status, status = Type)` | Add or refine documented responses. A bare status is an empty response; `status = Type` is a JSON body. |
| `override` | Allow `responses(..)` to replace an inferred response instead of reporting a conflict. |
| `path = "/full/path"` | Assert the final resolved path, including module prefixes. |
| `layer = expression` | Apply a tower layer to this route. |
| `doc = expression` | Run a hook over the operation's `HandlerDocs` before document checks. |
| `hidden` | Serve the route without documenting it. |
| `absolute` | Use the path as the full URL, ignoring inherited prefixes, tags, layers and guards. |
| `deprecated` | Mark the operation deprecated. |
| `public` | Add the `x-roundhouse-public` extension to the operation. |
| `validate = false` | Skip schema constraint validation for this route. The request still has to parse. |

### Module options

`prefix` is declared before a table's first entry; a path group uses its path as the prefix.
`tags`, `security`, `layer`, `layer_mode`, `hidden` and `doc` can be declared before a table's
first entry or in the brackets of a path group. `layer_mode = full` applies a module layer to the
whole subtree; `layer_mode = route` uses `route_layer`.

Root an app with `roundhouse::app!(state)` from the module that declares the table, or
`roundhouse::app!(state, routes)` to start at another module's table. `App::include` and
`App::fragment` compose roots directly.

## Schemas

`utoipa::ToSchema` defines schema shape. `#[derive(ApiSchema)]` is optional and adds:

- a named component (`$ref`) for the type;
- `#[roundhouse(unique_items)]`;
- the `url` to `uri` format mapping;
- `#[roundhouse(validate_with = path)]` hooks.

```rust
#[derive(serde::Serialize, serde::Deserialize, ToSchema, ApiSchema)]
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

`QueryParams` and `PathParams` derive the parameter documentation for query and path structs.
`RepeatedQuery<T>` reads repeated query keys (`?tag=a&tag=b`); axum's `Query<T>` cannot represent
them.

Component names come from utoipa. Use `#[schema(as = module::Name)]` when two types would
otherwise share a component name.

## Validation

With `schema-engine` enabled (the default), a route validates the arguments whose types publish
constraints. The emitted JSON Schema is the source of those constraints, and it is also the
enforcer:

- JSON bodies are validated from the raw request bytes before extraction, so a request type does
  not need `Serialize` bounding just to be checked.
- Path, query and form values are validated by the route shim, including handlers written against
  axum's own extractors.
- `validate = false` skips constraints for one route. Malformed input is still rejected.
- `App::validation_status(StatusCode::UNPROCESSABLE_ENTITY)` changes the status used for
  validation failures (`400` by default).
- `App::validation(Validation::Off)` disables schema execution for the app.
- `App::validate_responses()` validates JSON response bodies against the document at runtime and
  reports a mismatched response as a `500` problem.

`Strict<T>` makes enforcement part of the extractor (`Strict<axum::Json<T>>`,
`Strict<Path<T>>`, and so on). `Mapped<T>` only converts a foreign extractor's rejection into a
`Problem` and does not apply a schema.

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
- `Auth<A, Scopes>` also requires `Scopes: Authorize<A>`. The scope is documented only when an
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

`#[derive(ApiError)]` and `#[derive(ApiResponse)]` generate `IntoResponse` plus the documented
responses for their variants:

```rust
#[derive(Debug, thiserror::Error, ApiError)]
pub enum NoteError {
    #[error("note {0} was not found")]
    #[status(404, problem = "NoteNotFound")]
    NotFound(i64),

    #[error("the note was invalid")]
    #[status(422)]
    Invalid(#[from] ValidationErrors),
}
```

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
at `/docs`, and `App::docs_route(..)` moves it. The UI loads pinned Swagger UI assets from unpkg.

```rust
let built = app(state).build();
std::fs::write("target/openapi.json", built.openapi_json())?;
```

`Built::document()` returns the `OpenApi` value, `Built::openapi_30()` returns a 3.0.3-shaped
document, `Built::routes()` returns the resolved route table with source locations, and
`Built::diagnostics()` returns non-fatal observations.

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

```sh
cargo add roundhouse
cargo add axum
cargo add serde --features derive
```

Rust 1.88 or newer is required. The default features are `macros` and `schema-engine`.

| Feature | Enables |
|---|---|
| `macros` | `routes!`, `#[handler]` and the derives |
| `schema-engine` | Execute the emitted JSON Schema for request and response validation |
| `multipart` | `Multipart<F>`, `FilePart` and axum's multipart support |
| `multipart-tempfile` | `TempFile` (implies `multipart`) |
| `uuid`, `chrono`, `time`, `jiff`, `decimal` | Schema support for those types |

`roundhouse::prelude` re-exports `ToSchema` and the `utoipa` module used by the derives.
`#[derive(ApiSchema)]` needs `ToSchema` on the same type.

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

## License

MIT OR Apache-2.0.
