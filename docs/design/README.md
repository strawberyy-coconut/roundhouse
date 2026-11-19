# Design notes

Roundhouse has one source of truth: the declarations in your crate. The same `routes!` table,
`#[handler]` signatures, schema derives and response impls produce the runtime router and the
OpenAPI contract. These notes explain how that works, where the boundaries are, and why they are
there.

They are not the user guide. The crate [README](../../README.md) covers installation, quick start
and the authoring surface; rustdoc is the API reference. These pages are for contributors and for
anyone deciding whether the design fits their application.

## Invariants

- **The module tree is the complete route set.** There is no global registry, no linker discovery
  and no runtime `add_route`.
- **Production does not build documentation.** Schema derivation and the document builder run in
  the contract build under `target/roundhouse`; a production binary consumes the committed
  contract. A build with the `build` feature without a bundle builds one in-process for tests.
- **Strict builds are the default.** A missing success response, an unverified body or an
  unacknowledged merged router is an error. Opt-outs are explicit and stay visible in the
  document.
- **Response documentation is trait-driven, and Rust paths are not status codes.** A normal
  return or tail is a response (`DocumentedResponse`); `?` is an error path (`DocumentedError`).
  The traits choose the statuses. The analyzer selects the path, never a status code.
- **The document is a projection, not a second model.** `openapi.json` is produced from the same
  resolved routes the router uses; it is not a hand-maintained specification.

## Reading paths

- **New to the architecture:** [Architecture](architecture.md) → [Declarations](declarations.md)
  → [Contract build](contract-build.md) → [Runtime](runtime.md).
- **Changing how a request is routed or checked:** [Declarations](declarations.md) →
  [Runtime](runtime.md) → [Validation](validation.md).
- **Changing the emitted document:** [The document](document.md).
- **Adding support for a custom type or layer:** [Extension points](extension-points.md).
- **Asking "why is it like this?":** [Decisions](decisions.md).

## Index

- [Architecture](architecture.md) — the declaration → contract → runtime phases, crate map,
  feature policy and dependency direction.
- [Declarations](declarations.md) — `routes!`, `#[handler]`, `api!`, doc tags, schema derives,
  macro expansion and the checks that run at compile time.
- [Contract build](contract-build.md) — `build.rs`, the `prepare` harness, build modes, artifact
  shape, contract versioning and the CLI.
- [Runtime](runtime.md) — prefix/security/layer resolution, router assembly, state, `Built` and
  documentation serving.
- [Validation](validation.md) — the contract-backed request pipeline, enforcement matrix, schema
  engine, error shape and response verification.
- [The document](document.md) — `OperationDoc`, response documentation, prose precedence,
  component registration, document validity and extensions.
- [Extension points](extension-points.md) — the public traits, custom extractors and responses,
  documented layers and the internal probes.
- [Decisions](decisions.md) — the constraints and tradeoffs that produced the current shape,
  including rejected alternatives and open questions.

## Conventions

- These notes describe the current tree. Release history belongs in the
  [changelog](../../CHANGELOG.md), not here.
- **Constraints**, **decisions** and **open questions** live in [Decisions](decisions.md). Topic
  pages state the rule; the decision log explains the tradeoff.
- API reference belongs in rustdoc and the README. A design page points at a public item instead
  of duplicating its full signature or option table.
- Each fact has one owner. If two pages need it, one page states it and the other links to it.
