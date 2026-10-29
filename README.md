# roundhouse

A thin layer over axum that turns declarations into a `Router` and an OpenAPI document — with a
set of runnable examples that exercise different parts of the surface.

The repository is a Cargo workspace with `roundhouse` as the root package:

| Path | What it is |
|---|---|
| `src/` | the framework facade: registry, prefix resolution, router assembly, OpenAPI derivation, validation, `Problem` |
| `roundhouse-schema/` | `ApiSchema`, path/query declarations, `ValidationErrors` — no routes and no axum |
| `roundhouse-multipart/` | `Multipart<F>`, `FilePart`, `TempFile` and the multipart rejection |
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
| `specs/` | the design spec: crate boundaries, macro surface, document and validation decisions |

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
to `module_path!()`, so a linked dependency's routes cannot join by accident.

## API reference

The README and rustdoc are the contract for what the framework does today. `specs/roundhouse-spec.md`
is the design record — crate boundaries, the macro surface, and the decisions that are not implemented
yet (loud derivation, schema execution, the multipart rewrite). Where the two disagree, the README is
current behaviour and the spec is the target.

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

### Dump a document from CI or a Makefile

The `roundhouse` binary builds and runs an app package with its `--openapi` flag:

```sh
cargo run -q --bin roundhouse -- openapi -p notes-example
cargo run -q --bin roundhouse -- openapi -p validation-example --format 3.0
```

## What works (v0.1)

- **Discovery** — `routes!` and `mount!` submit const-evaluable records through `inventory`;
  `App::build()` joins them once, checks state `TypeId`s, and resolves every full path.
- **Routing** — module nesting is Rust module nesting; `mount!` prefixes, group mounts, module
  layers (full/route), entry layers, `absolute`, `hidden`, `#[cfg]` entries, grouped method tables,
  scoped discovery (`scope` / `include`), `App::layer`, `App::merge`, custom fallbacks, and a JSON
  `404 Problem` default.
- **Fidelity** — `App::router()` returns a real `axum::Router<S>`, `Result<T, E>` works whenever
  both sides are `IntoResponse`, and the example's fidelity test runs the same requests against a
  hand-written router and the discovered one, asserting status, headers and body are equal.
- **Handlers** — `#[handler]` reads `State<T>`, `Path<T>`, `Query<T>`, `Json<T>`, `Form<T>`,
  `Multipart<F>`, `Auth<A>`, `Option<Auth<A>>` and `Extension<T>` from the signature. It emits the
  documentation half plus a compile-time marker, so a handler listed in `routes!` without
  `#[handler]` fails at the table line.
- **Auth** — `AuthScheme` is pure metadata (`NAME`, `KIND`, failure statuses, `type Identity`);
  `Authenticate` receives `&Self::State` (normally `&AppState`) and returns the identity, so token
  lookups use app state instead of hiding a store in request extensions. `Auth<A>` yields the
  identity, `Auth<A, Scopes>` adds scope metadata, and `Auth<A>::State` pins the handler the same
  way `State<T>` does. Several required `Auth<A>` extractors AND into one OpenAPI requirement; an
  `Option<Auth<A>>` adds the unauthenticated alternative.
- **Extractor docs** — `#[derive(QueryParams)]` and `#[derive(PathParams)]` publish query/path
  parameters with the same `#[schema(..)]` constraints the runtime enforces. Auth publishes the
  scheme, 401/403 responses, `WWW-Authenticate`, optional-auth alternatives, and scope metadata.
- **Files and streams** — `#[derive(Multipart)]` with `FilePart`, `TempFile`, repeated text/file
  fields, per-part `max_bytes`/`accept`, one-pass extraction, and RFC 9457 responses; plus
  `Attachment<T>`, `File`, and axum's `Sse` with documented content types and headers. `App::max_body` sets the global body limit; multipart parts also enforce their own `max_bytes`.
- **Module-local tables** — a leaf module can keep its `routes!` table beside its handler and needs
  no `mount!`; only the module that owns the subtree declares the prefix. Descendants inherit its
  prefix, tags, security, and layers.
- **Erased-style handlers** — a handler that takes `Extension<Arc<T>>` instead of `State<T>` is
  stateless, so it works with unit app state; no feature flag or new request context is needed.
- **OpenAPI 3.1** — paths, operations, path parameters, JSON/form bodies, success and error
  responses, tags, security requirements, components, and a golden-file test
  (`tests/openapi.golden.json`).
- **Schemas** — `ApiSchema` derives structs and enums from serde attributes (`rename_all`,
  `rename`, `skip`, `default`, `skip_serializing_if`, `flatten`, `deny_unknown_fields`, `tag`,
  `content`, `untagged`, `Option`, `Vec`, maps), and registers components with a two-phase
  placeholder so recursive types do not loop. Optional features add `ApiSchema` for `uuid`,
  `chrono`, `time`, `jiff`, `rust_decimal`, and `bytes` (`jiff` maps timestamps, zoned datetimes,
  civil dates/times, spans, and signed durations).
- **Constraints** — `min_length` / `max_length` / `pattern` / `minimum` / `maximum` /
  `exclusive_*` / `min_items` / `max_items` / `unique_items` / `format` (`email`, `uuid`, `url`),
  `validate_with`, and `#[schema(via = utoipa)]` are emitted into the schema **and** enforced by
  the crate's `Json`, `Form`, `Path`, `Query`, and `Multipart` wrappers. Violations produce one RFC
  9457 `application/problem+json` response with a JSON Pointer per field. `App::validation_status`
  chooses 400 (default) or 422; `validate = false` and `#[schema(skip_validation)]` opt out.
- **Errors** — `Problem` is the default error body; `#[derive(ApiError)]` / `#[derive(ApiResponse)]`
  generate both `IntoResponse` and the documented response set (statuses, bodies, headers,
  `Location`, `Problem`). Data-carrying enum variants validate their fields with the same pointers.
- **Tooling** — `App::docs()` serves a Swagger UI; `App::validate_responses()` checks JSON
  responses against the generated schemas; `Built::openapi_30()` (and the CLI's `--format 3.0`)
  produces a 3.0.3-shaped document. Components with the same simple name are disambiguated by
  module path automatically; `#[schema(name = "..")]` overrides it. `App::max_body` sets the
  global request limit.

## Tests

```sh
cargo test --workspace
cargo clippy --workspace --all-targets

# after an intentional document change, regenerate the golden file:
ROUNDHOUSE_BLESS=1 cargo test -p roundhouse --test spine openapi_golden
```

The suite covers route resolution, grouped tables, leaf-module table inheritance, `#[cfg]`,
absolute routes, layer ordering, security/Auth documentation, query/path constraints, enum and
struct schemas, recursive schemas and component-name collisions, multipart/files/SSE, `ApiResponse`
headers, rejection suppression, validation status, `validate = false`, layer docs, docs UI, 3.0
downgrade, response validation, scope selection, duplicate-route and state-mismatch diagnostics,
the golden document, and the hand-written-vs-discovered differential.

## Out of scope

Explicit non-goals remain callbacks, links, webhooks, XML, GraphQL, client codegen, TLS termination,
and dependency injection. The response validator covers the document subset this crate emits
(types, `$ref`, required/properties, arrays, `oneOf`/`allOf`/`anyOf`, and additional-properties
policy); it is intentionally not a general JSON Schema engine. The 3.0 downgrade is best-effort for
the constructs emitted here.
