# The document

The OpenAPI document is a projection of the same resolved routes the router uses. There is no
side-car specification and no second description to keep in sync:

- `routes!` supplies each operation's path, method, tags, layers, security and route policy.
- `#[handler]` supplies the request, response and security contribution derived from the
  signature, plus prose tags and explicit response declarations.
- `#[roundhouse::schema]` supplies the body shape and the runtime validation hooks.
- `prepare` turns those contributions into the committed document.

This page explains the operation model, response documentation, prose precedence and the checks
that keep the artifact honest. The build pipeline and artifact format are in
[Contract build](contract-build.md).

## From declaration to operation

```text
#[handler] async fn show(Path(id): Path<NoteId>) -> Result<Json<Note>, Problem>
    │
    ├─ marker type: HandlerSpec (state, captures, required auth) + DocSource (operation)
    ├─ generated shim: extractors + ValidationContext + route policy
    │
    └─ contract build calls DocSource::operation(&mut DocContext)
            │
            ▼
        OperationDoc
            ├─ doc tags and typed response declarations
            ├─ the return type's DocumentedResponse impl
            ├─ argument contributions from ExtractDoc
            ├─ route policy (body validation, auth)
            └─ entry-level `prose = ..` and `doc = |docs|` hooks
            │
            ▼
        doc::build
            ├─ merge layer docs, guards, rejections and documented responses
            ├─ align captures with parameters
            ├─ apply typed declarations and response prose
            ├─ validate the operation's success and security
            └─ validate_document
            │
            ▼
        OpenApi 3.1 → .roundhouse/openapi.json
```

`OperationDoc` is data. The macro emits the code that fills it; the builder owns every merge rule.
Response documentation is not a separate input. For a named return type, the type's
`DocumentedResponse` impl supplies it directly. For an opaque return type, `prepare` first runs a
source analyzer over the library and writes a per-handler index; `#[handler]` reads that index at
expansion time and emits typed trait calls. The compiler verifies the calls, so a wrong inference
fails the contract build instead of publishing a wrong document. An explicit declaration supplies
whatever neither path can name. The probe, the analyzer and the unverified markers are described
in [Extension points](extension-points.md).

## Extractors and body documentation

`ExtractDoc::document` returns an `ArgDoc`; it does not mutate a shared bag. The builder merges
the contribution and owns the conflict rules. `Option<Self>` and `Result<Self, E>` are wrapper
impls: `Option` relaxes parameters, the body and required auth into their optional form; `Result`
drops extractor failure responses because the handler owns the rejection.

`Json<T>`, `Form<T>`, `Path<T>`, `Query<T>`, `RepeatedQuery<T>`, `Extension<T>`, `State<T>`,
`Bytes`, `Body`, `Auth<A>`, `Strict<..>`, `Mapped<..>` and the no-op axum parts extractors have
implementations. Automatic body documentation requires `T: ApiSchema`; a custom extractor
implements `ExtractArg` and, under `build`, `ExtractDoc`.

## Responses

A response type documents itself through `roundhouse::doc::DocumentedResponse`. The impl owns the
status, content type, body schema and headers; the trait signature and the extension guidance are
in [Extension points](extension-points.md). Built-ins are ordinary impls:

| Return type | Documented response |
|---|---|
| no `->`, `()` | `200`, empty |
| `Json<T>`, `Html<T>`, `String`, `&str`, `Vec<u8>`, `Bytes`, `Body`, `Attachment<T>`, `File`, `Sse` | `200` with the type's content type |
| `Status<CODE, T>` | exactly `CODE`, `application/json` body |
| `Empty<CODE>` | exactly `CODE`, no body |
| `Created<T>` | `201` |
| `NoContent` | `204` |
| generated wrappers in `roundhouse::status` (`NotFound`, `Conflict`, …) | their exact code; body wrappers are JSON, bodyless wrappers are empty |
| `StatusCode`, `(StatusCode, T)`, `Redirect` | `default` |
| `Option<T>`, `Infallible`, `Response` | none |
| `Result<T, E>` | `T`'s responses plus `E`'s (`T: DocumentedResponse`, `E: DocumentedError`) |
| `Problem` | `default`, problem+json (as a `DocumentedError`) |
| `impl IntoResponse`, `impl DocumentedResponse` | the body-analyzed success sites |

