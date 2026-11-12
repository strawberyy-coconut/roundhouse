# Traits and internal API

**Status: implemented.** Phases 2–9 are in the tree: private machinery, `DocContext`, the
four-probe surface, `Params` as the only parameter bound, `ArgDoc`, the `OperationDoc` /
`HandlerSpec` split, `Scopes`, and the `roundhouse-macros-core` split. The `doc = |docs|` hook
still exposes `OperationDoc` fields; builder methods are a follow-up. The inventory below
records the starting point, not the current tree.

This note is a target map for roundhouse's trait system. It exists because the current public
surface mixes three different kinds of trait:

1. **Domain specs** — what a type *is* (`ApiSchema`, `ApiResponse`, `AttachmentType`).
2. **Macro probes** — “does this unseen type have capability X?” (`ProbeSchemaFor`, `ProbeArg`,
   `ProbeResponses`, …).
3. **Type-system workarounds** — `ShimPolicy`, `ShimPolicyFor`, `SubstateOf`.

Only the first kind is a user-facing extension point. The other two are machinery. The problem is
not the number of traits — `wormx` has many more — it is that all three kinds are `pub`, are
documented together, and have no shared organising principle.

The target is not “fewer traits at any cost.” It is:

- every public trait has **one subject**, **one implementor audience** and **one consumer**;
- probes and shim plumbing live in `__private`, not `roundhouse::doc::*`;
- documentation contributions are returned as data and merged by the builder, instead of being
  mutated into a shared bag;
- the core keeps the current axum-shaped design. This note does **not** reopen the
  “no framework-neutral core” decision in [Layering](layering.md).

## The four-question test

A trait is targeted when all four answers are crisp:

| Question | Good answer | Bad answer |
|---|---|---|
| **Subject** | “a documented JSON body” | “handler metadata and docs and success statuses” |
| **Implementor** | “the extractor author” | “the macro, sometimes the user, sometimes the framework” |
| **Consumer** | “the document builder at build time” | “whatever calls it” |
| **Phase** | compile, build or request | all three at once |

`AttachmentType` passes: subject is an attachment body's content type, the app author implements
it, the handler macro consumes it at build time. `ProbeJsonResponseDoc` fails immediately: it is
implemented only by the framework, describes no domain concept, and exists to answer one macro
question. `HandlerMeta` fails the subject test because it describes five things.

## Current inventory

Twenty-nine public traits at the start of this refactor. “Kind” is the diagnosis; “Target” is the
disposition proposed by this note.

