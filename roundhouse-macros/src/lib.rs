//! The procedural macros behind `roundhouse`.
//!
//! Most users should import these through `roundhouse::prelude::*` rather than depend on this
//! crate directly. The macros are re-exported by `roundhouse` when its `macros` feature is
//! enabled (and `Multipart` additionally needs the `multipart` feature).
//!
//! The crate has two kinds of macros:
//!
//! - [`routes!`](macro@routes) declares a module's route table, its prefix, tags, security and
//!   layers, and composes child modules. It expands to items and must be invoked at module scope,
//!   not inside a function body.
//! - [`#[handler]`](macro@handler) and the derive macros attach the metadata that the route table
//!   needs to build both an `axum::Router` and an OpenAPI document.
//!
//! # Example
//!
//! ```ignore
//! use roundhouse::prelude::*;
//!
//! #[derive(serde::Serialize, ToSchema, ApiSchema)]
//! struct Greeting {
//!     message: String,
//! }
//!
//! routes! {
//!     prefix = "/v1",
//!     tags = ["greetings"],
//!     get("/hello/{name}", hello, summary = "Say hello"),
//! }
//!
//! #[handler]
//! async fn hello(Path(name): Path<String>) -> Json<Greeting> {
//!     Json(Greeting { message: format!("hello {name}") })
//! }
//! ```
//!
//! The macros never infer behavior that the router does not implement. If a declaration cannot
//! be described honestly, the build fails (or, for `try_build`, returns a
//! `roundhouse::BuildError`) instead of emitting a document that lies.

mod api_response;
mod api_schema;
mod handler;
mod multipart;
mod path_params;
mod query_params;
mod routes;
mod serde_shape;
mod util;

use proc_macro::TokenStream;
use syn::{DeriveInput, ItemFn, parse_macro_input};

