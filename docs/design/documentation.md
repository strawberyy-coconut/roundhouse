# Documentation

The document and the router come from the same declarations. There is no side-car annotation
file and no second description to keep in sync: a `routes!` entry supplies the path, method and
policy; `#[handler]` derives the request, response and security contribution from the signature;
explicit `@summary:`, `@description:` and `@responses:` doc tags supply the prose and response
declarations the signature cannot express; the document builder turns those contributions into an
operation.

## From signature to operation

```text
#[handler] async fn show(Path(id): Path<NoteId>, Query(q): Query<Filter>) -> Result<Json<Note>, NoteError>
    │
    ├─ marker type + HandlerMeta (state, path names, required auth, inferred success)
    ├─ generated shim (extractors + ValidationContext + policy)
    └─ docs_with(components, validate) ──► HandlerDocs
            │
            ▼
    doc::build::build_with_observations
        ├─ merge layer docs and guards
        ├─ merge parameters, fill template captures
        ├─ merge inferred responses, then `@responses:` declarations
        ├─ build security requirements and schemes
        └─ validate_document
            │
            ▼
        OpenApi 3.1
```

`HandlerDocs` is a plain value; the document is built when the app is built, not when the
macro expands. The macro emits the code that fills `HandlerDocs`.

## `#[handler]` and `HandlerMeta`

The attribute leaves the function untouched and emits a hidden marker type beside it. The
marker implements `roundhouse::doc::HandlerMeta`:

| Item | Source |
|---|---|
| `State` / `HAS_STATE` | `State<T>` in the signature, or `()` |
| `PATH_NAMES` | `Path<T>` where `T: PathNames` |
| `REQUIRED_AUTH` | `Auth<A>` (not `Option<Auth<A>>`) |
| `INFERRED_SUCCESS` | A small return-type classification, or `None` when unknown |
| `DECLARED_SUCCESS` | Success statuses declared by `@responses:` |
| `docs(components)` | Doc tags, one statement per argument plus the response statements |
| `docs_with(components, validate)` | `docs()` plus the body-verification pass |

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

    fn document(components: &mut Components, docs: &mut HandlerDocs);
    fn document_optional(components: &mut Components, docs: &mut HandlerDocs) { .. }
    fn document_result(components: &mut Components, docs: &mut HandlerDocs) { .. }
    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> { Ok(()) }
}
```

- `document` is the normal case.
- `document_optional` handles `Option<Self>`: optional bodies, optional query parameters,
  optional auth (a security alternative rather than a requirement).
- `document_result` handles `Result<Self, E>`: the successful shape is documented, but the
  extractor's failure responses are not promised because the handler owns the rejection.
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
argument is documented as an extractor the framework cannot infer:
`docs.unverified_extractor = true`. The build emits a diagnostic, and `strict()` rejects it.
This is the same fallback that keeps a handler using any unregistered extractor compiling.

A body extractor that claims `SHIM_VALIDATES` clears the `docs.unverified` marker during
`docs_with`; an unverified body keeps it, so the document carries
`x-roundhouse-unverified-body`.

## Responses: `ApiResponse`

A return type documents itself through `ApiResponse`:

```rust
pub trait ApiResponse {
    const SUCCESS: &'static [u16] = &[200];
    fn responses(components: &mut Components) -> Vec<ResponseDoc>;
}
```

`SUCCESS` declares the success statuses the type can produce. The macro's compile-time check uses
its own syntactic classification (`HandlerMeta::INFERRED_SUCCESS`), because a return type need
not implement `ApiResponse`; the trait constant is the contract for a custom response type.
`&[]` means "known, but no success" (`Option<T>`, `Infallible`); `&[0]` means "status chosen at
runtime" and is documented as OpenAPI's `default` response.

The macro still special-cases three shapes:

- **`Result<T, E>`** — the success and error types are documented independently, so an
  undocumented `E` becomes a diagnostic (`undocumented_error`) rather than a missing success.
- **`Vec<u8>`** — bytes on the wire, not a JSON array of integers; documented as
  `application/octet-stream`.
- **`impl Trait`** — cannot be named in a turbofish. `Sse<impl Stream>` and `File` keep their
  fixed shapes; every other hidden type declares its responses in `routes!`.

Everything else goes through `ResponseProbe<T>`, which uses the same autoref trick: a type
implementing `ApiResponse` contributes its responses; a type that only implements
`IntoResponse` contributes none. A `Json`/`Vec` shape the probe cannot see is marked
unverified; any other unknown shape reaches the undeclared-success check.

`Result<_, Problem>` documents a `default` problem response. `ApiError`/`ApiResponse` derives
register the responses of an error enum, so `Result<Json<T>, NoteError>` documents both sides.
A `Result` type alias is visible through the same blanket `ApiResponse for Result<T, E>` impl as
long as both sides implement `ApiResponse`; the macro's compile-time success check remains
syntactic and is skipped for aliases, so the build-time check is authoritative.

Variant `@description:` tags supply response descriptions. `#[status(description = "…")]` remains
the explicit override. `ApiError` also uses the text as the default `Problem` title unless
`problem = "…"` overrides it.

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
- it exposes the type's `@description:` through `description()`, used for single-body responses;
- it is the bound automatic body and JSON response documentation requires.

A type that only derives `ToSchema` can still be documented explicitly:

- `@responses:` with `200: Type` on the handler accepts either trait;
- a `doc = |docs| ..` hook can set a body or response by hand;
- `roundhouse::doc::json_body_to_schema::<T>` builds a `BodyDoc` in that hook.

What it cannot do is be inferred from `Json<T>` automatically. `Json<T>: ApiResponse` requires
`T: ApiSchema`, so a plain `ToSchema` return type lands on the unverified fallback and `strict()`
rejects it. That is intentional: the document would otherwise promise the unpatched shape.

Component names follow utoipa. Two types with the same simple name need
`#[schema(as = module::Name)]`.

## Building the operation

`doc::build::build_with_observations` walks the resolved routes in order:

1. **Skip hidden routes.** A hidden route is served but omitted; the document is marked
   `x-roundhouse-hidden`.
2. **Omit `any`.** An `any` method set cannot be one operation; a diagnostic is recorded.
3. **Compute `HandlerDocs`.** `(record.docs)(components, record.validate)` runs the marker's
   statements (including `@summary`, `@description` and `@responses`), then the entry's `doc`
   hook runs.
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
9. **Apply `@responses:` declarations.** Declarations add statuses or refine inferred ones.
   Replacing a different shape without `@responses(override):` is
   `BuildError::ConflictingResponse`; with the override, the operation is marked
   `x-roundhouse-overridden-response`. A non-canonical description is preserved when the status
   and shape match.
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
| `x-roundhouse-rejections: none` | Extractor rejection responses are not published |
| `x-roundhouse-unverified-body` | The body comes from an extractor whose schema could not be verified |
| `x-roundhouse-unverified-response` | A `Json`/`Vec` response shape could not be verified (for example a `ToSchema`-only type) |
| `x-roundhouse-unverified-extractor` | An argument's documentation could not be inferred |
| `x-roundhouse-overridden-response` | An `@responses(override):` declaration replaced an inferred shape |

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
