# Extension points

Roundhouse exposes extension points as ordinary traits. The macros ask the concrete type what it
contributes; they do not maintain a closed list of supported extractors or responses. Adding
support for a custom type is one impl in the crate that owns it, not a macro change.

## Pick the seam

| Need | Use |
|---|---|
| A custom axum extractor with parameters, a body or auth | `ExtractArg` + `ExtractDoc` |
| A custom success type with a status, body or headers | `DocumentedResponse` (and `#[handler(response(..))]` for a status override) |
| A custom error type with statuses, bodies or headers | `DocumentedError` (or return `Problem`) |
| A documented JSON body or response type | `ApiSchema` (or `#[roundhouse::schema]`) |
| A query or path struct | `#[derive(QueryParams)]`, `#[derive(PathParams)]` |
| A tower layer that terminates requests | `App::layer_documented` or a `doc = ..` hook |
| Metadata the declarations cannot derive | `Contract::document(..)` or `App::document(..)` (contract build) |
| A one-off operation addition | `doc = |docs| ..` on the `routes!` entry |
| Operation prose | `@summary:` and `@description:` in the handler doc block |
| Response declarations | `#[handler(response(..))]` on the handler |

## The public surface

| Trait | Subject | Implemented by | Used by | Phase |
|---|---|---|---|---|
| `ApiSchema` | A body/response JSON shape and its hooks | `#[roundhouse::schema]`, manual impls, primitives | contract build | contract |
| `DocumentedResponse` | A success type's statuses, content types, bodies and headers | built-ins, `Status<CODE, T>`, `Empty<CODE>`, generated status wrappers, custom response types | contract build | contract |
| `DocumentedError` | An error type's statuses, content types, bodies and headers | `Problem`, `Status<CODE, T>`, `Empty<CODE>`, generated status wrappers, custom error types | contract build | contract |
| `DocumentedIntoResponse` | Marks and erases a documented value at an opaque return site | blanket for `IntoResponse` (plus `DocumentedResponse` under `build`) | handler body analysis | compile |
| `Params` | A path/query/form type's runtime value | `PathParams`/`QueryParams` derives, scalars | request validation | request |
| `ValidateHook` | Custom validation attached to a value | schema derives | runtime validation | request |
| `ParamsDoc` | Parameter table and parts schema | `PathParams`/`QueryParams` derives | contract build | contract |
| `ExtractArg` | One handler argument's runtime validation | extractor author | handler shim | request |
| `ExtractDoc` | One handler argument's documentation | extractor author | contract build | contract |
| `HandlerSpec` | A handler's static facts | `#[handler]` | `routes!` | compile |
| `DocSource` | An operation's documentation | `#[handler]` | contract build | contract |
| `AuthScheme` | Scheme metadata | app author | macro, builder, guards | compile/build |
| `Authenticate` | Credential check | app author | `Auth<A>` extractor | request |
| `Scopes<A>` | Scope names and authorization | app author | `Auth<A, S>` | request |
| `AttachmentType` | An attachment body's content type | framework and custom attachment types | response documentation | contract |
| `MultipartForm` | Multipart field table and extraction | `Multipart` derive | extractor | request |
| `MultipartDoc` | Multipart body schema | `Multipart` derive | contract build | contract |
| `Rejection` | Extractor rejection → `Problem` | blanket for `IntoResponse` | shim | request |

### `ApiSchema`

```rust
pub trait ApiSchema {
    fn schema() -> RefOr<Schema>;
    fn description() -> Option<&'static str> { None }
    fn register(_context: &mut dyn SchemaContext) {}
}
```

`ApiSchema` is the adapter between a Rust type and the OpenAPI component table. It also carries
the type's description, which is the default prose for a JSON response. With `build`, the
schema derive supplies the shape; without it, the derive compiles only the runtime
`ValidateHook` impl and no utoipa paths are resolved. `roundhouse::doc::ApiSchema` is re-exported
only under `build`; the trait itself lives in `roundhouse-schema`.

`SchemaContext` lives in `roundhouse-schema`, so the schema crate stays axum-free. The facade
implements it for `DocContext`, which owns the component table and the collision ledger.

### `DocumentedResponse`

```rust
pub trait DocumentedResponse {
    fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}
```

