# Routing

A roundhouse app has one source of truth: the `routes!` tables. They produce a static module
tree, the tree is resolved into a route table, and that table feeds both `axum::Router`
assembly and the OpenAPI document. Nothing is discovered from the linker.

## Pipeline

```text
routes! { .. }
    │
    ├─ parse            tokens → module options, entries, groups, spreads
    ├─ flatten/check    table-local paths, duplicates, operation ids, success responses
    └─ expand           Module, RouteRecord, shim aliases, const assertions
            │
            ▼
    __roundhouse_fragment() ──► App::fragment / App::include / app!(..)
            │
            ▼
    resolve_tree
        ├─ join prefixes, merge tags/security/hidden/layers
        ├─ check state, templates, captures, path assertions
        ├─ probe the matchit tree (the same one axum will build)
        └─ reject duplicate routes and operation ids across tables
            │
            ├──► doc::build ──► OpenAPI 3.1 document
            └──► assemble  ──► axum::Router
            │
            ▼
          Built { router, routes, document, merges, diagnostics }
```

`App::build()` panics on failure and prints diagnostics. `App::try_build()` returns a
`BuildError` instead and is the check hook for CI. `App::strict()` promotes non-fatal
diagnostics into errors.

## Declare

`routes!` is invoked at module scope, not inside a function. One invocation emits:

- `pub const MODULE: Module` — the root of that table's tree;
- `__roundhouse_fragment()` — the explicit fragment `app!(state)` roots the app at;
- one `RouteRecord` per entry, plus generated route/layer/doc functions;
- a marker-type alias per handler, and const assertions tied to the entry's spans.

There can be at most one `routes!` per module, because it owns those names. Nested groups and
child modules exist so a large table does not have to be one invocation.

Every entry lives inside a path group:

```rust
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

A group may carry module policy in brackets, for example `"/notes" [tags = ["notes"]] { ... }`.
The flat `get("/path", handler)` and compact `get "/path" => handler` forms were removed in 0.3;
there is one way to declare a route.

`...module` spreads another table's `MODULE` at that point in the tree. That is the only
composition mechanism: there is no runtime route registration and no inventory/linker scan.

## Table-local checks

These run during macro expansion, because the tokens of one table are enough to decide them.
They fail `cargo check`, not the app build.

| Check | What it rejects |
|---|---|
| Path syntax | A path that does not start with `/`; a segment starting with `:` (the DSL uses `{id}`) |
| Duplicate route | Two entries with the same method and normalised template; `{id}` and `{post_id}` are the same shape |
| `any` conflict | An `any` entry next to a concrete method on the same template |
| Duplicate operation id | Two explicit ids, or two entries that would derive the same id from method and handler name |
| Undeclared success | A return shape the macro can classify as producing no 2xx/3xx response, unless the handler declares a success in `@responses:` |
| Path capture subset | A `Path<T: PathNames>` whose `NAMES` omits a capture in the leaf template |
| Missing `#[handler]` | The marker type does not exist, reported once through a type alias |
| Attribute syntax | Unknown options, non-type security schemes, removed attributes (see `tests/ui/`) |

The capture check is deliberately a subset check. A parent module prefix can add captures the
leaf table cannot see at expansion time, so `Path<T>` is allowed to declare extra names; the
resolved path is checked again at build time.

## Resolve

`resolve_tree` walks the module tree depth-first and carries inherited policy:

| Field | Rule |
|---|---|
| Prefix | Joined with `prefix::join`; trailing/leading slashes trimmed and repeated slashes collapsed |
| Tags | Appended in declaration order, deduplicated per operation |
| Security | Appended, deduplicated by scheme name, one guard per scheme |
| Hidden | Logical OR down the tree |
| Layers | A module layer wraps its whole router; `layer_mode = route` uses `route_layer` |
| Layer docs | A module `doc = ..` is merged into every operation below it |

A prefix may contain captures (`/{project_id}`) but not a catch-all. A prefix that repeats its
enclosing prefix is a diagnostic: it usually means a table was pointed at the wrong parent.

### Build-time checks

These need the composed tree, so they are `BuildError`s from `try_build`:

- invalid prefixes and templates, including a resolved path matchit cannot serve;
- duplicate routes across separate tables, and duplicate operation ids across tables;
- handler state that disagrees with the app state, or a security scheme whose state disagrees;
- a `path = "..."` assertion that does not match the resolved binding;
- a template that declares the same capture name twice;
- unknown path parameters (a documented path parameter with no capture);
- reserved-path collisions with `/openapi.json`, the docs UI or the Swagger assets;
- an empty module, an app with no routes, and a merged router that collides with a declared
  route;
- document validity: every capture has one required path parameter, every path parameter is
  used, operation ids are unique, every security requirement names a declared scheme, and
  every operation has a response.

A module that declares no routes and has no routes below it is a diagnostic; `strict()` turns
it into an error. The same applies to absolute routes that ignore enclosing policy, merged
routers, unbounded multipart file fields and the honesty markers described in
[Documentation](documentation.md).

### Operation ids

An entry may set `operation_id = "..."`. Otherwise the id is derived from the method, the
module path and the handler name:

```text
GET blog::posts::show  →  get_blog_posts_show
```

The id follows the declaration, not the URL. Moving a route under a different prefix does not
rename every generated client method. An explicit id always wins.

### Absolute routes

