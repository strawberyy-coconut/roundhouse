# Migrating to roundhouse 0.3

0.3 makes the authoring surface honest by default and removes the 0.2 compatibility spellings.
Read this section first; the rest of the guide documents the route-table changes that shipped
with the same release.

## Breaking changes at a glance

### Strict builds are the default

`App::try_build()` now returns `BuildError` for every build diagnostic, not only when
`App::strict()` is called. A route with an unverified body, response or extractor, an
undocumented `any` route, an empty module or an unacknowledged `App::merge` fails the build.

- Acknowledge one route with `unverified = "reason"`.
- Use `validate_body = false` for a body that should be parsed but not schema-checked.
- Use `App::permissive()` to downgrade the whole app to warnings while migrating.
- Replace `App::merge(router)` with `App::merge_undocumented(router, "reason")`.
- `any` routes must be `hidden` or carry an `unverified` acknowledgement.

### `public` is gone

`public` only added an `x-roundhouse-public` extension and never changed authentication, which
made it easy to confuse with `anonymous`. Use `anonymous` when the route clears inherited
security. If the extension mattered, add it with a `doc` hook.

### Legacy doc tags are gone

`@responses:`, `@responses(override):`, `@response-descriptions:` and `@path:` no longer parse.
Use the typed handler attribute:

```rust
// before
/// @responses:
///   201: Note
/// @path: id: The note id.
#[handler]
async fn create(...) { ... }

// after
#[handler(
    response(201, Note),
    path(id, "The note id."),
)]
async fn create(...) { ... }
```

### utoipa moved behind `roundhouse::interop`

The normal authoring path no longer mentions utoipa:

```rust
use roundhouse::prelude::*;

#[roundhouse::schema]
#[derive(serde::Serialize)]
pub struct Note {
    #[roundhouse(min_length = 1, max_length = 200)]
    pub title: String,
}
```

- `#[roundhouse::schema]` derives the schema and the roundhouse adapter; do not also derive
  `ToSchema` or `ApiSchema`.
- Constraints use `#[roundhouse(..)]`. `#[schema(..)]` remains accepted for utoipa interop.
- Component names use `#[roundhouse(name = "module.Name")]`, not `#[schema(as = ..)]`.
- `ToSchema`, `PartialSchema`, `Components` and the `utoipa` crate live in
  `roundhouse::interop`; import them only when sharing types with another utoipa project.
- `impl_api_schema!` is now `roundhouse::interop::impl_from_utoipa!`.
- `Info` and `OpenApi` no longer `Deref` to their utoipa values. Use `as_utoipa()` or
  `into_utoipa()` explicitly.

### Prefix groups are explicit

`"/path" [options] { .. }` is no longer a prefix group. Use:

```rust
routes! {
    prefix = "/api",

    "/notes" => { get(list), post(create) },       // path group
    prefix "/admin" [tags = ["admin"]] {           // nested prefix group
        "/audit" => { get(audit) },
    },
}
```

### Removed APIs

- `App::router()` — use `App::into_router()` or `Built::router()`.
- `HandlerDocs` — use `OperationDoc`.
- `App::merge()` for an undocumented router — use `App::merge_undocumented()`.
- `app_state!` outside the crate root is now a compile error at the call site.

### `cfg` duplicates

Table-local duplicate checks skip `#[cfg(..)]`-gated entries. A collision that survives `cfg`
is still a build error in the resolved route table.


## Route table shape

Every entry now lives inside a path group:

```rust
// 0.2
routes! {
    get("/notes", list),
    post("/notes", create),
    get("/notes/{id}", show),
    delete("/notes/{id}", destroy),
}

// 0.3
routes! {
    "/notes" => {
        get(list),
        post(create),
    },
    "/notes/{id}" => {
        get(show),
        delete(destroy),
    },
}
```

The flat `get("/path", handler)` and compact `get "/path" => handler` forms are compile errors.

## Operation prose

Replace `summary = "..."` and `description = "..."` on the entry with tags on the handler:

```rust
/// @summary: Create a note.
/// @description: Persists the note and returns it.
#[handler]
pub async fn create(...) -> ... { ... }
```

Untagged doc comments are ignored for OpenAPI. For a per-mount override, use a `doc` hook:

```rust
"/admin/notes" => {
    post(create, doc = |docs| {
        docs.set_summary("Create a note as an administrator");
    }),
},
```

## `doc` hooks use builder methods

`OperationDoc`'s fields are private in 0.3. A `doc` hook mutates through named methods, and
`ResolvedRoute::docs()` reads through accessors:

```rust
post(create, doc = |docs| {
    docs.push_param(Param::new("x-tenant", ParamIn::Header, schema::string()));
})
```

