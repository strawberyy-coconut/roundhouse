> Snapshot of `~/roundhouse/SPEC.md`, taken 2026-10-28 (revision 3).
> The canonical copy lives with the framework: edit there first, then re-copy.

# roundhouse — design spec

**Status:** draft v0.1 · **Name:** `roundhouse` — crates.io, the GitHub org and `roundhouse.rs` were
all unclaimed · fallbacks: `cadastre`, `axum-modular`
**Target:** axum 0.8.x · Rust 2024 edition · OpenAPI 3.1 · `utoipa` 6.x as the document model

> **Revision 3 — 2026-10-28.** Crate graph and boundaries (§3), multipart extraction (§7.2),
> milestones (§13), spike 14. Revision 2 added §1.1 (what may enter the core), §4.5 (loud
> derivation), §7.6.1 (declaration vs enforcement), §7.8 (response validation), §8 (fallible build),
> §17 (the schema-execution decision), and rewrote §5.3, §7.7, §11, §12, §13, §14, §15. The ruling
> idea: *silence is a bug.*

---

## 1. What this is

A thin layer over axum that turns declarations into a `Router` and an OpenAPI document.

You write:

```rust
mount!("/blog");                          // module owns a URL subtree

routes! {                                 // this module's leaves
    get("/{id}", get),
}

#[handler]
async fn get(Path(PostId { id }): Path<PostId>) -> Result<Json<Post>, ApiError>;
```

And you get `GET /blog/{id}` on a real `axum::Router`, plus a spec that knows the path params,
the response schema, and the auth scheme — without a single `Router::new().route(...)` call.

**Goal:** the router is a build artifact, not source code.

### Goals

| # | Goal |
|---|------|
| G1 | Zero router wiring. No `Router::new()`, no `nest()`, no manual `method_router` in user code |
| G2 | Discovery is automatic: adding a handler to a module's `routes!` table is enough to expose the endpoint |
| G3 | The OpenAPI document is derived from the same declarations, never hand-written |
| G4 | Errors at the right time for the mistakes that matter: path params at compile time; duplicate paths, duplicate operation ids and undeclared responses at build time — not silent 404s |
| G5 | Escape hatches in both directions: a plain `axum::Router` in, a plain `axum::Router` out |
| G6 | What the document says is enforced: `#[schema(..)]` constraints reject bad input with a `400` |
| G7 | Nothing is documented silently wrong: an omission is a build error or a marker, never a blank operation (§4.5) |

### Non-goals

- Not a DI container. No service registration, no lifetimes-for-services.
- Not an ORM, migration tool, or template engine.
- Not engine-agnostic. It emits axum; that is the point.
- Not a full-stack framework: no view layer, no client reactivity, no asset pipeline, no bundled
  components. That half of the ecosystem is being solved elsewhere; this is the API half.
- No runtime reflection, no `anyhow`-style dynamic dispatch in the hot path.

### 1.1 What may enter the core

The framework is thin in runtime indirection. It is not thin in opinion, and that distinction matters
because every opinion it ships is one the user cannot opt out of. A feature earns a place in the core
by exactly one test:

> **The divergence test.** Does this remove a place where the document and the server can disagree —
> or where the route table and the file tree can disagree — and can the framework point at the code
> that proves it?

| Candidate | Verdict | Because |
|---|---|---|
| Prefix derivation, discovery, doc assembly | core | the route table and the document are one artifact |
| `#[schema(..)]` constraints and their enforcement | core | one declaration, two outputs; the divergence is the product |
| `ApiResponse` / `ApiError` | core | the document *is* the variant set, not a description of it |
| `ApiSchema` over serde attributes | core | the wire format and the schema have one source |
| `Multipart<F>` | satellite crate (§3) | typed files are where documents lie hardest; the surface and its `tempfile` dependency are optional |
| Sessions, cookies, CORS, rate limiting, DI, ORM, templating | library | removes no divergence we can prove |

The corollary outranks every other rule in this document: **silence is a bug.** When the framework
cannot derive something it says so at build time; it does not emit an operation with the interesting
half missing and hope nobody diffs the spec. §4.5 is the enforcement of that sentence.

### The axum contract

Axum's philosophy is extractors in, response types out, tower in between — all resolved at compile
time, with no framework-owned request context. Topcoat is the opposite: it owns the router, the body
and a `Cx`, so a handler is `dyn Fn(&Cx) -> Response` and everything composes around that. This
framework sits on axum's side of that line.

| Layer | Owner |
|---|---|
| Extraction (`FromRequestParts`, `FromRequest`) | axum |
| State (`State<T>`, `Extensions`) | axum |
| Responses (`IntoResponse`) | axum — `#[derive(ApiResponse)]` *implements* it, never replaces it |
| Middleware (`tower::Layer`) | tower |
| Discovery, prefix derivation, OpenAPI, constraint checks | roundhouse — all decided before `main` runs |

**The rule: the framework may add compile-time knowledge; it may not add runtime indirection.** No
request context type, no service container, no custom `Next` chain, no protocol of its own. Anything
that would sit between the request and the handler belongs to axum or tower, which is why `Cx`-style
context injection was declined (§16) and why middleware documentation attaches to a tower layer
rather than defining a new middleware shape (§6).

One deliberate shadow to be honest about: the prelude's `Json`, `Query`, `Path`, `Form` and
`Multipart` are this crate's structs. They implement axum's traits and `Deref` to the same payloads,
so they are drop-in — they exist so validation and `Problem`-shaped rejections happen in the
extractor instead of in every handler. That is the only place the "feels like axum" rule bends, and
it bends at the import line, never inside a handler body.

### Same routing, different spelling

The routing **model** is axum's and stays axum's: a method + path pair dispatches to a handler, tower
layers wrap subtrees, `fallback` handles misses, `405` comes from the method set, precedence is
`matchit`'s. There is no second routing engine and no shadow semantics — only a change in *where the
declaration lives*. That change is the product.

| axum | roundhouse | Runtime |
|---|---|---|
| `Router::new()` | `roundhouse::app!(state)` | same `Router` underneath |
| `.route("/x", get(h))` | `get("/x", h)` in `routes!` | same `MethodRouter` |
| `.nest("/blog", r)` | `mount!("/blog")` | same `nest` |
| `.layer(l)` | `mount!(layer = l)` / `App::layer(l)` | same tower layer |
| `.route_layer(l)` | `mount!(layer_mode = route)` | same `route_layer` |
| `.with_state(s)` | `App::new(s)` | same call |
| `.fallback(f)` | `App::fallback(f)` | same |
| `.merge(r)` | `App::merge(r)` | same |
| `get(h)`, `post(h)` | unchanged | unchanged |
| `State<T>`, `FromRequest`, `IntoResponse` | unchanged | unchanged |

### The fidelity test

The surface is allowed to differ; the semantics are not. Two requirements, tested separately.

**Semantics — a differential test.** For every construct in the table above, the same request runs
against a hand-written `Router` and against the discovered one, and status, headers and body are
asserted equal. Routing, nesting, layer order, `405` method sets and fallback are axum's behaviour or
the release is broken — a style change is never an excuse for a semantic difference.

**Surface — five checks, the v0.1 acceptance criteria:**

1. **Bodies are untouched.** Take any axum example, delete its router wiring, keep the handler: it
   compiles as written. Same extractors, same return types, same `Result`.
2. **No new trait to start.** A first endpoint needs one `routes!` line and `#[handler]`, and nothing else. Derives are
   additive (`ApiSchema` on a body type), not prerequisites for serving.
3. **`axum::Router` out.** `App::router()` returns a real `Router<S>`, so `axum::serve`, custom
   `fallback`, hand-mounted routes and other crates' routers all still work.
4. **Errors are `IntoResponse`.** Any `Result<T, E>` where `T: IntoResponse` and `E: IntoResponse`
   compiles; documentation may be missing for a hand-rolled `E`, never a compile error.
5. **Deleting the framework leaves a program.** Remove the attributes and the `mount!` calls, wire
   the same handlers into a `Router` by hand, and it still serves. If a design needs a runtime piece
   that only roundhouse can supply, check 5 fails and the design is wrong.

Check 4 is the one currently in tension: `ApiResponse` and `ApiError` are *derives over*
`IntoResponse`, which is fine, but the extractor wrapper means `axum::Json` and `roundhouse::Json`
are different types. A handler that names `axum::Json` explicitly still compiles — check 1 holds —
it simply skips validation and `Problem`-shaped rejections.

### Why a registry at all

Rust has no runtime reflection, so "auto-discovery" needs one of three mechanisms:

| Mechanism | Cost | Verdict |
|---|---|---|
| Explicit `App::modules([blog::routes(), ...])` | manual list, fails G2 | fallback only |
| `build.rs` scanning `src/` | build scripts are fragile, breaks IDE save-run loop | deferred to v0.3 |
| **Link-time registration (`inventory`)** | statics must be const-evaluable | **v0.1 core** |

`inventory` gives us exactly what is needed: every `routes!` entry and `mount!` submits a `&'static`
record into a linker section; the app joins them at startup. No codegen, no file scanning,
no build script, and the compiler still type-checks every handler.

---

## 2. Mental model

```
crate
├── main.rs            → App::new(state).serve()
└── routes/
    ├── health.rs      → mount!(None)            → GET /healthz
    └── blog/
        ├── mod.rs     → mount!("/blog")         → prefix + middleware + tags
        └── post.rs    → mount!("/{id}")?  no — mount! is per module
```

Three rules, in order of importance:

1. **A route's path = every `mount!` on its ancestor modules, outermost first, joined with the
   route's own path.** A module with no `mount!` contributes nothing (empty prefix).
2. **A module's `mount!` applies to the whole subtree** — prefix, middleware, tags, security.
3. **Module nesting is Rust module nesting.** `blog::post` is a child of `blog` because of
   `mod post;`, not because of the directory name. Directory layout is convention, not magic.

Rule 3 is the important one: it means moving a file changes its URL only if you move the `mod`
declaration too, and `cargo doc`/`rust-analyzer` see the same tree the framework does.

**A module never enumerates its children.** `mod` is the only edge in the graph, which is the whole
difference from hand-written nesting:

```rust
// today: the parent lists every child, and owns their prefixes
pub fn router() -> OpenApiRouter<ArcAppState> {
    OpenApiRouter::new()
        .nest("/ingest", ingest::router())
        .nest("/documents", documents::router())
        .nest("/entities", entities::router())
        .nest("/query", query::router())
        .nest("/workers", worker::router())     // forget one, and it silently 404s
}

// with discovery: the parent lists modules, not routes
pub mod ingest;
pub mod documents;
pub mod entities;
pub mod query;
pub mod worker;
```

Each child declares its own prefix with `mount!`, so the child owns its URL instead of borrowing it
from a string in the parent file. The cost is that no single file shows the whole tree — which is
what `roundhouse routes` prints, and what the generated spec is for.

```
             mount!("/blog")                 GET /{id}
blog (mod) ─────────────────► prefix /blog ──────────────────► GET /blog/{id}
   └── post (mod) ──(no mount)──► prefix /blog
```

---

## 3. Crate layout

Four crates, published together. Two more are planned and gated on a consumer (§3.1).

| Crate | Holds | Depends on |
|---|---|---|
| `roundhouse-schema` | `ApiSchema` + derive, constraint attributes, `ValidationErrors`, the schema engine (§17) | utoipa, serde, regex, `jsonschema` (feature) |
| `roundhouse` | `App`, assembly, extractors, layers, document assembly, auth, `Problem`; re-exports the other three | schema, axum, tower, inventory |
| `roundhouse-multipart` | `Multipart<F>` + derive, `FilePart`, `TempFile` | axum, bytes, tempfile, schema |
| `roundhouse-macros` | `routes!`, `mount!`, `#[handler]`, the derives | syn, quote |
| `roundhouse-core` *(planned)* | records, prefix resolution, document assembly — no axum | schema, utoipa, inventory |
| `roundhouse-conformance` *(planned)* | boots a service and fuzzes it against its document | core, jsonschema |

`roundhouse` is a facade as well as the axum integration. The macros emit `::roundhouse::…` paths and
the facade re-exports the satellites, so moving code between crates never changes a user's import —
which is what makes the split refactorable at all.

```toml
[dependencies]
roundhouse = "0.2"   # schema, multipart and macros come through it
axum = "0.8"
```

`utoipa` is an implementation detail behind `roundhouse::doc`, re-exported so users never add it
to their own `Cargo.toml`.

### 3.1 Why these boundaries

**The consumer test.** A crate earns existence when something other than the facade can depend on it —
a user, a tool, a build script. "This file is long" is a module problem, not a crate problem.

| Boundary | Verdict | Because |
|---|---|---|
| routing vs document assembly | **one crate** | `RouteRecord` carries the handler, the state `TypeId` and the doc function (§5.1); splitting it declares the record twice or adds an adapter trait that exists only to justify the split |
| schema + validation vs document assembly | **split** | the schema half has no routes in it; a build script, an SDK or a test can use it, and it owns the two heaviest dependencies |
| multipart | **split** | independent dependency (`tempfile`), independent surface (four file field types), and a JSON-only API should not compile it |
| routing vs axum | **split later** | `MountLayer = fn(Router<()>) -> Router<()>` is the only part of the registry that fundamentally needs axum; a router-free record is what the v0.3 codegen mode needs. Build it when that consumer exists |
| tests / conformance | **separate tool, 1.0** | verifying a running service against its document is a different product; it should not be a framework dependency |

**What must not move.** The record, the resolved table and the document assembly stay in one crate.
They are the same artifact, and §5.2's "same data, two consumers" is true only while they share it.

### The state type

`axum::Router<S>` is generic and `inventory` statics are not, so the record that holds a handler
cannot be generic over the app's state. It also cannot *name* it: `RouteRecord` lives in the framework
crate, `AppState` lives in yours. So the type is **carried, not dictated** — each record erases its
handler and remembers which state it was built against:

```rust
pub struct RouteRecord {
    …
    pub state: fn() -> TypeId,            // TypeId::of::<AppState>()
    pub handler: fn() -> Box<dyn Any>,    // Box::new(MethodRouter::<AppState>)
}
```

`App::<S>::build()` compares each record's `TypeId` with `TypeId::of::<S>()` before downcasting, so a
route wired to the wrong state fails at startup with

```
route `blog::post::get` needs `blog::State`; app was built with `AppState`
```

instead of an inference error forty lines deep.

**Where the type comes from:** the handler. The macro reads `State<T>` out of the signature and emits
`MethodRouter<T>`; there is no attribute, no convention, and no second way to reach the state. Your
handlers are ordinary axum handlers, and `State<AppState>` is the only thing you write.

The one handler with no type to read is a stateless one (`healthz()`): `get(healthz)` is generic over
`S`, and something has to pin it. Those take the crate's `type AppState = …` or the enclosing
`mount!(state = ..)` — the only place a name is required, and only because a health check has no state
to infer from.

Erased mode (`feature = "erased-state"`) goes one step further: handlers take `Extension<Arc<T>>` and
records are `MethodRouter<()>`, so no type is named anywhere, stateless routes included. The cost
belongs in §15 — the guarantee moves from compile time to "we hope someone registered it".

---

## 4. Macros

### 4.1 `routes!` and `#[handler]`

A module's routes are listed once, in that module. The list replaces both `fn router()` and the
per-handler path/tag/params/responses attributes:

```rust
// src/routes/documents.rs
mount!("/documents", tags = ["Documents"]);

routes! {
    get("/",        list_documents,  summary = "List documents"),
    get("/{id}",    get_by_id,       summary = "Read a document"),
    delete("/{id}", destroy,         responses(204)),
}
```

`path` uses axum 0.8 syntax: `{id}` for a capture, `{*rest}` for a wildcard. Verbs are
`get | post | put | patch | delete | head | options | trace`. Entries are `<verb>("<path>", <fn>)`, or
grouped by path to mirror `MethodRouter` chaining:

```rust
routes! {
    "/"     => { get(list), post(create) },
    "/{id}" => { get(show), patch(update), delete(destroy) },

    #[cfg(feature = "admin")]
    get("/admin/audit", audit),
}
```

`#[cfg(..)]` above an entry gates that entry's generated items — the table's version of the
conditional composition a builder gets from being a value. Dynamic composition stays `App::router()`.

| Per-entry attribute | Type | Default | Meaning |
|---|---|---|---|
| `summary` / `description` | string | — | OpenAPI text |
| `operation_id` | string | fn name | OpenAPI `operationId` |
| `tags` | `["a"]` | inherited from mounts | merged with module tags |
| `responses(...)` | list | inferred from return type | override or add |
| `security` | `["bearer"]` | inferred from extractors | schemes required |
| `layer` | expr | — | per-route middleware |
| `hidden` / `absolute` | — | false | omit from the doc / ignore the module prefix |

The handler carries a one-line adapter:

```rust
#[handler]
async fn get_by_id(
    Path(DocumentId { id }): Path<DocumentId>,
    State(app): State<AppState>,
) -> Result<Json<Document>, ApiError> { … }
```

`#[handler]` declares nothing about routing. It reads the signature — extractors become parameters,
the return type becomes the response set — and emits only the documentation half. The routing half
comes from the table, and the two are joined by the function's address. The table also emits a
compile-time reference to the handler's marker item, so a listed handler without `#[handler]` fails
to build *at the table line*, not at runtime:

```rust
const _: () = { let _ = get_by_id_ROUNDHOUSE_HANDLER; };
```

**Why two macros rather than one.** A `macro_rules!` table sees an identifier, not a signature, and
recovering the signature from the type is blocked by coherence:

```rust
impl<R: IntoResponse> ApiResponse for R { … }                   // claims Result<_, _> too
impl<T: IntoResponse, E: ApiError> ApiResponse for Result<T, E>  // … and conflicts with it
```

`Result<T, E>` is the most common return shape, so the blanket impl that would make a
signature-free table work is the one Rust forbids. The attribute parses the signature; the table
declares the route. Neither repeats the other, and `#[route]` no longer exists — a route that leaves
the module tree says so with `absolute`.

Everything both halves store is `'static` and const-evaluable, which is what `inventory` requires.

### 4.2 `mount!`

```rust
mount!("/blog", tags = ["blog"], layer = TraceLayer::new_for_http());

// a group: tags and layers, no URL segment of its own
mount!(tags = ["internal"], security = ["bearer"]);
```

| Attribute | Type | Default | Meaning |
|---|---|---|---|
| positional | string | — (group) | prefix for this module's subtree |
| `tags` | `["a"]` | — | tags applied to every route below |
| `layer` | expr | — | middleware applied to the whole subtree |
| `layer_mode` | `full` \| `route` | `full` | `full` = `Router::layer`, `route` = `Router::route_layer` |
| `security` | `["bearer"]` | — | default security for routes below |
| `hidden` | — | false | exclude subtree from the document |

`mount!` must appear at module level (top of the file, conventionally after `mod` declarations).
It records `module_path!()`, so it attaches to the module it is written in — including `mod.rs`.
Omitting the prefix makes the module a *group*: it contributes policy to its subtree and nothing to
the URL, which is how a wrapper module like `routes/mod.rs` holds crate-wide tags or a rate limit
without adding a path segment.

Two `mount!` in one module is a compile error (duplicate const name), which is the right failure.

### 4.3 `#[derive(PathParams)]`

```rust
#[derive(Deserialize, PathParams)]
pub struct PostId { pub id: i64 }
```

Emits:

```rust
impl ::roundhouse::doc::PathNames for PostId {
    const NAMES: &'static [&'static str] = &["id"];
    fn params() -> Vec<::roundhouse::doc::Param> { … }   // id => integer/int64, required
}
```

Field types map to parameters through `doc::ApiSchema` (the crate's own trait, which produces
`utoipa::openapi::schema::RefOr<Schema>`): `i64 → integer/int64`, `Uuid → string/uuid`,
`String → string`, unit enums → `string` + `enum`.

### 4.4 `#[derive(QueryParams)]` (v0.2)

Same idea for `Query<T>`: every field becomes a query parameter with `required = !Option<_>`.

### 4.5 Loud derivation

`#[handler]` reads the signature's syntax, not its types. That is forced — §4.1 — so the parser will
always be incomplete, and the incompleteness is the risk. A parser that answers "nothing to document"
whenever it does not recognise a type emits exactly the document this project exists to prevent.

Three outcomes, no fourth:

| Outcome | Example | Result |
|---|---|---|
| **Derived** | `-> Json<Post>`, `Json<CreatePost>`, `Path<PostId>` | documented |
| **Declared** | `-> impl IntoResponse` with `responses(200)` | documented, because you asked |
| **Undeclared** | `-> impl IntoResponse`, no `responses(..)` | **build error** at the table entry |

