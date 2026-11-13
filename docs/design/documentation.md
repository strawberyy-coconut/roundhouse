# Documentation

The document and the router come from the same declarations. There is no side-car annotation
file and no second description to keep in sync: a `routes!` entry supplies the path, method and
policy; `#[handler]` derives the request, response and security contribution from the signature;
explicit prose tags and the handler's typed `response(..)` metadata supply
the prose and response declarations the signature cannot express; the document builder turns those
contributions into an operation.

## From signature to operation

```text
#[handler] async fn show(Path(id): Path<NoteId>, Query(q): Query<Filter>) -> Result<Json<Note>, NoteError>
    │
    ├─ marker type + HandlerSpec (state, path names, required auth, inferred success)
    ├─ generated shim (extractors + ValidationContext + policy)
    └─ DocSource::operation(context) ──► OperationDoc
            │
            ▼
    doc::build::build_with_observations
        ├─ merge layer docs and guards
        ├─ merge parameters, fill template captures
        ├─ merge inferred responses, then typed response declarations
        ├─ build security requirements and schemes
        └─ validate_document
            │
            ▼
        OpenApi 3.1
```

`OperationDoc` is a plain value; the document is built when the app is built, not when the
macro expands. The macro emits the code that fills `OperationDoc`.

## `#[handler]`, `HandlerSpec` and `DocSource`

The attribute leaves the function untouched and emits a hidden marker type beside it. The
marker implements `roundhouse::doc::HandlerSpec` and `roundhouse::doc::DocSource`:

| Item | Source |
|---|---|
| `State` / `HAS_STATE` | `State<T>` in the signature, or `()` |
| `PATH_NAMES` | `Path<T>` where `T: Params` |
| `REQUIRED_AUTH` | `Auth<A>` (not `Option<Auth<A>>`) |
| `INFERRED_SUCCESS` | A small return-type classification, or `None` when unknown |
| `DECLARED_SUCCESS` | Success statuses declared by `#[handler(response(..))]` |
| `operation(context)` | Doc tags, one statement per argument plus the response statements |
| `verification.body_verified()` | Whether every body argument enforces its own schema during extraction |

`routes!` registers a generated shim (`__roundhouse_shim::<P>`), not the function. The shim has
the handler's extractors plus a `ValidationContext` and the route policy's auth extractor; it
validates after extraction and then calls the handler. The marker is what makes
`routes!` able to find the metadata without re-parsing the function.

The marker type is also the missing-attribute diagnostic: `routes!` aliases it before use, so a
plain `async fn` produces one "type not found" error pointing at the missing `#[handler]`
instead of a cascade of module-path errors.

Only the signature is read. The body is never parsed, so routing metadata cannot be inferred
from what the function does.

## Extractors: `ExtractArg`

Each extractor owns its documentation and its route-level validation:

```rust
pub trait ExtractArg {
    const IS_BODY: bool = false;
    const SHIM_VALIDATES: bool = false;

    fn document(context: &mut DocContext<'_>) -> ArgDoc;
    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> { Ok(()) }
}
```

- `document` returns the argument's contribution as `ArgDoc`; it does not mutate a shared bag.
  The builder owns the merge rules.
- `Option<Self>` and `Result<Self, E>` are wrapper impls. `Option` relaxes parameters, the body
  and required auth into their optional form; `Result` drops extractor failure responses because
  the handler owns the rejection.
- `validate` is the shim's post-extraction hook. The default is a no-op, which is right for an
  extractor that enforces its own contract.
- `IS_BODY` keeps the one-body rule and the validation marker honest.
- `SHIM_VALIDATES` says the extractor already executed the schema during extraction, so the
  shim should not execute it a second time.

`Json<T>`, `Form<T>`, `Path<T>`, `Query<T>`, `RepeatedQuery<T>`, `Extension<T>`, `State<T>`,
`Bytes`, `Body`, `Auth<A>`, `Strict<..>`, `Mapped<..>` and the no-op axum parts extractors have
implementations in `src/doc/arg.rs`. A custom extractor gets one impl in the crate that owns
the type; see [Extensibility](extensibility.md).

### The concrete-type probe

The macro sees only the token `T` of an argument. It cannot ask "does `T` implement
`ExtractArg`?" with a normal bound, because a foreign extractor without an impl must still
compile. The probe uses autoref specialization:

