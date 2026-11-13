//! The procedural macros behind `roundhouse`.
//!
//! Most users should import these through `roundhouse::prelude::*` rather than depend on this
//! crate directly. The macros are always re-exported by `roundhouse` (`Multipart` additionally
//! needs roundhouse's `multipart` feature).
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
//!     "/hello/{name}" => {
//!         get(hello),
//!     },
//! }
//!
//! /// @summary: Say hello.
//! #[handler]
//! async fn hello(Path(name): Path<String>) -> Json<Greeting> {
//!     Json(Greeting { message: format!("hello {name}") })
//! }
//! ```
//!
//! The generated router and document come from the same declarations. A declaration that cannot
//! be represented honestly fails the build, or returns a `roundhouse::BuildError` from
//! `App::try_build`, rather than publishing a document that disagrees with the router.

use proc_macro::TokenStream;

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
/// Every entry lives inside a path group:
///
/// ```ignore
/// routes! {
///     "/notes" => {
///         get(list),
///         post(create),
///     },
///     "/notes/{id}" => {
///         get(show),
///         delete(destroy),
///     },
/// }
/// ```
///
/// A module declares its prefix and shared policy before the groups, and a string followed by a
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
///         "/" => {
///             get(blog::list),
///             post(blog::create),
///         },
///     },
/// }
/// ```
///
/// The supported verbs are `get`, `post`, `put`, `patch`, `delete`, `head`, `options`, `trace`
/// and `any`. `any` is served but cannot be represented as one OpenAPI operation, so it is
/// omitted from the document with a build diagnostic. Commas between entries are optional.
///
/// A group may carry outer attributes, most commonly `#[cfg(..)]`. In a path group the attribute
/// applies to every entry in that group:
///
/// ```ignore
/// routes! {
///     #[cfg(feature = "admin")]
///     "/admin" => {
///         get(admin),
///     },
/// }
/// ```
///
/// # Paths
///
/// Paths must start with `/`; use `"/"` for the module's own path. Captures use `{id}` for a
/// segment and `{*rest}` for a trailing wildcard. Escaped braces use `{{` and `}}`.
///
/// If the handler takes `Path<T>` where `T: roundhouse::Params`, the macro statically checks
/// that every capture in the leaf path is declared by `T`. The check is deliberately a subset
/// check: a parent module prefix may add captures that the entry cannot see. The full resolved
/// path is validated again when the app is built.
///
/// # Handler prose and typed metadata
///
/// Operation prose stays in the handler's doc block; structured response and path metadata lives
/// on the handler attribute:
///
/// ```ignore
/// /// @summary: Create a note.
/// /// @description: Persists the note and returns it.
/// #[handler(
///     response(201, Note),
///     response(409, Problem),
/// )]
/// async fn create(...) -> ... { ... }
/// ```
///
/// `@summary:` and `@description:` set the operation text. `response(status, Type)` declares a
/// JSON body, `response(status)` an empty response, `default_response(Type)` OpenAPI's `default`,
/// and `override_response(status, Type)` replaces an inferred shape.
/// `response_description(status, "...")` overrides one description, and
/// `path(name, "...")` describes a path parameter with no field description. `Json<T>` is
/// unwrapped to `T`.
///
/// # Entry options
///
/// - `operation_id = "..."` — an explicit operation id. When omitted, an id is derived from the
///   method, module path and handler name. Explicit ids must be unique within a document.
/// - `tags = ["tag", ...]` — operation tags, appended to the tags inherited from enclosing
///   modules.
/// - `security = [Scheme, ...]` — scheme types enforced on this entry, appended to the guards
///   inherited from enclosing modules. One scheme is enforced by the generated handler shim as an
///   extractor; several form one AND requirement on the guard layer. Entries are types (`Bearer`),
///   not strings. A handler that already takes `Auth<A>` is not authenticated a second time.
/// - `path = "/v1/notes/{id}"` — assert the resolved path this entry declares, including every
///   parent module prefix. Useful for keeping a library's public URL stable.
/// - `layer = expression` — a tower layer applied to this route's `MethodRouter`.
/// - `doc = expression` — a hook of type `fn(&mut roundhouse::doc::OperationDoc)` that runs after
///   the handler's inferred documentation and before document checks. It uses the operation's
///   builder methods (`set_summary`, `push_param`, `set_body`, `push_response`, …) to override
///   prose or add hand-written parameters, bodies or responses; the additions still go through
///   conflict and capture validation.
/// - `prose = expression` — a `&'static str` (often `include_str!("...")`) whose first line is
///   the operation summary and whose remaining text is the description. The expression is
///   evaluated at compile time; `doc` still runs afterwards and may override it.
/// - `hidden` — serve the route but omit it from the document.
/// - `absolute` — the path owns the full URL. Module prefixes, tags, security, layers and hidden
///   flags do not apply; a `security` declared on the entry itself still does.
/// - `deprecated` — mark the operation deprecated in the document.
/// - `anonymous` — clear inherited `security` guards for this entry and document `security: []`.
///   A handler's own `Auth<A>` still applies.
/// - `validate = false` — parse the request as usual but skip `#[schema(..)]` constraint
///   validation for this route. Malformed bodies are still rejected. The document records
///   `x-roundhouse-validation: false`. This is a whole-route opt-out; `unverified = "reason"`
///   acknowledges it so `strict()` passes.
/// - `validate_body = false` — keep path, query and form validation, but treat the request body
///   as opaque: do not install the raw JSON layer, run body schemas or body hooks. Malformed
///   JSON is still rejected during extraction. The document records
///   `x-roundhouse-body-validation: false`; unlike `validate = false`, this is quiet under
///   `strict()` because it is a precise body policy.
/// - `unverified = "reason"` — acknowledge an intentional unverified body, response or extractor
///   on this operation. It suppresses the build diagnostic and lets `strict()` pass, while the
///   `x-roundhouse-unverified-*` extension still tells document consumers the truth.
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
/// - `state = AppState` — override the router state for every entry in this table. Without it,
///   the table uses the crate-level `app_state!(Type)` alias. A handler's `State<T>` arguments
///   then need `T: FromRef<AppState>`; `State<AppState>` itself still works. Inline groups inherit
///   the state unless they declare their own, and `state = infer` restores per-handler inference.
///   A spread module resolves the same crate-level alias without repeating it.
/// - `tags = ["tag", ...]` — tags added to every non-`absolute` route below the module.
/// - `security = [Scheme, ...]` — scheme types installed as an `Auth<Scheme>` guard on every
///   non-`absolute` route below the module, and published in the document. Several schemes form
///   one AND requirement. Scopes belong on the handler argument (`Auth<Bearer, ReadPosts>`).
/// - `layer = expression` — a tower layer applied to the module's router.
/// - `layer_mode = full` (default) — apply the layer to the whole subtree, like
///   `axum::Router::layer`; `route` uses `route_layer`.
/// - `hidden` — keep the module's routes in the router but omit them from the document.
/// - `anonymous` — clear security guards inherited from enclosing modules for this subtree.
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
///   the handler declares one with `#[handler(response(..))]`.
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
///     "/notes/{id}" => {
///         get(show, operation_id = "get_note"),
///     },
///     "/notes" => {
///         post(create),
///     },
///     "/notes/{id}/revisions" => {
///         get(revisions),
///         delete(clear_revisions, deprecated),
///     },
/// }
/// ```
#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    roundhouse_macros_core::expand_routes(input.into()).into()
}

