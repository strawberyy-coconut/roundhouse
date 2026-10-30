# roundhouse

An OpenAPI-first framework for [axum](https://github.com/tokio-rs/axum). Declare routes, parameters,
schemas, auth and errors once; roundhouse builds the `axum::Router` and the OpenAPI 3.1 document
from the same declarations. The route tree is the wiring: no link-time registry, no hand-maintained spec.

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

`GET /v1/hello/world` answers JSON, and `GET /openapi.json` describes it — including the `name` path
parameter that `Path<String>` cannot name by itself.

## Features

| Feature | What you get |
|---|---|
| **Declarative routing** | `routes!` tables live beside their handlers and also declare the module's prefix, tags, layers and enforced `security`. Child modules compose with `...module`; `App::build()` walks that explicit tree. |
| **Single source of truth** | Paths, parameters, bodies, responses and security requirements are derived from the router's own declarations, so the document matches what is enforced. `security = [Bearer]` installs the `Auth<Bearer>` guard, not just the OpenAPI requirement. |
| **Early diagnostics** | Duplicate routes and operation ids, missing success responses and auth/state mismatches are compile errors where the declaration is local. Checks that need the composed tree — collisions across modules, prefix joins, the final document — are `BuildError`s, and `App::try_build()` never panics. |
| **Document as artifact** | The OpenAPI document is always built; `App::openapi(..)` only decides whether it is served. `Built::openapi_json()` lets CI diff it. |
| **Typed extractors** | `#[handler]` reads `State`, `Path`, `Query`, `Json`, `Form`, `Multipart`, `Auth`, `Option<Auth>` and `Extension` from the signature. `Json`, `Form`, `Path`, `Query` and `Extension` are axum's; `RepeatedQuery` is the opt-in repeated-key parser. Wrap in `Strict<…>` for a `Problem` rejection. |
| **Schema generation** | `utoipa::ToSchema` is the only schema author (`rename_all`, `skip`, `flatten`, `tag`, `untagged`, recursion, …). `#[derive(ApiSchema)]` is optional: add it for a named `$ref` component, `unique_items`, the `url` patch, or `validate_with`. `uuid`, `chrono`, `time`, `jiff` and `decimal` are supported. |
| **Validation** | `min_length`, `pattern`, numeric bounds, `min_items`, `unique_items`, `format` and `validate_with` are emitted into the schema *and* enforced, returning RFC 9457 problem details with JSON Pointers. Entries validate by default, including handlers that use `axum`'s own extractors; `validate = false` opts out. |
| **Errors** | `Problem` is the default error body; `#[derive(ApiError)]` / `#[derive(ApiResponse)]` generate `IntoResponse` plus the documented responses. |
| **Auth** | `Auth<A>` extracts an identity, `Auth<A, Scopes>` requires an `Authorize` implementation, and `security = [A]` on an entry or module installs the check for that route or subtree. A handler that already takes `Auth<A>` is not authenticated twice. Schemes, 401/403 and `WWW-Authenticate` are documented automatically. |
| **Files & streams** | Opt-in `multipart` for `Multipart<F>`, `FilePart` and `TempFile`; plus `Attachment<T>`, `File` and SSE. |
| **Schema engine (default)** | Requests and responses are validated by executing the emitted JSON Schema (2020-12). Constraints have one source and one enforcer; `validate_with` remains the explicit custom hook. |
| **Tooling** | `App::docs()` serves Swagger UI, `Built::openapi_30()` produces a 3.0.3 document, and merged routers are flagged so an incomplete spec can't slip through. |

## Install

```sh
cargo add roundhouse
cargo add axum
cargo add serde --features derive
```

Requires Rust 1.88 (MSRV). Default features include the macros and the schema engine; everything else is opt-in:

| Feature | Enables |
|---|---|
| `multipart` | `Multipart<F>`, `FilePart` and axum's multipart support |
| `multipart-tempfile` | `TempFile` (implies `multipart`) |
| `schema-engine` | Execute the emitted JSON Schema (default); disabling it leaves validation off unless requested |
| `uuid`, `chrono`, `time`, `jiff`, `decimal` | `ToSchema`/`ApiSchema` support for those types |

`ToSchema` is re-exported by `roundhouse::prelude` (and as `roundhouse::utoipa::ToSchema`), so a
crate using the prelude does not need a direct `utoipa` dependency. If you import items manually,
bring `utoipa` into scope with `use roundhouse::utoipa;` — the derived code refers to it by name.

## Migrating from the pre-`ToSchema` derives

Schema shape is generated by `utoipa`, not by roundhouse:

```rust
#[derive(serde::Serialize, ToSchema)]
pub struct Note { .. }
```

`ApiSchema` is optional. Add it when the type should be a named `$ref` component, or uses
`unique_items`, the `url` format patch, or `validate_with`:

```rust
#[derive(serde::Serialize, ToSchema, ApiSchema)]
pub struct Note { .. }
```

Component names follow utoipa. Use `#[schema(as = module::Name)]` when two types share a simple
name. The roundhouse-only constraints moved to their own attribute:

```rust
#[schema(min_items = 1, max_items = 5)]
#[roundhouse(unique_items)]
pub tags: Vec<String>,

#[roundhouse(validate_with = validate_age)]
pub age: u8,
```

## Examples

The workspace ships runnable examples. From the repo root:

| Command | Port | Shows |
|---|---|---|
| `cargo notes` | 3000 | module-local tables, explicit composition, schemas, errors |
| `cargo blog` | 3001 | nested modules, grouped tables, query params, auth |
| `cargo layers` | 3002 | global/module/entry layers, fallback, merge |
| `cargo validation` | 3003 | constraints, JSON Pointers, generated schemas |
| `cargo uploads` | 3004 | multipart, `TempFile`, attachments, SSE, docs UI |
| `cargo showcase` | 3005 | enums, `validate_with`, utoipa interop, response validation |
| `cargo auth` | 3006 | scopes, optional auth, two-scheme AND, 401/403 |

```sh
curl localhost:3000/notes
curl localhost:3001/api/blog/posts
curl localhost:3004/reference
```

Each example serves its document (`/openapi.json`, or `/spec.json` for uploads). `examples/notes`
is the smallest; start there.

## Usage notes

**Checks run as early as they can.** A single `routes!` table is checked during macro expansion:
duplicate method+template pairs, duplicate operation ids, an entry with no success response, and
an entry-level scheme that authenticates against a different state than the handler are compile
errors. Checks that need the composed tree — collisions between modules, prefix joins, state
against the app, the final document — run at `App::build()`. Keep that gate in CI without starting
a server:

```rust
#[test]
fn routes_build() {
    app(state()).try_build().expect("route declarations are consistent");
}
```

**Emit the document.** Building the app produces the document whether or not it is served:

```rust
let built = app(state).build();
std::fs::write("target/openapi.json", built.openapi_json())?;
```

`Built::document()` returns the `OpenApi` value and `Built::routes()` the resolved table with source
locations.

**Validation is a route policy.** Entries validate by default. A JSON body is validated from the
raw request bytes before extraction, so the request type does not need `Serialize`; path, query and
form values are validated by the route's shim. Every path executes the same emitted schema:

```rust
routes! {
    post("/notes", create, validate = false), // opt out for this operation
}
```

`Result<Extractor, E>` arguments own their rejection and are not validated automatically. A
handler called directly in a test still behaves exactly as written. Malformed JSON is a `Problem`
before axum sees it; `Strict<…>` still maps the remaining axum rejections (path/query/extension,
and a post-validation deserialization failure) to the same shape.

`Json`, `Form`, `Path`, `Query` and `Extension` are axum's extractors. Use `RepeatedQuery<T>` when
the operation accepts repeated keys (`?tag=a&tag=b`); axum's `Query` cannot represent them. Wrap any
extractor in `Strict<…>` when you need the framework's `Problem` rejection for the paths the raw
layer does not cover.

The enforcer is the emitted JSON Schema, compiled once by `App::build()`; the `schema-engine`
feature (and `Validation::Schema`) is on by default. A handler installed directly on a plain axum
router, outside `App`, has no engine and therefore no constraint checks — build through `App` (or
`App::router()`) to get them, or call `.validation(Validation::Off)` to make the absence explicit.

**Auth is enforced where it is declared.** `security` takes scheme types, not names; the same
declaration installs the check and publishes the requirement. An entry with one scheme is enforced
by the handler shim (before the body is read); a module installs a guard for its whole subtree:

```rust
routes! {
    prefix = "/admin",
    security = [Bearer],
    ...admin,
}
```

Scopes stay on the handler (`Auth<Bearer, ReadPosts>`). A handler that already takes
`Auth<Bearer>` is not authenticated twice: the successful extraction marks the request and the
shim's check is a no-op.

**Auth backends are yours.** The schemes in `examples/auth` are a reference, not a supported backend.
Use a maintained library (`jsonwebtoken`, `openidconnect`, `tower-sessions`) or an authenticating
gateway. roundhouse guarantees the contract around a scheme; your implementation must verify
signatures, expiry, audience and issuer, and compare secrets in constant time.

Full API documentation:

```sh
cargo doc --workspace --no-deps --open
```

## Development

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features

# after an intentional document change, regenerate the golden file:
ROUNDHOUSE_BLESS=1 cargo test -p roundhouse --test spine openapi_golden
```

Or use the dev container: VS Code *Reopen in Container*, or

```sh
podman compose -f compose.dev.yaml up -d
podman compose -f compose.dev.yaml exec dev bash
```

## Non-goals

Callbacks, links, webhooks, XML, GraphQL, client codegen, TLS termination and dependency injection
are out of scope. For templates, sessions, an ORM or hot reload, use a full-stack framework —
roundhouse targets contract-first JSON APIs where a generated client and a CI-checked document are
part of the deliverable.

## License

MIT OR Apache-2.0.