This is the content contract for a success response. A built-in is special only because it
implements the trait; a custom response type implements it once and is documented the same way.
`Status<CODE, T>` and `Empty<CODE>` document an exact status (`Status` with a JSON body, `Empty`
without one); the named wrappers in `roundhouse::status` are generated from the IANA catalog and
follow the same rules. A custom impl depends on documentation-only types, so mark it with
`#[roundhouse::build_only]`: a production build that compiles only the `contract` feature drops the
item instead of failing to resolve the trait.
For a named return type the macro probes the declared type: an impl publishes its responses, and
a missing impl leaves the operation without a success for the strict build to reject. For an
opaque return type the build-time analyzer selects the site and the macro emits a typed call to
this trait. There is no name-based fallback.

### `DocumentedError`

```rust
pub trait DocumentedError {
    fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc>;
}
```

The failure counterpart. `Result<T, E>` requires `T: DocumentedResponse` and `E:
DocumentedError`. `Problem` is one implementor; a custom enum can list every status it can
produce. `Status<CODE, T>`, `Empty<CODE>` and the generated `roundhouse::status` wrappers
implement both traits, so they can appear on either side of `Result`. `?` documents `E`, not the
error it converted, so return the specific error type when its statuses matter. The trait is
build-only, like `DocumentedResponse`: custom impls belong
to the documentation build, and the contract harness compiles the application with the `build` feature
unified.

### `DocumentedIntoResponse`

```rust
pub trait DocumentedIntoResponse: IntoResponse + DocumentedResponse {
    fn into_documented_response(self) -> Documented {
        Documented(self.into_response())
    }
}
```

A marker for an opaque return site. It erases the receiver to `Documented`, so branches with
different documented types can share one `impl IntoResponse` return type. The method does not
change the response; it only erases it. The build-time analyzer records the receiver's type at
each `.into_documented_response()` call site, so the erased value itself needs no static response
shape. Under a non-`build` build the documentation supertrait is compiled out and the method
remains available, so handler source does not need `cfg`. `.into_response()` without the marker is
also transparent to a statically documented receiver; only a receiver that is itself erased stays
opaque.

### `Params`, `ParamsDoc`, `ValidateHook`

```rust
pub trait Params {
    const NAMES: &'static [&'static str] = &[];
    fn parts_value(&self) -> serde_json::Value;
}

#[cfg(feature = "contract")]
pub trait ParamsDoc: Params {
    fn register(_context: &mut dyn SchemaContext) {}
    fn params(context: &mut dyn SchemaContext) -> Vec<Param>;
    fn scalar_schema() -> Option<RefOr<Schema>> { None }
    fn parts_schema(context: &mut dyn SchemaContext) -> RefOr<Schema>;
    fn parts_doc() -> Option<PartsDoc> { None }
}
```

The runtime only needs capture names and the JSON value the contract engine validates. Document
metadata is the build-only half. `Path<T>`, `Query<T>` and `RepeatedQuery<T>` bound on
`Params`; the contract build adds `ParamsDoc` when it documents the operation.

`ValidateHook` is separate because `validate_with` hooks must run in a production build where the
schema derive is compiled out.

### `ExtractArg` and `ExtractDoc`

```rust
pub trait ExtractArg {
    const IS_BODY: bool = false;
    const SHIM_VALIDATES: bool = false;

    fn validate(&self, context: &ValidationContext) -> Result<(), Problem> {
        Ok(())
    }
}

#[cfg(feature = "contract")]
pub trait ExtractDoc: ExtractArg {
    fn document(context: &mut DocContext<'_>) -> ArgDoc;
}
```

Runtime extraction is axum's job. `ExtractArg` adds post-extraction validation and two flags the
shim needs:

- `IS_BODY` — the argument consumes the request body. It keeps the one-body rule and the
  validation marker honest.
- `SHIM_VALIDATES` — the shim's post-extraction validation counts as the operation's body
  verification. The contract build clears the unverified-body marker when the route validates and
  this is set.
- `validate` — post-extraction checks. The default is a no-op; return a `Problem` to reject.

`ExtractDoc` contributes the argument's `ArgDoc` to the contract build. A custom extractor
implements both; without `ExtractDoc`, the operation is marked unverified and the strict build
rejects it unless acknowledged.