Rules:

- **No empty success path.** An operation that derives no 2xx/3xx response and declares none fails
  `App::build()`, naming the table entry and the handler's line, exactly like a duplicate route.
- **Foreign extractors are visible, not silent.** A signature naming `axum::Json` instead of
  `roundhouse::Json` must still compile — surface check 1 depends on it — so the failure moves to the
  document: the operation carries `x-roundhouse-unverified-body: true`, and `App::build()` prints one
  line per such route. `App::strict()` turns that line into an error. The default is loud, not silent.
- **`-> StatusCode` is a declared shape.** It documents a `default` response with no body, because the
  status is chosen at runtime. Not an omission; the honest shape.
- **Attributes either work or are rejected.** `deprecated` → `deprecated: true`; `public` →
  `x-roundhouse-public: true` and no other effect (it states intent; it does not change access).
  Neither stays a parsed no-op. `responses(200 = Post)` uses `Post` or is a compile error — parsing a
  type and discarding it is the same lie one level down.
- **A library's table follows the same rules.** The build that includes it is the one that fails.

The cost is a word on a few table entries. The benefit is that "the document is derived from your
code" stops meaning "derived, unless the parser blinked".

---

## 5. Registry and resolution

### 5.1 Records

```rust
pub struct RouteRecord {
    pub module_path: &'static str,       // "blog::post" (via module_path!())
    pub file: &'static str,
    pub line: u32,
    pub method: http::Method,
    pub path: &'static str,              // "/{id}" — leaf only
    pub operation_id: &'static str,
    pub summary: Option<&'static str>,
    pub description: Option<&'static str>,
    pub tags: &'static [&'static str],
    pub security: &'static [&'static str],
    pub state: fn() -> TypeId,               // TypeId::of::<AppState>() — erases the generic
    pub handler: fn() -> Box<dyn Any>,       // Box::new(MethodRouter::<AppState>) — see §3
    pub docs: fn(&mut doc::Components) -> doc::Operation,   // roundhouse wrappers over utoipa types
}

pub struct MountRecord {
    pub module_path: &'static str,
    pub file: &'static str,
    pub line: u32,
    pub prefix: &'static str,            // "/blog", or "" for none
    pub tags: &'static [&'static str],
    pub security: &'static [&'static str],
    pub layer: Option<fn() -> LayerFactory>,   // fn item = const; factory builds the boxed layer
    pub layer_mode: LayerMode,
}
```

`LayerFactory` is `Box<dyn Fn(Router<AppState>) -> Router<AppState> + Send + Sync>`. The macro emits
a small `fn __layer() -> LayerFactory { Box::new(move |r| r.layer(TraceLayer::new_for_http())) }`;
the function item is const, the box is created at startup.

### 5.2 Resolution algorithm

```
for each route r:
    ancestors = mounts where m.module_path is an ancestor-or-self of r.module_path
    sort ancestors by segment count ascending
    prefix     = join(ancestors.map(|m| m.prefix))
    full_path  = join(prefix, r.path)

ancestor(m, r) := m.module_path == r.module_path
               || r.module_path.starts_with(m.module_path + "::")
```

`join` rules:

| Step | Rule |
|---|---|
| trim | trailing `/` from left side, leading `/` from right side |
| empty | `"" + "/x"` → `/x`; `"/x" + ""` → `/x`; both empty → `/` |
| collapse | repeated `/` → single `/` |
| preserve | `{id}` and `{*rest}` are opaque; no decoding, no case folding |

Resolution happens once, in `App::build()`, and produces a sorted `Vec<ResolvedRoute>` that is
also the input to OpenAPI generation. Same data, two consumers — the document cannot drift from the
router.

### 5.3 Conflicts and diagnostics

Resolution is fallible, and `App::build()` is where it fails — with a two-line report, not a bare
panic:

```
error: duplicate route
  GET /blog/{id}   blog::post::get      src/routes/blog/post.rs:12
  GET /blog/{id}   blog::post::get_one  src/routes/blog/get.rs:8
```

| Check | Phase | Failure |
|---|---|---|
| `Path<T>::NAMES` equals the template's captures | compile time (`const`) | assertion naming both sides |
| `:id` syntax, empty path, missing leading `/` | macro expansion | `compile_error!` at the literal |
| two `mount!` in one module | macro expansion | duplicate const name — Rust's own error is the right one |
| duplicate `(method, normalised template)` | build | `BuildError::DuplicateRoute` |
| duplicate `operationId` | build | `BuildError::DuplicateOperationId`, both sources, hint to set one |
| mount with no routes below it | build | warning, not a failure |
| scheme name registered with two kinds | build | `BuildError::SchemeKindConflict` |
| parameter declared twice with different schemas | build | `BuildError::ConflictingParameter` |

Two rules make the duplicate check honest:

- **Templates are normalised before comparison.** `{id}` and `{post_id}` are the same route shape to
  `matchit`; the framework must say so before axum panics with a worse message. Normalise captures to
  `{}` and wildcards to `{*}`, then compare.
- **`operationId` is unique.** OpenAPI requires it and client generators rely on it. The default is
  the function name, which collides the moment two modules both define `show` — so a collision is a
  build error, not an automatic rename. Silent renames break clients; a build error costs one
  `operation_id` string.

`App::try_build() -> Result<Built, BuildError>` returns those diagnostics as values, so a server can
log and exit instead of unwinding and a test can assert on the error instead of `#[should_panic]`.
`App::build()` remains the panicking convenience and is implemented in terms of `try_build()`.

Only the first row can fail at compile time; the rest are build-time by construction, and that is why
they are `BuildError` variants rather than panics.

### 5.4 Scoping: which routes belong to this app

Discovery is global by default, which means a linked dependency's routes would join your app.
Topcoat solves this by scoping — `module_router!()` takes `module_path!()` at its call site and only
installs records beneath that prefix — and so do we:

```rust
// src/main.rs
let app = roundhouse::app!(state);   // == App::new(state).scope(module_path!())
```

`module_path!()` in `main.rs` is the crate name, so the default scope is "everything in this crate,
nothing from dependencies". A route library opts in explicitly:

```rust
// in the library
pub const ROUTES: &str = module_path!();          // "my_routes"

// in the app
roundhouse::app!(state)
    .scope(module_path!())
    .include(my_routes::ROUTES)
```

Matching is on `::` boundaries: `app` must not claim `application::*`. That is the bug this kind of
code usually ships with, so the test list is: `app` owns `app` and `app::users`, but not
`admin::users` and not `application::users`.

Scoping resolves three things at once — the supply-chain question (§15), two independent routers in
one binary, and the library-crate story, where the library names its own root instead of demanding
that the app adopt it.

---

## 6. Router assembly

```
Router::<AppState>::new()
 ├── module "" (root)
 │    ├── route GET /healthz
 │    └── nest /blog ──► module "blog"        [layer: Trace]
 │                         └── route GET /{id} ──► /blog/{id}
 └── global layers (App::layer)
```

Order of application, outermost first:

1. `App::layer(layer)` — global, wraps everything including 404s. `App::layer_documented(layer, doc)`
   when the layer terminates requests and you want that in the document (§6.1)
2. module layers, outermost module first
3. entry layer (`layer = ..` in `routes!`)
4. the handler

Rules that need stating because axum is subtle here:

- A module's layer is applied to the module's own `Router` **before** it is nested into its parent,
  so it sees the full path (`/blog/{id}`), not the stripped one. This is the behavior people expect
  from `nest`, and axum gives it for free with this construction order.
- `layer_mode = route` uses `route_layer`, so requests that match nothing under the prefix fall
  through to the parent (no 404 from a middleware that only applies to matched routes).
- Fallback: `App` installs `fallback(404 json)` only if the user has not set one.

### 6.1 Middleware: silence unless declared

A tower layer is opaque: the router can order it, but nothing can see what it returns. So nothing is
added for it. A `429` from a rate limiter stays out of the document until someone says otherwise:

```rust
mount!(
    "/api",
    layer = RateLimitLayer::new(100, Duration::from_secs(60)),
    doc   = doc::layer::rate_limit(429),          // optional
);
```

`doc` is optional, and **absence is a legitimate answer** — no entry, no guess, no "probably a 500".
A client that reads a fabricated `429` stops trusting the rest of the file, so the framework would
rather be incomplete than wrong.

When declared, `LayerDoc` — responses, parameters the layer requires, response headers it adds,
security it enforces — merges into every operation beneath the mount, exactly as extractor docs do
(§7.4). The built-ins cover the layers that terminate requests:

| Helper | Documented effect |
|---|---|
| `rate_limit(429)` | `429` + `Retry-After` response header |
| `timeout(504)` | `504` |
| `body_limit(413)` | `413` |
| `auth(Bearer)` | the same security + `401`/`403` as §7.4 |
| `none()` | nothing observable, stated explicitly |

`mount!(security = ["bearer"])` is sugar for `doc = doc::layer::auth(..)`: one mechanism, two
spellings for the common case.

Merge rules match extractors: responses union by status and collapse to `Problem` on conflict,
response headers union, security ANDs, parameters dedupe by `(location, name)`. Docs are a *set* —
runtime order stays §6's.

Out of reach, stated once: a layer that rewrites responses (compression, transcoding) is not
expressible beyond its headers, and layers applied through the raw `Router` escape hatch are outside
the guarantee by definition.

---

## 7. OpenAPI: their model, our derivation

`utoipa` stays — but only as the **document model**. We use
`utoipa::openapi::{OpenApi, Components, PathItem, Operation, RequestBody, Response, Schema}` and its
3.1 serializer. We never ask users for `#[utoipa::path]`, and our derive emits utoipa's builders
instead of requiring `#[derive(ToSchema)]`.

The gap against ASP.NET was never JSON Schema serialization — utoipa does that well. It is the layer
above: utoipa's macros make you restate, per route, what the handler signature and the module tree
already know. So the split is:

| Layer | Owner |
|---|---|
| OpenAPI 3.1 model + serialization | `utoipa` |
| Everything that fills it: params, bodies, files, errors, security | `roundhouse` |

No fork, no hand-written JSON writer, no schema-format treadmill — and the inference is ours.

### 7.0 What may enter your document

One line: **the framework documents what it guarantees and what you declare. It never guesses about
what it cannot see.**

