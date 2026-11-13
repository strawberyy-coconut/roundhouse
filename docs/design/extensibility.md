# Extensibility

roundhouse exposes extension points as ordinary traits. The macros ask the concrete type what
it contributes; they do not maintain a closed list of supported extractors or responses. Adding
support for a custom type is one impl in the crate that owns it, not a macro change.

## Pick the seam

| Need | Use |
|---|---|
| A custom axum extractor with parameters, a body or auth | `ExtractArg` |
| A custom response type with a status, body or headers | `ApiResponse` |
| A documented JSON body or response type | `ApiSchema` (or `#[roundhouse::schema]`) |
| A query or path struct | `#[derive(QueryParams)]`, `#[derive(PathParams)]` |
| A tower layer that terminates requests | `App::layer_documented` or a `doc = ..` hook |
| Metadata the declarations cannot derive | `App::document(..)` |
| A one-off operation addition | `doc = |docs| ..` on the `routes!` entry |
| Operation prose | `@summary:` and `@description:` in the handler doc block |
| Response declarations | `#[handler(response(..))]` on the handler |

## Custom extractors: `ExtractArg`

A custom extractor is a normal axum extractor plus one impl:

```rust
use roundhouse::doc::{ArgDoc, DocContext, ExtractArg, Param, ParamIn, SchemaContext, schema};

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

impl ExtractArg for Tenant {
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
rejection type is already `Problem`, so no mapping is needed.

`document` returns `ArgDoc`; it does not mutate `OperationDoc`. The builder merges the contribution,
so a trait cannot read or overwrite another argument's work. `Option<T>` and `Result<T, E>` are
wrapper impls that adjust the returned value: optional parameters and bodies are relaxed into
their optional form, and a `Result` drops extractor failure responses because the handler owns
the rejection.

The trait has more hooks for the cases axum's built-ins need:

- `IS_BODY` — the argument consumes the request body. Set it when the extractor reads the body;
  it keeps the one-body rule and the validation marker honest.
- `SHIM_VALIDATES` — the extractor already executed the operation's schema, so the shim should
  not execute it again. `Strict` and the byte/body extractors set this.
- `validate` — post-extraction checks. The default is a no-op. Return a `Problem` to reject.

A body extractor that implements `validate` does not need `SHIM_VALIDATES`; that flag is only
for "the schema already ran during extraction".

### Foreign types

The orphan rule applies. If the extractor type is foreign and the trait is roundhouse's, wrap
it:

```rust
pub struct SignedBody<T>(pub T);
// ... FromRequest impl ...
impl<T: ExtractArg> ExtractArg for SignedBody<T> {
    fn document(context: &mut DocContext<'_>) -> ArgDoc {
        T::document(context)
    }
}
```

For an extractor you own, implement `ExtractArg` directly.

### Why a probe instead of a bound

`#[handler]` cannot write `where T: ExtractArg`, because custom extractors that do not
implement it must still compile. Instead it calls `ArgProbe::<T>::new().document(..)`, which
resolves through autoref specialization:

- if `T: ExtractArg`, the specific impl runs;
- otherwise the fallback returns an `ArgDoc` with `unverified_extractor` set.

The build emits a diagnostic, and the strict build rejects the operation. This is why forgetting an
impl is not a compile error but is a contract gap CI can catch. The same mechanism keeps any
unregistered extractor compiling.

## Custom responses: `ApiResponse`

A response type implements `IntoResponse` as usual, then `ApiResponse`:

```rust
use roundhouse::doc::{ApiResponse, ResponseDoc, schema};

pub struct Csv;

impl axum::response::IntoResponse for Csv {
    fn into_response(self) -> axum::response::Response {
        (
            axum::http::StatusCode::OK,
            [(axum::http::header::CONTENT_TYPE, "text/csv")],
            "id,name\n",
        )
            .into_response()
    }
}

impl ApiResponse for Csv {
    const SUCCESS: &'static [u16] = &[200];

    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc {
            status: 200,
            description: "OK".to_string(),
            content_type: Some("text/csv".to_string()),
            schema: Some(schema::string()),
            headers: Vec::new(),
        }]
    }
}
```

