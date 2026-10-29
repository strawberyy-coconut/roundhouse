# roundhouse

An OpenAPI-first framework for [axum](https://github.com/tokio-rs/axum). Declare routes, parameters,
schemas, auth and errors once; roundhouse builds the `axum::Router` and the OpenAPI 3.1 document
from the same declarations. No separate wiring step, no hand-maintained spec.

```rust
use roundhouse::prelude::*;

#[derive(serde::Serialize, ApiSchema)]
pub struct Hello {
    pub message: String,
}

mount!("/v1", tags = ["hello"]);
routes! { get("/hello/{name}", hello) }

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
| **Declarative routing** | `routes!` tables live beside their handlers; `mount!` declares a subtree's prefix, tags, security and layers once. `App::build()` joins everything. |
| **Single source of truth** | Paths, parameters, bodies, responses and security requirements are derived from the router's own declarations, so the document matches what is enforced. |
| **Build-time diagnostics** | Duplicate routes, drifting parameter names, undeclared success responses, state mismatches and unenforced security are `BuildError`s. `App::try_build()` never panics. |
| **Document as artifact** | The OpenAPI document is always built; `App::openapi(..)` only decides whether it is served. `Built::openapi_json()` lets CI diff it. |
| **Typed extractors** | `#[handler]` reads `State`, `Path`, `Query`, `Json`, `Form`, `Multipart`, `Auth`, `Option<Auth>` and `Extension` from the signature. |
| **Schema generation** | `#[derive(ApiSchema)]` follows serde attributes (`rename_all`, `skip`, `flatten`, `tag`, `untagged`, …), handles recursion, and supports `uuid`, `chrono`, `time`, `jiff`, `decimal` and `bytes`. |
| **Validation** | `min_length`, `pattern`, numeric bounds, `min_items`, `format` and `validate_with` are emitted into the schema *and* enforced, returning RFC 9457 problem details with JSON Pointers. |
| **Errors** | `Problem` is the default error body; `#[derive(ApiError)]` / `#[derive(ApiResponse)]` generate `IntoResponse` plus the documented responses. |
| **Auth** | `Auth<A>` extracts an identity, `Auth<A, Scopes>` requires an `Authorize` implementation, and schemes, 401/403 and `WWW-Authenticate` are documented automatically. |
| **Files & streams** | Opt-in `multipart` for `Multipart<F>`, `FilePart` and `TempFile`; plus `Attachment<T>`, `File` and SSE. |
| **Optional schema engine** | `schema-engine` validates requests and responses against the emitted JSON Schema (2020-12). |
| **Tooling** | `App::docs()` serves Swagger UI, `Built::openapi_30()` produces a 3.0.3 document, and merged routers are flagged so an incomplete spec can't slip through. |

## Install

```sh
cargo add roundhouse
cargo add axum
cargo add serde --features derive
```

Requires Rust 1.88 (MSRV). Default features include the macros; everything else is opt-in:

| Feature | Enables |
|---|---|
| `multipart` | `Multipart<F>`, `FilePart` and axum's multipart support |
| `multipart-tempfile` | `TempFile` (implies `multipart`) |
| `schema-engine` | `Validation::Schema` and `App::validate_responses()` |
| `uuid`, `chrono`, `time`, `jiff`, `decimal` | `ApiSchema` impls for those types |

## Examples

The workspace ships runnable examples. From the repo root:

| Command | Port | Shows |
|---|---|---|
| `cargo notes` | 3000 | module-local tables, inherited mounts, schemas, errors |
| `cargo blog` | 3001 | nested mounts, grouped tables, query params, auth |
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

**Route checks run at build time.** Discovery is link-time, so `cargo check` cannot see a duplicate
route or state mismatch. Run the same gate in CI without starting a server:

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