The method families are `set_*`/`push_*`/`extend_*` for mutation and plain getters
(`summary()`, `params()`, `body()`, `responses()`, …) for inspection. `response_mut(status)`
adjusts one inferred response in place.

## Responses

Replace `responses(...)` and `override` on the entry with typed metadata on the handler:

```rust
// 0.2
post("/notes", create, responses(201 = Note)),
delete("/notes/{id}", destroy, responses(204)),
get("/x", x, responses(200 = Payload), override),

// 0.3
#[handler(response(201, Note))]
async fn create(...) -> ... { ... }

#[handler(response(204))]
async fn destroy(...) -> ... { ... }

#[handler(override_response(200, Payload))]
async fn x(...) -> ... { ... }
```

`Json<T>` is accepted in a declaration and unwrapped to `T` for the schema comparison. The older
`@responses:` and `@responses(override):` doc tags were removed.

## Response descriptions

`ApiResponse`/`ApiError` variants can use a doc comment:

```rust
#[derive(ApiResponse)]
pub enum NoteResponse {
    /// @description: The note was created.
    #[status(201)]
    Created(Json<Note>),
}
```

`#[status(description = "...")]` remains the explicit override. A body type's `@description:`
supplies the description when the type is a single JSON response body.

## Application state

Declare the application state once at the crate root instead of repeating `state = ...` on every
table:

```rust
roundhouse::app_state!(AppState);
```

Every `routes!` table without `state = ...` uses that type, including spread modules. A table that
intentionally uses a different state still writes `state = OtherState`; `state = infer` restores
the old per-handler inference.

## Unauthenticated routes

A route inside a guarded module can opt out of the inherited schemes:

```rust
"/health" => { get(health, anonymous) },
```

`anonymous` clears inherited guards and documents `security: []`. A handler's own `Auth<A>` still
applies. A group can use the same flag to make a whole subtree anonymous.

## Response fields

Every field in an `ApiResponse`/`ApiError` variant must say where it goes: `#[body]`,
`#[header("name")]`, `#[location]`, `#[from]`, `#[source]`, or `#[response(ignore)]`. An
unannotated field is now a compile error instead of being silently dropped. Use a `Problem` body
for a variant that returns a problem document:

```rust
#[status(404, description = "The note does not exist.")]
NotFound(Problem),
```

## Parameter descriptions

Query and path struct fields can use `@description:`:

```rust
#[derive(QueryParams)]
pub struct Filter {
    /// @description: Free-text search.
    pub q: Option<String>,
}
```

`#[roundhouse(description = "...")]` takes priority.

## Doc-tag sections

`@summary:` ends at a blank line. Previously any following untagged rustdoc prose became part of
the summary. `@description:` still keeps blank lines as paragraphs.

Doc comments are for prose. Structured response and path metadata lives on the handler attribute:

```rust
#[handler(
    response(201, Note),
    response_description(201, "The stored note."),
    path(id, "The note identifier."),
)]
async fn create(...) -> ... { ... }
```

The older `@responses:`, `@response-descriptions:` and `@path:` tags were removed in this release;
use the typed form.

## Components

Two different Rust types that register the same component name are now a
`BuildError::ComponentCollision`. Disambiguate with
`#[roundhouse(name = "module.Name")]`; the document uses the dotted form (`module.Name`).

Generated `ToSchema`-only types can opt into the schema adapter through the interop surface:

```rust
roundhouse::interop::impl_from_utoipa!(generated::Row);
```

## Traits and extractors

`AuthScopes` and `Authorize<A>` are one trait now. The scope names and the decision live together:

```rust
// 0.2
impl AuthScopes for ReadPosts {
    const SCOPES: &'static [&'static str] = &["read:posts"];
}
impl Authorize<BearerAuth> for ReadPosts {
    fn authorize(user: &User) -> Result<(), Problem> { Ok(()) }
}

// 0.3
impl Scopes<BearerAuth> for ReadPosts {
    const NAMES: &'static [&'static str] = &["read:posts"];
    fn authorize(user: &User) -> Result<(), Problem> { Ok(()) }
}
```

`ExtractArg::document` now returns an `ArgDoc` instead of mutating `HandlerDocs`. The
`document_optional` and `document_result` hooks are gone; `Option<T>` and `Result<T, E>` are
wrapper impls that adjust the returned value.

```rust
// 0.2
impl ExtractArg for Tenant {
    fn document(_components: &mut Components, docs: &mut HandlerDocs) {
        docs.params.push(Param::new("x-tenant", ParamIn::Header, schema::string()));
    }
}

// 0.3
impl ExtractArg for Tenant {
    fn document(_context: &mut DocContext<'_>) -> ArgDoc {
        ArgDoc {
            params: vec![Param::new("x-tenant", ParamIn::Header, schema::string())],
            ..ArgDoc::default()
        }
    }
}
```