/// Derive the routing metadata that [`routes!`](macro@routes) needs from a handler's signature.
///
/// Apply `#[handler]` to a function item. The attribute accepts typed response and path metadata
/// (`response(..)`, `override_response(..)`, `default_response(..)`,
/// `response_description(..)`, `path(..)`); routing policy belongs in the `routes!` table. It
/// leaves the function untouched and appends a hidden marker
/// type implementing `roundhouse::doc::HandlerSpec`, which supplies the state type, path capture
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
/// The macro no longer classifies argument types. Each extractor contributes its own
/// documentation and validation through `roundhouse::doc::ExtractArg`; the macro asks the
/// concrete type and marks an extractor without an implementation as unverified.
///
/// - `State<T>` — extracted from the route table's `state = S` using `T: FromRef<S>`. Without a
///   table state, a single `State<T>` is the default router state. The app state is still exact at
///   the `App::new` boundary; the table only names it.
/// - `Path<T>` — records the capture names from `T: roundhouse::Params` and the schema for a
///   scalar `Path<T>`. With a scalar type the names are unknown, so the compile-time check is
///   skipped and the build validates the document instead.
/// - `Query<T>` and `RepeatedQuery<T>` — document query parameters from `T: roundhouse::Params`.
/// - `Json<T>`, `Form<T>` and `Multipart<F>` — document the request body. `Json<T>` and `Form<T>`
///   require `T: roundhouse::ApiSchema` for automatic documentation. A handler may read the
///   request body at most once; a second body extractor is a compile error.
/// - `Auth<A>` and `Option<Auth<A, Scopes>>` — document the security scheme, required or optional
///   authentication, and the 401/403 responses.
/// - `Strict<axum::Json<T>>`, `Strict<Path<T>>`, … — a pure axum extractor with the framework's
///   `Problem` rejection; documentation comes from the inner type.
/// - `Extension<T>`, `HeaderMap`, `Method`, `Uri`, … — known axum extractors with their documented
///   contribution. A custom extractor is supported by implementing `ExtractArg`; without it, the
///   operation is marked `x-roundhouse-unverified-extractor` and a strict build rejects it.
/// - `Result<Extractor, E>` — documents the extractor's successful shape but lets the error type
///   own the rejection response.
///
/// Extractors that reject a request contribute their failure responses automatically (for
/// example a malformed JSON body or a failed constraint).
///
/// # Responses
///
/// The return type documents itself through `roundhouse::doc::ApiResponse`. The framework provides
/// impls for the common axum shapes:
///
/// - no return type or `()` — `200 OK`;
/// - `Json<T>` — `200` with an `application/json` schema for `T: ApiSchema`;
/// - `Html<T>` — `200` with a `text/html` schema;
/// - `String` and `&str` — `200` with `text/plain`;
/// - `Vec<u8>`, `Bytes` and `Body` — a binary response; other `Vec<T>` values are JSON arrays;
/// - `Attachment<T>`, `File` and `Sse` — the corresponding attachment/file/event-stream response;
/// - `StatusCode` — the OpenAPI `default` response, because the status is chosen at runtime;
/// - `(StatusCode, Body)` — the `default` response. Other tuples are not classified; declare
///   them with `#[handler(response(..))]`;
/// - `Result<T, E>` — `T`'s success response plus every response registered by the error type
///   `E`. Derive `ApiError` / `ApiResponse` on `E`, or use `Problem`, so that `E` has documented
///   responses; otherwise the build reports an undocumented error.
///
/// A known shape whose body type publishes no schema (for example `Created<T>` where `T` is only
/// `Serialize`) is documented with its status and `x-roundhouse-unverified-response`; `strict()`
/// rejects it unless the entry acknowledges it with `unverified = "reason"`.
///
/// A custom response type implements `ApiResponse` and its `SUCCESS` statuses. For any other
/// `IntoResponse` type, declare the response on the handler with `#[handler(response(..))]`. An
/// operation with no declared success response is a build error.
///
/// # Limitations
///
/// Handlers cannot be generic, and the attribute cannot be applied to a method inside an `impl`
/// block. Use `#[handler]` on a free function.
///
/// The handler's doc block carries prose: `@summary:` (ends at a blank line) and `@description:`
/// (keeps paragraphs). Untagged prose stays rustdoc-only; use `prose = include_str!(..)` on the
/// route entry when one public document should supply both summary and description. Structured
/// response and path metadata belongs in the attribute, not the doc block.
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
    roundhouse_macros_core::expand_handler(attr.into(), item.into()).into()
}