| Tier | Example | Automatic? |
|---|---|---|
| **Certain by construction** — the framework owns the code that produces it | `Json<T>` → `400`/`415`; `#[schema(min_length = 1)]` → `400`; `Auth<A>` → `401`/`403`; `-> Json<T>` → `200` | yes |
| **Declared** — you wrote it | `responses(404)`, `doc = layer::rate_limit(429)`, `mount!(security = …)` | yes, because you asked |
| **Opaque** — the framework cannot see it | a tower layer, `impl IntoResponse`, a hand-mounted `Router` | never |

Why the first row is not "modifying your schema": `Json<Post>` in a signature *is* a declaration, and
the extractor that rejects a malformed body is the framework's own code. The `400` is not an
inference about your program, it is the same code path, documented. A `RateLimitLayer::new(100)` is
the opposite: nothing about it is readable, and a fabricated `429` would poison trust in every other
entry.

That distinction is the test for every future feature: *can the framework point at the code that
guarantees this?* If yes, document it. If no, require a declaration or stay silent.

`App::strict_docs()` (v0.3) drops even the first row, leaving only what you declared — for teams that
want the document to be a contract they signed line by line rather than partly derived.

### 7.1 Derivation: `#[derive(ApiSchema)]`

```rust
#[derive(Serialize, Deserialize, ApiSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Post {
    pub id: i64,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtitle: Option<String>,
    #[schema(example = "hello", min_length = 1, max_length = 200)]
    pub body: String,
    pub tags: Vec<String>,
    pub author: Author,
}
```

serde is the source of truth. The derive reads the same attributes serde does, so the document
cannot disagree with the wire format, and it emits utoipa's builders:

```rust
impl ApiSchema for Post {
    fn schema() -> RefOr<utoipa::openapi::Schema> {
        ObjectBuilder::new()
            .property("id", Integer::new(Int64))
            .required("id")
            .property("title", StringSchema)
            .required("title")
            .property("subtitle", string_or_null())         // Option<String>, not required
            …
            .into()
    }
}
```

| Rust / serde | Emitted |
|---|---|
| `Option<T>` | `type: [T, "null"]` (3.1 — not `nullable`) |
| `#[serde(rename_all = "camelCase")]` | property names |
| `#[serde(rename = "x")]` | one property |
| `#[serde(skip)]`, `skip_serializing` | omitted |
| `#[serde(skip_serializing_if = …)]`, `#[serde(default)]` | not in `required` |
| `#[serde(flatten)]` | `allOf` |
| `#[serde(deny_unknown_fields)]` | `additionalProperties: false` |
| `#[serde(tag = "type")]` | `oneOf` + `discriminator.propertyName` |
| `#[serde(tag = "t", content = "c")]` | `oneOf` of `{t: const, c: …}` |
| `#[serde(untagged)]` | `oneOf` |
| `HashMap<String, T>` | `additionalProperties: {T}` |
| `i64` / `u64` | `integer/int64` / `integer/uint64` + `minimum: 0` |
| `f64` | `number/double` |
| `Vec<u8>` | `string/byte` (json) or `string/binary` (octet-stream context) |

Foreign types get `impl ApiSchema` in the core: `Uuid`, `chrono::{DateTime, NaiveDate}`,
`time::OffsetDateTime`, `rust_decimal::Decimal`, `IpAddr`, `serde_json::Value`, `bytes::Bytes`.

Already have `#[derive(ToSchema)]` types? `#[schema(via = utoipa)]` calls that impl and returns the
same `RefOr<Schema>` — no conversion, no wrapper, no extra dependency. That is the whole interop
story, because both sides speak utoipa's model.

### 7.2 Signature → request and response bodies

The argument type picks the media type; the return type picks the response. Both are already in the
signature, so nothing is declared twice.

| Handler signature | Document |
|---|---|
| `Json<T>` | request `application/json`, schema `T` |
| `Form<T>` | request `application/x-www-form-urlencoded` |
| `Multipart<F>` | request `multipart/form-data`, `FilePart` → `string/binary` |
| `Bytes` / `Body` | request `application/octet-stream` (or `*/*` with `content_type`) |
| `-> Json<T>` | 200 `application/json` |
| `-> Html<T>` | 200 `text/html` |
| `-> Attachment<F>` | 200, media type from `F`, plus `Content-Disposition` |
| `-> File` | 200 `application/octet-stream` |
| `-> Sse<S>` | 200 `text/event-stream` |
| `-> StatusCode` / `-> ()` | no content |

```rust
// routes! { post("/{id}/avatar", upload, summary = "Upload an avatar") }

#[handler]
pub async fn upload(
    Path(PostId { id }): Path<PostId>,
    Multipart(form): Multipart<AvatarForm>,       // defined below
) -> Result<Json<Avatar>, AppError>;

// routes! { get("/{id}/avatar.png", download) }

#[handler]
pub async fn download(/* … */) -> Result<Attachment<Vec<u8>>, AppError> {
    Ok(Attachment::png(bytes).filename("avatar.png"))
}
```

`Multipart` and `Attachment` are ours (thin wrappers over `axum::extract::Multipart` and
`IntoResponse`), so the runtime and the document share one definition of "what a file is".

#### `Multipart<F>` in detail

axum's `Multipart` is a stream of untyped `Field`s: order-independent, impossible to document, and
reimplemented differently in every handler that touches it. `Multipart<F>` wraps the same stream and
hands you a struct.

```rust
#[derive(Multipart, ApiSchema)]
pub struct AvatarForm {
    pub file: FilePart,                                        // required, buffered
    #[file(max_bytes = 5_000_000, accept = ["image/png", "image/jpeg"])]
    pub thumbnail: Option<FilePart>,
    pub tags: Vec<String>,                                     // repeated field name
    pub caption: Option<String>,
}
```

> **Status (0.2).** The v0.1 extractor collects every part into a `Vec<Part>` *before* field logic
> runs. That makes the rules below aspirational: `max_bytes` is checked after buffering, two private
> constants (16 MiB per part, 32 MiB total) override larger declarations, `TempFile` copies from
> memory instead of streaming, `TempFile::read` rewinds on every call, and a duplicate single-valued
> field is silently dropped. The rewrite is part of 0.2 (§13), and the trait it freezes is
> `fields() -> &'static [FieldSpec]` — not `from_parts(Vec<Part>)`.

The derive emits a **field table** — `name → Kind`, plus each field's `max_bytes` and `accept` — so the
extractor never has to collect first:

1. Parse the `Content-Type` boundary. Anything else → `415` as a `Problem`.
2. Look the part's `name` up in the field table. Unknown names are ignored by default;
   `#[multipart(deny_unknown)]` makes them a `400`. A second part for a single-valued field is a `400`
   (`duplicate_field`), not a silent first-wins.
3. Text parts buffer up to the field limit, then deserialize through serde, so an enum with
   `#[serde(rename_all)]` works in a form exactly as it does in JSON.
4. File parts are written **as they arrive**: `FilePart` into a bounded buffer, `TempFile` straight
   into the already-unlinked file. `max_bytes` → `413` and `accept` → `415` are decided while the
   bytes are still on the wire, not after.
5. At end of stream, missing required fields become one `400` listing all of them, with pointers.

| Field type | Behavior |
|---|---|
| `FilePart` | buffered, bounded by the field's `max_bytes`; `{ bytes, file_name, content_type }` |
| `TempFile` | chunks written to an already-unlinked file during the read; `Read` + `Seek` from 0 |
| `Vec<FilePart>` / `Vec<TempFile>` | repeated names in arrival order; each subject to the field limit |
| `Vec<String>` / scalars | repeated or single text parts, serde-parsed |
| `Json<T>` (v0.3) | a part whose `Content-Type` is `application/json` |

Limits are one knob with narrowing, not two competing ones: `App::max_body(32 MiB)` is the global
default (every body extractor, not only multipart), `#[file(max_bytes = N)]` narrows a field, and
`App::max_parts(n)` bounds the part count — many tiny parts are a DoS axis of their own. Exceeding any
of them is a `413`; never a silent truncation, and never a buffer that grows until the process dies. A
zero-byte file is valid and distinguishable from a missing one.

**OpenAPI 3.1 has no standard maximum-size keyword for binary parts**, so the schema carries what is
standard and the limits ride alongside as vendor extensions — an honest limitation instead of a
pretend one:

```yaml
requestBody:
  content:
    multipart/form-data:
      schema:
        type: object
        required: [file]
        properties:
          file:      { type: string, format: binary }
          thumbnail: { type: string, format: binary, x-max-bytes: 5000000, x-accept: [image/png, image/jpeg] }
          tags:      { type: array, items: { type: string } }
          caption:   { type: ["string", "null"] }
```

Security rules, because filenames are attacker input:

- `file_name` is reduced to its basename with `..` and NUL stripped; the original is kept as metadata
  only and never used to build a path.
- A part's `content_type` is client-supplied and untrusted. `accept` is a convenience filter, not a
  security control — anything that matters must be verified from the bytes.
- `accept` matches the media type only: case-insensitive, parameters stripped, so
  `Content-Type: text/plain; charset=utf-8` matches `accept = ["text/plain"]`. Exact string equality
  makes the filter unusable with real clients, and that is what 0.1 does.
- `TempFile` writes to an already-unlinked path, so a crash leaves no orphaned uploads.

Failures reuse the `Problem` type from §7.6, so a missing field, a wrong content type and a size
violation all carry the same JSON Pointer (`/thumbnail`) and one client parser handles all three.

### 7.3 Responses are a type, not a `Result`

`AppError` is not special. It is the error-shaped instance of one mechanism, and the same derive
works on any enum — including the success path:

```rust
#[derive(ApiResponse)]
pub enum CreatePost {
    #[status(201, description = "Created")]
    Created {
        #[body]
        body: Json<Post>,
        #[location]
        location: String,
    },

    #[status(409, problem = "SlugTaken")]
    SlugTaken { slug: String },

    #[status(422)]
    Invalid(#[from] ValidationErrors),
}

// routes! { post("/posts", create) }

#[handler]
async fn create(Json(body): Json<NewPost>) -> CreatePost { … }
```

The derive emits both halves — `IntoResponse` and `ApiResponse::responses()` — so the operation is
*exactly* the variants. No inference, no `responses(...)` attribute, and adding a variant changes the
document (and the golden file) in the same commit, which is the review you want anyway.

Rules:

- every variant carries `#[status(..)]`, or the derive refuses to compile;
- a variant with a payload contributes that body + its schema; without one, `Problem` when the status
  is ≥ 400 and no content otherwise;
