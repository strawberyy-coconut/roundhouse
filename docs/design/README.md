# Design notes

These documents record the constraints, decisions and open questions behind roundhouse.
They are for contributors, and for users who need to know *why* the API looks the way it
does rather than only how to call it.

User-facing documentation lives elsewhere:

- the [root README](../../README.md) is the pitch, quick start and feature tour;
- the crate documentation (`cargo doc --workspace --no-deps --open`) is the API reference;
- the [examples](../../examples) are runnable end-to-end programs.

## Index

- [Layering and capability policy](layering.md) — the crate map, the dependency direction,
  and why roundhouse is axum-specific with no `roundhouse-core`/`roundhouse-axum` split.
- [Routing](routing.md) — the `routes!` → `Module` → `App::build` pipeline, the checks at
  each stage, and how module policy becomes an axum router.
- [Documentation](documentation.md) — how `#[handler]` plus `ApiSchema`, `ExtractArg` and
  `ApiResponse` produce an OpenAPI operation, and what the document promises.
- [Validation](validation.md) — why the published schema is also the enforced schema, the
  request pipeline, the schema engine, and response validation.
- [Extensibility](extensibility.md) — the trait seams, how to document a custom extractor
  or response, and where a `doc` hook is the right tool instead.
- [Macros](macros.md) — why `routes!` is the product, how the expansion is organised, and
  the diagnostic philosophy.
- [Doc tags](doc-tags.md) — the explicit `@summary:`, `@description:` and `@responses:` metadata
  layer, its precedence rules and its limits.

## Conventions

Each document separates:

- **Constraints** — deliberate limits that keep the design coherent.
- **Decisions** — tradeoffs that have already been made.
- **Future** — work that is not implemented. Nothing in a *Future* section is an API
  promise.

When a change alters one of these documents, update the relevant note in the same change.
If a decision is superseded, keep the history in the document rather than silently
rewriting it: the reason a path was rejected is part of the design.

The notes describe the current `0.3` line. Version-specific behaviour that changes is
called out explicitly; everything else should be read as "this is how the current code
behaves".