| Trait | Subject | Implementor | Consumer | Phase | Kind | Target |
|---|---|---|---|---|---|---|
| `ApiSchema` | A body/response JSON shape and its hooks | derive, manual, primitives | document builder, engine | build, request | domain | keep, change `register` to take a schema context |
| `PathValidate` | Custom path hooks | `PathParams` derive | shim | request | domain facet | fold into `Params` |
| `PartsSchema` | Path/query schema | `PathParams`/`QueryParams` derives | document builder, engine | build | domain facet | fold into `Params` |
| `PartsValue` | Path/query value as JSON | same derives | shim | request | domain facet | fold into `Params` |
| `PathNames` | Capture names, scalar schema, params | `PathParams` derive | `routes!`, document builder, shim | compile, build, request | domain facet | fold into `Params` |
| `QueryParams` | Query parameter table and hooks | `QueryParams` derive | document builder, shim | build, request | domain facet | fold into `Params` |
| `ExtractArg` | One handler argument's docs and validation | extractor author | handler macro, shim, builder | build, request | domain | keep, return `ArgDoc` |
| `ApiResponse` | A return type's documented outcomes | derive, manual, built-ins | handler macro, builder | build | domain | keep |
| `ApiError` | An error-shaped response marker | `ApiError` derive + blanket impl | `ApiError` derive, document builder | compile, build | marker | keep as a marker; not an extension point |
| `HandlerMeta` | Handler state, path names, auth, success, docs | `#[handler]` | `routes!`, builder | compile, build | domain | split into `HandlerSpec` + `DocSource` |
| `AuthScheme` | Scheme metadata | app author | builder, guard, shim | build | domain | keep |
| `Authenticate` | Credential check | app author | `Auth` extractor, shim | request | domain | keep |
| `AuthScopes` | Scope names | app author | `Auth<A, Scopes>`, builder | compile, build | domain facet | fold into `Scopes<A>` |
| `Authorize<A>` | Authorization decision | app author | `Auth` extractor | request | domain facet | fold into `Scopes<A>` |
| `AttachmentType` | Attachment body content type | app author | handler macro | build | domain | keep |
| `MultipartForm` | Multipart field table, extraction, schema | `Multipart` derive | extractor, builder | build, request | domain | keep, take `&mut dyn SchemaContext` |
| `Rejection` | Extractor rejection → `Problem` | blanket for `IntoResponse` | shim | request | adapter | keep |
| `ProbeSchemaFor` | Which schema source a type has | framework | handler macro, derives | build | probe | collapse into `ProbeSchema` |
| `ProbeDescription` | Body description | framework | handler macro, derives | build | probe | collapse into `ProbeSchema` |
| `ProbeValidateFor<T>` | Recursive `validate_with` hooks | framework | derives | build | probe | collapse into `ProbeSchema` |
| `ProbeJsonResponse` | Inferred `ApiSchema` JSON response | framework | handler macro | build | probe | collapse into `ProbeSchemaResponse` |
| `ProbeToSchemaJsonResponse` | Inferred `ToSchema` JSON response | framework | handler macro | build | probe | collapse into `ProbeSchemaResponse` |
| `ProbeJsonResponseDoc` | Explicit `@responses:` declaration | framework | handler macro | build | probe | collapse into `ProbeSchemaResponse` |
| `ProbeTextResponse` | Non-JSON response | framework | handler macro | build | probe | collapse into `ProbeSchemaResponse` |
| `ProbeResponses` | Whether a type implements `ApiResponse` | framework | handler macro | build | probe | collapse into `ProbeApiResponse` |
| `ProbeArg<T>` | Whether an argument implements `ExtractArg` | framework | handler macro | build, request | probe | keep as `ProbeArg`, but private |
| `SubstateOf<S>` | `FromRef<S>` alias with a diagnostic | blanket | `routes!` shim bounds | compile | plumbing | move to `__private` |
| `ShimPolicy` | State/auth types for the generated shim | framework | `routes!` | compile, build | plumbing | move to `__private` |
| `ShimPolicyFor<S>` | State equality bound for the shim | blanket | `routes!` | compile | plumbing | move to `__private` |

Two consequences are visible in the table:

- The **params cluster** is five traits for one concept. A path or query type has a name table, a
  schema, a JSON value and hooks; those are facets, not five independent extension points.
- The **probe cluster** is nine traits for three questions: schema source, response source and
  extractor source. (The shim adds three more plumbing traits of its own.) Rust has no stable
  specialization, so the macro uses the autoref trick (`impl for &T`, `impl for &&T`) to ask each
  question. That is a legitimate technique, but it is implementation detail, not API.

## Target public surface

Six role traits — `ApiSchema`, `Params`, `ApiResponse`, `ExtractArg`, `HandlerSpec` and
`DocSource` — plus the three auth traits and three narrow contracts that already have a clear
subject.

| Trait | One sentence | Must not do |
|---|---|---|
| `ApiSchema` | Describe a body/response type's JSON shape and custom hooks. | Know about axum, routes or HTTP responses. |
| `Params` | Describe, convert and validate a path/query/form type. | Install extractors or know the route path. |
| `ApiResponse` | Enumerate a return type's documented outcomes. | Perform I/O or inspect the request. |
| `ExtractArg` | Contribute one argument's documentation and validate its extracted value. | Mutate global state or know the operation. |
| `HandlerSpec` | Expose a handler's static facts: state, captures, auth, success. | Build a router or document an operation. |
| `DocSource` | Produce an operation's documentation. | Know how the document is assembled. |
| `AuthScheme` / `Authenticate` / `Scopes<A>` | Metadata / credential check / authorization decision. | Apply router layers or publish the operation. |
| `AttachmentType`, `MultipartForm`, `Rejection` | Narrow body/field/rejection contracts. | Grow into general routing or document seams. |

