# Migrating to roundhouse 0.3

0.3 moves operation prose and response declarations onto handler doc tags and removes the
equivalent `routes!` options. It also adds the ready `App::into_router()`.

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
        docs.summary = Some("Create a note as an administrator".to_string());
    }),
},
```

## Responses

Replace `responses(...)` and `override` on the entry with `@responses:` on the handler:

```rust
// 0.2
post("/notes", create, responses(201 = Note)),
delete("/notes/{id}", destroy, responses(204)),
get("/x", x, responses(200 = Payload), override),

// 0.3
/// @responses:
///   201: Note
#[handler]
async fn create(...) -> ... { ... }

/// @responses:
///   204
#[handler]
async fn destroy(...) -> ... { ... }

/// @responses(override):
///   200: Payload
#[handler]
async fn x(...) -> ... { ... }
```

`Json<T>` is accepted in a declaration and unwrapped to `T` for the schema comparison.

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

## Parameter descriptions

Query and path struct fields can use `@description:`:

```rust
#[derive(QueryParams)]
pub struct Filter {
    /// @description: Free-text search.
    pub q: Option<String>,
}
```

`#[schema(description = "...")]` takes priority.

## Doc-tag sections

`@summary:` and `@responses:` now end at a blank line. Previously any following untagged rustdoc
prose became part of the summary. `@description:` still keeps blank lines as paragraphs. A handler
can describe path parameters with `@path: name: description`; this is the way to describe a scalar
`Path<T>`, which has no field for a description.

## Components

Two different Rust types that register the same component name are now a
`BuildError::ComponentCollision`. Disambiguate with `#[schema(as = module::Name)]`; utoipa formats
the component name with dots (`module.Name`).

Generated `ToSchema`-only types can opt into the schema adapter without a hand-written impl:

```rust
roundhouse::impl_api_schema!(generated::Row);
```

## Routers

`App::router()` is deprecated. Use:

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

## Generic responses

`ApiResponse`/`ApiError` now support generic enums. `Created<T>` and `NoContent` are available for
the common created/no-content cases. `Result` aliases are documented through the existing blanket
`ApiResponse for Result<T, E>` impl when both sides implement `ApiResponse`.
