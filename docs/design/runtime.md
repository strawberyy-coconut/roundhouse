# Runtime

The runtime starts from the same module tree the contract build resolved. It joins prefixes,
merges policy, checks the cross-table invariants and assembles an `axum::Router`. It then consumes
the committed contract for request validation and documentation serving. It does not derive
schemas or build a document.

## Pipeline

```text
app!(state, module) / App::include / App::fragment
        │
        ▼
resolve_tree
    ├─ join prefixes, merge tags/security/hidden/layer docs
    ├─ check state, templates, captures, path assertions
    ├─ probe the matchit tree axum will build
    └─ reject duplicate routes and operation ids across tables
        │
        ├── contract build ──► .roundhouse/contract.json + openapi.json
        └── assemble       ──► axum::Router
        │
        ▼
Built { router, routes, merges, diagnostics }
```

`App::build()` panics on failure and prints diagnostics. `App::try_build()` returns a `BuildError`
instead and is the check hook for CI. Builds are strict by default; non-fatal diagnostics are
errors, and `App::permissive()` downgrades them to warnings.

## Resolution

`resolve_tree` walks the module tree depth-first and carries inherited policy:

| Field | Rule |
|---|---|
| Prefix | Joined with `prefix::join`; trailing/leading slashes trimmed and repeated slashes collapsed. |
| Tags | Appended in declaration order, deduplicated per operation. |
| Security | Appended, deduplicated by scheme name, one guard per scheme. |
| Anonymous | Clears inherited guards at that module or entry; the operation documents `security: []`. |
| Hidden | Logical OR down the tree. |
| Layers | A module layer wraps its whole router; `layer_mode = route` uses `route_layer`. |
| Layer docs | A module `doc = ..` is merged into every operation below it. |

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
  used, operation ids are unique, every security requirement names a declared scheme, and every
  operation has a response.

A module that declares no routes and has no routes below it is a diagnostic; the strict build
turns it into an error. The same applies to absolute routes that ignore enclosing policy, merged
routers, unbounded multipart file fields and the honesty markers described in
[The document](document.md).

### Operation ids

An entry may set `operation_id = "..."`. Otherwise the id is derived from the method, the module
path and the handler name:

```text
GET blog::posts::show  →  get_blog_posts_show
```

The id follows the declaration, not the URL. Moving a route under a different prefix does not
rename every generated client method. An explicit id always wins.

### Absolute routes

`absolute` makes the entry own its full URL. Module prefixes, tags, security, layers, hidden and
layer documentation do not apply; a `security` declared on the entry itself still does. The build
reports anything the entry silently ignored, because an absolute route inside a guarded, tagged
module is usually a mistake.

## Assembly

`assemble` turns each `ResolvedRoute` into an `axum::Router`:

1. The record's boxed `MethodRouter<S>` is downcast to the handler's state type and `with_state`
   is applied. A type mismatch here should have been caught during resolve.
2. The validation-policy layer is applied (`skip` or `skip_body`), then the raw JSON body
   validator when the route validates a JSON body, then the entry's `layer`, and finally the auth
   guards. Guards are outermost so a layer cannot answer before the scheme is checked.
3. Modules are rebuilt from the same records and merged; module layers wrap their subtree.
4. Absolute routes are added at the root, outside every module router.
5. The fallback is a JSON `404 Problem`, and `method_not_allowed_fallback` answers `405` with the
   same shape instead of axum's empty body.
6. `App::merge` routers and the document/docs/Swagger routes are added; the `405` fallback,
   `App::max_body`, response verification, the validation-config middleware and global layers
   are applied.

The resulting layer order, from outermost to innermost, is:

```text
App global layers
  validation config (installs ValidationState + engine + multipart limits)
    response verification (opt-in)
      body limit (App::max_body)
        module layers (outer module → inner module)
          auth guard
            entry layer
              raw JSON body validation (Schema routes with a JSON body)
                validation-policy marker (Off / PartsOnly)
                  generated handler shim
                    handler
```

For a `Schema` route with a JSON body, the raw body layer runs before extraction. For `Off` and
`PartsOnly`, the policy marker is installed and the raw body layer is not. The shim is where
path/query/form validation and `validate_with` hooks run; see [Validation](validation.md).