`absolute` makes the entry own its full URL. Module prefixes, tags, security, layers, hidden and
layer documentation do not apply; a `security` declared on the entry itself still does. The
build reports anything the entry silently ignored, because an absolute route inside a guarded,
tagged module is usually a mistake.

## Assemble

`assemble` turns each `ResolvedRoute` into an `axum::Router`:

1. The record's boxed `MethodRouter<S>` is downcast to the handler's state type and
   `with_state` is applied. A type mismatch here should have been caught during resolve.
2. Route-scoped layers are applied: the opt-out marker, the raw JSON body validator, the entry's
   `layer`, and finally the auth guards. Guards are outermost so a layer cannot answer before
   the scheme is checked.
3. Modules are rebuilt from the same records and merged; module layers wrap their subtree.
4. Absolute routes are added at the root, outside every module router.
5. The fallback is a JSON `404 Problem`, and `method_not_allowed_fallback` answers `405` with
   the same shape instead of axum's empty body.
6. `App::merge` routers and the document/docs/Swagger routes are added; the `405` fallback,
   `App::max_body`, `App::validate_responses`, the validation-config middleware and global
   layers are applied.

The resulting layer order, from outermost to innermost, is:

```text
App global layers
  validation config (sets ValidationState + engine)
    response validation (opt-in)
      module layers (outer module → inner module)
        auth guard
          entry layer
            raw JSON body validation
              generated handler shim
                handler
```

`App::into_router()` returns the ready `axum::Router<()>`; state has already been applied, so it
needs no second `with_state` call. `App::router()` remains for one release as a deprecated
`Router<S>` wrapper around a fallback service. `App::merge()` is the other escape hatch: the merged
routes are served but cannot be discovered for the document, so the document is marked
`x-roundhouse-merged` and the merge is recorded in `Built::merges()`.

### State and `FromRef` substates

The app state is exact at the boundary: `App::new` must be built with the same state the route
table declares. Inside the table, handlers may take substates through axum's ordinary `FromRef`
contract:

```rust
routes! {
    state = AppState,
    prefix = "/notes",
    "/" => { get(list) },
}

#[handler]
async fn list(State(db): State<Db>) -> Json<Vec<Note>> {
    // Db: FromRef<AppState>
    Json(db.list())
}
```

An inline group inherits the table's `state` and may override it. A spread module is a separate
`routes!` expansion and declares its own state when it uses substates. Without `state = ...`, the
default remains the handler's first `State<T>`, the auth scheme's state, or `()`.

The shim is generic over the chosen state: `routes!` instantiates it with `State<S>` and a
`ShimPolicyFor<S>` bound, and each `State<T>` argument contributes `T: SubstateOf<S>` (a hidden
alias for `FromRef`). This is why a table-level declaration is enough; the handler macro never
needs to know the app state.

The declaration is required because `#[handler]` expands before `App::new` and a `RouteRecord` is a
`const`; the shim's router state must be monomorphized when the table is expanded. A `state = ...`
on the app root cannot reach a separately expanded module, and `App::new`'s inferred type is not
visible to a proc macro. The table is the smallest scope that knows the router state.

A spread therefore cannot inherit its parent's state. The child table's `MODULE` already contains
`MethodRouter<S>` values for the state it declared; re-typing them at the spread site would
require an owned, generic module builder and give up the `'static` tree. The supported way to
centralize the name is a crate-local wrapper that expands every table with the same state:

```rust
macro_rules! routes {
    ($($tokens:tt)*) => {
        roundhouse::routes! { state = crate::AppState, $($tokens)* }
    };
}
```

This keeps the declaration in one place without pretending the macro can see another expansion.

`Auth<A>` remains exact: `AuthScheme::State` must equal the table state. A state mismatch is a
`BuildError::StateMismatch` from `App::try_build()`.

## Built

`Built` is the resolved artifact:

- `router()` — the ready `axum::Router<()>`;
- `routes()` — the resolved table, sorted by path then method, with each declaration's source;
- `document()` — the always-built OpenAPI 3.1 document;
- `openapi_30()` — a best-effort 3.0.3 shape;
- `merges()` and `diagnostics()` — what the build had to work around;
- `oneshot*()` — request helpers for tests.

The document is always built even when it is not served. `App::openapi(..)` controls the
`/openapi.json` route and the info object, not whether the document exists.

## Constraints

- **The module tree is the complete route set.** There is no global registry, no attribute
  discovery and no runtime `add_route`.
- **Compile time and build time are different tools.** One table can check its own token
  stream; only the aggregate tree can check cross-module collisions, state and the document.
  Do not move a check earlier than the information exists.
- **`any` is served but not documented.** It has no single OpenAPI operation, so it is omitted
  with a diagnostic. That diagnostic is always a diagnostic: there is no correct document
  shape to fall back to.
- **`App::merge` is an escape hatch, not composition.** Declared routes and merged routes have
  different guarantees. The ledger exists so CI can see the difference.
- **State is exact at the boundary, structural inside it.** A table names the app state once;
  `State<T>` arguments may then use `FromRef`. The table state still has to equal `App::new`'s
  state, and the build error names both when they differ.

## Future

- The `roundhouse-macros-core` split (see [Macros](macros.md)) would not change this pipeline;
  it would only make the expansion unit-testable.
- A macro-free builder is not planned. It would move duplicate, capture and operation-id checks
  from `cargo check` into `App::build`, which is a strictly weaker position.
- Cross-table checks stay at build time. A proc macro cannot see sibling invocations, and
  inventing a registry to work around that would reintroduce the linker discovery the design
  avoids.