`ApiError` remains a marker for error-shaped derives. It is not an extension point; the blanket
impl means users do not implement it directly.

### `ApiSchema`

The shape stays close to today's. The important change is that registration receives a context,
so collision detection no longer needs a thread-local ledger.

```rust
pub trait ApiSchema {
    fn schema() -> RefOr<Schema>;
    fn description() -> Option<&'static str> { None }
    fn register(cx: &mut dyn SchemaContext);
    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {}
    fn validate(&self) -> Result<(), ValidationErrors> { /* default */ }
}
```

`SchemaContext` lives in `roundhouse-schema`, so the schema crate stays axum-free; the facade
implements it for `DocContext`. `DocContext` owns the component table, the collision ledger and
the current source span.

```rust
pub trait SchemaContext {
    fn components(&mut self) -> &mut utoipa::openapi::schema::Components;
    fn claim(&mut self, name: &str, type_name: &'static str) -> Claim;
}
```

### `Params`

One trait replaces `PathNames`, `PathValidate`, `PartsSchema`, `PartsValue` and `QueryParams`.

```rust
pub trait Params {
    const NAMES: &'static [&'static str] = &[];

    fn params(cx: &mut dyn SchemaContext) -> Vec<Param> { Vec::new() }
    fn schema(cx: &mut dyn SchemaContext) -> RefOr<Schema>;
    fn scalar_schema() -> Option<RefOr<Schema>> { None }
    fn value(&self) -> serde_json::Value;
    fn validate_at(&self, pointer: &str, errors: &mut ValidationErrors) {}
}
```

The `PathParams` and `QueryParams` derives still exist separately; they emit different
`ParamIn` values and parameter metadata. The difference belongs to the derive, not to five traits.
`Path<T>` uses `Params::NAMES` and `scalar_schema`; `Query<T>` and `RepeatedQuery<T>` use
`Params::params` and `value`.

### `ApiResponse`

Unchanged in role; the context parameter is the only mechanical adjustment.

```rust
pub trait ApiResponse {
    const SUCCESS: &'static [u16] = &[200];
    fn responses(cx: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}
```

### `ExtractArg`

The three document methods become one contribution, and the validation method stays.

```rust
pub trait ExtractArg {
    const IS_BODY: bool = false;
    const SHIM_VALIDATES: bool = false;

    /// Associated, because the macro documents the extractor *type* before a value exists.
    fn document(cx: &mut DocContext) -> ArgDoc;

    /// Runtime, because validation runs on the value the extractor produced.
    fn validate(&self, cx: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }
}
```

`Option<T>` and `Result<T, E>` become wrapper impls that adjust the returned `ArgDoc`. That
deletes the repeated `document_optional` and `document_result` bodies from every extractor impl.
`Strict<T>`, `Mapped<T>` and the multipart wrapper follow the same pattern.

### `HandlerSpec` and `DocSource`

`HandlerMeta` currently carries both static facts and the documentation function. Split it:

```rust
pub trait HandlerSpec: Send + Sync + 'static {
    type State: 'static;
    const HAS_STATE: bool;
    const PATH_NAMES: &'static [&'static str] = &[];
    const REQUIRED_AUTH: &'static [&'static str] = &[];
    const INFERRED_SUCCESS: Option<&'static [u16]> = None;
    const DECLARED_SUCCESS: Option<&'static [u16]> = None;
}

pub trait DocSource {
    fn operation(cx: &mut DocContext) -> OperationDoc;
}
```

`routes!` uses `HandlerSpec`; the document builder uses `DocSource`. Route policy is applied by
the builder to the returned `OperationDoc`, so `docs_with(validate: bool)` disappears. A handler
describes what it is; a route decides how it is enforced.

### Security

