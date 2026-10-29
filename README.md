# roundhouse

An OpenAPI-first framework for axum. You declare routes, parameters, schemas, auth and errors;
roundhouse assembles a real `axum::Router` and the OpenAPI 3.1 document that describes it. There is
no separate wiring step and no hand-maintained spec.

The opinions are deliberate:

- **Declarations, not wiring.** A route lives beside its handler in a `routes!` table and a subtree
  declares its prefix once in `mount!`. `App::build()` joins the tables and is the only place the
  application is assembled.
- **One source of truth.** Paths, query parameters, bodies, responses and security requirements are
  derived from the same declarations the router runs, so what is documented is what is enforced:
  a documented scope needs an `Authorize` implementation, and a `security` name needs an extractor
  or documented layer that checks it.
- **Loud by default.** Missing success responses, duplicate routes, drifting parameter names,
  undocumented error types and unsupported state are build diagnostics — errors under
  `App::strict()`, and `App::try_build()` never panics. A path capture that no `Path<T>` covers is
  documented as a string and reported the same way, so the document never silently guesses.
- **The document is a build artifact.** It is always built; `App::openapi(..)` only decides
  whether it is also served. `Built::openapi_json()` exposes the same document the server
  serves, so CI can diff it.

## Quickstart

Requires Rust 1.88 (the declared MSRV, checked in CI). The JSON Schema executor is behind the
optional `schema-engine` feature; the default build uses the generated checks.

Multipart extraction is opt-in. Enable `multipart` for `Multipart<F>`/`FilePart` and axum's
multipart body support; enable `multipart-tempfile` as well for `TempFile`. A JSON-only app that
leaves both off does not compile or link `roundhouse-multipart` or axum's multipart parser.

```toml
[dependencies]
roundhouse = "0.1"
axum = "0.8"
serde = { version = "1", features = ["derive"] }
```

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

`GET /v1/hello/world` answers JSON; `GET /openapi.json` describes it, including the `name` path
parameter that `Path<String>` cannot name by itself.

## Why roundhouse?

The nearest neighbours are `aide`, `utoipa-axum`, `poem-openapi` and `okapi`. They attach
annotations to routes; roundhouse derives the route table itself, so a subtree declares its prefix,
tags, security and layers once in `mount!`, and the module tree groups declarations while every
route is registered at its full resolved path. The differences
that matter are:

- **Mount inheritance.** The tree is the composition unit; there is no `router()` function to keep
  in sync with the document.
- **Enforcement, not annotation.** A scope without `Authorize`, a `security` name without an
  enforcing extractor, or a declared response that contradicts the handler is a build error.
- **One error shape.** `Problem` documents and produces the same RFC 9457 body everywhere.

If you want a full-stack framework (templates, sessions, an ORM, hot reload), this is not it, and it
does not try to be. It is for contract-first JSON APIs where a generated client and a CI-checked
document are part of the deliverable.

The repository is a Cargo workspace with `roundhouse` as the root package:

| Path | What it is |
|---|---|
| `src/` | the framework facade: registry, prefix resolution, router assembly, OpenAPI derivation, validation |
| `roundhouse-schema/` | `ApiSchema`, path/query declarations, `ValidationErrors` — no routes and no axum |
| `roundhouse-problem/` | the canonical RFC 9457 `Problem` body, shared by the facade and satellites |
| `roundhouse-multipart/` | opt-in `Multipart<F>`, `FilePart`; `TempFile` behind `multipart-tempfile` |
| `roundhouse-macros/` | `routes!`, `mount!`, `#[handler]`, `#[derive(ApiSchema / PathParams / QueryParams / Multipart / ApiResponse / ApiError)]` |
| `examples/notes/` | module-local route tables and inherited `mount!` |
| `examples/blog/` | nested mounts, grouped tables, `hidden`/`absolute`, auth, response enums |
| `examples/layers/` | global/module/entry layers, fallback, `App::merge`, embedding |
| `examples/validation/` | constraints, JSON Pointers, `validate = false`, schema output |
| `examples/uploads/` | multipart, `TempFile`, attachments, raw files, SSE, docs UI, max body |
| `examples/showcase/` | enum validation, `validate_with`, utoipa interop, collisions, cfg/any/hidden, response validation |
| `examples/auth/` | `Auth<A>`, scopes, optional auth, two-scheme AND, security registry, `WWW-Authenticate` |
| `tests/` | the framework's surface, conflict and golden-document tests |
| `examples/*/tests/` | each example's integration tests |

The example app has no `router()` function. Its wiring is declarations:

```rust
// examples/notes/src/routes/notes/mod.rs
pub mod create;
pub mod list;
pub mod show;

// The only mount in this subtree. Every descendant inherits `/notes` and these tags.
mount!("/notes", tags = ["notes"]);
```

```rust
// examples/notes/src/routes/notes/show.rs
routes! {
    get("/{id}", show, summary = "Read a note"),
}

#[handler]
pub async fn show(
    State(app): State<AppState>,
    Path(NoteId { id }): Path<NoteId>,
) -> Result<Json<Note>, ApiError> {
    app.get(id).map(Json).ok_or(ApiError::NoteNotFound { id })
}
```

`list.rs` and `create.rs` have the same shape: their own `routes!` table and handler, with no
`mount!` of their own. Their module paths (`...::notes::list`, `...::notes::create`) still inherit
the `/notes` prefix from the parent module.

`examples/notes/src/main.rs` starts the discovered app; `roundhouse::app!(state)` scopes discovery
to the calling crate (`env!("CARGO_CRATE_NAME")`), so a linked dependency's routes cannot join by
accident and moving the call into a submodule keeps the same scope.

## API reference

The README and rustdoc are the contract for what the framework does today: the crate split, loud
derivation, and schema execution and response validation on the same engine.

```sh
cargo doc --workspace --no-deps --open
```

## Run the examples

Dev container (VS Code: *Reopen in Container*), or:

```sh
podman compose -f compose.dev.yaml up -d
podman compose -f compose.dev.yaml exec dev bash
```

| Command | Package | Port | Shows |
|---|---|---|---|
| `cargo notes` | `notes-example` | 3000 | module-local tables, inherited mounts, `ApiSchema`, `ApiError`, `ApiResponse` |
| `cargo blog` | `blog-example` | 3001 | `/api/blog/posts` nesting, grouped tables, `Query`, `hidden`, `absolute`, `Auth<Bearer>`, `Location` |
| `cargo layers` | `layers-example` | 3002 | global/module/entry layers, `route_layer`, fallback, merge, `App::router()` embedding |
| `cargo validation` | `validation-example` | 3003 | every constraint family, nested JSON Pointers, `validate = false`, generated schema |
| `cargo uploads` | `uploads-example` | 3004 | multipart + `TempFile`, `Attachment`, `File`, `Sse`, custom docs/OpenAPI routes |
| `cargo showcase` | `showcase-example` | 3005 | enum validation, `validate_with`, utoipa interop, name collisions, cfg/any/hidden, response validation |
| `cargo auth` | `auth-example` | 3006 | required/optional auth, scopes, bearer + API key AND, 401/403 documentation |

Each alias is `cargo run -p <package>` from `.cargo/config.toml`. Try:

```sh
curl localhost:3000/notes
curl localhost:3001/api/blog/posts
curl -i localhost:3002/full/ok
curl -X POST localhost:3003/profiles -H 'content-type: application/json' -d '{"username":"AB"}'
curl -i localhost:3001/api/blog/admin/audit
curl -H 'Authorization: Bearer demo' localhost:3001/api/blog/admin/audit
curl -F file=@README.md localhost:3004/uploads
curl localhost:3004/reference
curl localhost:3005/docs
curl -H 'Authorization: Bearer reader' localhost:3006/private
curl -H 'Authorization: Bearer admin' -H 'X-API-Key: secret' localhost:3006/admin
```

The notes, blog, and validation examples serve `/openapi.json`; uploads serves `/spec.json` plus a
docs UI at `/reference`; showcase serves `/openapi.json` and `/docs`.

### Route checks run when the app is built

Discovery is link-time, so `cargo check` cannot see a duplicate route, a state mismatch or a bad
mount: those are resolved by `App::build()`. Call `App::try_build()` from a test or a small check
binary to run the same gate in CI without starting the server:

```rust
#[test]
fn routes_build() {
    app(state()).try_build().expect("route declarations are consistent");
}
```

`App::build()` panics with the report `try_build()` returns; neither can catch a declaration error
before the binary links.

### Read the document from a build

The framework produces the document; emitting it is the application's job. The document is built
whether or not it is served. After `App::build()`, `Built::openapi_json()` and
`Built::openapi_30_json()` return pretty JSON, `Built::document()` returns the `OpenApi` value,
and `Built::routes()` exposes the resolved table with each declaration's source location:

```rust
let built = app(state).build();
let json = built.openapi_json();
std::fs::write("target/openapi.json", json)?;
```

## What works (v0.1)

