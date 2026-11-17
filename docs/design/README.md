# Design notes

Roundhouse has one source of truth: the declarations in your crate. The same `routes!` table and
`#[handler]` signatures produce the runtime router and the OpenAPI contract. These notes describe
how that works in 0.4 and why the boundaries are where they are.

## The system at a glance

```text
src/lib.rs + #[roundhouse::schema] + routes! + #[handler]
        │
        │  macro expansion
        ▼
Module / Fragment / RouteRecord + DocSource + ExtractArg/ExtractDoc metadata
        │  + DocumentedResponse impls
        │
        │  build.rs → roundhouse_build::prepare()
        │     target/roundhouse harness
        │     one strict build
        ▼
.roundhouse/contract.json     runtime plans, response table, shared $defs
.roundhouse/openapi.json      OpenAPI 3.1 document
        │
        │  include_contract!()
        ▼
contract-runtime: Router + request validation + verbatim docs + response verification
```

Two rules shape the design:

- **The runtime never builds documentation.** Schema derivation and the document builder are
  compiled only under the `contract` feature, and normally only in the `target/roundhouse`
  harness. A production binary consumes the committed contract.
- **The document is a glass box.** Response documentation comes from a public
  `DocumentedResponse` impl or an explicit `response(..)` declaration. No type name carries
  meaning; the macro and CLI never decide a response from a spelling. A missing success is a
  strict-build error, not a guess.

## Index

- [Contract](contract.md) — the build pipeline, `api!`, artifacts, versioning and the CLI. Start
  here.
- [Routing](routing.md) — `routes!` → `Module` → `App::build` → `axum::Router`, including the
  static checks at each stage.
- [Documentation](documentation.md) — how a handler signature, extractor and response trait
  contribute an `OperationDoc`, how responses are merged, and how the document is emitted.
- [Validation](validation.md) — contract-backed request validation, the JSON Schema engine,
  validation hooks, rejection bodies and response verification.
- [Traits](traits.md) — the current public extension traits versus internal macro machinery.
- [Macros](macros.md) — what `routes!`, `#[handler]`, `api!` and the derives expand to, and which
  mistakes are compile-time versus build-time errors.
- [Extensibility](extensibility.md) — the supported seams for custom extractors, responses,
  schemas, layers and operation metadata.
- [Layering](layering.md) — crate boundaries, dependency direction and the feature matrix.
- [Doc tags](doc-tags.md) — prose tags, typed metadata and their precedence.

## Conventions

Each note describes the code as it exists in 0.4:

- **Constraints** are deliberate limits that keep the design coherent.
- **Decisions** are tradeoffs that have already been made.
- **Future** is not implemented and is not an API promise.

Version-specific migration details belong in `MIGRATING-0.4.md`, not in these notes. Historical
alternatives are noted only when they explain why the current shape was chosen.