`Status<CODE, T>` and `Empty<CODE>` keep the status in the type, so `DocumentedResponse` can see
it. `(StatusCode, T)` cannot: the trait receives a type, not the runtime value, so it documents
OpenAPI's `default`. Use `Status<CODE, T>` when the status is known at compile time.

The named wrappers are generated from the IANA registry snapshot at
`roundhouse-schema/data/status-codes.json`; `tools/status-codes` refreshes it.

A custom response type implements the trait once and is documented exactly like a built-in:

```rust
pub struct Csv;

impl roundhouse::doc::DocumentedResponse for Csv {
    fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc {
            status: 200,
            description: "CSV export".to_string(),
            content_type: Some("text/csv".to_string()),
            schema: Some(roundhouse::doc::schema::string()),
            headers: Vec::new(),
        }]
    }
}
```

Errors are documented by `DocumentedError`. `Result<T, E>` requires `T: DocumentedResponse` and
`E: DocumentedError`; the builder merges both. The distinction is the path, not the status code:
a normal `return`/tail is a response and uses `DocumentedResponse`; `?` is an error path and uses
the handler's `E: DocumentedError`. Each trait impl chooses its own statuses; the analyzer never
infers a code from the expression. `?` contributes nothing on its own: it converts into `E`, and
the document describes `E`. If `From<DbError> for Problem` maps a specific error to
the catch-all, return `DbError` (or a union enum) when its statuses matter.

### Rust paths are not HTTP status codes

`Result<T, E>` is Rust control flow. It says whether the function continued or took an early
return. `?` is an early return for a handled situation: it stops the rest of the handler. Neither
fact implies an HTTP status.

- A **normal return or tail expression** is the response path. It is documented by
  `DocumentedResponse`.
- A **`?`** is an error path. It converts into the handler's `E`, documented by
  `DocumentedError`.
- The **status class** is chosen by the trait impl. Either path may document any status; the
  analyzer does not require a normal return to be 2xx or a `?` to be 4xx/5xx.

A normal return documenting 500 and a `?` documenting 409 are both valid:

```rust
struct FailureReport;

impl DocumentedResponse for FailureReport {
    fn responses(_: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::problem(500, "Report could not be produced")]
    }
}

struct Conflict;

impl DocumentedError for Conflict {
    fn responses(_: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::problem(409, "Conflict")]
    }
}

async fn work(...) -> Result<FailureReport, Conflict> {
    if something_is_broken() {
        return Ok(FailureReport);   // response path, 500
    }
    let value = check()?;           // early return, 409
    // ...
}
```

The document gets both `500` and `409`. The analyzer selected `FailureReport` because it is on
the normal path, and `Conflict` because it is the handler's error type; it did not look at either
status.

The document builder still has two status-shaped rules, and they are document rules rather than
Rust-path classification:

- **A success somewhere.** An operation must have a `default` or a 2xx/3xx response. If the only
  normal return documents `500`, the strict build still reports `UndeclaredSuccess` until another
  response or declaration supplies a document success. That does not reclassify the `500` as an
  error.
- **One shape per status.** Two different shapes at the same status are a document conflict;
  for statuses `>= 400` the builder collapses them to `Problem`. That is an OpenAPI merge rule,
  not an inference from the Rust path.

`Problem` is one implementor, not the mandatory error type. A custom documentation impl is
marked `#[roundhouse::build_only]`, so a production build that compiles only
the `contract` feature drops it instead of failing to resolve the documentation types:

```rust
#[roundhouse::build_only]
impl roundhouse::doc::DocumentedError for NoteMissing {
    fn responses(_: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::problem(404, "Note not found")]
    }
}
```

`impl IntoResponse` has no nameable concrete type. The analyzer resolves the body's tail and
`return` expressions and the macro emits:

```rust
<Json<Note> as DocumentedResponse>::responses(context)
<DbError as DocumentedError>::responses(context)
```

A site the analyzer cannot resolve marks the operation `x-roundhouse-unverified-response`; the
strict build rejects an operation with no success. `examples/notes` is the end-to-end reference:
`routes/notes.rs` uses `?` on two different error types, and `error.rs` documents both with
`#[roundhouse::build_only]` `DocumentedError` impls. `.into_response()` keeps the receiver's
documented type, so `Json<Note>.into_response()` still documents `Json<Note>`; only a receiver that
is itself erased — a bare `Response`, a trait object, or a helper returning `Response` — stays
opaque. `.into_documented_response()` additionally erases the value to `Documented`, so branches
with different documented types can share one return type. An explicit declaration remains the
escape hatch:

```rust
#[handler(response(200, Json<Note>), response(404, Problem))]
pub async fn show(...) -> impl IntoResponse { ... }
```

`response(status, Type)` calls `Type::responses(..)` and overrides only the status; a bare
`response(status)` is status-only. `Type` must implement `DocumentedResponse`; the declaration
does not accept a `ToSchema`-only type. A typed declaration that contradicts the return type's
shape needs `override_response(..)`. For exact error codes, either return the specific error type
or declare the status.

### Response description precedence

The builder applies descriptions in this order, so the last applicable source wins:

1. handler `@response <status>: text` or typed `response_description(status, "…")`; when both
   target one status, `@response` is applied after the typed declaration;
2. the response body type's `ApiSchema::description()` or `Problem`'s literal title;
3. `api! response_defaults` for a status that still has only its canonical description;
4. the canonical HTTP reason phrase.

A duplicated `@response` tag for one status is a compile error. A typed
`response_description(..)` for a status that is not documented is a diagnostic. A typed
declaration whose shape differs from the return type's documented response is
`BuildError::ConflictingResponse` unless it uses `override_response(..)`.

## `ApiSchema` vs `ToSchema`

`ApiSchema` is the roundhouse adapter between a Rust type and the OpenAPI component table:

- it registers the type as a named component (`$ref`);
- it applies roundhouse-only patches (`unique_items`, the `url` → `uri` format mapping);
- its generated `ValidateHook` impl carries `validate_with` hooks (`validate_at`) that survive
  without `build`;
- it exposes the type's `#[roundhouse(description = "…")]` or `@description:` through
  `description()`, used for response prose;
- it is the bound automatic body and JSON response documentation requires.

`#[roundhouse::schema]` is the one-annotation form. With `build` it injects the schema derive
and `ApiSchema`; without `build` it keeps the runtime `ValidateHook` impls and drops the utoipa
tokens before name resolution. `roundhouse::doc::ApiSchema` is a build-only re-export; the
trait itself lives in `roundhouse-schema`.

A type that only implements utoipa's `ToSchema` can still be documented explicitly:

- adapt it with `roundhouse::interop::impl_from_utoipa!(Type);`, then use it like any
  `ApiSchema` type;
- implement `DocumentedResponse` (or `DocumentedError` for a failure) for it and return it;
- use a `doc = |docs| ..` hook for hand-written response metadata or a one-off schema.

`Json<T>`'s response impl requires `T: ApiSchema`; a plain `ToSchema` type is not automatically
documented. A blanket `impl<T: ToSchema> ApiSchema for T` would overlap the derive and primitive
implementations.

Component names are the Rust type name by default. Two different Rust types that register the
same name are a `BuildError::ComponentCollision`; `#[roundhouse(name = "module.Name")]`
disambiguates them. The document uses the dotted form (`module.Name`).

## Building the operation

`doc::build` walks the resolved routes in order:

1. **Skip hidden routes.** A hidden route is served but omitted; the document is marked
   `x-roundhouse-hidden`.
2. **Omit `any`.** An `any` method set cannot be one operation; a diagnostic is recorded unless
   the entry acknowledges it.
3. **Compute `OperationDoc`.** The handler marker's statements run, the route's validation policy
   is applied to the body marker, then the entry's `prose` and `doc` hook run.
4. **Merge layer docs.** Parameters, body, responses, auth and schemes from module/global
   `LayerDoc`s are added. A layer body only fills an absent body.
5. **Merge guards.** Security guards installed by construction become requirements even if the
   handler did not mention them. A handler's scoped `Auth<A, Scopes>` wins over a bare guard for
   the same scheme. A guard turns `Option<Auth<A>>` into a required scheme while keeping the
   handler's scopes.
6. **Merge parameters.** Parameters are deduplicated by name and location; two schemas for the
   same parameter are `BuildError::ConflictingParameter`.
7. **Fill captures.** Every `{capture}` gets a required path parameter. A scalar `Path<T>` can
   supply the schema when the template has one capture; otherwise a string fallback is used with
   a diagnostic. A path parameter with no capture is `BuildError::UnknownPathParameter`.
8. **Merge responses.** The return type's `DocumentedResponse` output (or, for an opaque return,
   the analyzer's typed sites and the handler's `DocumentedError`), declared responses, extractor
   failures and layer responses are merged by status. Identical shapes merge their
   headers. Two different 4xx/5xx shapes collapse to a `Problem`, because that is what every
   failure path produces. Two different success bodies keep the first and emit a diagnostic.
