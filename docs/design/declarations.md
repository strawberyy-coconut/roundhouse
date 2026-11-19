# Declarations

An application declares routes, handlers, schemas and responses. The macros turn those
declarations into a static module tree plus the metadata the contract build and the runtime both
read. This page describes the authoring model and what expansion produces. The README owns the
syntax examples; this page explains the rules behind them.

## One module tree

`routes!` is invoked at module scope and owns the names it emits:

- `pub const MODULE: Module` — the root of that table's tree;
- `__roundhouse_fragment()` — the explicit fragment `app!(state, module)` roots the app at;
- one `RouteRecord` per entry, plus generated route, layer and doc functions;
- a marker-type alias per handler and a const assertion tied to the entry's span.

There can be at most one `routes!` per module, because it owns those names. Larger applications
compose tables with `...module` spreads, path groups and child modules. `...module` is the only
composition mechanism: there is no runtime route registration, inventory scan or global registry.

Every entry lives inside a path group:

```rust
routes! {
    prefix = "/api",
    tags = ["api"],

    "/notes" => {
        get(list),
        post(create),
    },

    prefix "/admin" [security = [Bearer]] {
        "/audit" => {
            get(audit),
        },
    },
}
```

A path group, `"/path" => { .. }`, declares routes directly. A prefix group,
`prefix "/path" [options] { .. }`, nests another table. Routes are always declared inside a
group.

## Entry and module options

The README's [Route tables](../../README.md#route-tables) section is the option reference. The
design-relevant rules are:

- `validate = false` lowers to `ValidationPolicy::Off`; `validate_body = false` lowers to
  `PartsOnly`; the default is `Schema`. The contradictory `validate = false, validate_body = true`
  pair is a compile error.
- `anonymous` clears inherited guards for the entry or subtree and cannot be combined with
  `security`. A handler's own `Auth<A>` still applies.
- `absolute` makes the entry own its full URL; inherited prefixes, tags, layers and guards are
  reported if the entry ignores them.
- `prose = ..` fills only prose fields the handler left empty; `doc = ..` runs after the handler's
  contributions. Both are expressions type-checked in the emitted context.
- `layer_mode = full` uses `Router::layer`; `layer_mode = route` uses `Router::route_layer`.
- `unverified = "reason"` acknowledges an intentional marker; it does not remove the marker from
  the document.

## `#[handler]`

The attribute leaves the handler untouched and emits a hidden marker type beside it. The marker
implements two traits:

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

`routes!` registers a generated shim, not the function. The shim has the handler's extractors plus
a `ValidationContext` and the route policy's auth extractor; it validates after extraction and
then calls the handler. The marker is what lets `routes!` find the static facts without parsing
the function again, and it is the diagnostic when `#[handler]` is missing.

`#[handler]` reads the signature and doc attributes. For a named return type it probes the type's
`DocumentedResponse` impl. For an opaque return type it reads the handler index produced by the
build-time analyzer; the analyzer selects the type at each tail/`return` site, and the macro emits
typed trait calls instead of guessing from syntax. A handler's implementation can therefore
change an opaque handler's documented response, but only by changing the concrete type, and the
compiler verifies the result. See [The document](document.md).

The shim's extractor classification is syntactic, for wiring only: whether to wrap an argument in
`Mapped`, the argument order (the auth extractor goes before the body), path names for `Path<T>`
and state discovery. Documentation and validation are delegated to `ExtractArg`/`ExtractDoc`.

## Prose and typed metadata

Doc comments carry prose. Structured response and path metadata lives in the handler attribute.

| Tag | Where | Meaning |
|---|---|---|
| `@summary:` | handler | OpenAPI operation summary. Ends at a blank line. |
| `@description:` | handler, schema type, path/query field | OpenAPI description. Blank lines become paragraphs. |
| `@response <status>: text` | handler | Description for one response. `default` targets OpenAPI's `default`. |

Tags start a line after optional indentation. Summary lines are joined with spaces; descriptions
preserve paragraphs. A duplicate tag in one doc block is a compile error, as is a tag in the wrong
context. Untagged text is ignored for OpenAPI: internal rustdoc prose should not leak into a
public document. Route-entry doc comments are not read; use `prose = ..` or `doc = ..` there.