`Option<T>` and `Result<T, E>` wrap the docs: `Option` relaxes a parameter, body or required auth;
`Result` drops extractor failure responses because the handler owns the rejection.

### `HandlerSpec` and `DocSource`

Both are generated by `#[handler]`; applications do not implement them.

```rust
pub trait HandlerSpec: Send + Sync + 'static {
    type State: 'static;
    const HAS_STATE: bool;
    const PATH_NAMES: &'static [&'static str] = &[];
    const REQUIRED_AUTH: &'static [&'static str] = &[];
}

#[cfg(feature = "contract")]
pub trait DocSource {
    fn operation(context: &mut DocContext<'_>) -> OperationDoc;
}
```

`routes!` uses `HandlerSpec` to install the shim and check state, captures and auth. The contract
build uses `DocSource` to produce the operation. Route policy is applied by the builder to the
returned `OperationDoc`; a handler describes what it is, and the route decides how it is enforced.

Without `build`, `DocSource` exists as a no-op trait so generated route records keep the same
shape while every documentation body is compiled out.

### Auth

`AuthScheme` is metadata (`NAME`, `KIND`, rejection statuses), `Authenticate` performs the
credential check, and `Scopes<A>` publishes and enforces a scope. They are separate because the
scheme can be documented without a request and the state needed for authentication should not
leak into the document model.

`NoScopes` is the only built-in `Scopes` implementation. A scoped route must implement
`Scopes<A>` explicitly, so publishing a scope always has an authorization decision behind it.

### Narrow contracts

- `AttachmentType` chooses the content type for `Attachment<T>`.
- `MultipartForm` is the derive-generated field table; `MultipartDoc` is its build-only schema.
- `Rejection` is a blanket adapter from an extractor's `IntoResponse` rejection to `Problem`.

## Custom extractors

A custom extractor is a normal axum extractor plus a runtime impl and, under `build`, a
documentation impl:

```rust
use roundhouse::doc::{ArgDoc, DocContext, ExtractDoc, Param, ParamIn, schema};

pub struct Tenant(pub String);

impl<S> axum::extract::FromRequestParts<S> for Tenant
where
    S: Send + Sync,
{
    type Rejection = roundhouse::Problem;

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &S,
    ) -> Result<Self, Self::Rejection> {
        let tenant = parts
            .headers
            .get("x-tenant")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("anonymous");
        Ok(Self(tenant.to_string()))
    }
}

impl roundhouse::ExtractArg for Tenant {}

#[cfg(feature = "contract")]
impl ExtractDoc for Tenant {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            params: vec![
                Param::new("x-tenant", ParamIn::Header, schema::string())
                    .description("tenant id"),
            ],
            ..ArgDoc::default()
        }
    }
}
```

`#[handler]` now documents the header and the document builder accepts it. The extractor's
rejection type is already `Problem`, so no mapping is needed. A production build compiles only
`ExtractArg`; the documentation impl is behind `build`.

`document` returns `ArgDoc`; it does not mutate `OperationDoc`. The builder merges the
contribution, so a trait cannot read or overwrite another argument's work.

### Foreign types

The orphan rule applies. If the extractor type is foreign and the trait is roundhouse's, wrap it:

```rust
pub struct SignedBody<T>(pub T);
// ... FromRequest impl ...

impl<T: roundhouse::ExtractArg> roundhouse::ExtractArg for SignedBody<T> {}

#[cfg(feature = "contract")]
impl<T: ExtractDoc> ExtractDoc for SignedBody<T> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        T::document(context)
    }
}
```

For an extractor you own, implement `ExtractArg` and, under `build`, `ExtractDoc`.

### Why a probe instead of a bound

`#[handler]` cannot write `where T: ExtractDoc`, because custom extractors that do not implement
it must still compile. Instead it calls `ArgDocProbe::<T>::new().document(..)`, which resolves
through autoref specialization:

- if `T: ExtractDoc`, the specific impl runs;
- otherwise the fallback returns an `ArgDoc` with `unverified_extractor` set.

The build emits a diagnostic, and the strict build rejects the operation. This is why forgetting
an impl is not a compile error but is a contract gap CI can catch. The same mechanism keeps any
unregistered extractor compiling.

## Custom responses