`Params` is the single bound for path, query and form types. `PathNames`, `PathValidate`,
`PartsSchema`, `PartsValue` and the `QueryParams` trait are gone; `Path<T>`, `Query<T>` and
`RepeatedQuery<T>` require `Params`, and the `PathParams` and `QueryParams` derives emit it.

Macro probes and the shim policy traits moved to `roundhouse::__private`. They were never a stable
extension point; use the domain traits (`ExtractArg`, `ApiSchema`, `ApiResponse`, `Scopes`) instead.

`ApiSchema::register`, `ApiResponse::responses`, `Params` and `MultipartForm::schema` now take a
`SchemaContext` (`DocContext` in the facade) instead of `&mut Components`. The component-collision
ledger is part of the context, not a thread-local, so registration is explicit and reentrant.

`HandlerMeta` is split into `HandlerSpec` (state, captures, required auth and success statuses)
and `DocSource` (the operation document). The old `HandlerMeta` remains as a combined marker.
`HandlerDocs` is now `OperationDoc`; the old name was removed. The `doc = |docs|` hook still
receives `OperationDoc` through its builder methods.

## Routers

`App::router()` was removed. Use:

```rust
let router = app.into_router(); // axum::Router<()>
```

Application state is already applied; do not call `with_state` again. `Built::router()` is
unchanged.

## State and `FromRef`

The app state is still exact at `App::new`, but a `routes!` table can now name it and handlers can
take substates:

```rust
routes! {
    state = AppState,
    prefix = "/notes",
    "/" => { get(list) },
}

#[handler]
async fn list(State(db): State<Db>) -> Json<Vec<Note>> { ... }
```

`Db` must implement `FromRef<AppState>`; `State<AppState>` itself still works. Inline groups
inherit the table state. A spread module is a separate `routes!` expansion and declares its own
state when it uses substates. `AuthScheme::State` must still equal the table state.

## Return-type inference

A handler with no return type documents `200 OK`, matching the response axum actually sends
(`()` is `200`). It used to document `204 No Content`.

A known return shape whose body type publishes no schema is now documented with its bare status
and `x-roundhouse-unverified-response` instead of leaving the response out and failing the build.
This applies to `Json`/`Vec` (as before) and now also to `Html`, `Created` and the other known
shapes. The strict default rejects the marker unless the entry acknowledges it with
`unverified = "reason"`.

Tuples other than `(StatusCode, T)` are no longer classified from their last element. They are
treated as unknown, so they need an explicit `#[handler(response(..))]` declaration.

## Generic responses

`ApiResponse`/`ApiError` now support generic enums. `Created<T>` and `NoContent` are available for
the common created/no-content cases. `Result` aliases are documented through the existing blanket
`ApiResponse for Result<T, E>` impl when both sides implement `ApiResponse`. A bare
`response(200)` now asserts the success while keeping an inferred body shape, so an alias does
not need to repeat its type.

## Documentation UI

`App::docs()` now serves the vendored Swagger UI assets by default (the `swagger` feature is part
of the default feature set), so the page works without browser network access. Call
`App::docs_cdn()` to opt back into the pinned unpkg assets. `App::swagger()` remains available for
a second local UI at `/docs/swagger`.

## Authorization and body limits

`AuthScheme::Identity` now requires `Clone`. A module guard caches the authenticated identity so a
later `Auth<A>` or `Option<Auth<A>>` extractor on the same request reuses it instead of
authenticating twice.

`Scopes::authorize` is now required. Publishing a scope with `NAMES` no longer silently grants
every authenticated identity; implement `authorize`, or use `NoScopes` when authentication is the
only check.

The default request-body limit is 2 MiB for every extractor, including multipart. Use
`App::max_body` to raise it for uploads; a field's `#[file(max_bytes = ..)]` can only narrow it.

`ConnectInfo<T>` is now installed by `App::serve` and `App::serve_with`.

## Response headers and document hooks

`HeaderDoc` no longer carries `required` or `append`; the utoipa header model this crate uses
cannot represent them, so they are not silently ignored API. `Attachment` always emits
`Content-Disposition: attachment` (with a filename when supplied).

`App::document(..)` hooks are for metadata such as tags, servers and extensions. They must not
change request/response validation schemas; the schema engine is compiled from the route
declarations.

## New opt-in route policies

These are additions, not required migrations:

- `validate_body = false` keeps path/query validation and treats the request body as opaque.
- `unverified = "reason"` acknowledges an intentional unverified body/response/extractor.
- `prose = include_str!(..)` supplies operation summary and description from one string.
- `response_description(..)` overrides the prose of an individual response.
- `#[roundhouse::schema]` derives both `ToSchema` and `ApiSchema` with one annotation.