9. **Apply typed response declarations.** Declarations add statuses or refine documented ones. A
   bare `status` declares the status as a success and keeps an already documented response for
   that status. A typed declaration that replaces a different shape without
   `override_response(..)` is `BuildError::ConflictingResponse`; with the override the operation
   is marked `x-roundhouse-overridden-response`.
10. **Apply response prose.** `@response`, `response_description` and `api! response_defaults`
    follow the precedence above.
11. **Require a success.** An operation with no `default` and no 2xx/3xx response is
    `BuildError::UndeclaredSuccess`.
12. **Build security.** Requirements are ANDed in declaration order. `Option<Auth<A>>` adds the
    unauthenticated alternative. Every scheme name must have a registered kind, or the build fails
    with `UnverifiedSecurity`.
13. **Attach extensions** and finish the operation. Request body and parts observations are
    recorded for `contract.json` on the way.

The finished document is validated by `validate_document`, including after
`Contract::document(..)` hooks. The same build records request plans and the response table in
`contract.json`; the runtime compiles the plans rather than the document. See
[Validation](validation.md).

## Rejection responses

By default (`DocumentRejections::Auto`), every extractor publishes the failures it can produce:

| Extractor | Responses |
|---|---|
| `Json<T>`, `Form<T>` | `400`, `415` |
| `Path<T>`, `Query<T>` | `400` |
| `Auth<A>` | `A::UNAUTHENTICATED`, plus `A::FORBIDDEN` when configured |
| `Extension<T>` via `Strict` | `500` |
| `Multipart<F>` | `400`, `413`, `415` |

`App::document_rejections(false)` publishes only the responses the handler and declarations
supply and marks operations with `x-roundhouse-rejections: none`. This is useful when a gateway
owns the error contract.

## Extensions

Roundhouse adds a small set of `x-roundhouse-*` keys rather than inventing response codes:

| Extension | Meaning |
|---|---|
| `x-roundhouse-hidden` | The document omits at least one route. |
| `x-roundhouse-merged` | A hand-written router was merged and is not documented. |
| `x-roundhouse-validation: false` | The route opted out of constraint validation. |
| `x-roundhouse-body-validation: false` | The route kept parts validation and opted the body out. |
| `x-roundhouse-rejections: none` | Extractor rejection responses are not published. |
| `x-roundhouse-unverified-body` | The body comes from an extractor whose schema could not be verified. |
| `x-roundhouse-unverified-response` | A permissive build published a `default` placeholder for a missing success. |
| `x-roundhouse-unverified-extractor` | An argument's documentation could not be resolved. |
| `x-roundhouse-overridden-response` | An `override_response(..)` declaration replaced the return type's shape. |

## The document artifact

`openapi.json` is OpenAPI 3.1. The runtime serves it verbatim. `Built::openapi_30()` produces a
best-effort 3.0.3 shape by rewriting `type: [T, "null"]`, numeric
`exclusiveMinimum`/`exclusiveMaximum`, nullable `oneOf`, `const` and `examples`. It covers the
constructs this crate emits; it is not a general 3.1 → 3.0 converter.

Two tests keep the artifact honest:

- `tests/openapi.golden.json` is compared against the spine test's document. Regenerate it with
  `ROUNDHOUSE_BLESS=1 cargo test -p roundhouse --test spine openapi_golden` after an intentional
  change.
- `doc::validate_document` runs on every contract build, including after `Contract::document(..)`
  hooks, so a hook cannot publish a path that contradicts its own parameters.

`Built::document()` is a `build`-only convenience that parses the embedded artifact lazily. A
production `contract` build should use `Built::openapi_json()` or `Built::routes()`
instead.

## Constraints

- **Automatic body docs require `ApiSchema`; automatic response docs require
  `DocumentedResponse`.** `ToSchema` alone is an explicit declaration, not automatic.
- **No parallel extractor types.** The traits are implemented for axum's types; a custom extractor
  is an impl, not a conversion.
- **No second document model.** The document is utoipa's model behind a thin facade. Roundhouse
  patches schemas and owns assembly, but does not define an alternative OpenAPI type.
- **Unrepresentable routes are omitted, not guessed.** `any` routes are served and excluded.
  Merged routers are served and marked. The document never claims to describe them.
- **Production does not assemble documents.** A `contract` build has no `DocSource`
  execution and no schema derivation for the document.