For a status that is known at compile time, prefer `Status<CODE, T>` (JSON body) or `Empty<CODE>`
(no body); both document the exact code. The named wrappers in `roundhouse::status` (`Created`,
`NotFound`, …) are generated from the IANA registry. A custom response type is for a different
content type, headers, or a shape the built-ins do not express.

A response type implements `IntoResponse` to serve and `DocumentedResponse` to document. The
documentation impl is marked `build_only` so a lean production build compiles it out:

```rust
use roundhouse::doc::{DocumentedResponse, ResponseDoc, SchemaContext, schema};

pub struct Csv;

impl axum::response::IntoResponse for Csv {
    fn into_response(self) -> axum::response::Response {
        (
            [(axum::http::header::CONTENT_TYPE, "text/csv")],
            "id,name\n",
        )
            .into_response()
    }
}

impl DocumentedResponse for Csv {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc {
            status: 200,
            description: "CSV export".to_string(),
            content_type: Some("text/csv".to_string()),
            schema: Some(schema::string()),
            headers: Vec::new(),
        }]
    }
}
```

The impl owns the status, content type, body schema and headers, exactly like `Json<T>` or
`Problem`. A handler that returns the type documents it automatically:

```rust
#[handler]
pub async fn csv() -> Csv {
    Csv
}
```

An error type implements the same shape under `DocumentedError`, and a handler returning
`Result<T, E>` documents both sides:

```rust
#[roundhouse::build_only]
impl DocumentedError for NoteMissing {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::problem(404, "Note not found")]
    }
}

#[handler]
pub async fn show(...) -> Result<Json<Note>, NoteMissing> { ... }
```

An `impl IntoResponse` handler is documented from its body. `.into_response()` keeps the
receiver's documented type; `.into_documented_response()` additionally erases the value so
branches with different documented types can unify. A receiver that is itself erased (`Response`,
a trait object, a helper returning `Response`) is `x-roundhouse-unverified-response`.

A declaration overrides only the status while keeping the type's body:

```rust
#[handler(response(206, Csv))]
pub async fn partial_csv() -> Csv {
    Csv
}
```

`doc = |docs| ..` remains available for metadata the type cannot express, such as an operation
summary or an extra parameter. There is no hidden response trait behind the public one.

## Schemas

For a documented body or JSON response, use the one-annotation form:

```rust
#[roundhouse::schema]
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Signup {
    #[roundhouse(min_length = 3, max_length = 32)]
    pub user_name: String,
    #[roundhouse(unique_items)]
    pub roles: Vec<String>,
}
```

The schema engine supplies the shape; `ApiSchema` registers it as a named component, applies the
roundhouse patches and carries `validate_with` hooks. Automatic `Json<T>` / `Form<T>`
documentation requires it.

A type that only implements utoipa's `ToSchema` can be adapted explicitly:

```rust
roundhouse::interop::impl_from_utoipa!(generated::Row);
```

It then implements `ApiSchema` and is used like any other documented body. An explicit
`DocumentedResponse` impl is the alternative for a response that cannot be adapted. A type used
only in a request body needs `ApiSchema` for automatic documentation; a `doc` hook can attach a
hand-written `BodyDoc`, but it does not register the type's component or make the request
validated. Explicit documentation and automatic enforcement are the same decision.

## Query and path types

`#[derive(QueryParams)]` and `#[derive(PathParams)]` generate the parameter table, the parts
schema and the value conversion used by the engine. A field's `@description:` supplies the
parameter description; `#[roundhouse(description = "...")]` takes priority:

```rust
#[derive(serde::Deserialize, roundhouse::QueryParams)]
pub struct Filter {
    /// @description: Free-text search.
    #[roundhouse(min_length = 1)]
    pub search: Option<String>,
    pub tags: Vec<String>,
}

#[derive(serde::Deserialize, roundhouse::PathParams)]
pub struct NoteId {
    pub id: u64,
}
```

`PathParams` and `QueryParams` both implement `Params`, which is what lets `routes!` check
captures at compile time and what the document builder uses for parameters, schemas and runtime
validation. Most applications never implement `Params` by hand; the derives are the supported
path.

## Layers

A tower layer that terminates requests can make a promise the handler signature cannot express.
Document it either globally:

```rust
app = app.layer_documented(rate_limit_layer, || {
    roundhouse::doc::layer::rate_limit(429)
});
```

or on a module or entry:

```rust
routes! {
    doc = roundhouse::doc::layer::timeout(504),
    "/slow" => {
        get(slow),
    },
}
```

`LayerDoc` carries responses, parameters, a body, required/optional auth and schemes. The
document builder merges them into every operation below the declaration. The built-in helpers
cover the common cases: `rate_limit`, `timeout`, `body_limit` and `auth::<Scheme>`.

The layer still has to exist; `layer_documented` applies it and records the documentation in one
call. A `doc` hook without the layer would be a lie.

## One-off operation additions

`doc = |docs| { .. }` on a `routes!` entry runs after the handler's documented responses and
before the document checks. It can set summary/description, add parameters, a body or responses:

```rust
routes! {
    "/legacy" => {
        get(legacy, doc = |docs| {
            docs.push_param(
                roundhouse::doc::Param::new(
                    "x-api-version",
                    roundhouse::doc::ParamIn::Header,
                    roundhouse::doc::schema::string(),
                )
                .description("legacy clients only"),
            );
        }),
    },
}
```

Additions still go through deduplication, conflict detection and document validity. The hook
cannot bypass the checks; it can only supply data the signature could not.

## Document-level hooks

`Contract::document(..)` (or `App::document(..)` in a contract build) runs over the finished
document before it is written. It is the escape hatch for metadata no declaration owns, such as
tag descriptions or a custom extension:

```rust
app = app.document(|document| {
    document.tags = Some(vec![/* ... */]);
});
```

`api! servers = [..]` or the build-only `App::server(..)`/`App::servers(..)` add `servers`
entries. The validity checks run again after the hook, so a hook cannot publish a path that
contradicts its own parameters.

Hooks are metadata-only. The schema engine is compiled from the route declarations; changing a
request or response validation schema in a hook would publish a document the runtime does not
enforce. Use an `ApiSchema` implementation or a `doc = |docs|` entry hook for schema changes.

## Internal machinery

These traits are `#[doc(hidden)]` or live in `__private`. They are not extension points:

- `ProbeResponses` discovers whether a declared return type implements `DocumentedResponse`, so a
  handler whose body type publishes no schema still compiles.
- The handler index (`target/roundhouse/handlers.json`) is written by the `roundhouse-build`
  analyzer. `#[handler]` reads it only when `ROUNDHOUSE_HANDLER_INDEX` is set and emits typed
  trait calls; it is not a public extension point.
- `ProbeArg`, `ProbeSchema` and the schema probes implement autoref specialization, because the
  macro cannot ask "does this unseen type implement X?" with a normal bound.
- `ShimPolicy`, `ShimPolicyFor` and `SubstateOf` are the state/auth plumbing behind the generated
  route shim.
- `HandlerMeta` is the combined `HandlerSpec + DocSource` bound.

The autoref pattern is:

```rust
impl<T: Trait> Probe<T> for &ProbeType<T> { /* real */ }
impl<T>       Probe<T> for &&ProbeType<T> { /* fallback */ }
```

Rust prefers the more specific receiver when the bound holds. That keeps an unregistered extractor
or response type compiling; the strict build then reports the missing documentation instead of
the macro guessing.

## Why the split is worth it

The split follows the two phases of the application:

- **Request time** needs extraction, `Params`, `ValidateHook` and the shim. It must not carry
  utoipa, the document builder or response probes.
- **Contract time** needs `ApiSchema`, `ExtractDoc`, `ParamsDoc`, `DocSource` and the document
  builder. It runs in the `target/roundhouse` harness, not in production.

The result is one runtime schema model, one contract builder and no duplicated engine. The cost is
a set of build-only traits and probes, all hidden from normal application code.

## Constraints

- **No parallel extractor types.** `roundhouse::Json` is `axum::Json`. The traits go on axum's
  types; a custom type is an impl, not a conversion layer.
- **No global registry.** `ExtractArg` and the built-in response docs are resolved at the concrete
  type by the generated code. The document build owns a `DocContext` and its component ledger;
  there is no process-wide map to register into.
- **Forgetting a trait is not a compile error.** It is an unverified marker plus a diagnostic,
  promoted by the strict build. This keeps foreign types usable without forcing impls.
- **The document checks still apply to extensions.** A custom parameter, body or response is
  merged like any other, including capture and conflict validation.