`const SUCCESS` declares the success statuses your type guarantees. Keep it accurate: it is the
trait-level contract for the type, even though the route macro's compile-time check uses its own
syntactic classification. Use `&[]` for a type that is known but never succeeds (like
`Option<T>`), and `&[0]` for a status chosen at runtime.

For an enum of responses, `#[derive(ApiResponse)]` / `#[derive(ApiError)]` generates both
`IntoResponse` and `ApiResponse` from `#[status(..)]`, `#[body]`, `#[header(..)]`,
`#[location]` and `#[from]` attributes. That is usually less code than a hand-written impl.

A type that only implements `IntoResponse` still compiles, but contributes no responses. The
build reports it as undocumented, and the strict build rejects it. Use `#[handler(response(..))]` if
you cannot change the type.

Generic enums are supported. The derive copies the enum's generics and `where` clause, then adds
the bounds each field shape needs. `Json<T>` needs `T: ApiSchema` for documentation and
`Json<T>: IntoResponse` at runtime; a bare body needs `T: ApiSchema + IntoResponse`; a header or
location needs `T: ApiSchema + TryInto<HeaderValue>`. The built-in `Created<T>` and `NoContent`
wrappers cover the common cases without a derive.

## Schemas: `ApiSchema`

For a documented body or JSON response, derive both:

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

A type that only implements utoipa's `ToSchema` is still usable explicitly through the interop
surface:

```rust
#[handler(response(200, Report))]
async fn report(...) -> ... { ... }
```

The response declaration accepts either trait. For a body, use a `doc` hook with the schema
helper:

```rust
routes! {
    "/import" => {
        post(import, doc = |docs| {
            docs.set_body(roundhouse::doc::json_body_to_schema::<Import>());
        }),
    },
}
```

That is more verbose, and the request will not be automatically validated unless the type also
implements `ApiSchema`. Explicit documentation and automatic enforcement are the same decision.

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
validation. `Params` carries the capture names, the parameter table, the parts schema, the JSON
value and the `validate_with` hooks.

Most applications never implement `Params` by hand; the derives are the supported path.

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

The layer still has to exist; `layer_documented` applies it and records the documentation in
one call. A `doc` hook without the layer would be a lie.

## One-off operation additions

`doc = |docs| { .. }` on a `routes!` entry runs after the handler's inferred documentation and
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

`App::document(..)` runs over the finished document after route assembly. It is the escape hatch
for metadata no declaration owns, such as tag descriptions or a custom extension:

```rust
app = app.document(|document| {
    document.tags = Some(vec![/* ... */]);
});
```

`App::server(..)` / `App::servers(..)` add `servers` entries. The validity checks run again
after the hook, so a hook cannot publish a path that contradicts its own parameters.

Hooks are metadata-only. The schema engine is compiled from the route declarations; changing a
request or response validation schema in a hook would publish a document the runtime does not
enforce. Use an `ApiSchema`/`ApiResponse` implementation or a `doc = |docs|` entry hook for schema
changes.

## Constraints

- **No parallel extractor types.** `roundhouse::Json` is `axum::Json`. The traits go on axum's
  types; a custom type is an impl, not a conversion layer.
- **No global registry.** `ExtractArg` and `ApiResponse` are resolved at the concrete type by
  the generated code. The document build owns a `DocContext` and its component ledger; there is
  no process-wide map to register into.
- **Forgetting a trait is not a compile error.** It is an unverified marker plus a diagnostic,
  promoted by the strict build. This keeps foreign types usable without forcing impls.
- **The document checks still apply to extensions.** A custom parameter, body or response is
  merged like any other, including capture and conflict validation.

## Future

- The autoref probes are a workaround for the lack of specialization/negative bounds. If Rust
  gains a way to express "documents itself, else fallback" directly, the probes can collapse;
  the public traits stay.
- Derive support for documenting a newtype wrapper without writing `ExtractArg` by hand is a
  possible convenience, not a committed API. The trait is the contract.
