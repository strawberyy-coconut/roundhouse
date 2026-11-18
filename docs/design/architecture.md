# Architecture

Roundhouse is an axum framework built around one direction of flow:

```text
declarations ──► contract build ──► committed artifacts ──► runtime
```

The same module tree and the same type-level metadata drive both the served router and the
OpenAPI document. This page describes the phases, the crate boundaries and the capability policy.
The detailed mechanics live on the pages linked from each section.

## The phases

```text
src/lib.rs
  api!  +  routes!  +  #[handler]  +  #[roundhouse::schema]
        │
        │  proc-macro expansion (cargo check)
        ▼
  Module / RouteRecord / HandlerSpec / DocSource
  ExtractArg + ExtractDoc / DocumentedResponse / DocumentedError / ApiSchema
        │
        ├── contract build ── `build` feature, `target/roundhouse` harness
        │      analyze opaque returns → handlers.json → resolve_tree → doc::build
        │      → validate_document
        │      ▼
        │   .roundhouse/contract.json     runtime plans, response table, shared $defs
        │   .roundhouse/openapi.json      OpenAPI 3.1 document
        │
        └── runtime build ── `contract`, application binary
               resolve_tree → assemble
               compile request plans, serve openapi.json verbatim
               ▼
            Built { router, routes, merges, diagnostics }
```

There are three distinct times, and they must not be confused:

1. **Compile time** expands the macros and runs the checks a single invocation can make.
2. **Contract build** analyzes opaque handler bodies, evaluates the whole module tree, the
   response traits and the explicit declarations, then writes the artifacts. It runs in a
   generated harness, not in the application binary.
3. **Runtime build** resolves the same tree again, assembles the `axum::Router`, compiles the
   request plans from `contract.json` and serves the committed `openapi.json`.

The runtime never derives a schema or assembles a document. A `build` build without a bundle
is the exception used by tests and checks: `App::build` builds the contract in-process, then
consumes it exactly like the committed artifact.

## Two artifacts

`prepare` writes two siblings from one build:

- **`contract.json`** is the runtime artifact. It contains the operation table, request validation
  plans, the response table used by verification and the shared `$defs` schemas. Body plans are
  keyed by operation id and stable type name; parts plans are keyed by stable type name. Request
  validation never depends on `TypeId`.
- **`openapi.json`** is the documentation artifact. It is the OpenAPI 3.1 projection of the same
  routes. The runtime serves it verbatim.

The artifact shape and version policy are in [Contract build](contract-build.md).

## Crate map

```text
roundhouse                 application facade: App, Built, routing, extractors, auth,
                           Problem, attachments, multipart, response wrappers
├── roundhouse-macros      proc-macro entry point
├── roundhouse-macros-core parsing and expansion for the proc macros
├── roundhouse-path        path joining and template validation shared with the macros
├── roundhouse-schema      ApiSchema, Params/ParamsDoc, validation errors, schema engine
├── roundhouse-build       contract generation library for build.rs
└── roundhouse-cli         the `cargo roundhouse` binary
```

`roundhouse-build` and `roundhouse-cli` are build-time tools, not runtime dependencies.
`Problem` and the typed multipart support are modules inside `roundhouse`; they are not separate
crates.

## Dependency direction

| Crate | May depend on | Uses axum? |
|---|---|---|
| `roundhouse` | macros, path, schema; axum, tower, serde, serde_json, serde_html_form, bytes, http | yes |
| `roundhouse-macros` | `roundhouse-macros-core`, `proc-macro2` | no |
| `roundhouse-macros-core` | `syn`, `quote`, `proc-macro2`, `proc-macro-crate`, path | no |
| `roundhouse-path` | `matchit` | no |
| `roundhouse-schema` | serde, serde_json, bytes; `jsonschema` (optional), utoipa (`build` only) | no |
| `roundhouse-build` | serde_json, Cargo process invocation | no |
| `roundhouse-cli` | clap, serde_json, toml_edit, `roundhouse-build` | no |

The arrows point one way. `roundhouse-schema` has no axum dependency, so it can be used by a build
script, an SDK or a test that never starts a server. The facade re-exports its public items, so
application code normally imports them from `roundhouse`.

Generated code names the facade through `proc-macro-crate`, which keeps `rh = { package =
"roundhouse", .. }` working. `tests/fixtures/renamed` pins that behaviour.

## Capability policy

The README's [feature table](../../README.md#features) is the canonical user-facing list. The
policy behind it is:

- **Schema execution** (`validation`) is separable from schema publication; a document without
  an engine is still honest about its shape.
- **The server runtime** (`server`) is optional so tests and check binaries can build a router
  without hyper, tokio or tracing.
- **Vendored docs** (`swagger-ui`) add roughly 1.6 MB of assets; disabling them keeps the unpkg UI
  available.
- **Multipart** and **multipart-tempfile** pull in `multer` and `tempfile`; a JSON-only app does
  not pay for them.
- **`contract`** is the lean production contract consumer. It implies `validation` and
  does not pull in utoipa.
- **`build`** adds schema derivation and the document builder. Only the contract harness or a
  docs/check build enables it; production does not.
- **`uuid`/`chrono`/`time`/`jiff`/`decimal`** forward schema support for those types.

Runtime capabilities are optional; the authoring model is not. `roundhouse-macros` is a normal
dependency, and there is no `macros` feature.

Axum's default features are disabled workspace-wide. `roundhouse` enables only the pieces it
uses: `form`, `json`, `matched-path`, `original-uri` and `query`; `server` adds `http1` and
`tokio`. A no-default build must not pull in tokio, hyper, tracing, `jsonschema`, `regex`,
`utoipa`, `multer` or `tempfile`; CI checks the dependency graph and the feature matrix.

## Deliberate boundaries

- **No `roundhouse-core` / `roundhouse-axum` split.** The extension points are axum's own traits.
  `Json<T>` is `axum::Json<T>`. A framework-neutral core would require a parallel extractor and
  response model, which roundhouse deliberately does not have.
- **The macros are the authoring model.** There is no supported macro-free builder. A runtime
  builder would move duplicate, capture and operation-id checks out of `cargo check` and weaken
  the guarantees.
- **utoipa is an implementation detail.** Application code writes `#[roundhouse::schema]` and
  `#[roundhouse(..)]`. The interop surface is explicit and build-only.
- **The schema crate is the reusable half.** Routing and document assembly are axum-shaped on
  purpose; the schema and validation model is the part that travels.
- **No second document model.** The document is utoipa's model behind a thin facade. Roundhouse
  owns assembly and the merge rules, not a competing OpenAPI type.

See [Decisions](decisions.md) for the alternatives these boundaries reject.