- `#[from]` gives `impl From<Source>`, so `?` works inside the handler;
- 5xx bodies are generic in release builds, schema unchanged;
- `#[body]` marks the payload when the variant also carries headers; a lone unnamed field is the
  body by default, so the annotation is only needed when there is a choice.

#### Headers are fields

axum gives a response three ways to set a header: a `HeaderMap` in the tuple
(`(StatusCode, HeaderMap, T)`), `response.headers_mut().insert(..)`, or `AppendHeaders`. All three
are invisible to a document, because a `HeaderMap` is a runtime value with no schema. Here a header
is a field of the variant, and the derive emits both the runtime insertion and the `headers` map:

| Field | Documented as | Runtime |
|---|---|---|
| `#[header("X-Request-Id")] id: Uuid` | `headers: { X-Request-Id: { schema } }` | inserted |
| `#[header("Set-Cookie", append)] cookie: Cookie<'static>` | same, `array` | appended (multi-value) |
| `#[header("Retry-After")] after: Option<u64>` | `required: false` | omitted when `None` |
| `#[location] url: String` | `Location`, `format: uri` | inserted |

The value must convert to a `HeaderValue` (`TryInto<HeaderValue>`); a conversion failure is a
programming error, so it answers `500` `Problem` rather than a `400`. Headers are never inferred:
`(StatusCode, HeaderMap, T)` keeps working, and stays undocumented — the §7.0 rule applied to
headers.

`Result<T, E>` is then pure sugar over the same trait, and the return shapes are:

| Return type | Documented responses |
|---|---|
| `Result<T, E>` | `Ok(T)` at the status `T` implies, plus `E`'s variants |
| `T` (plain) | one response, `T`, at the status `T` implies |
| `MyEnum` (`ApiResponse`) | exactly its variants, headers included |
| `(StatusCode, HeaderMap, T)` | status + body; **headers undocumented** |
| `impl IntoResponse` | whatever `responses(...)` declares — the one case where the document is a claim, not a derivation |

`Result<T, E>` is then pure sugar over the same trait, and the four return shapes are:

| Return type | Documented responses |
|---|---|
| `Result<T, E>` | `Ok(T)` at the status `T` implies, plus `E`'s variants |
| `T` (plain) | one response, `T`, at the status `T` implies |
| `MyEnum` (`ApiResponse`) | exactly its variants |
| `impl IntoResponse` | whatever `responses(...)` declares — the one case where the document is a claim, not a derivation |

Status inference for `Ok(T)` is deliberately shallow, because deep inference is where documents start
lying: `Json<T>` → 200, `(StatusCode, Json<T>)` → that status, `StatusCode` → no body, `()` → 204.
Anything more interesting deserves an enum.

Why keep `Result` at all: most handlers have one success shape and one error enum, and a union there
is noise. `ApiError` survives as the error-only flavour of the derive: same expansion, plus it
enforces 4xx/5xx statuses and requires `Display`, so an error enum can never quietly become the
success path. Start with `Result`; promote a route to an enum the day it grows a second success
shape, or needs a `Location` header, or has two kinds of 409.

The trade to accept: an explicit union makes the document exact and the diff reviewable, but it costs
one variant per outcome — and the compiler will not tell you that you forgot a 404, only that you
wrote one without a status.

A `#[test]` the derive emits asserts that every variant has a status and that no two variants share
one without distinct schemas.

### 7.4 Extractors declare their own documentation

An extractor is the only place that knows what a handler reads from the request. So an extractor is
also the thing that documents it — the route macro collects a declaration from each parameter type
and merges them into the operation.

```rust
pub trait ExtractorDoc {
    /// Parameters read from path / query / header / cookie.
    fn params() -> Vec<Param> { vec![] }

    /// The request body, if this extractor consumes one.
    fn body() -> Option<BodyDoc> { None }

    /// Responses produced before the handler runs.
    fn failures() -> Vec<FailureDoc> { vec![] }

    /// Security this extractor requires.
    fn security() -> Vec<SecurityRequirement> { vec![] }

    /// Drives the "body extractor must be last" diagnostic.
    const CONSUMES_BODY: bool = false;
}
```

| Extractor | Params | Body | Failures | Security |
|---|---|---|---|---|
| `State<S>` | — | — | — | — |
| `Extension<T>` | — | — | 500 | — |
| `Path<T>` | path | — | 400 | — |
| `Query<T>` | query | — | 400 | — |
| `Header<T>`, `TypedHeader` | header | — | 400 | — |
| `Json<T>` | — | `application/json` | 400, 415 | — |
| `Form<T>` | — | `x-www-form-urlencoded` | 400, 415 | — |
| `Multipart<F>` | — | `multipart/form-data` | 400, 413, 415 | — |
| `Auth<A>` | — | — | 401, 403 | `A` |
| `Option<Auth<A>>` | — | — | — | `A`, optional |
| `Cookie<T>` (v0.2) | cookie | — | 400 | — |

Merging rules, applied when the document is built:

- **Params** dedupe by `(location, name)`. The same name declared twice with different schemas is a
  build error naming both extractors — the alternative is a document where one of them silently wins.
- **Body** at most one. Two body extractors is already an axum `Handler` error; `CONSUMES_BODY` lets
  the macro say "`Json<Post>` and `Bytes` both read the body" instead.
- **Failures** dedupe by status code. Same status from two extractors with different schemas collapses
  to `Problem`, so the operation never claims two different shapes for one `400`.
- **Security** unions: schemes from different extractors are **AND**-ed within one requirement.

Third-party extractors are documented by implementing `ExtractorDoc` in six lines; anything that does
not implement it contributes nothing and stays invisible, which is the right default for `State`-like
extractors.

#### Security extractors

`Auth<A>` is an ordinary extractor that happens to carry a scheme, so the moment it appears in a
signature the operation requires it — no annotation to keep in sync, and no way to enforce auth
without documenting it.

```rust
pub struct Bearer;

impl AuthScheme for Bearer {
    const NAME: &'static str = "bearer";
    const KIND: SecuritySchemeKind = SecuritySchemeKind::Http { scheme: "bearer" };
    const UNAUTHENTICATED: StatusCode = StatusCode::UNAUTHORIZED;
    const FORBIDDEN: Option<StatusCode> = Some(StatusCode::FORBIDDEN);
}

// routes! { get("/admin/stats", stats) }

#[handler]
async fn stats(_auth: Auth<Bearer>) -> Json<Stats> { … }
```

→ `security: [{bearer: []}]`, a `401` with `WWW-Authenticate: Bearer`, and a `403`.

The three shapes that matter:

| Signature | OpenAPI |
|---|---|
| `Auth<Bearer>` | `security: [{bearer: []}]` — required |
| `Option<Auth<Bearer>>` | `security: [{bearer: []}, {}]` — OpenAPI's spelling of optional |
| `Auth<Bearer>` + `Auth<ApiKey>` | `security: [{bearer: [], apiKey: []}]` — both required |

`Option<Auth<A>>` deserves its own warning: it is the one signature where a missing guard is
invisible to the type system. A handler that never matches on `None` is an open endpoint that reads
as intentional, and it will be documented that way too. Deliberately public routes should say so —
a `public` flag on the table entry — and a `debug_assert` flags any handler that takes `Option<Auth<_>>` without
using it.

Scopes are the one place where the declaration needs help, because a Rust type cannot invent an
OAuth scope string: `Auth<User, scope::ReadPosts>` carries `["read:posts"]` into the requirement, or
`security(bearer, scopes = ["read:posts"])` on the table entry narrows it A `mount!(security =
["bearer"])` covers routes protected by middleware instead of an extractor, and contributes the same
`401`/`403` to every operation underneath.

Scheme conflicts are a build error, not a surprise at runtime: the same `NAME` registered with two
different `KIND`s panics with both file:line locations, and schemes no route uses never appear in
`components.securitySchemes`.

#### Rejections

Every extractor failure is already a `Problem` (§7.6), which is why the operations in §7.2 can list
`400`/`413`/`415` without any per-route attributes. Two rules keep that from turning into noise:

- `App::document_rejections(..)` — `Auto` by default (add every extractor failure), `None` for teams
  that publish a curated surface.
- If the parameter is written as `Result<Json<T>, JsonRejection>`, the framework documents the
  *input* but adds no failure response: the handler has taken responsibility for shaping that error,
  so guessing its status would be a lie.

### 7.5 Assembly and the golden test

```rust
let doc = doc::build(OpenApi::new(Info::new("Blog API", "0.1.0")), &built.routes);
let json = doc.to_json()?;              // no server needed
```

Paths and operations come from the same resolved table used to build the `Router`, so the document
cannot describe a route that does not exist. Components dedup by type name; collisions get the module
path (`blog.Post`).

`tests/openapi.golden.json` + `roundhouse::assert_openapi_matches!()`: any accidental route rename or
response change fails CI with a diff, reviewed like a migration. Served at `GET /openapi.json` when
configured (debug default).

### 7.6 Constraints enforce themselves

The same `#[schema(...)]` attribute that documents a constraint also enforces it. Not a second
annotation, not a separate validator crate: one token stream emits the JSON Schema keyword and the
check, so the document cannot promise something the server does not do.

> **Revision 2 note.** The declaration surface below is unchanged. *Where the second half runs* is not
> settled: from 0.2 the emitted schema executes at runtime (§17), and the generated `validate()` keeps
> only what a schema cannot express. Read "Enforced as" as "what the reader may assume", not "where the
> code lives".

```rust
#[derive(Deserialize, ApiSchema)]
pub struct CreatePost {
    #[schema(min_length = 1, max_length = 200)]
    pub title: String,

    #[schema(minimum = 0, maximum = 1_000_000)]
    pub views: u64,

    #[schema(pattern = r"^[a-z0-9-]+$")]
    pub slug: String,

    #[schema(min_items = 1, max_items = 5)]
    pub tags: Vec<String>,
}
```

`#[derive(ApiSchema)]` fills in **both halves of the trait** — `schema()` and `validate()` — and the
extractors call them. The extractors are the crate's own — `Json<T>`, `Query<T>`, `Form<T>`,
`Path<T>`, `Multipart<T>` — which is what makes enforcement uniform and every rejection share one
body shape:

```rust
pub trait ApiSchema {
    fn schema() -> RefOr<Schema>;

    /// No-op by default: constraints opt in by appearing in `#[schema(..)]`.
    fn validate(&self) -> Result<(), ValidationErrors> {
        Ok(())
    }
}

impl<T, S> FromRequest<S> for Json<T>
where
    T: DeserializeOwned + ApiSchema,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(req: Request, state: &S) -> Result<Self, Problem> {
        let axum::Json(value) = axum::Json::<T>::from_request(req, state)
            .await
            .map_err(Problem::from_rejection)?;      // malformed JSON → 400, same shape
        value.validate().map_err(Problem::from_validation)?;   // constraint → 400
        Ok(Self(value))
    }
}
```

| `#[schema(..)]` | Emitted | Enforced as |
|---|---|---|
| `min_length` / `max_length` | `minLength` / `maxLength` | `chars().count()` (not bytes) |
| `pattern` | `pattern` | regex compiled once in a `OnceLock` |
| `minimum` / `maximum` | `minimum` / `maximum` | numeric comparison |
| `exclusive_minimum` / `exclusive_maximum` | matching keywords | numeric comparison |
| `min_items` / `max_items` | `minItems` / `maxItems` | `len()` |
| `unique_items` | `uniqueItems` | `HashSet` insert |
| `format = "email" \| "uuid" \| "url"` | `format` | format check (v0.2) |
| `validate_with = path` | — | `path(&value) -> Result<(), …>` |

Response on violation — RFC 9457 plus JSON Pointer, so the client can map errors to the exact field
it sent:

```json
{
  "type": "https://roundhouse.rs/problems/validation",
  "title": "Validation failed",
  "status": 400,
  "errors": [
    { "pointer": "/title", "code": "min_length", "message": "must be at least 1 character", "limit": 1 },
    { "pointer": "/tags",  "code": "max_items",  "message": "must contain at most 5 items", "limit": 5 }
  ]
}
```

Rules:

- **400 by default.** `App::validation_status(422)` for people who prefer the semantic-code
  split; either way it is one setting, not a per-route decision.
- **All violations at once**, collected rather than short-circuited. Clients should not have to fix
  one field, resubmit, and discover the next.
- **Pointers follow the request body**, including nesting (`/author/name`) and arrays (`/tags/2`).
- `Option<T>` validates only when `Some`; `#[serde(flatten)]` fields contribute to the parent
  pointer.
- Malformed JSON, wrong `Content-Type`, and missing query parameters produce the *same* `Problem`
  shape, so a client has exactly one error parser.
- Opt-outs: `validate = false` on the table entry, `#[schema(skip_validation)]` for a
  field that is documented but checked elsewhere.

This is the concrete difference from both directions: `utoipa` documents without enforcing,
`validator`/`garde` enforce without documenting. A constraint here is a single declaration with two
outputs, which is why §7.7 can be a conformance test instead of a promise.

#### 7.6.1 What changes when the schema executes

Four decisions the two-half design hides, and that §17 makes explicit:

- **`skip_validation` loses its meaning and must go.** Today it keeps the keyword in the document and
  skips the check, which is defensible only if "checked elsewhere" really enforces the same rule. Once
  the schema is the validator there is no "skip here": the keyword is published and enforced, or
  neither. The escape hatch becomes `validate = false` on the entry, which skips the whole body and is
  visible in the table.
- **`format` becomes an assertion on purpose.** 2020-12 defines `format` as an annotation; validators
  ignore it unless told otherwise. This framework tells it otherwise
  (`should_validate_formats(true)`) and says so, because a `format: email` nobody checks is a lie in
  the other direction. `url` is a custom format — JSON Schema's standard name is `uri`; either the
  emit becomes `uri` or the engine learns `url`. Pick one and test both.
- **`validate_with` stays Rust.** A predicate over a typed value is not expressible in the schema, so
  it runs after the schema passes, contributes the same `Problem` shape, and is published as
  `x-validate-with: true` so the reader knows a constraint exists that the document does not spell
  out. This is the one sanctioned second half.
- **Multipart is the exception until 0.3.** File parts are bytes, not JSON; field checks stay
  generated and the schema keeps `string/binary` plus the vendor limits. Stated here so it is a
  decision rather than an oversight.

### 7.7 Constraint conformance harness

The harness is a 0.1 acceptance test, not a v0.3 nicety. For every type in the test suite it runs the
same fixture through both halves and asserts they agree:

1. **Verdict parity.** `value.validate()` (generated) and `jsonschema::is_valid(&value, &schema)` must
   return the same boolean for every fixture and every generated violation.
2. **Pointer parity.** The generated `ValidationError.pointer` must equal the engine's `instance_path`
   for the same violation.
3. **Schema validity.** Every emitted component passes `jsonschema::meta::is_valid` — the document is
   itself a valid schema, the cheapest proof that the emitter is not improvising.
4. **Serde parity.** `serde_json::from_str::<T>(fixture)` succeeds exactly when the schema accepts the
   fixture. This is where `flatten` + `deny_unknown_fields` (`allOf` plus
   `additionalProperties: false`), `1.0` as an integer, `Option` versus `null`, and
   `skip_serializing_if` are confirmed or found broken.

Fixtures come from the example apps' types, so the harness covers what users actually write. The first
two keep the halves honest while they coexist; the second two find emitter bugs no hand-written unit
test was going to find.

### 7.8 Response validation is the same engine

`App::validate_responses()` must not be a third implementation of schema semantics. The current
`doc/response.rs` is a subset (types, `required`, `properties`, arrays, `oneOf`/`allOf`/`anyOf`,
`additionalProperties`) that silently ignores `minLength`, `pattern`, `format`, numeric bounds and
`uniqueItems` — so a response can violate the document and pass validation.

Response validation reuses the engine from §17: at build time each `(method, path, status)` maps to a
compiled validator; the middleware reads the body, validates, and rebuilds the response. Same error
mapping, same messages, one dependency. The subset interpreter is deleted, and the "the document lies"
test becomes stronger because the engine knows every keyword the emitter can produce.

---

## 8. Runtime API

```rust
// server
App::<AppState>::new(state)
    .layer(TraceLayer::new_for_http())
    .openapi(|| Info::new("Blog API", env!("CARGO_PKG_VERSION")))
    .serve("0.0.0.0:3000")
    .await?;

// embed in an existing axum app — the state is already provided inside
let router: Router<()> = App::new(state).router();
let app = Router::new().nest("/api", router).route("/legacy", get(old_handler));

// tests
let app = App::new(test_state()).try_build()?;
let res = app.oneshot_get("/blog/1").await;     // helper over tower::ServiceExt
assert_eq!(res.status(), StatusCode::OK);
```

`App::build()` returns a `Built` that is `Clone + Send + Sync`, holds the resolved table, and lazily
caches the OpenAPI document behind a `OnceLock`. `App::try_build()` returns
`Result<Built, BuildError>` with the same diagnostics as values (§5.3).

`App::router()` returns `Router<()>`, not `Router<S>`: the state is applied inside, so the `S` would be
a parameter the caller supplies and the framework immediately discards.

---

## 9. Worked example

```
src/
├── main.rs
├── state.rs
├── error.rs
└── routes/
    ├── mod.rs
    ├── health.rs
    └── blog/
        ├── mod.rs
        └── post.rs
```

```rust
// src/main.rs
mod error;
mod routes;
mod state;

pub type AppState = state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let state = AppState::connect().await?;
    roundhouse::app!(state)
        .openapi(|| roundhouse::openapi::Info::new("Blog API", "0.1.0"))
        .serve("0.0.0.0:3000")
        .await
}
```

```rust
// src/routes/mod.rs
pub mod blog;
pub mod health;
```

```rust
// src/routes/health.rs
use roundhouse::prelude::*;

routes! {
    get("/healthz", healthz, summary = "Liveness probe", tags = ["ops"]),
}

#[handler]
pub async fn healthz() -> &'static str {
    "ok"
}
```

```rust
// src/routes/blog/mod.rs
use roundhouse::prelude::*;

pub mod post;

mount!(
    "/blog",
    tags = ["blog"],
    layer = tower_http::trace::TraceLayer::new_for_http(),
);
```

```rust
// src/routes/blog/post.rs
use roundhouse::prelude::*;

#[derive(serde::Deserialize, PathParams)]
pub struct PostId {
    pub id: i64,
}

#[derive(serde::Serialize, ApiSchema)]
pub struct Post {
    pub id: i64,
    pub title: String,
}

#[derive(serde::Deserialize, ApiSchema)]
pub struct CreatePost {
    pub title: String,
    pub body: String,
}

routes! {
    get("/{id}", get, summary = "Read a post"),
    post("/", create, summary = "Create a post"),
}

#[handler]
pub async fn get(
    Path(PostId { id }): Path<PostId>,
    State(db): State<AppState>,
) -> Result<Json<Post>, AppError> {
    let post = db.post(id).await?.ok_or(AppError::not_found(id))?;
    Ok(Json(post))
}

#[handler]
pub async fn create(
    State(db): State<AppState>,
    Json(body): Json<CreatePost>,
) -> Result<(StatusCode, Json<Post>), AppError> {
    let post = db.create(body).await?;
    Ok((StatusCode::CREATED, Json(post)))
}
```

Note there is no `responses(...)` in the table: `AppError` (§7.3) supplies 404/422/500, and each
handler's return type supplies 200/201 with the `Post` schema.

```rust
// src/error.rs
#[derive(Debug, thiserror::Error, ApiError)]
pub enum AppError {
    #[error("post {0} not found")]
    #[status(404, problem = "PostNotFound")]
    NotFound(i64),

    #[error("invalid input")]
    #[status(422)]
    Validation(#[from] ValidationErrors),

    #[error(transparent)]
    #[status(500)]
    Internal(#[from] anyhow::Error),
}
```

Resolved:

| Method | Path | Source | Middleware |
|---|---|---|---|
| GET | `/healthz` | `routes::health::healthz` | global |
| GET | `/blog/{id}` | `routes::blog::post::get` | global → Trace |
| POST | `/blog` | `routes::blog::post::create` | global → Trace |

Note `POST "/"` + `mount!("/blog")` → `/blog`, because `join` trims the trailing slash. Being able
to write `"/"` for "the module's own path" is worth the normalization rule.

---

## 10. Errors

```rust
pub trait ApiError: IntoResponse {
    fn responses() -> &'static [ResponseDoc] {
        &[]
    }
}
```