- **Discovery** — `routes!` and `mount!` submit const-evaluable records through `inventory`;
  `App::build()` joins them once, checks state `TypeId`s, and resolves every full path.
  `App::try_build()` is total: it returns a `BuildError` (never a panic) for duplicate routes
  (after normalising `{id}` and `{post_id}` to the same shape), duplicate `operationId`s,
  conflicting parameters and security-scheme kinds, undeclared success responses, state mismatches,
  empty scopes and apps with no routes. `App::build()` is the panicking convenience over it, and
  prints collected diagnostics to stderr.
- **Routing** — every operation is registered at its full resolved path and the module routers are
  merged, so handlers and layers see the full request URI and `MatchedPath` is the document path.
  `mount!` prefixes may contain named captures: `mount!("/{project_id}/issues")` with a leaf
  `/{issue_id}` produces `/{project_id}/issues/{issue_id}`, and one `Path<T>` sees both captures.
  A mount prefix cannot contain a catch-all; a final `{*rest}` belongs in the leaf route.
  Group mounts, module layers (full/route), entry layers, `absolute`, `hidden`, `#[cfg]` entries,
  grouped method tables, scoped discovery (`scope` / `include`), `App::mount(prefix, scope)` for
  cross-crate composition and several prefixes over one table, `mount!`'s typed `MOUNT` handle,
  `App::layer`,
  `App::layer_documented` for layers that terminate requests, `App::merge`, custom fallbacks, and
  JSON `404 Problem` and `405 Problem` defaults. `scope(..)` selects declarations; mounts above the scope still
  provide the structural prefix (and their tags, security and layers), so scoping below
  `mount!("/api/v1")` keeps `/api/v1` in routes and documents. A router given to `App::merge` is
  served but not discovered, so **the served document is incomplete**; it carries
  `x-roundhouse-merged` and a warning, which `App::strict()` turns into an error, and
  `Built::merges()` exposes each merge source for an app's own CI ledger.
- **Fidelity** — `App::router()` returns a real `axum::Router<S>`, `Result<T, E>` works whenever
  both sides are `IntoResponse`, and the example's fidelity test runs the same requests against a
  hand-written router and the discovered one, asserting status, headers and body are equal.
- **Handlers** — `#[handler]` reads `State<T>`, `Path<T>`, `Query<T>`, `Json<T>`, `Form<T>`,
  `Multipart<F>`, `Auth<A>`, `Option<Auth<A>>` and `Extension<T>` from the signature. It emits the
  documentation half plus a compile-time marker, so a handler listed in `routes!` without
  `#[handler]` fails at the table line. Extractors are resolved by type, not identifier: an alias
  or re-export of `axum::Json` is still marked unverified rather than inheriting the validating
  extractor's promises. Derivation is loud: a route that derives no 2xx/3xx
  response and declares none is a build error, a foreign `axum::Json` body marks the operation
  `x-roundhouse-unverified-body` (a warning by default, a `BuildError` under `App::strict()`) and
  is still documented from `T` when `T: utoipa::ToSchema` (the marker means *unenforced*, not
  unknown), a custom extractor whose parameters the macro cannot infer is marked
  `x-roundhouse-unverified-extractor` (also a warning by default), a file field without
  `max_bytes` is named in a warning (an error under `strict`), and
  `-> StatusCode` or `(StatusCode, Json<T>)` documents a `default` response. `responses(200 = Post)`
  uses `Post`'s schema; replacing an inferred body needs `override` and emits
  `x-roundhouse-overridden-response`. `deprecated` and `public` emit `deprecated: true` and
  `x-roundhouse-public: true`. A `doc = |docs| ..` hook on a table entry fills the gap for anything
  inference cannot see; it runs before the parameter conflict and capture checks, so hand-written
  documentation is checked like derived documentation. `path = "/resolved/{id}"` on a table entry
  asserts the full module prefix chain, so a move fails the build instead of silently changing
  public URLs.