/// Declare the HTTP routes owned by the current module.
///
/// `routes!` is invoked at module scope. Every entry contributes one route to the router and one
/// operation to the OpenAPI document. Each handler must have been annotated with
/// [`#[handler]`](macro@handler); the route table uses the marker type that attribute generates.
///
/// A module may contain at most one `routes!` invocation: it emits `MODULE` and
/// `__roundhouse_fragment()` beside the routes. Use a nested group or a child module when a table
/// grows large, and `...module` to compose it.
///
/// # Syntax
///
/// The flat form names each path explicitly:
///
/// ```ignore
/// routes! {
///     get("/notes", list),
///     post("/notes", create, summary = "Create a note"),
///     get("/notes/{id}", show, tags = ["notes"]),
/// }
/// ```
///
/// The grouped form shares one path across several methods:
///
/// ```ignore
/// routes! {
///     "/notes" => {
///         get(list),
///         post(create, summary = "Create a note"),
///     },
///     "/notes/{id}" => {
///         get(show),
///         delete(destroy),
///     },
/// }
/// ```
///
/// A module declares its prefix and shared policy before the entries, and a string followed by a
/// block is a nested module with its own prefix and options. `...module` composes another
/// module's tree at the point it appears:
///
/// ```ignore
/// // routes/mod.rs
/// mod notes;
/// mod blog;
///
/// routes! {
///     prefix = "/api",
///     tags = ["api"],
///     ...notes,
///     "/blog" [tags = ["blog"]] {
///         get "/" => blog::list,
///         post "/" => blog::create, responses(201 = Post),
///     },
/// }
/// ```
///
/// The compact entry form `get "/path" => handler` is equivalent to `get("/path", handler)`.
///
/// The supported verbs are `get`, `post`, `put`, `patch`, `delete`, `head`, `options`, `trace`
/// and `any`. `any` is served but cannot be represented as one OpenAPI operation, so it is
/// omitted from the document with a build diagnostic. Commas between entries are optional.
///
/// An entry may carry outer attributes, most commonly `#[cfg(..)]`. In the grouped form the
/// attribute applies to every entry in that group:
///
/// ```ignore
/// routes! {
///     #[cfg(feature = "admin")]
///     get("/admin", admin),
/// }
/// ```
///
/// # Paths
///
/// Paths must start with `/`; use `"/"` for the module's own path. Captures use axum 0.8 /
/// matchit syntax: `{id}` for a segment and `{*rest}` for a trailing wildcard. The older axum
/// 0.7 syntax (`:id`) is rejected at compile time.
///
/// If the handler takes `Path<T>` where `T: roundhouse::PathNames`, the macro statically checks
/// that every capture in the leaf path is declared by `T`. The check is deliberately a subset
/// check: a parent module prefix may add captures that the entry cannot see. The full resolved
/// path is validated again when the app is built.
///
/// # Entry options
///
/// - `summary = "..."` / `description = "..."` — the operation's OpenAPI summary and description.
/// - `operation_id = "..."` — an explicit operation id. When omitted, an id is derived from the
///   method, module path and handler name. Explicit ids must be unique within a document.
/// - `tags = ["tag", ...]` — operation tags, appended to the tags inherited from enclosing
///   modules.
/// - `security = [Scheme, ...]` — scheme types enforced on this entry, appended to the guards
///   inherited from enclosing modules. One scheme is enforced by the generated handler shim as an
///   extractor; several form one AND requirement on the guard layer. Entries are types (`Bearer`),
///   not strings. A handler that already takes `Auth<A>` is not authenticated a second time.
/// - `responses(status, status = Type, ...)` — declare responses that the handler's return type
///   cannot express. A bare status documents an empty response; `status = Type` documents a JSON
///   body of `Type`. Declarations add to or refine the inferred responses.
/// - `override` — permit a `responses(..)` declaration to replace an inferred response with a
///   different status/body. Without it, a contradictory declaration is a build error.
/// - `path = "/v1/notes/{id}"` — assert the resolved path this entry declares, including every
///   parent module prefix. Useful for keeping a library's public URL stable.
/// - `layer = expression` — a tower layer applied to this route's `MethodRouter`.
/// - `doc = expression` — a hook of type `fn(&mut roundhouse::doc::HandlerDocs)` that runs after
///   the handler's inferred documentation and before document checks. It can add hand-written
///   parameters, bodies or responses; the additions still go through conflict and capture
///   validation.
/// - `hidden` — serve the route but omit it from the document.
/// - `absolute` — the path owns the full URL. Module prefixes, tags, security, layers and hidden
///   flags do not apply; a `security` declared on the entry itself still does.
/// - `deprecated` — mark the operation deprecated in the document.
/// - `public` — add the `x-roundhouse-public` extension to the operation.
/// - `validate = false` — parse the request as usual but skip `#[schema(..)]` constraint
///   validation for this route. Malformed bodies are still rejected. The document records
///   `x-roundhouse-validation: false`.
///
///   Validation is on by default. The route's generated shim checks every argument whose type
///   publishes constraints, including `axum::Json`, `axum::extract::Path`, `axum::extract::Query`
///   and `axum::Form`; the framework's own extractors already validate during extraction and are
///   not checked twice. `Result<Extractor, E>` arguments own their rejection and are left alone.
///
/// # Module options
///
/// The root table accepts these before the first entry; a `"/path" [..] { .. }` group accepts the
/// same options in its brackets:
///
/// - `prefix = "/path"` — the module's URL segment (root table only; a group uses its path).
/// - `tags = ["tag", ...]` — tags added to every non-`absolute` route below the module.
/// - `security = [Scheme, ...]` — scheme types installed as an `Auth<Scheme>` guard on every
///   non-`absolute` route below the module, and published in the document. Several schemes form
///   one AND requirement. Scopes belong on the handler argument (`Auth<Bearer, ReadPosts>`).
/// - `layer = expression` — a tower layer applied to the module's router.
/// - `layer_mode = full` (default) — apply the layer to the whole subtree, like
///   `axum::Router::layer`; `route` uses `route_layer`.
/// - `hidden` — keep the module's routes in the router but omit them from the document.
/// - `doc = expression` — an expression returning `roundhouse::doc::layer::LayerDoc`, merged into
///   every operation below the module.
///
/// Prefixes join on the tree: `routes! { prefix = "/api", ...notes }` plus a child module with
/// `prefix = "/notes"` serves `/api/notes/...`. Captures such as `/{project_id}` are allowed in a
/// prefix; catch-alls are not.
///
/// # Compile-time checks
///
/// One invocation is checked during expansion, before the router exists:
///
/// - two entries that resolve to the same method and normalised template (`{id}` and `{post_id}`
///   are the same shape), including an `any` entry against a concrete method;
/// - two entries with the same `operation_id`, or two entries that would derive the same id from
///   their method and handler name;
/// - an entry whose return type the macro can classify as producing no success response, unless
///   the entry declares one with `responses(..)`.
///
/// Checks that need the composed tree — collisions across separate modules, resolved prefix
/// joins, a handler state that disagrees with the app — remain `BuildError`s from
/// `App::try_build()`.
///
/// # Example
///
/// ```ignore
/// use roundhouse::prelude::*;
///
/// routes! {
///     prefix = "/api",
///     tags = ["notes"],
///     get("/notes/{id}", show, operation_id = "get_note"),
///     post(
///         "/notes",
///         create,
///         summary = "Create a note",
///         responses(201 = Note, 422),
///     ),
///     "/notes/{id}/revisions" => {
///         get(revisions),
///         delete(clear_revisions, deprecated),
///     },
/// }
/// ```
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as routes::RoutesInput);
    routes::expand(input).into()
}