`App::into_router()` returns the ready `axum::Router<()>`; state has already been applied, so it
needs no second `with_state` call. `App::merge()` is the escape hatch: merged routes are served
but cannot be discovered for the document, so the document is marked `x-roundhouse-merged` and
the merge is recorded in `Built::merges()`. An unacknowledged merge is a strict build error; use
`App::merge_undocumented(router, reason)` to accept it.

## State

The app state is exact at the boundary: `App::new` must be built with the same state the route
tables use. Declare it once at the crate root:

```rust
roundhouse::app_state!(AppState);
```

That expands to a hidden `pub type __RoundhouseAppState = AppState;`. Every `routes!` table
without `state = ...` uses that type, including spread modules. Inside a table, handlers may take
substates through axum's ordinary `FromRef` contract:

```rust
routes! {
    prefix = "/notes",
    "/" => { get(list) },
}

#[handler]
async fn list(State(db): State<Db>) -> Json<Vec<Note>> {
    // Db: FromRef<AppState>
    Json(db.list())
}
```

An inline group inherits the table's state and may override it. `state = OtherState` overrides one
table; `state = infer` restores per-handler inference (the first `State<T>`, the auth scheme's
state, or `()`). The alias is a type path, so separate macro expansions resolve the same type
without a runtime registry.

The shim is generic over the chosen state: `routes!` instantiates it with `State<S>` and a
`ShimPolicyFor<S>` bound, and each `State<T>` argument contributes `T: SubstateOf<S>` (a hidden
alias for `FromRef`). This is why a table-level declaration is enough; the handler macro never
needs to know the app state.

The declaration is required because `#[handler]` expands before `App::new` and a `RouteRecord` is
a `const`; the shim's router state must be monomorphized when the table is expanded. A
`state = ...` on the app root cannot reach a separately expanded module, and `App::new`'s inferred
type is not visible to a proc macro. The crate-level alias is the smallest scope that every
expansion can name.

`Auth<A>` remains exact: `AuthScheme::State` must equal the table state. A state mismatch is a
`BuildError::StateMismatch` from `App::try_build()`.

## `Built`

`Built` is the resolved artifact:

- `router()` — the ready `axum::Router<()>`;
- `routes()` — the resolved table, sorted by path then method, with each declaration's source;
- `openapi_json()` — the embedded `openapi.json` string;
- `document()` — a lazily parsed `OpenApi` view, available under `contract`;
- `openapi_30()` / `openapi_30_json()` — a best-effort 3.0.3 shape, available under `contract`;
- `merges()` and `diagnostics()` — what the build had to work around;
- `oneshot*()` — request helpers for tests.

A production build does not build a document. It consumes the committed contract bundle,
compiles the request plans and serves the embedded `openapi.json`.

## Serving documentation

`App::contract(include_contract!())` installs the committed bundle. Production configuration is
routing and UI:

- `App::openapi_route(path)` chooses where `openapi.json` is served; the body is the committed
  string, byte for byte.
- `App::docs()` serves a Swagger UI at `/docs`; `App::docs_route(path)` moves it.
- `App::docs_cdn()` opts back into the pinned unpkg assets instead of the vendored ones.
- `App::swagger()` serves the local UI at `/docs/swagger`; `App::swagger_route(path)` moves it.

With the default `swagger` feature, the CSS, JavaScript and OAuth assets are embedded, so `/docs`
works in an air-gapped deployment. `App::openapi(..)`, `App::document(..)`,
`App::layer_documented(..)` and `App::server(s)(..)` are contract-build APIs; a production build
does not run them.

The runtime checks that `contract_version` is supported and that runtime routes do not collide
with the document, docs or Swagger paths. `Built::routes()` stays available in every
configuration, so a production build can inspect the route table without the document model.

## Constraints

- **The module tree is the complete route set.** There is no global registry, no attribute
  discovery and no runtime `add_route`.
- **Compile time and build time are different tools.** One table can check its own token stream;
  only the aggregate tree can check cross-module collisions, state and the document. Do not move
  a check earlier than the information exists.
- **`any` is served but not documented.** It has no single OpenAPI operation, so it is omitted
  with a diagnostic. That diagnostic is always a diagnostic: there is no correct document shape
  to fall back to.
- **`App::merge` is an escape hatch, not composition.** Declared routes and merged routes have
  different guarantees. The ledger exists so CI can see the difference.
- **State is exact at the boundary, structural inside it.** A table names the app state once;
  `State<T>` arguments may then use `FromRef`. The table state still has to equal `App::new`'s
  state, and the build error names both when they differ.