`AuthScopes` and `Authorize<A>` collapse into one `Scopes<A>` trait:

```rust
pub trait Scopes<A: AuthScheme>: Send + Sync + 'static {
    const NAMES: &'static [&'static str] = &[];
    fn authorize(identity: &A::Identity) -> Result<(), Problem> { Ok(()) }
}

pub struct NoScopes;
impl<A: AuthScheme> Scopes<A> for NoScopes {}
```

`Auth<A, Scopes = NoScopes>` keeps its shape. `AuthScheme` stays pure metadata and `Authenticate`
stays behavior; that split is already justified in the auth module docs.

## Data replaces bags

The traits above return data. The document builder owns the merge rules; no trait mutates a
shared `OperationDoc`.

```rust
pub struct DocContext<'a> {
    components: &'a mut Components,
    ledger: &'a mut ComponentLedger,
    source: Source,
}

pub struct ArgDoc {
    pub params: Vec<Param>,
    pub body: Option<BodyDoc>,
    pub parts: Vec<PartsDoc>,
    pub auth: Vec<AuthDoc>,
    pub failures: Vec<ResponseDoc>,
    pub verification: Verification,
}

pub struct OperationDoc {
    pub summary: Option<String>,
    pub description: Option<String>,
    pub params: Vec<Param>,
    pub body: Option<BodyDoc>,
    pub responses: Vec<ResponseDoc>,
    pub declared_responses: Vec<ResponseDoc>,
    pub auth: AuthDocSet,
    pub verification: Verification,
    // path descriptions, parts, body type, response descriptions
}

pub struct Verification {
    body: bool,
    response: bool,
    extractor: bool,
    undocumented_error: Option<&'static str>,
    unbounded_files: Vec<&'static str>,
}

pub enum ValidationPolicy { Schema, PartsOnly, Off }

pub enum BodyPolicy {
    Validate,
    Opaque,
    Unverified(&'static str),
}
```

`Verification` replaces `unverified`, `unverified_response`, `unverified_extractor`,
`undocumented_error` and `unbounded_file_fields` as five independent fields. `BodyPolicy`
replaces the `validate` / `validate_body` / `unverified` triple on `RouteRecord`. Illegal
combinations become unrepresentable: a route cannot be `validate = false` and also claim its body
was verified.

The field list of `OperationDoc` is not the important change. The name, the builder methods and the
merge rules are. The `doc = |docs| …` hook still receives `OperationDoc` with public fields;
the follow-up is to expose methods such as `push_param`, `set_body`, `add_response` and
`mark_unverified` instead.

## Probe and shim consolidation

### Probes: nine to four

All probes live in `roundhouse::__private` and are absent from the documented API.

| Current | Target |
|---|---|
| `ProbeSchemaFor` | `ProbeSchema::probe_schema()` |
| `ProbeDescription` | `ProbeSchema::probe_description()` |
| `ProbeValidateFor<T>` | `ProbeSchema::probe_validate_at(value, …)` |
| `ProbeResponses` | `ProbeResponses::probe_responses(cx)` |
| `ProbeJsonResponse` | removed; it was unused |
| `ProbeToSchemaJsonResponse` | removed; it was unused |
| `ProbeTextResponse` | removed; it was unused |
| `ProbeJsonResponseDoc` | `ProbeJsonResponseDoc::probe_json_response_doc(cx, status)` |
| `ProbeArg<T>` | `ProbeArg<T>` (private) |

`ProbeSchema<T>` is one trait with three autoref levels:

```rust
pub trait ProbeSchema<T: ?Sized> {
    fn probe_schema(self) -> RefOr<Schema>;
    fn probe_description(self) -> Option<&'static str>;
    fn probe_register(self, context: &mut dyn SchemaContext);
    fn probe_validate_at(self, value: &T, pointer: &str, errors: &mut ValidationErrors);
}

impl<T: ApiSchema + ?Sized> ProbeSchema<T> for &SchemaProbe<T> { .. }
impl<T: ToSchema + ?Sized> ProbeSchema<T> for &&SchemaProbe<T> { .. }
impl<T: ?Sized> ProbeSchema<T> for &&&SchemaProbe<T> { .. }
```