- **Auth** — `AuthScheme` is pure metadata (`NAME`, `KIND`, failure statuses, `type Identity`);
  `Authenticate` receives `&Self::State` (normally `&AppState`) and returns the identity, so token
  lookups use app state instead of hiding a store in request extensions. `Auth<A>` yields the
  identity; `Auth<A, Scopes>` publishes the scope in the security requirement and only compiles
  when `Scopes: Authorize<A>`, so a scope marker cannot be documented without an authorization
  decision. `Authorize` is a route-level gate over the identity: it can answer role/scope
  questions but cannot see a path resource, so row-level membership checks belong in the handler
  (see `examples/auth`'s admin check). Several required `Auth<A>` extractors AND into one OpenAPI
  requirement; an
  `Option<Auth<A>>` adds the unauthenticated alternative, and a present-but-invalid token is a
  rejection rather than anonymous access. `Auth<A>::State` pins the handler the same way
  `State<T>` does. `mount!(security = [...])` is accepted only when the operation also enforces
  that scheme; scheme kinds are never guessed from names.
- **Extractor docs** — `#[derive(QueryParams)]` and `#[derive(PathParams)]` publish query/path
  parameters with the same `#[schema(..)]` constraints the runtime enforces. Both derives share the
  `ApiSchema` serde parser: `rename`, container and field `rename_all`, `default` (field and
  container), `skip`, `skip_deserializing` and `flatten` produce the same names and requiredness
  serde does. `Vec<T>` query parameters are documented as `style: form, explode: true` and the
  extractor uses `serde_html_form`, so `?tag=a&tag=b` parses. Auth publishes the
  scheme, 401/403 responses, `WWW-Authenticate`, optional-auth alternatives, and scope metadata.
- **Files and streams** — with the `multipart` feature, `#[derive(Multipart)]` with a generated
  field table: parts are routed by name as they arrive, `FilePart` buffers under the field's
  `max_bytes`, and duplicate single-valued fields are a `400`. Unknown fields are ignored unless
  `#[multipart(deny_unknown)]`, and `accept` matches the media type (parameters and case stripped).
  Missing required fields collect into one `Problem`; limits are decided while reading and narrow
  the app's `max_body`, with `App::max_parts` bounding the part count. The `multipart-tempfile`
  sub-feature adds `TempFile`, which streams straight to an already-unlinked file (with
  `Read`/`Seek`); per-field and per-request limits bound one request's spool, but not aggregate
  concurrent disk use, so a full spool filesystem becomes a `500 Problem`. Prefer `FilePart` for
  ordinary uploads. Plus `Attachment<T>`, `File`, and axum's `Sse` with documented content types
  and headers.
- **Module-local tables** — a leaf module can keep its `routes!` table beside its handler and needs
  no `mount!`; only the module that owns the subtree declares the prefix. Descendants inherit its
  prefix, tags, security, and layers. Serving `/v1` and `/v2` over the same table is
  `App::mount("/v1", "crate::routes")` and `App::mount("/v2", "crate::routes")`; the same
  declarations, checks and layers apply at both prefixes. Merged hand-written routers are served
  but not discovered for the document; `App::merge` marks the document `x-roundhouse-merged`.
- **Erased-style handlers** — a handler that takes `Extension<Arc<T>>` instead of `State<T>` is
  stateless, so it works with unit app state; no feature flag or new request context is needed.
- **OpenAPI 3.1** — paths, operations, path parameters, JSON/form bodies, success and error
  responses, tags, security requirements, components, and a golden-file test
  (`tests/openapi.golden.json`).
- **Schemas** — `ApiSchema` derives structs and enums from serde attributes (`rename_all`,
  `rename`, `skip`, `default`, `skip_serializing_if`, `flatten`, `deny_unknown_fields`, `tag`,
  `content`, `untagged`, `Option`, `Vec`, maps), and registers components with a two-phase
  placeholder so recursive types do not loop. Optional features add `ApiSchema` for `uuid`,
  `chrono`, `time`, `jiff` and `decimal` (`rust_decimal`); `bytes` support is always compiled in
  (`jiff` maps timestamps, zoned datetimes, civil dates/times, spans, and signed durations).
- **Constraints** — `min_length` / `max_length` / `pattern` / `minimum` / `maximum` /
  `exclusive_*` / `min_items` / `max_items` / `unique_items` / `format` (`email`, `uuid`, `url`),
  `validate_with`, and `#[schema(via = utoipa)]` are emitted into the schema **and** enforced by
  the crate's `Json`, `Form`, `Path`, `Query`, and `Multipart` wrappers. Violations produce one RFC
  9457 `application/problem+json` response with a JSON Pointer per field. `App::validation_status`
  chooses 400 (default) or 422; `validate = false` on the table entry opts the whole body out.
  `#[schema(skip_validation)]` was removed: a constraint is published and enforced, or it is not
  declared. `url` is emitted as JSON Schema's standard `uri` format.
- **Schema execution (opt-in)** — `App::validation(Validation::Schema)` compiles the emitted
  component and inline body schemas with `jsonschema` (2020-12, `format` assertions on, the `regex`
  pattern engine) and validates request bodies against the document. It requires the
  `schema-engine` feature, which is off by default so a generated-checks build does not compile
  `jsonschema`; `Validation::Off` disables checks and `Validation::Generated` is the default.
  Composite bodies (`Vec<T>`, `Option<T>`, maps, tuples) compile their operation's inline schema.
  `validate_with` still runs after the engine and is published as `x-validate-with`.
  `tests/engine.rs` is the conformance harness: verdict and JSON Pointer parity between the
  generated checks and the engine, plus serde/`deny_unknown_fields` fixtures.
- **Errors** — `Problem` is the default error body; `#[derive(ApiError)]` / `#[derive(ApiResponse)]`
  generate both `IntoResponse` and the documented response set (statuses, bodies, headers,
  `Location`, `Problem`). Data-carrying enum variants validate their fields with the same pointers.
- **Tooling** — `App::docs()` implies `/openapi.json` and serves a pinned Swagger UI (with SRI).
  The UI assets are loaded from unpkg, so `docs()` needs browser network access; an air-gapped
  deployment should serve its own copy at the document route instead. `App::validate_responses()`
  checks JSON responses with the same schema engine (so `minLength`,
  `pattern`, numeric bounds and `uniqueItems` are no longer ignored); `Built::openapi_30()`
  produces a 3.0.3-shaped document. Components with the same simple name
  are disambiguated by module path automatically; `#[schema(name = "..")]` overrides it.
  `App::server`/`App::servers` populate the document's `servers` array, and
  `App::document(|d| ..)` supplies metadata such as tag descriptions. `App::serve_with(addr,
  shutdown)` drives axum's graceful shutdown. `App::max_body` sets the global request limit; with
  the `multipart` feature, `App::max_parts` bounds multipart part counts. `Built::routes()`
  exposes the resolved table with each
  declaration's source. `roundhouse::openapi::{Info, OpenApi, Components}` is the stable facade over
  the document model; `as_utoipa()`/`into_utoipa()` are the explicit interop escape hatch.

## Auth backends

The schemes in `examples/auth/` are a reference implementation, not a supported backend. For
production, use a maintained library or service: [`jsonwebtoken`](https://docs.rs/jsonwebtoken)
for JWT verification, [`openidconnect`](https://docs.rs/openidconnect) for OIDC discovery and
ID-token validation, [`tower-sessions`](https://docs.rs/tower-sessions) for session-backed
identities, or an API gateway that authenticates at the edge and forwards a verified identity.
The framework guarantees the contract around a scheme: `Authenticate` runs at the extractor,
`presents_credentials` separates missing credentials (`None`) from invalid ones (a rejection),
`Authorize` is required before a scope is documented, and 401/403 responses carry the documented
`WWW-Authenticate` challenge. `Authorize` sees only the identity and is a route-level gate; check
resource membership (project, tenant, row ownership) in the handler after extracting the path.
Your scheme must guarantee the identity itself: verify signatures
and expiry, compare secrets in constant time, check the token's audience and issuer rather than
only that it parses, and return `presents_credentials = false` only when credentials are absent.

## Tests

```sh
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features

# after an intentional document change, regenerate the golden file:
ROUNDHOUSE_BLESS=1 cargo test -p roundhouse --test spine openapi_golden
```

The suite covers route resolution, grouped tables, leaf-module table inheritance, `#[cfg]`,
absolute routes, layer ordering, captures in `mount!` prefixes (including full-path layers,
`MatchedPath`, 405s and conflict diagnostics), security/Auth documentation and enforcement,
path-capture documentation (including scalar and foreign `Path`), query/path constraints, enum and
struct schemas, recursive schemas and component-name collisions, multipart/files/SSE, `ApiResponse`
headers, rejection suppression, validation status, `validate = false`, layer docs, docs UI, 3.0
downgrade, response validation, scope selection, duplicate-route, duplicate-operation-id,
normalised-template, undeclared-success, state-mismatch, empty-scope and empty-app diagnostics, the
schema-execution conformance harness, the golden document, the hand-written-vs-discovered
differential, `doc = |docs| ..` hooks, foreign utoipa schemas, `app!` crate scoping, `App::mount`,
and the `App::merge` receipt.

Two tests encode the invariant the project is named for: `examples/auth` walks every operation with
a security requirement and asserts an unauthenticated request is rejected, and the path tests assert
that every `{capture}` has a documented parameter. Add the same checks to your own app if you
generate clients from the document.

## Out of scope

Explicit non-goals remain callbacks, links, webhooks, XML, GraphQL, client codegen, TLS termination,
and dependency injection. Response validation executes the emitted JSON Schema (2020-12) through
`jsonschema`; the 3.0 downgrade is best-effort for the constructs emitted here.