```rust
impl<T: ExtractArg> ProbeArg<T> for &ArgProbe<T> { /* the real documentation */ }
impl<T>           ProbeArg<T> for &&ArgProbe<T> { /* mark unverified */ }
```

The compiler prefers the more specific receiver when the bound holds. When it does not, the
argument's `ArgDoc` has `unverified_extractor` set. The build emits a diagnostic, and `strict()`
rejects it. This is the same fallback that keeps a handler using any unregistered extractor
compiling.

The builder clears `OperationDoc::verification`'s body marker when the route validates and the
marker reports `body_verified()`; an unverified body keeps it, so the document carries
`x-roundhouse-unverified-body`.

## Responses: `ApiResponse`

A return type documents itself through `ApiResponse`:

```rust
pub trait ApiResponse {
    const SUCCESS: &'static [u16] = &[200];
    fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}
```

`SUCCESS` declares the success statuses the type can produce. The macro classifies the return type
once (`ReturnShape` in `roundhouse-macros-core`) and derives both
`HandlerSpec::INFERRED_SUCCESS` and the document statements from that value, so the compile-time
check and the published response cannot disagree. `&[]` means "known, but no success"
(`Option<T>`, `Infallible`, a bare `impl Trait`); `&[0]` means "status chosen at runtime" and is
documented as OpenAPI's `default` response.

| Return type | Inferred status |
|---|---|
| no `->`, `()` | `200` |
| `Json<T>`, `Html<T>`, `String`, `&str`, `Vec<u8>`, `Bytes`, `Body`, `Attachment<T>`, `File`, `Sse` | `200` |
| `Created<T>` | `201` |
| `NoContent` | `204` |
| `StatusCode`, `(StatusCode, T)` | `0` (`default`) |
| `Option<T>`, `Infallible`, bare `impl Trait` | none |
| `Result<T, E>` | `T`'s statuses |
| other tuples, custom `ApiResponse` | unknown (probe or declaration) |

The macro still special-cases three shapes:

- **`Result<T, E>`** — the success and error types are documented independently, so an
  undocumented `E` becomes a diagnostic (`verification.undocumented_error()`) rather than a
  missing success.
- **`Vec<u8>`** — bytes on the wire, not a JSON array of integers; documented as
  `application/octet-stream`.
- **`impl Trait`** — cannot be named in a turbofish. `Sse<impl Stream>` and `File` keep their
  fixed shapes; every other hidden type declares its responses in `routes!`.

Everything else goes through `ResponseProbe<T>`, which uses the same autoref trick: a type
implementing `ApiResponse` contributes its responses; a type that only implements
`IntoResponse` contributes none. A known shape whose probe finds no `ApiResponse` (an `Html` or
`Created` body without `ApiSchema`, for example) is published with its bare status and
`x-roundhouse-unverified-response`; `strict()` rejects it. An unknown shape reaches the
undeclared-success check.

`Result<_, Problem>` documents a `default` problem response. `ApiError`/`ApiResponse` derives
register the responses of an error enum, so `Result<Json<T>, NoteError>` documents both sides.

### Choosing the error shape

`Result<T, E>` documents `T` and `E` independently, which makes the split matter:

- **Outcome enum on the `Ok` side.** `enum ShowNote { Found(Json<Note>), NotFound { id } }`
  describes exactly the statuses the endpoint can return. Endpoint-specific failures belong here,
  not in the error side.
- **One crate-wide infrastructure error on the `Err` side.** `enum InternalError { Storage(#[from]
  std::io::Error) }` gives every fallible handler the shared `500` without repeating
  infrastructure variants.
- **Do not use a shared domain error enum on every handler.** The operation would document every
  domain failure whether or not that route can produce it.
- **Do not create a per-endpoint error enum just to narrow the document.** That is what the
  outcome enum is for; keep the error side for infrastructure.
- **No `Outcome<T>` helper.** A wrapper that hides the variants would hide which statuses the
  endpoint can produce, which is the reason to write the enum. Derive `ApiResponse` and let the
  body type's description fill the prose.

A `Result` type alias is visible through the same blanket `ApiResponse for Result<T, E>` impl as
long as both sides implement `ApiResponse`; the macro's compile-time success check remains
syntactic and is skipped for aliases, so the build-time check is authoritative.