/// Derive the routing metadata that [`routes!`](macro@routes) needs from a handler's signature.
///
/// Apply `#[handler]` to a function item. The attribute takes no arguments; routing metadata
/// belongs in the `routes!` table. It leaves the function untouched and appends a hidden marker
/// type implementing `roundhouse::doc::HandlerMeta`, which supplies the state type, path capture
/// names, request documentation and inferred responses.
///
/// It also emits a hidden shim beside the marker. `routes!` registers the shim instead of the
/// function, so route-entry policy (`validate`, `security`) is applied without changing the
/// handler's signature: the shim validates the arguments the route asked it to and, for a
/// `security = [Scheme]` entry, enforces the scheme before the body is read. A handler called
/// directly still behaves exactly as written.
///
/// # Extractors
///
/// The macro reads the signature's resolved types, not just their spelling, so aliases and
/// re-exports keep working. It understands:
///
/// - `State<T>` — sets the app state type. Every handler in one app must agree on it.
/// - `Path<T>` — records the capture names from `T: roundhouse::PathNames` and the schema for a
///   scalar `Path<T>`. With a scalar type the names are unknown, so the compile-time check is
///   skipped and the build validates the document instead.
/// - `Query<T>` and `RepeatedQuery<T>` — document query parameters from `T: roundhouse::QueryParams`.
/// - `Json<T>`, `Form<T>` and `Multipart<F>` — document the request body. A handler may read the
///   request body at most once; a second body extractor is a compile error.
/// - `Auth<A>` and `Option<Auth<A, Scopes>>` — document the security scheme, required or optional
///   authentication, and the 401/403 responses.
/// - `Strict<axum::Json<T>>`, `Strict<Path<T>>`, … — a pure axum extractor with the framework's
///   `Problem` rejection; documentation comes from the inner type.
/// - `Extension<T>` and other extractors — documented when the extractor participates in the
///   framework's probing protocol. An extractor the macro cannot inspect is marked as unverified;
///   a strict build rejects it rather than pretending to know its parameters.
/// - `Result<Extractor, E>` — probes the extractor but lets the error type own the rejection
///   response.
///
/// Extractors that reject a request contribute their failure responses automatically (for
/// example a malformed JSON body or a failed constraint).
///
/// # Responses
///
/// The return type determines the success response when it can be described unambiguously:
///
/// - no return type — `204 No Content`;
/// - `()` — `200 OK`;
/// - `Json<T>` — `200` with an `application/json` schema for `T`;
/// - `Html<T>` — `200` with a `text/html` schema;
/// - `String` and `&str` — `200` with `text/plain`;
/// - `Vec<u8>`, `Bytes` and `Body` — a binary response;
/// - `Attachment<T>`, `File` and `Sse` — the corresponding attachment/file/event-stream response;
/// - `StatusCode` — the OpenAPI `default` response, because the status is chosen at runtime;
/// - `(StatusCode, Body)` and similar tuples — the `default` response when a leading status code
///   makes the status dynamic, otherwise the body's response;
/// - `Result<T, E>` — `T`'s success response plus every response registered by the error type
///   `E`. Derive `ApiError` / `ApiResponse` on `E`, or use `Problem`, so that `E` has documented
///   responses; otherwise the build reports an undocumented error.
///
/// For any other `IntoResponse` type, declare the response in `routes!` with `responses(..)` or
/// derive `ApiResponse`. An operation with no declared success response is a build error.
///
/// # Limitations
///
/// Handlers cannot be generic, and the attribute cannot be applied to a method inside an `impl`
/// block. Use `#[handler]` on a free function.
///
/// # Example
///
/// ```ignore
/// use roundhouse::prelude::*;
///
/// #[handler]
/// async fn show(
///     State(state): State<AppState>,
///     Path(id): Path<NoteId>,
///     Query(filter): Query<NoteFilter>,
/// ) -> Result<Json<Note>, NoteError> {
///     state.load(id).await.map(Json).map_err(NoteError::from)
/// }
/// ```
#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "`#[handler]` takes no arguments; routing metadata lives in `routes!`",
        )
        .to_compile_error()
        .into();
    }
    let item = parse_macro_input!(item as ItemFn);
    handler::expand(item).into()
}