`#[derive(ApiError)]` is `#[derive(ApiResponse)]` with a policy: error statuses only, and `Display`
required. Both emit `IntoResponse` + `responses()` from the enum (§7.3). Rules:

- every variant carries `#[status(..)]` or the derive refuses to compile — a response you can
  produce but cannot describe is a bug in the document;
- `#[from]` variants map the source error into `type` + `detail`;
- 5xx bodies are generic in release builds, but their schemas stay;
- `Problem` (RFC 9457 `application/problem+json`) is the default body and ships with the crate;
- route-level `responses(...)` merges with, and overrides, the enum's set;
- validation failures reuse the same `Problem` type and report every violation with JSON Pointers
  (§7.6), so a 400 from a bad body and a 422 from a domain rule share one client-side parser.

---

## 11. Compatibility and surprises

| Topic | Rule |
|---|---|
| axum version | 0.8.x required; 0.8 path syntax (`{id}`, `{*rest}`) — older `:id` is rejected by the macro with a migration hint |
| OpenAPI version | 3.1 only, via utoipa 6's serializer; a 3.0 downgrade pass is a v0.3 opt-in |
| utoipa version | 6.x; re-exported so users never depend on it directly (open question 5) |
| state | carried per record and checked at startup by `TypeId` (§3), so a route declares its own; erased mode names none at the cost of the compile-time guarantee |
| Linking | the crate must be linked, i.e. `main.rs` must mention `roundhouse` — the re-export in the prelude does it; documented because it is the #1 confusion in `inventory`-based crates |
| Extractors | `Json`, `Query`, `Form`, `Path`, `Multipart` come from `roundhouse::prelude`, not `axum::` — the wrappers are where validation and problem-shaped rejections live. Using axum's directly compiles (surface check 1) but marks the operation `x-roundhouse-unverified-body` and prints one startup line per route (§4.5) |
| `#[handler]` in libraries | works: the state type comes from the handler's own `State<T>` (§3). Routes still join the app's registry unless the app scopes discovery (§5.4) |
| IDE | no build script → save-and-run works; `rust-analyzer` expands the macros fine (they are small) |
| ordering | routes are sorted by path, not declaration order; axum's matcher is priority-based, so this does not affect dispatch |
| one subtree, one mount | a `mount!` is a link-time constant: the same subtree cannot be mounted at two prefixes (`/v1`, `/v2`) or built twice against different state. `App::router()` plus `nest` is the way to instantiate twice, and that copy is undocumented by construction |
| merged routers | `App::merge(router)` composes but is invisible to the document and to validation. It is the one-way escape hatch, stated rather than hidden |
| two apps, one binary | possible only with disjoint scopes and the same state type: a record belongs to a scope, not to an app |

---

## 12. Spikes (do these before v0.1 is called design-complete)

Ordered by how much they can invalidate the design.

1. **`module_path!()` inside a proc-macro-generated `inventory::submit!`.** Must expand to the
   containing module (`blog::post`), not to the macro crate. Topcoat 0.11 does exactly this in
   production (`ModuleRouterBuilder::new(module_path!())` plus `inventory::iter`), so this is a
   formality — verify it anyway, in our shape, before §4 is called settled.
2. **Const-evaluability of the record.** `fn()` items, `&'static str`, `&'static [&'static str]`
   are const; `Box` is not. Verify the layer-factory trick (fn item returning a boxed closure).
3. **`MethodRouter<AppState>` behind a `fn()` pointer** — confirm `axum::routing::get(handler)`
   coerces to a `fn() -> MethodRouter<_>` with the right state type inferred.
4. **Const assertion comparing `T::NAMES` with the parsed template** (string comparison in const
   context). Fallback: compare lengths at compile time, names in a debug assertion.
5. **utoipa 6 programmatic assembly** — build a full `OpenApi` from parts (`PathItem`,
   `Components`) without a `#[derive(OpenApi)]` root; confirm the 3.1 output and that components
   dedup by key.
6. **Schema/serde conformance** — for each test type, serialize a fixture and validate it against
   the emitted schema with the `jsonschema` crate. This is how we learn whether `flatten` +
   `rename_all` + `skip_serializing_if` compose the way we think.
7. **`nest` + `layer` ordering** — a 20-line test proving module middleware sees the unstripped
   path and that 404s behave as specified.
8. **Constraint coverage** — for each keyword, confirm three things agree: the emitted schema
   keyword, the runtime check, and the JSON Pointer in the 400. Includes `chars().count()` vs bytes,
   regex compile-once, and pointers through `flatten` + nested structs.
9. **Multipart streaming** — one pass over axum's field stream with limits enforced *during* the
   read (`max_bytes` before buffering, `413` for the rest of the connection), `TempFile` unlink
   semantics, and repeated field names into `Vec<_>`.
10. **Extractor merging** — two `Query<T>` of different types, param name collisions, `Option<Auth<A>>`
    emitting `[{}, {bearer: []}]`, `Result<Json<T>, _>` adding no failures, and the
    "body extractor must be last" diagnostic.

Spikes 1–5 and 7–10 are implemented in this repository; 6 has not been, and is promoted to 0.1 as
§7.7. Three new spikes decide §17:

11. **Schema execution.** Compile one `jsonschema::Validator` per request type from the emitted
    components, validate the raw `Value`, map `kind()` + `instance_path` onto `Problem`. Budget: p99
    under 50 µs for a 20-field body on a warm validator, or the engine stays opt-in.
12. **`$ref` framing.** `{"$ref": "#/$defs/NewNote", "$defs": <components.schemas>}` must resolve
    recursive and sibling references with `offline()`, or the document root is used directly and
    `$defs` is only a rename.
13. **serde/JSON Schema edges.** `flatten` + `deny_unknown_fields`, `integer` versus `1.0`, `format`
    assertion, `pattern` engine (`regex` versus `fancy-regex`), `uniqueItems` on objects. Each becomes
    a fixture in §7.7; each can falsify the engine choice.
14. **Multipart extraction.** Field-table routing, limits decided while reading, `TempFile` written
    during the read plus a `Seek`-based `Read`, duplicate single-value fields rejected, `accept`
    matched by media type. The 0.1 extractor fails all five; §7.2 states the target and the interface
    the rewrite freezes.

---

## 13. Milestones

**v0.1 — the spine (shipped).** `routes!`, `#[handler]`, `mount!`, `PathParams`, registry, prefix
resolution, router assembly, `ApiSchema` over serde, bodies and responses from the signature,
extractor declarations with automatic rejection responses, constraints with JSON Pointers,
`ApiResponse`/`ApiError`, auth from extractors, multipart, files and streams, the golden document, and
the CLI. The README's "What works" is the exact surface; the honesty pass below is not in it yet.

**v0.2 — structure, then honesty.** The crate split (§3): `roundhouse-schema` and
`roundhouse-multipart` leave the facade while the existing tests stay green and the public paths do not
change. The multipart extractor is rewritten to the field-table design (§7.2). Then loud derivation
(§4.5), unique operation ids, `try_build` (§5.3), the conformance harness (§7.7), `trybuild` tests,
`roundhouse routes`, packaging metadata, schema execution opt-in (§17), and response validation on the
engine (§7.8).

**v0.3 — the long tail.** Codegen mode — and the `roundhouse-core` split it needs — generics in
`ApiSchema`, per-module `servers`/`tags`, the docs site, the migration guide, and schema execution as
the default with `validate_at` removed from `ApiSchema`. `schemars` is evaluated only if utoipa is ever
replaced. The 3.0 downgrade shipped early and stays where it is.

Out of scope for 1.0: other engines, DI, code generation of clients (separate crate, later).

---

## 14. Open questions

1. **`mount!` vs `scope!`** — "mount" is accurate but clashes conceptually with axum's `Router::nest`.
   Leaning `mount!` because users never call `nest` here.