/// Define the roundhouse extensions to a schema.
///
/// Wire shape and component naming come from `utoipa::ToSchema`; this derive is paired with it:
///
/// ```ignore
/// #[derive(serde::Serialize, serde::Deserialize, roundhouse::ToSchema, roundhouse::ApiSchema)]
/// pub struct Signup { .. }
/// ```
///
/// `ApiSchema` registers the type as a named component (`$ref`) and applies the roundhouse-only
/// patches. It is the trait automatic body and response documentation requires; a type that
/// derives only `ToSchema` can still be published with an explicit `#[handler(response(..))]`
/// declaration or a `doc` hook. Derive `ApiSchema` when the type is a documented JSON body or
/// response, should be a named component, uses `unique_items` or the `url` to `uri` format
/// mapping, or has a `validate_with` hook.
///
/// Component names follow utoipa; use `#[schema(as = module::Name)]` when two types share a simple
/// name.
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
/// Constraints are published and enforced: the emitted schema is compiled and executed by the
/// `schema-engine` feature. `#[schema(..)]` constraints work with `ToSchema` alone; `unique_items`,
/// the `url` to `uri` mapping and `validate_with` require this derive. `validate_with` is Rust
/// code, so it runs after the engine. Opt a route out of constraint checks with `validate = false`
/// in `routes!`.
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
    roundhouse_macros_core::expand_api_schema(input.into()).into()
}