The handler attribute owns the structured half:

| Form | Meaning |
|---|---|
| `response(status, Type)` | `Type`'s documented body at `status`. |
| `response(status)` | An empty response at `status`. |
| `default_response(Type)` | OpenAPI's `default` response documented by `Type`. |
| `override_response(status, Type)` | Replace the return type's documented response at `status`. |
| `response_description(status, "text")` | Override one response description. |
| `path(name, "text")` | Describe a path parameter with no field description. |

`Type` must implement `roundhouse::doc::DocumentedResponse` (a success shape) or
`roundhouse::doc::DocumentedError` (a failure shape); the probe tries the success trait first and
falls back to the error trait. The declaration calls that impl and overrides only the status. Use
`Json<T>` for a JSON body, `Status<CODE, T>` for an exact status with a JSON body, `Empty<CODE>`
for an exact bodyless status, `Problem` for problem+json, or a custom response type. A typed
declaration that changes the shape of an already documented response needs `override_response`.
`response(200, Note)` is not an implicit JSON wrapper.

`?` documents the handler's error type, not the expression it converts. A handler returning
`Result<T, E>` documents `T` and `E`; converting a specific error into `Problem` documents
`Problem`'s `default`. Return the specific error type when its statuses matter. The distinction is
the control-flow path, not the status code; see
[Rust paths are not HTTP status codes](document.md#rust-paths-are-not-http-status-codes).

`prose = include_str!(..)` on an entry fills summary and description from one string; it only
fills fields the handler left empty. A `doc = |docs|` hook runs after the handler's contributions
and can override prose, add parameters, a body or responses. Additions still pass through
deduplication, conflict detection and document validity.

The full response-description precedence is owned by [The document](document.md).

## Schemas and parameter types

`#[roundhouse::schema]` is the one-annotation form. With `build` it derives the schema and the
roundhouse adapter, and constraints use the `#[roundhouse(..)]` namespace. The explicit
`#[derive(ToSchema, ApiSchema)]` form is still supported; with `build` enabled, import utoipa's
`ToSchema` and `PartialSchema` from `roundhouse::interop`.

- `#[derive(ApiSchema)]` adds a named component (`$ref`), `#[roundhouse(unique_items)]`, the
  `url` → `uri` format mapping and `#[roundhouse(validate_with = path)]` hooks.
- `#[derive(QueryParams)]` and `#[derive(PathParams)]` generate the parameter table, parts schema
  and the runtime `Params` value. A field's `@description:` supplies the parameter description;
  `#[roundhouse(description = "…")]` takes priority.
- `RepeatedQuery<T>` reads repeated query keys (`?tag=a&tag=b`); axum's `Query<T>` cannot
  represent them.
- With the `multipart` feature, `#[derive(Multipart)]` generates the field table and extraction.

Without `build`, `#[roundhouse::schema]` keeps the runtime `ValidateHook` impl and compiles
out the schema tokens, so a production build resolves no utoipa paths. Automatic `Json<T>` and
JSON response documentation require `ApiSchema` when the contract build is present. A type that
only implements utoipa's `ToSchema` can be adapted with
`roundhouse::interop::impl_from_utoipa!` or documented by an explicit `DocumentedResponse` impl;
there is no blanket `impl<T: ToSchema> ApiSchema for T`, because it would overlap the derive.

Component names are the Rust type name by default. Two different types that claim one name are a
`BuildError::ComponentCollision`; `#[roundhouse(name = "module.Name")]` disambiguates them. The
document uses the dotted form (`module.Name`).

## State

Declare the app state once at the crate root:

```rust
roundhouse::app_state!(AppState);
```

That expands to a hidden `pub type __RoundhouseAppState = AppState;`. Every table without
`state = ...` uses that type, including spread modules; `state = OtherState` overrides one table
and `state = infer` restores per-handler inference. The declaration is required because
`#[handler]` expands before `App::new` and a `RouteRecord` is a `const`; the shim's router state
must be monomorphized when the table is expanded. The crate-level alias is the smallest scope
every expansion can name.

[Runtime](runtime.md) owns the exact-boundary rule, `FromRef` substates, `Auth<A>` state
compatibility and the shim plumbing.

## `api!`

`api!` is the crate-root configuration macro. It emits
`__roundhouse_contract() -> Contract<State>` under `build`; without the feature the function
is compiled out. It validates option names and requires `title`, `version` and `root`.

| Option | Meaning |
|---|---|
| `state = Type` | Contract state. Defaults to `crate::__RoundhouseAppState`. |
| `title = "..."` | OpenAPI info title. |
| `version = ...` | OpenAPI info version, usually `env!("CARGO_PKG_VERSION")`. |
| `root = module::MODULE` | Root of the route tree. |
| `servers = [..]` | OpenAPI `servers` entries. |
| `register = [Type, ..]` | Bodies or responses the harness cannot name through an extractor or return type. |
| `response_defaults = { 401 => "...", .. }` | Shared response prose by status. |

`api!` must be called at the crate root, not in a nested module.

## Expansion anatomy and checks

```text
routes! { ... }
  ├─ syn parses module options, entries, groups and spreads
  ├─ flatten groups into leaf paths
  ├─ check the table
  └─ emit, recursively:
       const MODULE: Module
       fn __roundhouse_fragment() -> Fragment
       per entry:
         type __RoundhouseHandlerN = <marker type>;
         const _: () = { /* capture assertion */ };
         fn __roundhouse_route_N() -> Box<dyn Any + Send + Sync>
         fn __roundhouse_layer_N(..)            (optional)
         fn __roundhouse_entry_doc_N(..)        (optional)
         const __ROUNDHOUSE_ROUTE_N: RouteRecord
       per group: a hidden const child Module
```

`__roundhouse_route_N` builds the `MethodRouter` at runtime and boxes it, because the concrete
state type differs per handler. Resolution downcasts it back to the handler's state. The
`RouteRecord` carries the static metadata — path, method, options, function pointers — so the
build can work without the macro being present.

One invocation can check its own tokens. These fail `cargo check`:

| Check | What it rejects |
|---|---|
| Path syntax | A path that does not start with `/`; a segment starting with `:` (the DSL uses `{id}`). |
| Duplicate route | Two entries with the same method and normalised template; `{id}` and `{post_id}` are the same shape. |
| `any` conflict | An `any` entry next to a concrete method on the same template. |
| Duplicate operation id | Two explicit ids, or two entries that would derive the same id. |
| Path capture subset | A `Path<T: Params>` whose `NAMES` omits a capture in the leaf template. |
| Missing `#[handler]` | The marker type does not exist, reported once through a type alias. |
| Attribute syntax | Unknown options, misplaced prose and non-type security schemes. |

`#[cfg(..)]` and other outer attributes on an entry are copied to every generated item for that
entry and to direct entries in a group. The macro does not interpret them. A nested group or
spread is a separate item and must carry its own `cfg`. Table-local duplicate checks skip
`cfg`-gated entries because the macro cannot evaluate the predicate; a collision that survives
`cfg` is caught by the resolved route table at build time.

What one invocation cannot see — a spread's prefix, cross-table collisions, handler state,
the composed document — is checked by `resolve_tree` and the document builder. See
[Runtime](runtime.md) and [The document](document.md).

## Constraints

- **Module scope only.** `routes!` emits items and cannot be invoked inside a function.
- **One `routes!` per module.** It owns `MODULE` and `__roundhouse_fragment()`.
- **Free functions, no generics on handlers.** `#[handler]` rejects generic handlers and cannot
  be applied to a method in an `impl` block. Response body types may be generic.
- **The handler must be reachable by path.** A local closure or an unannotated alias has no
  marker.
- **`doc` and `layer` are expressions.** `doc` must coerce to `fn(&mut OperationDoc)`; `layer`
  must be a tower layer over the relevant router.
- **Two layers, one API.** `routes!` is the routing DSL; doc tags and the handler attribute are
  the prose and response-metadata layer. There is no parallel builder.
- **No name-based response inference.** The macro never assigns a status from a type spelling or
  a function body.