Response descriptions are resolved in this order: `#[status(description = "…")]`, the variant's
`@description:` tag, the body type's `ApiSchema::description()`, then the canonical HTTP reason
phrase. The body type description also feeds a direct `Json<T>` or `Vec<T>` response, and a
handler can override an individual status with `response_description(..)`. `ApiError` uses the
resolved text as the `Problem` title as well; `problem_type = "…"` only sets the RFC 9457 `type`
URI (`https://roundhouse.rs/problems/…`).

Every field in a response variant must declare its destination: `#[body]`, `#[header("name")]`,
`#[location]`, `#[from]`, `#[source]`, or `#[response(ignore)]`. The derive rejects an unannotated
field instead of silently dropping it. A variant that returns a problem can use a `Problem` body:
`NotFound(Problem)`. `#[response(ignore)]` is the explicit way to keep data on the variant without
publishing it.

Generic `ApiResponse`/`ApiError` enums copy their generics and `where` clause into each impl and add
only the bounds a field shape needs: `Json<T>` needs `T: ApiSchema` for docs and `Json<T>:
IntoResponse` at runtime, a bare body needs `T: IntoResponse` and `T: ApiSchema`, and a header or
location needs `T: TryInto<HeaderValue>` and the schema bound. The built-in `Created<T>` and
`NoContent` wrappers cover the common created/no-content cases without a derive.

## `ApiSchema` vs `ToSchema`

`utoipa::ToSchema` supplies the shape. `ApiSchema` is the roundhouse adapter:

- it registers the type as a named component (`$ref`);
- it applies the roundhouse-only patches (`unique_items`, the `url` → `uri` format mapping);
- it carries `validate_with` hooks (`validate_at`);
- it exposes the type's `#[schema(description = "…")]` or `@description:` through `description()`,
  used for response prose;
- it is the bound automatic body and JSON response documentation requires.

`#[roundhouse::schema]` is the one-annotation form: it re-emits a struct or enum with
`#[derive(ToSchema, ApiSchema)]`. The two-derive form remains valid and explicit. A derive cannot
invoke another derive, so an attribute macro is the only way to make one annotation provide both;
the attribute must be the outermost attribute and cannot be combined with a separate `ToSchema` or
`ApiSchema` derive.

A type that only derives `ToSchema` can still be documented explicitly:

- `#[handler(response(200, Type))]` on the handler accepts either trait;
- a `doc = |docs| ..` hook can set a body or response by hand;
- `roundhouse::doc::json_body_to_schema::<T>` builds a `BodyDoc` in that hook;
- for generated types, `roundhouse::impl_api_schema!(Type);` implements the adapter. It inserts
  the type's own component and registers everything reachable through `ToSchema::schemas()`.

What it cannot do is be inferred from `Json<T>` automatically. `Json<T>: ApiResponse` requires
`T: ApiSchema`, so a plain `ToSchema` return type lands on the unverified fallback and `strict()`
rejects it. That is intentional: the document would otherwise promise the unpatched shape. A
blanket `impl<T: ToSchema> ApiSchema for T` is impossible because it overlaps the derive and the
primitive implementations.

Component names follow utoipa. Two different Rust types that register the same name are a
`BuildError::ComponentCollision`; `#[schema(as = module::Name)]` disambiguates them (utoipa formats
the component name with dots: `module.Name`).

## Building the operation

`doc::build::build_with_observations` walks the resolved routes in order:

1. **Skip hidden routes.** A hidden route is served but omitted; the document is marked
   `x-roundhouse-hidden`.
2. **Omit `any`.** An `any` method set cannot be one operation; a diagnostic is recorded.
3. **Compute `OperationDoc`.** `(record.docs)(context)` runs the marker's statements
   (including prose tags and typed response declarations), applies the route's validation
   policy to the body marker, then runs the entry's `doc` hook.
4. **Merge layer docs.** Parameters, body, responses, auth and schemes from module/global
   `LayerDoc`s are added. A layer body only fills an absent body.
5. **Merge guards.** Security guards installed by construction are promoted into requirements
   even if the handler did not mention them. A handler's scoped `Auth<A, Scopes>` wins over a
   bare guard for the same scheme. A guard turns `Option<Auth<A>>` into a required scheme while
   keeping the handler's scopes.
6. **Merge parameters.** Parameters are deduplicated by name and location; two schemas for the
   same parameter are `BuildError::ConflictingParameter`.
7. **Fill captures.** Every `{capture}` gets a required path parameter. A scalar `Path<T>` can
   supply the schema when the template has one capture; otherwise a string fallback is used
   with a diagnostic. A path parameter with no capture is
   `BuildError::UnknownPathParameter`.