/// Derive both `ToSchema` and `ApiSchema` with one annotation.
///
/// This is the one-annotation form for documented bodies and responses:
///
/// ```ignore
/// use roundhouse::prelude::*;
/// use serde::{Deserialize, Serialize};
///
/// #[roundhouse::schema]
/// #[derive(Serialize, Deserialize)]
/// pub struct Note {
///     #[schema(min_length = 1)]
///     pub title: String,
/// }
/// ```
///
/// The attribute re-emits the item with `#[derive(roundhouse::ToSchema,
/// roundhouse::ApiSchema)]`, so `#[schema(..)]`, `#[serde(..)]` and `#[roundhouse(..)]` keep
/// working exactly as they do with the two derives. Use it on a struct or enum, as the outermost
/// attribute; do not also derive `ToSchema` or `ApiSchema` on the same item.
#[proc_macro_attribute]
pub fn schema(attr: TokenStream, item: TokenStream) -> TokenStream {
    roundhouse_macros_core::expand_schema(attr.into(), item.into()).into()
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
    roundhouse_macros_core::expand_query_params(input.into()).into()
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
/// The derive implements `Params`, so [`routes!`](macro@routes) can check at compile time that
/// every `{capture}` in the leaf path is declared by the struct. It also publishes the parts
/// schema and runs the `#[schema(..)]` constraints through `Params::validate_at`.
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
    roundhouse_macros_core::expand_path_params(input.into()).into()
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
    roundhouse_macros_core::expand_multipart(input.into()).into()
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
///   reason phrase and the variant's `@description:` doc tag. `#[status(code, problem = "slug")]`
///   documents an error variant as an RFC 9457 problem with type
///   `https://roundhouse.rs/problems/{slug}`. `#[response(status = code)]` is an equivalent
///   spelling.
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
/// A variant's `@description:` doc tag supplies the response description unless the status
/// attribute provides one explicitly.
///
/// A variant without a body and a status below 400 produces an empty response with that status.
/// A variant without a body and a status of 400 or above produces a `roundhouse::Problem`.
///
/// The derive is for enums only. Generic enums are supported: the derive copies the enum's
/// generics and `where` clause, then adds only the bounds each documented field needs. A `Json<T>`
/// body needs `T: ApiSchema` for documentation and `Json<T>: IntoResponse` at runtime; a bare body
/// needs `T: ApiSchema + IntoResponse`; a header or `#[location]` needs `T: ApiSchema +
/// TryInto<HeaderValue>`. `Created<T>` and `NoContent` cover the common created/no-content cases
/// without a derive.
#[proc_macro_derive(
    ApiResponse,
    attributes(status, response, body, header, location, from)
)]
pub fn derive_api_response(input: TokenStream) -> TokenStream {
    roundhouse_macros_core::expand_api_response(input.into(), false).into()
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
    roundhouse_macros_core::expand_api_response(input.into(), true).into()
}