/// Derive the optional roundhouse extensions to a schema.
///
/// Wire shape and component naming come from `utoipa::ToSchema`; this derive must be paired with
/// it:
///
/// ```ignore
/// #[derive(serde::Serialize, serde::Deserialize, roundhouse::ToSchema, roundhouse::ApiSchema)]
/// pub struct Signup { .. }
/// ```
///
/// **Deriving `ApiSchema` is optional.** A type that derives only `ToSchema` can still be a JSON
/// request or response body: the document inlines its `ToSchema` schema and registers the
/// components it references. Derive `ApiSchema` when you want the type published as a named
/// component (`$ref`), when it uses a roundhouse-only keyword (`unique_items`, a `url` format
/// patch), or when it needs a `validate_with` hook. Using utoipa for the shape means serde's
/// `rename_all`, `skip`, `flatten`, tagging and recursion rules are implemented in one place, not
/// two. Component names follow utoipa as well; use `#[schema(as = module::Name)]` when two types
/// share a simple name.
///
/// # Constraints
///
/// Constraints are read from `#[schema(..)]` where utoipa understands them, and from
/// `#[roundhouse(..)]` for the roundhouse-only checks:
///
/// - `#[schema(min_length = n)]` / `max_length = n` — character-count bounds for strings.
/// - `#[schema(pattern = "regex")]` — a regular expression the string must match.
/// - `#[schema(minimum = n, maximum = n)]`, `exclusive_minimum`, `exclusive_maximum` —
///   numeric bounds.
/// - `#[schema(min_items = n, max_items = n)]` — collection bounds.
/// - `#[schema(format = "email" | "uuid" | "url" | "uri" | ...)]` — an OpenAPI format. `email`,
///   `uuid`, `url` and `uri` are also checked at runtime; other formats are documentation only.
/// - `#[schema(example = expression)]` — an example value for the schema.
/// - `#[roundhouse(unique_items)]` — uniqueness for a collection.
/// - `#[roundhouse(validate_with = path)]` — a function `fn(&T) -> Result<(), ValidationErrors>`
///   run in addition to the declared constraints. Its errors keep the field's JSON Pointer.
///
/// Constraints are published **and** enforced: the emitted schema is compiled and executed by the
/// `schema-engine` feature, so there is one declarative source rather than a second generated
/// check. `#[schema(..)]` constraints work with `ToSchema` alone. `unique_items`, the `url` format
/// patch and `validate_with` require this derive. `validate_with` is Rust code, not expressible
/// in JSON Schema, and runs after the engine. `#[schema(skip_validation)]` was removed on purpose:
/// opt out for a whole route with `validate = false` in `routes!` instead.
///
/// # Example
///
/// ```ignore
/// use roundhouse::{ApiSchema, ToSchema, ValidationErrors};
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize, ToSchema, ApiSchema)]
/// #[serde(rename_all = "camelCase", deny_unknown_fields)]
/// pub struct Signup {
///     #[schema(min_length = 3, max_length = 32)]
///     pub user_name: String,
///     #[schema(format = "email")]
///     pub email: String,
///     #[schema(minimum = 18)]
///     #[roundhouse(validate_with = validate_age)]
///     pub age: u8,
/// }
///
/// fn validate_age(age: &u8) -> Result<(), ValidationErrors> {
///     // ...
///     # Ok(())
/// }
/// ```
#[proc_macro_derive(ApiSchema, attributes(schema, serde, roundhouse))]
pub fn derive_api_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    api_schema::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive OpenAPI query-parameter documentation and validation for a struct.
///
/// Use it together with `serde::Deserialize`: `Deserialize` handles extraction and
/// `QueryParams` describes the parameters and enforces their `#[schema(..)]` constraints for
/// `Query<T>` handler arguments.
///
/// ```ignore
/// use roundhouse::{QueryParams};
/// use serde::Deserialize;
///
/// #[derive(Deserialize, QueryParams)]
/// #[serde(rename_all = "camelCase")]
/// pub struct NoteFilter {
///     #[schema(min_length = 1)]
///     pub search: Option<String>,
///     #[serde(default)]
///     pub limit: Option<u32>,
///     #[schema(min_items = 1, max_items = 20)]
///     pub tags: Vec<String>,
/// }
/// ```
///
/// # Rules
///
/// - The type must be a struct with named fields, and it cannot be generic.
/// - `Option<T>` and fields with `#[serde(default)]` are optional; every other field is required.
/// - `#[serde(rename = "..")]` and `#[serde(rename_all = "..")]` control the wire names.
/// - `#[serde(skip)]` and `#[serde(skip_deserializing)]` remove a field from the documentation.
/// - `#[serde(flatten)]` delegates to the inner type's `QueryParams` implementation.
/// - `Vec<T>` and `Option<Vec<T>>` are documented as exploded form-style arrays. Read them with
///   `RepeatedQuery<T>`; axum's `Query<T>` uses `serde_urlencoded` and cannot represent repeated
///   keys.
/// - `#[schema(..)]` constraints are published and validated with JSON Pointers.
#[proc_macro_derive(QueryParams, attributes(serde, schema, roundhouse))]
pub fn derive_query_params(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    query_params::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive OpenAPI path-parameter documentation and capture checking for a struct.
///
/// Use it together with `serde::Deserialize` and place the struct inside `Path<T>`:
///
/// ```ignore
/// use roundhouse::{PathParams};
/// use serde::Deserialize;
///
/// #[derive(Deserialize, PathParams)]
/// #[serde(rename_all = "camelCase")]
/// pub struct NotePath {
///     pub project_id: u64,
///     pub note_id: u64,
/// }
///
/// #[handler]
/// async fn show(Path(path): Path<NotePath>) -> String {
///     format!("{} {}", path.project_id, path.note_id)
/// }
/// ```
///
/// The derive implements `PathNames`, so [`routes!`](macro@routes) can check at compile time that
/// every `{capture}` in the leaf path is declared by the struct. It also implements
/// `PathValidate` for the `#[schema(..)]` constraints.
///
/// # Rules
///
/// - The type must be a struct with named fields, and it cannot be generic.
/// - `#[serde(rename = "..")]` and `#[serde(rename_all = "..")]` control the capture names.
/// - `#[serde(skip)]`, `#[serde(skip_deserializing)]`, `#[serde(default)]` and
///   `#[serde(flatten)]` are rejected: a path capture is always present and path names are flat.
/// - `#[schema(..)]` constraints are published and validated for the extracted values.
#[proc_macro_derive(PathParams, attributes(serde, schema, roundhouse))]
pub fn derive_path_params(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    path_params::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `multipart/form-data` extraction and an OpenAPI request-body schema for a struct.
///
/// This derive requires roundhouse's `multipart` feature. The resulting value is extracted with
/// `Multipart<T>`, and the same field table drives streaming limits, validation and the document.
///
/// ```ignore
/// use roundhouse::prelude::*;
///
/// #[derive(roundhouse::Multipart)]
/// pub struct Upload {
///     pub file: FilePart,
///     #[file(max_bytes = 1_000_000, accept = ["text/plain", "text/markdown"])]
///     pub notes: Option<FilePart>,
///     pub description: Option<String>,
///     pub tags: Vec<String>,
/// }
///
/// #[handler]
/// async fn upload(Multipart(form): Multipart<Upload>) -> StatusCode {
///     let _ = form;
///     StatusCode::NO_CONTENT
/// }
/// ```
///
/// # Field kinds
///
/// The field's type selects how the part is read:
///
/// - `FilePart` — one required file; `Option<FilePart>` — zero or one; `Vec<FilePart>` — zero or
///   more; `Option<Vec<FilePart>>` — an optional list.
/// - `TempFile` and its `Option`/`Vec` spellings behave the same but stream to an already-unlinked
///   temporary file. `TempFile` requires the `multipart-tempfile` feature.
/// - Any other `Deserialize` type is a text part (`String`, numbers, `bool`, and their
///   `Option`/`Vec` spellings). Text is parsed as JSON when possible, then as a JSON string.
///
/// A field is required when it is a bare `FilePart`, `TempFile` or text type. `Option`, `Vec`,
/// and `Option<Vec>` fields are optional.
///
/// # Attributes
///
/// - `#[file(max_bytes = expression, accept = ["mime/type", ...])]` — per-file streaming limit
///   and accepted media types. `max_bytes` is enforced while the part is read; `accept` is
///   checked against the part's content type. The limit is also published as `x-max-bytes`.
/// - `#[multipart(deny_unknown)]` on the struct — reject parts that do not match a field.
/// - `#[serde(rename = "..")]` on a field — use a different part name.
///
/// File fields without `max_bytes` fall back to the app-wide body limit. The build reports them
/// as a diagnostic (and a strict build rejects them), so the most permissive declaration is
/// visible.
#[proc_macro_derive(Multipart, attributes(multipart, file, serde))]
pub fn derive_multipart(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    multipart::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive `IntoResponse` plus documented responses for a response enum.
///
/// `ApiResponse` is for any enum whose variants are responses. Every variant must declare its
/// status; a response the code can produce but the document cannot describe is treated as a
/// bug.
///
/// ```ignore
/// use roundhouse::prelude::*;
///
/// #[derive(ApiResponse)]
/// pub enum CreateNote {
///     #[status(201, description = "Created")]
///     Created {
///         #[body]
///         body: Json<Note>,
///         #[header("Location")]
///         location: String,
///     },
///     #[status(409, problem = "NoteAlreadyExists")]
///     Conflict,
/// }
/// ```
///
/// # Variant attributes
///
/// - `#[status(code)]` — required. `#[status(code, description = "..")]` overrides the canonical
///   reason phrase. `#[status(code, problem = "slug")]` documents an error variant as an RFC 9457
///   problem with type `https://roundhouse.rs/problems/{slug}`.
/// - `#[response(status = code)]` is accepted as a migration alias for `#[status(code)]`.
///
/// # Field attributes
///
/// - `#[body]` — serialize this field as the response body. A single unnamed field is the body
///   implicitly when it has no other response attribute.
/// - `#[header("Name")]` — copy this field into a response header. Add `, append` to use
///   `HeaderMap::append` instead of `insert`.
/// - `#[location]` — copy this field into the `Location` header, documented as a URI.
/// - `#[from]` — accept a `ValidationErrors` field and turn it into a problem response with this
///   variant's status.
///
/// A variant without a body and a status below 400 produces an empty response with that status.
/// A variant without a body and a status of 400 or above produces a `roundhouse::Problem`.
///
/// The derive is for enums only, and generic enums are not supported.
#[proc_macro_derive(
    ApiResponse,
    attributes(status, response, body, header, location, from)
)]
pub fn derive_api_response(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    api_response::expand(input, false)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

/// Derive an error response enum: every `ApiResponse` behavior plus an error contract.
///
/// `ApiError` is the error-side counterpart of [`ApiResponse`](macro@ApiResponse). It accepts the
/// same `#[status(..)]`, `#[body]`, `#[header(..)]`, `#[location]` and `#[from]` attributes, and
/// additionally requires every status to be `4xx` or `5xx`.
///
/// The type must implement `std::error::Error`; the derive enforces that with a compile-time
/// assertion. The error's `Display` output becomes the `detail` of a generated problem response.
///
/// ```ignore
/// use roundhouse::prelude::*;
///
/// #[derive(Debug, thiserror::Error, ApiError)]
/// pub enum NoteError {
///     #[error("note {0} was not found")]
///     #[status(404, problem = "NoteNotFound")]
///     NotFound(i64),
///     #[error("the note was invalid")]
///     #[status(422)]
///     Invalid(#[from] ValidationErrors),
/// }
/// ```
#[proc_macro_derive(ApiError, attributes(status, response, body, header, location, from))]
pub fn derive_api_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    api_response::expand(input, true)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
