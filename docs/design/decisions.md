# Decisions

These records explain why the current shape was chosen. A topic page states the rule; this page
records the tradeoff and the alternatives that were rejected. Entries are not a roadmap promise.

## D1 — The module tree is the complete route set

**Decision.** `routes!` emits a static `Module` tree. `...module` spreads and `App::include` are
the only composition mechanisms.

**Why.** A proc macro sees one invocation. A global registry or linker scan would make it possible
to discover sibling routes, but it would also make feature-gated and private modules harder to
reason about and create a second registration mechanism beside the tree.

**Consequences.** One invocation checks its own tokens; the resolved tree performs cross-table
checks. A table cannot see a spread's prefix at expansion time, so those checks happen at build
time. A macro-free builder would move duplicate, capture and operation-id checks into
`App::build`, which is a strictly weaker position.

**Rejected.** Inventory/linker discovery; a runtime `add_route` API; a global route registry.

## D2 — Compile time and build time are different tools

**Decision.** Keep table-local checks in the macro and cross-table/document checks in
`resolve_tree` and `doc::build`.

**Why.** A check can only be as early as the information it needs. One invocation has its own
tokens; the composed tree, the app state and the document need the aggregate.

**Consequences.** Some mistakes are `cargo check` errors and some are `BuildError`s from
`App::try_build` or `prepare`. The split is intentional; moving a check earlier requires the
information first.

**Rejected.** Re-parsing sibling modules from a proc macro; deferring all checks to runtime;
duplicating the same check in both phases.

## D3 — Documentation is a build-time contract

**Decision.** Schema derivation, the document builder and response probes run in a `contract`
build, normally the `target/roundhouse` harness. Production consumes a committed contract.

**Why.** A production binary should not carry utoipa, the document builder or response probes.
Generating once and committing the result also makes drift reviewable in CI.

**Consequences.** `App::build` under `contract` can still build a contract in-process for tests
and checks; `contract-runtime` without a bundle fails when validation is enabled. The committed
artifact must be regenerated after intentional changes.

**Rejected.** Runtime document assembly as the default; requiring utoipa in production; a
separate hand-maintained OpenAPI file.

## D4 — Two artifacts with different audiences

**Decision.** `prepare` writes `contract.json` for the runtime and `openapi.json` for document
consumers.

**Why.** Runtime validation needs operation-keyed plans, `$defs` and a response table. Document
consumers need an OpenAPI 3.1 document. One artifact would force one audience to parse the other's
representation, and a runtime engine would pay for a full document it never reads.

**Consequences.** The files are siblings from one build and are committed together.
`contract_version` gates the runtime layout. `openapi.json` is served verbatim.

**Rejected.** Serving the contract JSON itself as OpenAPI; deriving runtime plans from a parsed
OpenAPI document; a single merged artifact.

## D5 — Response documentation is trait-driven

**Decision.** A response type implements `DocumentedResponse`. A handler's return type publishes
its statuses, content types, bodies and headers directly. Explicit declarations supply what the
type cannot name.

**Why.** The document should not depend on how a function body happens to spell a return
expression. Type-name heuristics and source analysis are fragile for aliases, `impl IntoResponse`,
generated types and changed implementations. A trait is a stable, local contract.

**Consequences.** `impl IntoResponse` has no nameable concrete type, so it needs an explicit
declaration. A type that only implements `ToSchema` is not automatically documented; adapt it
with `ApiSchema` interop or implement `DocumentedResponse`. A missing success is a strict-build
error, not a guess.

**Rejected.** Semantic response inference; inferring a JSON body from a type name; treating
`response(200, Note)` as an implicit `Json<Note>`.

## D6 — Strict builds are the default

**Decision.** Missing successes, unverified bodies, unresolved extractors, undocumented `any`
routes, empty modules and unacknowledged merged routers are errors. `App::permissive()` downgrades
them to warnings; `unverified = "reason"` acknowledges an intentional gap.

**Why.** A silent gap in the document is worse than a failed build. Acknowledgements keep the
normal case loud while allowing a deliberate decision to be quiet.

**Consequences.** Acknowledgement suppresses the diagnostic but not the document marker. A
permissive build publishes a marked placeholder response instead of pretending the success is
known.

**Rejected.** Warning-only defaults; a global allow-by-default switch that hides individual gaps;
guessing a fallback response.

## D7 — `Problem` is the universal rejection body

**Decision.** Malformed JSON, rejected extractors, failed constraints, `404`, `405` and response
verification failures use an RFC 9457 `Problem`. Validation defaults to `400`; axum's occasional
`422` is normalised.

**Why.** One error shape is easier for clients and lets the document promise the same body for
every failure path. A per-extractor error body would make the document depend on which layer
rejected first.

**Consequences.** `App::validation_status(..)` can change the validation status.
`App::document_rejections(false)` is available when a gateway owns the error contract. A custom
response type can still serve its own body; it just has to document it.

**Rejected.** Axum's default rejection bodies; a separate error model per extractor.

## D8 — `jsonschema` is the only validation engine

**Decision.** The published document and the enforcer are the same JSON Schema. `validate_with`
is the escape hatch for rules the schema cannot express.

**Why.** A second validation DSL would make the document a lossy projection and create two
sources of truth. The contract build records plans; the runtime compiles them with one engine.

