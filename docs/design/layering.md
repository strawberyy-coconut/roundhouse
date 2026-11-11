# Layering and capability policy

roundhouse is an axum framework, and it is meant to stay one. The crate split exists to keep
the macro thin and the schema half usable without a server — not to abstract over web
frameworks.

## Crate map

```text
roundhouse
├── roundhouse-macros      proc-macro entry: routes!, #[handler], the derives
├── roundhouse-schema      ApiSchema, parts schemas, validation errors, schema engine
├── roundhouse-problem     the RFC 9457 Problem body
└── roundhouse-multipart   Multipart<F>, FilePart, TempFile (optional)
```

`roundhouse` is the application facade. It owns the app builder, route resolution, router
assembly, document assembly, the extractor wrappers, the reusable `Created<T>` / `NoContent`
response wrappers and auth. It re-exports the derives and `routes!` at the crate root and in the
prelude.

`roundhouse-schema` has no axum dependency. It can be used by a build script, an SDK or a
test that never starts a server. The facade re-exports its public items, so application code
normally imports them from `roundhouse`.

Generated code refers to `::roundhouse` (or to the renamed dependency) through
`proc-macro-crate`, so the macros keep working when the crate is renamed. `examples/renamed`
exists to keep that path honest.

## Dependency direction

| Crate | May depend on | axum? |
|---|---|---|
| `roundhouse` | macros, schema, problem, multipart (optional) | yes |
| `roundhouse-macros` | `syn`, `quote`, `proc-macro2`, `proc-macro-crate` | no |
| `roundhouse-schema` | serde, utoipa, jsonschema (optional) | no |
| `roundhouse-problem` | axum (no default features), http, schema | yes |
| `roundhouse-multipart` | axum (multipart only), problem, schema | yes |

The arrows only point one way: `schema` → `problem` → `multipart` → `roundhouse`. The macro
crate emits token streams and has no runtime dependency on any of them.

## Constraints

### There is no `roundhouse-core` / `roundhouse-axum` split

This is a decision, not an oversight.

- The extension points are axum's own traits. `Json<T>` is `axum::Json<T>`; a framework-neutral
  core would need a parallel set of extractor and response types, which roundhouse explicitly
  does not have.
- The route table only ever compiles into an `axum::Router`. There is no second backend on the
  horizon that would justify a backend-neutral plan layer.
- The reusable half of the framework is the schema and validation crate, and it is already
  framework-free.
- The product is the authoring surface: `routes!` plus `#[handler]`. A neutral core would move
  the important code away from the thing users actually write.

The same reasoning rules out a framework-neutral OpenAPI crate. `roundhouse-schema` is the
part that travels; the rest is axum-shaped on purpose.

### The macros are mandatory

`roundhouse-macros` is a normal dependency, not an optional feature. Runtime capabilities are
feature-gated; the authoring model is not.

A `--no-default-features` build still exposes `routes!`, `#[handler]`, the derives and the
document builder. What it loses is the schema engine and the server runtime. There is no
supported macro-free way to declare a route: `Module::__new` and `RouteRecord::__new` are
public only because macro expansions in other crates need to call them.

### Extractor types are axum's

`roundhouse::Json`, `Form`, `Path` and `Query` are re-exports, not wrappers. The only extra
extractor types are policy helpers that compose axum rather than replace it:

- `RepeatedQuery<T>` — `serde_html_form`, because `serde_urlencoded` cannot represent repeated
  keys;
- `Mapped<T>` — maps a foreign extractor's rejection to `Problem` without claiming a schema;
- `Strict<T>` — enforces the operation's schema during extraction.

`Created<T>` and `NoContent` are response wrappers, not a second response model: they implement
axum's `IntoResponse` and roundhouse's `ApiResponse`. `ApiSchema::description` is a defaulted trait
method, so adding it does not break downstream implementors, and no new crate or dependency is
introduced.

A handler can therefore use `axum::extract::Path` directly and still be documented and
validated.

## Feature policy

The default feature set is `schema-engine` + `serve` + `swagger`.

| Feature | Enables | Why it is optional |
|---|---|---|
| `schema-engine` | Compile and execute the emitted JSON Schema, plus the build-time regex check | Pulls in `jsonschema`; a document-only build does not need it |
| `serve` | `App::serve`/`App::serve_with`, axum `http1`/`tokio`, tokio, tracing, `ConnectInfo<T>` docs | Pulls in hyper, tokio and tracing; tests and check binaries usually use `try_build` instead |
| `multipart` | `Multipart<F>`, `FilePart`, and axum's multipart support | Pulls in `roundhouse-multipart` and `multer` |
| `multipart-tempfile` | `TempFile` (implies `multipart`) | Pulls in `tempfile` |
| `swagger` | Vendored Swagger UI assets for local `App::docs()` and `App::swagger()` (default) | Adds ~1.6 MB of embedded assets to the binary; disable it to use the unpkg UI |
| `uuid`, `chrono`, `time`, `jiff`, `decimal` | Schema support for those types | Forwarded to `roundhouse-schema` and utoipa |

Two details are consequences of axum's own feature layout:

- **axum's default features are disabled workspace-wide.** roundhouse enables only the pieces
  it uses: `form`, `json`, `matched-path`, `original-uri`, `query`. `serve` adds `http1` and
  `tokio`.
- **`ConnectInfo<T>` documentation requires `serve`**, because axum gates `ConnectInfo` behind
  its `tokio` feature. The `ExtractArg` impl is gated the same way.

The result is the intended dependency shape:

```text
cargo tree -p roundhouse --no-default-features
  axum, axum-core, utoipa

cargo tree -p roundhouse --no-default-features --features serve
  axum, hyper, hyper-util, tokio, tracing
```

The no-default build must not pull in tokio, hyper, tracing, `jsonschema` or `regex`. CI checks
this through the feature matrix in `.github/workflows/ci.yml`.

## Decisions

- **Gate runtime capabilities, not the authoring model.** The `macros` feature was removed
  rather than kept as a default-on escape hatch. It advertised a macro-free workflow that did
  not exist.
- **Keep `roundhouse-schema` free of axum.** `Problem` lives beside axum because a rejection is
  a response; a schema error is not.
- **Inherit workspace package metadata.** All crates share edition, license, repository and
  MSRV from `[workspace.package]`, so a version bump cannot drift.
- **CI runs the interesting feature combinations**, not the full power set: no-default,
  `schema-engine`, `serve`, `multipart`, plus the all-features test run. `serve` and
  `schema-engine` are independent; `multipart` has its own crate-level check.

## Future

- **`roundhouse-macros-core`.** Moving parsing, table validation and expansion into a normal
  library crate would let the macro be unit-tested without `trybuild` and make the expansion
  reusable. This is under consideration; the split has not been made.
- **No framework-neutral core.** This is not planned. If a second framework ever appeared,
  the schema crate is the part that would be reused; the route/document model would be
  reimplemented, not abstracted in advance.
- **No macro-free builder.** It would be a runtime-checked copy of the route table with fewer
  guarantees, and it would compete with the DSL rather than extend it.