The `&` level is `ApiSchema`, `&&` is a `ToSchema`-only type, and `&&&` is the no-schema
fallback. That keeps the `ApiSchema` / `ToSchema` decision in one probe instead of three. The
three inferred-response probes had no callers, so they were deleted rather than merged.

If the framework later drops automatic support for `ToSchema`-only types, the `&&` level goes
away and the fallback is a direct bound. That is a product decision, not a mechanical cleanup;
see Open questions.

### Shim: three to private

`SubstateOf`, `ShimPolicy` and `ShimPolicyFor` are generated-code plumbing. They move to
`roundhouse::__private`, and `shim.rs` becomes `pub(crate)` plus a `#[doc(hidden)]` re-export if a
downstream macro needs a stable path. `ValidationContext` stays public because custom extractors
use it; it belongs in `extract`, not in the shim namespace.

## Migration order

The phases are ordered so the tree keeps compiling and the OpenAPI golden file does not change
until a phase explicitly changes behaviour.

1. **Publish this map.** Annotate the existing traits with their role and mark probes/shim as
   internal in docs. No code change.
2. **Hide the machinery (no signature changes).** *(done)* Add `__private`, re-export the probes and shim
   traits there, and stop re-exporting `probe::*` from `doc`. Leave their signatures alone for
   now. This is behaviour preserving and immediately makes the public surface smaller. Update
   `tests/ui` only if a diagnostic path changes.
3. **Introduce `DocContext`.** *(done)* Replace the thread-local `LEDGER` and `&mut Components` parameters
   with `DocContext` / `SchemaContext`. This touches `ApiSchema::register`,
   `ExtractArg::document`, `ApiResponse::responses`, `MultipartForm::schema` and every derive
   expansion. `MultipartForm` takes `&mut dyn SchemaContext` rather than `DocContext`, because
   `roundhouse-multipart` must not depend on the facade. Phase 3 is the largest mechanical
   change; doing it before the trait folds avoids rewriting the same signatures twice.
4. **Combine the probes (now that the signatures are stable).** *(done)* Nine probes to four:
   `ProbeSchema`, `ProbeResponses`, `ProbeJsonResponseDoc` and `ProbeArg`. They stay in
   `__private`; the public surface does not grow. The three unused inferred-response probes were
   removed rather than merged.
5. **Fold the params cluster.** *(done)* Introduce `Params`, port the path/query derives, and
   remove `PathNames`, `PathValidate`, `PartsSchema`, `PartsValue` and the `QueryParams` trait.
   The derives emit `Params` directly.
6. **Make `ExtractArg` return `ArgDoc`.** *(done)* Add the `Option`/`Result` wrapper impls, delete
   `document_optional`/`document_result`, and update the shim to merge contributions. Update
   `docs/design/extensibility.md` and the custom-extractor example.
7. **Replace `HandlerDocs` with `OperationDoc`.** *(done, except hook ergonomics)* Split
   `HandlerMeta` into `HandlerSpec` and `DocSource`, remove `docs_with(validate)`, and move route
   policy into the builder. `HandlerDocs` remains as a compatibility alias; the `doc = |docs|`
   hook still uses public fields, and builder methods are a follow-up.
8. **Fold auth scopes.** *(done)* Replace `AuthScopes` + `Authorize<A>` with `Scopes<A>`, and update
   `Auth<A, Scopes>` and the auth example.