**Consequences.** Only declarative rules are projected. Cross-field, database and async rules live
in hooks or the handler. Opted-out body schemas are not compiled, which is the useful compile-time
split; dropping parts validation would not remove the engine dependency.

**Rejected.** `validator`, `garde`, `axum-valid` or `validify` bridges; a Rust-native validator as
the default; streaming JSON Schema validation, because `required`, `additionalProperties` and
`oneOf` need the whole value.

## D9 — Extractor and response types are axum's

**Decision.** `Json<T>`, `Form<T>`, `Path<T>` and `Query<T>` are axum's types. The only added
extractors are policy helpers: `RepeatedQuery<T>`, `Mapped<T>` and `Strict<T>`.

**Why.** A parallel extractor model would duplicate axum's API and force users to choose between
“axum types” and “roundhouse types”. A framework-neutral core would need a parallel backend and
response model, which the project does not have.

**Consequences.** Custom extractors are trait impls, not conversions. `roundhouse-schema` is the
reusable, axum-free half. There is no `roundhouse-core`/`roundhouse-axum` split and no planned
second backend.

**Rejected.** Wrapper types for every axum extractor; a framework-neutral route/document plan; a
separate framework-neutral OpenAPI crate.

## D10 — The macros are mandatory

**Decision.** `roundhouse-macros` is a normal dependency. There is no `macros` feature and no
supported macro-free builder.

**Why.** The product is the authoring surface. Runtime capabilities are optional; the authoring
model is not. A builder would be a runtime-checked copy with fewer guarantees.

**Consequences.** A `--no-default-features` build still exposes `routes!`, `#[handler]` and the
runtime derives. `Module::from_spec`/`RouteRecord::from_spec` are public only because macro
expansions in other crates need them.

**Rejected.** A builder API; a default-on `macros` escape hatch that advertised a workflow that
did not exist.

## D11 — utoipa is an implementation detail

**Decision.** Application code writes `#[roundhouse::schema]` and `#[roundhouse(..)]`. utoipa
names live behind `roundhouse::interop` and only when `contract` is enabled.

**Why.** A production build should not resolve utoipa paths. Hiding the dependency also leaves
roundhouse free to change the document model without breaking application code.

**Consequences.** Schema derivation and the document builder are gated by `contract`. Interop is
explicit for projects that already share `ToSchema` types. A blanket `impl<T: ToSchema>
ApiSchema for T` is impossible because it would overlap the derive and primitive impls.

**Rejected.** Re-exporting utoipa as the public schema API; a permanent `__private::utoipa`
dependency in production.

## D12 — State is exact at the boundary, structural inside

**Decision.** The app state is declared once with `app_state!(..)`. A table may override it;
handlers may take `FromRef` substates; `AuthScheme::State` must equal the table state.

**Why.** A `RouteRecord` is a `const` and `#[handler]` expands before `App::new`, so the shim's
router state must be monomorphized when the table expands. A crate-level alias is the smallest
scope every expansion can name.

**Consequences.** `state = infer` is available for per-handler inference, but the supported large
app declares the state once. A mismatch names both states in a `BuildError`.

**Rejected.** A runtime `TypeId` registry for state; implicit inference for every handler; making
`Auth<A>` accept a substate.

## D13 — JSON bodies are parsed twice

**Decision.** With schema mode, a JSON body is read and validated from raw bytes before
extraction, then parsed again by the extractor. `validate_body = false` is the body-only opt-out.

**Why.** Validating the raw bytes keeps the `Serialize` bound off the handler, lets plain
`axum::Json<T>` be validated, and gives malformed JSON the same `Problem` shape as a constraint
failure.

**Consequences.** The body is buffered and parsed twice; the body limit bounds memory.
`validate = false` is whole-route and requires an acknowledgement; `validate_body = false` is
quieter because it is a precise body decision. A future validate-after-extraction mode for
`Serialize` bodies would need an explicit extractor or route mode.

**Rejected.** A silent size threshold; validating after extraction by default; streaming
validation; making `Strict` an escape hatch for large bodies.

## D14 — The contract harness lives under `target/`

**Decision.** `build.rs` calls `roundhouse_build::prepare()`. The harness is generated under
`target/roundhouse`; no source file, binary target or feature is generated.

**Why.** The source tree should stay untouched after `init`, and a generated `src/bin` or
generated feature would leak into the application's build. `target/` is disposable and already
ignored.

**Consequences.** `cargo build` and `cargo test` refresh `.roundhouse/` before `include_contract!`
expands. The harness sets `ROUNDHOUSE_BUILDING_CONTRACT`, and the generated build script respects
CI unless `ROUNDHOUSE_AUTO_CONTRACT=1`. `roundhouse-build` is the build-dependency;
`roundhouse-cli` is only the binary.

**Rejected.** A generated `src/bin/roundhouse-contract.rs`; a generated Cargo feature.

## Open questions

These are deliberately not commitments:

- Whether a `#[derive(DocumentedResponse)]` for multi-status response enums is worth the extra
  macro surface.
- Whether an after-extraction validation mode for `Serialize` body types can remove the second
  parse without losing the raw-byte check.
- Whether multipart text fields should be validated from the emitted schema in addition to the
  extractor's field table and byte limits.
- Whether real traffic observation can grow richer than the current contract response table.
- Whether more extractors should gain `ExtractArg`/`ExtractDoc` implementations upstream without
  a macro change.
