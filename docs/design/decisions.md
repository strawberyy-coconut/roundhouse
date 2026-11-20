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

**Decision.** Schema derivation, the document builder and response probes run in a build with the
`build` feature, normally the `target/roundhouse` harness. Production consumes a committed contract.

**Why.** A production binary should not carry utoipa, the document builder or response probes.
Generating once and committing the result also makes drift reviewable in CI.

**Consequences.** `App::build` with the `build` feature can still build a contract in-process for tests
and checks; `contract` without a bundle fails when validation is enabled. The committed
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

**Decision.** A success type implements `DocumentedResponse`; an error type implements
`DocumentedError`. Those impls own statuses, content types, bodies and headers. A named return
type publishes them directly. An opaque return type (`impl IntoResponse`, `impl
DocumentedResponse`) is resolved by a build-time source analyzer that selects the type at each
tail/`return` site; the macro emits typed trait calls and the compiler verifies them. Explicit
declarations supply what neither path can name.

**Why.** The document should not depend on how a function body happens to spell a return
expression, and it should not depend on a name-based guess. A trait is a stable content contract.
The analyzer is only a selector: it says *which* documented type leaves the handler, never what
that type's status or schema is. The generated `<T as DocumentedResponse>` call makes a wrong
selection a compile error rather than a wrong document. For a named type the compiler already
knows the type, so no analysis is needed.

**Consequences.** Named returns are exactly trait-driven. Opaque returns are documented from
their body; a site the analyzer cannot resolve is `x-roundhouse-unverified-response`, and the
strict build still rejects an operation with no success. `.into_response()` is transparent to a
statically documented receiver; only a receiver that is itself erased (`Response`, a trait object,
a helper returning `Response`) stays opaque. `.into_documented_response()` additionally erases the
value to `Documented`, so branches with different documented types unify. `Result<T, E>` requires
`T: DocumentedResponse` and `E: DocumentedError`; `?` documents `E`, not the source of the
question mark. `Problem` is one `DocumentedError`, not a mandatory error type. A type that only
implements `ToSchema` is not automatically documented; adapt it with `ApiSchema` interop or
implement the response trait.

**Rejected.** Inferring a status or schema from syntax; type-name heuristics; a blanket
`ToSchema` adapter; treating `response(200, Note)` as an implicit `Json<Note>`; documenting an
already-erased `Response` or trait object.

## D5a — Opaque returns use a verified handler index

**Decision.** `prepare` analyzes the application sources before the contract harness runs and
writes `target/roundhouse/handlers.json`. `#[handler]` reads the index at expansion time and
emits typed `DocumentedResponse` / `DocumentedError` calls into its generated `DocSource`.

**Why.** An `impl Trait` return hides its concrete type from every item outside the function,
including the harness. The macro cannot name the type, and a macro-internal file reader would
break Cargo incrementality and module resolution. A pre-pass index can resolve helper and method
return types over the whole crate; the typed calls turn a wrong resolution into a compiler error.
The index is disposable (`target/`), and `check` regenerates it, so it cannot silently drift.

**Consequences.** The analyzer is conservative and only handles opaque success positions. It does
not trace `From` impls or `?` sources. A missing index entry or an unresolved site sets the
unverified marker rather than guessing. Named error types are read from the signature and need no
analysis; opaque error types are a v1 diagnostic.

## D5b — Rust return paths are not HTTP status codes

**Decision.** A normal return or tail expression is a response path, documented by
`DocumentedResponse`. A `?` is an error path, documented by the handler's `E: DocumentedError`.
The trait impls choose the statuses; either path may document any status class. The analyzer
selects by expression position and never by status code.

**Why.** `Result<T, E>` records whether the function continued or took an early return for a
handled situation. It is control flow, not HTTP semantics. Conflating the two makes a normal
return documenting 500 look like an error and a `?` documenting 409 look impossible. The wire
shape is a separate documentation decision.

**Consequences.** A normal return documenting 500 and a `?` documenting 409 are valid; both
appear in the document. The builder's status-based rules — a required `default` or 2xx/3xx, and
collapsing two shapes at the same status `>= 400` to `Problem` — are document policies over the
finished response map, not Rust-path classification. A normal-only 500 still needs a document
success somewhere; that does not turn the 500 into an error.

**Rejected.** Classifying responses by status range in the analyzer; requiring `?` to be 4xx/5xx;
assuming a normal return is a 2xx.

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
names live behind `roundhouse::interop` and only when `build` is enabled.

**Why.** A production build should not resolve utoipa paths. Hiding the dependency also leaves
roundhouse free to change the document model without breaking application code.

**Consequences.** Schema derivation and the document builder are gated by `build`. Interop is
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
expands. The harness sets `ROUNDHOUSE_BUILDING_CONTRACT`, and `prepare()` itself respects CI
unless `ROUNDHOUSE_AUTO_CONTRACT=1`, so a hand-written build script gets the same policy.
`roundhouse-build` is the build-dependency; `roundhouse-cli` is only the binary.

**Rejected.** A generated `src/bin/roundhouse-contract.rs`; a generated Cargo feature.

## D15 — The harness compiles a documentation slice, not the application

**Decision.** `prepare` generates a documentation-only package under `target/roundhouse/slice`.
It contains a line-preserving copy of the application's documentation surface: `api!`,
`routes!`, handler signatures, schema types, `Documented*` impls and the imports they use.
`#[handler]` and `routes!` expose a documentation mode selected by the slice's marker file; it
emits the documentation traits but not handler bodies, router dispatch or layer application.
`ROUNDHOUSE_HARNESS` selects `slice`, `linked` or `auto`.

**Why.** A package with only a binary target has no library for the harness to depend on, and a
library package compiled a second time in the harness wastes work. The contract build only ever
evaluates metadata: `Contract::build` resolves the route tree and documentation traits and never
calls `Record::handler`. Handler bodies, `IntoResponse`, database code and the application's
private dependencies are not part of the document.

**Consequences.** Bin-only packages generate contracts. The harness has no dependency edge to the
application package. The slice copies source text rather than re-emitting tokens, because
`#[handler]` keys its analyzer index by a hash of the handler signature and body; re-emitting
would change the key. Dropped items are blanked and stubs are padded to the original line count,
so `file!()`/`line!()` keep committed source locations stable. Types in schema positions are
copied in full; a type reached only through a hand-written documentation impl can be stubbed.
Build-script environment values and custom cfg are not reproduced; those applications use
`ROUNDHOUSE_HARNESS=linked`.

**Rejected.** Depending on the application binary target; generating a `src/bin` file inside the
application; having `api!` generate the harness (macro expansion runs after the artifacts are
needed and cannot spawn a nested cargo build safely).

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