9. **Split the macro implementation (separate decision).** *(done)* Move parsing, table validation and
   expansion into `roundhouse-macros-core`, as already anticipated in
   [Macros](macros.md#future). This does not change the runtime trait surface; it lets the
   expansion and probe tests move out of `trybuild` and keeps macro internals out of the facade.

## Constraints

- **Proc macros see tokens, not types.** The handler macro cannot see whether an argument's type
  implements a trait except through a probe or a generated bound. That is why probes exist.
- **Rust has no stable specialization.** The autoref `&T` / `&&T` trick is the available
  workaround. The target confines it; it does not pretend it is unnecessary.
- **axum's extractors are open.** `ExtractArg` is the seam for arbitrary extractors. Requiring it
  unconditionally would be cleaner, but it would reject third-party extractors that compile
  today. That trade-off is a decision, not an accident.
- **utoipa's schema traits are runtime.** `ToSchema` returns owned values and registers
  components at build time. A fully compile-time document is not available without replacing
  utoipa.
- **The tree is `'static`.** State is erased to `TypeId` and `dyn Any` at the `RouteRecord`
  boundary. The target confines erasure to that boundary; it does not remove it.
- **No framework-neutral core.** Per [Layering](layering.md), the route and document model stays
  axum-shaped. “Private” here means module visibility and doc hiding, not a new core crate.

## Decisions

- **Three kinds of trait must not share one namespace.** Domain specs are public; probes and shim
  plumbing are `__private`; internal helpers are `pub(crate)`.
- **Six roles, not thirty traits.** `ApiSchema`, `Params`, `ApiResponse`, `ExtractArg`,
  `HandlerSpec`/`DocSource`, and the auth triple are the extension surface. Narrow traits such as
  `AttachmentType`, `MultipartForm` and `Rejection` stay because they have one clear subject.
- **Traits return data; the builder owns merge rules.** No trait receives `&mut OperationDoc`.
- **Policy is a value, not booleans.** `BodyPolicy` and `ValidationPolicy` replace independent
  flags on `RouteRecord`.
- **Renaming is separate from reshaping.** `ApiSchema`, `ApiResponse` and `ExtractArg` can keep
  their names through this refactor. The role names in this note are working names, not a
  commitment to rename.
- **A clean break is better than compatibility traits.** A deprecated trait cannot preserve
  method-based impls without overlap tricks. Put the mapping in `MIGRATING-*.md`.

## Open questions

1. **Keep the `ToSchema`-only fallback?** It is the reason `ProbeSchema` exists. Requiring
   `ApiSchema` (with `impl_api_schema!` for generated types) would remove the probe and simplify
   `ApiSchema`-based responses. It would also be a stricter product.
2. **Keep permissive unknown extractors?** `ProbeArg` exists so a custom extractor compiles and
   is marked unverified. Requiring `ExtractArg` would remove the probe and make `strict()` the
   default for extractors.
3. **Where does `Params` live?** It needs `DocContext`, which is axum-shaped. Put the trait in
   `roundhouse` and re-export it, or define a minimal `ParamsContext` in `roundhouse-schema` so
   the schema crate can own the trait without depending on axum.
4. **Should the `doc = |docs|` hook move to builder methods?** `OperationDoc` fields are still
   public for compatibility. A method-only surface would protect the merge invariants, but it is
   a breaking change for existing hooks.
5. **Should `ApiError` remain a marker?** It lets the derive enforce a 4xx/5xx-only contract. If
   that contract moves into `ApiResponse::SUCCESS` validation, the marker can go.
6. **Is `ApiError` worth keeping?** It is a marker that lets the derive enforce a 4xx/5xx-only
   contract. If that contract moves into `ApiResponse::SUCCESS` validation, the marker can go.

## Future work elsewhere

The three follow-ups this map depends on belong to the notes that own those areas:

- the `roundhouse-macros-core` split, including moving probe and expansion tests out of
  `trybuild`, is tracked in [Macros](macros.md#future);
- the probe collapse when Rust gains specialization or negative bounds is tracked in
  [Extensibility](extensibility.md#future);
- compile-time document fragments are tracked in [Documentation](documentation.md#future).

## Summary

Today the trait surface is three layers interleaved: domain specs, macro probes and type-system
plumbing. The target separates them, reduces the params cluster from five traits to one, reduces
the probe cluster from nine to four (or one), moves the shim traits to `__private`, and replaces
`OperationDoc` mutation with `ArgDoc` / `OperationDoc` data. The resulting public surface is six
role groups plus a few narrow contracts — each with a clear subject, implementor and consumer.