2. **Trailing-slash policy** — normalize `"/"` to the module path (spec'd), or reject it to force
   an explicit `""`? Current spec normalizes; it is friendlier and cheap.
3. **`responses(...)` syntax** — `responses(200 = Post, 404)` is compact but parsing user types out
   of a `meta` list is fiddly. Alternative: `response(200, Post)` repeated. It is now load-bearing,
   because an undeclared success path fails the build (§4.5) — so decide before the 0.2 honesty pass,
   not during a later spike.
4. **State override per module** — allow `#[roundhouse(state = X)]` on a module for crates that host
   two apps? Defer to v0.3.
5. **Public utoipa types.** Re-export utoipa wholesale (`roundhouse::doc::utoipa`) or wrap every
   builder? Re-exporting costs nothing and keeps fidelity; wrapping is prettier but re-implements the
   model we just decided not to own.
6. **Component naming** — dedup by type name with a module-path prefix on collision (`blog.Post`).
   Good enough, or should `#[schema(name = "Post")]` be the only source of truth?
7. **400 vs 422** for constraint failures. 400 is the default (§7.6) because malformed input is
   malformed input; 422 is a one-line config for teams that split "unparseable" from "unacceptable".
8. **Response validation scope** — the mechanism is settled (§7.8: the same engine), the switch is
   not. App-wide `validate_responses()` is the test/tooling shape; a per-route
   `#[handler(validate_response)]` would make it a debug-build default. Leaning app-wide plus
   `debug_assertions`.
9. **Name — decided, provisionally.** `roundhouse` + `roundhouse-macros`; GitHub org `roundhouse`;
   `roundhouse.rs` (crates.io, the org and the domain were all free — `.dev` and `.io` are not).
   Reversal now costs a docs rename; reversal after publication costs every URL, macro path and blog
   post. Fallbacks if it does not survive contact with users: `cadastre`, `axum-modular`.
10. **`skip_validation`** — remove it (§7.6.1) or reinterpret it as "documentation only", which
    reintroduces the lie it was meant to avoid. Leaning remove.
11. **Two engines during 0.2** — generated checks stay the default and the schema engine is opt-in,
    with §7.7 proving they agree. Is a minor version of dual paths acceptable, or does 0.2 flip the
    default and keep the generated path behind a feature? Depends on spike 11's numbers.

---

## 15. Gaps and non-goals

What this spec does not answer yet, ordered by when it will bite.

| Gap | Why it bites | Proposed answer |
|---|---|---|
| Routes in a library crate | a library must declare its own `type AppState` for the record type, so the app has to adopt it | root scoping (§5.4) lets the lib own its prefix; the app still has to share the state type or use erased mode |
| Routes injected by a dependency | discovery is global, so any linked crate can register handlers your `main` never mentions | root scoping (§5.4) excludes other crates by default; the startup listing prints every route grouped by crate |
| Observability | the framework knows every route template; users rebuild them by hand for metrics, and get high-cardinality labels wrong | default tracing span carrying `http.route` = the template (§6), opt-in OTel layer |
| WebSockets | `WebSocketUpgrade` is absent from §7.4, and OpenAPI has no standard for `101` | extractor row + `101` response with `x-websocket: true` |
| Middleware-produced responses | a 429 or 503 from a layer never reaches the document | opt-in `doc = …` at the application site (§6); omission leaves them out — silence over invention |
| Recursive schemas | `Node { children: Vec<Node> }` recurses forever without a lazy registry | two-phase components: declare by name, fill later, reference with `$ref` |
| Panics | a panicking handler kills the connection instead of answering | `CatchPanicLayer` installed by `serve()` unless overridden → `500` `Problem` |
| Docs UI | only `/openapi.json` is specified | `ui` feature serving `/docs`, a static page over the same JSON |
| `405` | axum answers it; nothing in the document mentions it | leave undocumented per operation, state it once in the API description |
| Deprecated routes | parsed and dropped today — the worst outcome | `deprecated` on the table entry → `deprecated: true`, or a compile error; never a no-op (§4.5) |
| Semver and MSRV | derive output is public API; an attribute change is a breaking change | MSRV = stable − 2; `#[non_exhaustive]` on public enums; a written list of what counts as breaking |
| CSRF | cookie auth plus form posts is the classic hole | documented guidance in v0.2, a `csrf` layer in v0.3 |

**Non-goals for 1.0:** callbacks, links, webhooks, XML, GraphQL, client codegen, TLS termination,
and dependency injection.

Four of those gaps need design rather than a line item:

**Library crates.** The singleton `AppState` is what makes `inventory` work, and it is also what
makes cross-crate route registration hard: the macro must name a state type that exists where the
macro expands. Because the state type now comes from the handler signature (§3), a route library can
carry its own, and anything else exports a plain `Router` — which is a supported, documented path,
not a failure.

**Observability.** `RouteRecord` already carries the module path, the file, the line and the resolved
template. That means the framework can label every span and metric with `http.route = "/blog/{id}"`
instead of the raw URI — the difference between a usable dashboard and a cardinality explosion. It is
the cheapest high-value feature available and it needs one middleware plus one convention.

**Recursive schemas.** Component registration must be two-phase, or a self-referential type either
loops or inlines itself to death. Declare the name, emit `$ref`, fill the body on the second pass.
This is a spike, not a paragraph — it interacts with `Option`, `Vec`, `flatten` and generics all at
once.

**Instantiability.** Discovery is link-time, so a declaration is a singleton: one `mount!` cannot be
mounted twice, and one `App` cannot hold two states. That is the price of G2 and it does not go away.
The answer is not to hide it — `App::router()` composes into any axum tree, and the README says out
loud that the second instance is a copy the document cannot see. A `mount!(prefix = const)` variant
that takes the prefix from a generic parameter is the only design that would fix it, and it breaks
`inventory`'s `'static` requirement. So: documented limitation, not a roadmap item.

---

## 16. Prior art: Topcoat

[`tokio-rs/topcoat`](https://github.com/tokio-rs/topcoat) 0.11 (Oct 2026) is the closest prior art for
the **discovery mechanism** — and a different product. It is a full-stack framework with its own
router, its own rendering pipeline, client reactivity, an asset bundler and a component library.
Nothing here is borrowed from that half of it.

**What it validates.** The exact spine in §5 — `module_path!()` at a call site, records collected at
link time, discovery filtered to a root prefix — ships in a tokio-rs project. Spike 0 is a formality
rather than a gamble. It also supplies two pieces worth keeping:

- **root scoping** (§5.4), which closes the dependency-injection and multi-router gaps;
- **the discipline around prefix matching** — `::`-boundary comparison, with the `app` /
  `application` confusion as a test case — and the rule that two discovered layers at one path panic,
  because link-time discovery defines no order.

**What we deliberately do not borrow.** Topcoat derives URL segments from module names: `blog_posts`
becomes `/blog-posts`, `_marketing` becomes an invisible group, `module_param!` turns a directory into
a `{post_id}`, `segment!` rewrites any of it. That is right for pages, where a rename is a cosmetic
change. It is wrong for an API, where `/blog-posts` is a contract a client compiled against: renaming
`blog_posts.rs` must not silently move the endpoint. roundhouse keeps prefixes **explicit** —
`mount!("/blog")` and `get("/{id}", show)` — so the URL changes only when someone edits the string
that defines it and the golden spec (§7.5) shows the diff. Grouping without a URL segment is still
available, as `mount!("")`.

**The stack difference is deliberate too.** Topcoat owns `Router`, `Body` and a `Cx` request context,
so a handler erases to `dyn Fn(&Cx) -> Response` and state never becomes a type parameter. roundhouse
stays axum-native: `Router`, `Body`, `FromRequest`, `State<T>` and every tower layer remain axum's,
which is exactly why the record needs `Box<dyn Any>` (§3) and why `Cx`-style context injection was
declined. The price of staying axum is that erasure; the return is that every axum tutorial,
extractor and middleware keeps working.

---

## 17. Decision: validate by executing the schema

**Status:** proposed for 0.2, gated by §7.7 and spike 11. Supersedes the "both halves" implementation
in §7.6; the declaration surface there stands.

### 17.1 The problem, stated precisely

Schema semantics are encoded three times:

| Place | What it encodes | How it can be wrong |
|---|---|---|
| `api_schema.rs` (1,204 lines) | serde attributes → schema keywords | the schema is right and the check is wrong |
| the same file, `validate_at` | the same keywords as Rust comparisons | the check is right and the schema is wrong |
| `doc/response.rs` (347 lines) | a third, partial interpreter | both are right and responses go unchecked |

The promise — "the document cannot lie" — is currently an invariant maintained by hand in three
encodings. The response interpreter ignores `format`, `minLength`, `pattern`, `minimum` and
`uniqueItems`, so a response can violate the document and pass. §7.7 exists to catch exactly that and
does not yet run.

### 17.2 The options

| Option | Cost | Verdict |
|---|---|---|
| **A. Status quo** — generated checks plus a hand-written interpreter | three encodings; each keyword is a treadmill; the subset grows forever | rejected as the destination |
| **B. Execute the emitted schema** (`jsonschema` 0.60) | one dependency, one extra `Value` pass, error-message mapping; wording becomes ours to generate | **chosen** |
| **C. Replace the emitter too** (`schemars` 1.x) | a 2020-12 → utoipa conversion layer, a second attribute vocabulary, loss of the emit/check pairing | deferred; pays off only if utoipa is ever dropped |

C is tempting for the 1,200-line derive, but it moves the problem: the drift is between the emitter
and the *enforcer*, and C still needs the engine. Keep the emitter — it is tuned to the document model
and to `#[schema(..)]`, which is the product — and let the engine enforce.

### 17.3 The shape

- **One root per type.** At `App::build()`, `components.schemas` becomes `$defs`, and each documented
  type gets `{"$ref": "#/$defs/<ComponentName>", "$defs": {…}}`, compiled once:
  `draft202012::options().offline().should_validate_formats(true)
  .with_pattern_options(PatternOptions::regex())`. The `regex` engine, not `fancy-regex`, because the
  constraints are written as Rust `regex` today and the semantics must not move.
- **Keyed by `TypeId`.** `ComponentTypeRecord` already registers a type name; add
  `type_id: fn() -> TypeId`. The compiled set is `Arc<SchemaRegistry>`, inserted as a request
  extension by the middleware that carries `ValidationConfig` today. The extractor looks up
  `TypeId::of::<T>()`; `Json<T>` never needs to know the document.
- **Order.** Parse to `Value` → validate → `from_value::<T>`. Deserialisation stays the final arbiter
  (`1.0` is an integer to JSON Schema and not to `i64`), and a deserialisation failure remains a `400`
  with a serde detail, not a pointer.
- **Errors.** `iter_errors` → `ValidationError { pointer: instance_path, code: keyword, message }`,
  all violations at once. Wording comes from a small keyword → message table, so the error body clients
  already parse does not change.
- **Opt-outs.** `validate = false` skips the lookup (the middleware and `SkipValidation` already
  exist). `App::validation(Validation::Generated | Schema | Off)` selects the engine per app.
- **Startup cost.** One compilation per component. If that is measurable on a large API, compile
  lazily per first use behind the same `Arc` — measure before optimising.
- **Do not use `jsonschema`'s compile-time `validator!` macro yet.** It needs a literal schema; the
  derive could stringify its own output, but each type needs a root that also carries its siblings, and
  the runtime path is not proven. Revisit only if startup time is the constraint.

### 17.4 What the engine cannot do

| Case | Because | Answer |
|---|---|---|
| `validate_with = fn(&T)` | not expressible in JSON Schema | runs after the schema, published as `x-validate-with` (§7.6.1) |
| multipart file parts | bytes, not JSON | keep the generated field checks; document `string/binary` |
| `Bytes` / binary bodies | not JSON | no schema validation; the content type is the contract |
| "checked elsewhere" | the engine only sees the schema | `skip_validation` removed (§7.6.1) |
| unknown formats | 2020-12 leaves them as annotations | `should_validate_formats(true)` plus a registered `url` |
| `discriminator` | OpenAPI keyword, not JSON Schema | unknown keywords are ignored; `oneOf` does the work |

### 17.5 Migration and falsifiers

1. **0.1 — harness only.** `jsonschema` is a dev-dependency; generated checks stay the runtime.
   Nothing user-visible changes, and the harness immediately reports which types diverge.
2. **0.2 — engine opt-in.** `App::validation(Validation::Schema)`; response validation moves onto it
   (§7.8); the subset interpreter is deleted.
3. **0.3 — engine by default.** `validate_at` leaves `ApiSchema`; the derive shrinks to schema,
   registration and `validate_with`.

**Falsifiers.** If the harness cannot make verdicts match for a type, that type keeps generated checks
and the spec names it. If p99 validation exceeds 50 µs on the benchmark body, the engine stays opt-in.
If the error wording cannot be preserved within a small mapping table, the engine ships with its own
wording and `code`/`message` changes in a minor.

The reason to do this is not the 1,200 lines. It is that the document is already a JSON Schema
document, and the enforcement should be the document.