8. **Merge responses.** Inferred responses, extractor failures (when enabled), and layer
   responses are merged by status. Identical shapes merge their headers. Two different
   4xx/5xx shapes collapse to a `Problem`, because that is what every failure path produces.
   Two different success bodies keep the first and emit a diagnostic.
9. **Apply typed response declarations.** Declarations add statuses or refine inferred ones.
   A bare `status` declares the status as a success and keeps an already-inferred response for
   that status; when no response was inferred it adds an empty one. This is the alias-friendly
   success declaration. A typed declaration that replaces a different shape without
   `override_response(..)` is `BuildError::ConflictingResponse`; with the override, the operation
   is marked `x-roundhouse-overridden-response`. A non-canonical description is preserved when
   the status and shape match. `response_description(..)` then replaces the description of an
   existing status; a description for a status the operation does not document is a diagnostic.
10. **Require a success.** An operation with no `default` and no 2xx/3xx response is
    `BuildError::UndeclaredSuccess`.
11. **Build security.** Requirements are ANDed in declaration order. `Option<Auth<A>>` adds the
    unauthenticated alternative. Every scheme name must have a registered kind, or the build
    fails with `UnverifiedSecurity`.
12. **Attach extensions** and finish the operation.

The document is then validated and, if request validation is enabled, the schema engine is
compiled from the same components and inline body/parts schemas. See
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
supply, and marks operations with `x-roundhouse-rejections: none`. This is useful when a
gateway owns the error contract.

## Extensions

roundhouse adds a small set of `x-roundhouse-*` keys rather than inventing response codes:

| Extension | Meaning |
|---|---|
| `x-roundhouse-hidden` | The document omits at least one route |
| `x-roundhouse-merged` | A hand-written router was merged and is not documented |
| `x-roundhouse-public` | The entry is marked `public` |
| `x-roundhouse-validation: false` | The route opted out of constraint validation |
| `x-roundhouse-body-validation: false` | The route kept parts validation and opted the body out |
| `x-roundhouse-rejections: none` | Extractor rejection responses are not published |
| `x-roundhouse-unverified-body` | The body comes from an extractor whose schema could not be verified |
| `x-roundhouse-unverified-response` | A `Json`/`Vec` response shape could not be verified (for example a `ToSchema`-only type) |
| `x-roundhouse-unverified-extractor` | An argument's documentation could not be inferred |
| `x-roundhouse-overridden-response` | An `override_response(..)` declaration replaced an inferred shape |

## The document artifact

The document is OpenAPI 3.1. `Built::openapi_30()` produces a best-effort 3.0.3 shape by
rewriting `type: [T, "null"]`, numeric `exclusiveMinimum`/`exclusiveMaximum`, nullable
`oneOf`, `const` and `examples`. It covers the constructs this crate emits; it is not a general
3.1 → 3.0 converter.

Two tests keep the artifact honest:

- `tests/openapi.golden.json` is compared against the spine test's document. Regenerate it with
  `ROUNDHOUSE_BLESS=1 cargo test -p roundhouse --test spine openapi_golden` after an intentional
  change.
- `doc::validate_document` runs on every build, including after `App::document(..)` hooks, so a
  hook cannot publish a path that contradicts its own parameters.

## Constraints

- **Automatic body and response docs require `ApiSchema`.** `ToSchema` alone is an explicit
  declaration, not an inference.
- **No parallel extractor types.** The traits are implemented for axum's types; a custom
  extractor is an impl, not a conversion.
- **No second document model.** The document is utoipa's model behind a thin facade. roundhouse
  patches schemas and owns assembly, but does not define an alternative OpenAPI type.
- **Unrepresentable routes are omitted, not guessed.** `any` routes are served and excluded.
  Merged routers are served and marked. The document never claims to describe them.

## Future

- More extractors can gain `ExtractArg` impls upstream without a macro change; that is the point
  of the seam.
- The `Result<T, E>`, `Vec<u8>` and `impl Trait` special cases could shrink if Rust gained a
  way to express the needed bounds. Until then they stay in the macro, because the alternative
  is guessing at wire shapes.
- **Compile-time document fragments.** If utoipa ever exposes const-friendly schema builders,
  the operation skeleton could become a `const` and the runtime build would only merge
  configuration. The document model does not assume this; see
  [Traits and internal API](traits.md).
