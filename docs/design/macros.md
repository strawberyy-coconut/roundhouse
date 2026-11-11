# Macros

The macro is the product. `routes!` plus `#[handler]` are why roundhouse exists: they make the
route table, its compile-time checks and its documentation one declaration. A builder API would
be a runtime-checked copy with fewer guarantees, so it is not on the roadmap.

## What the crate provides

| Macro | Kind | Responsibility |
|---|---|---|
| `routes!` | function-like | Parse the table, check it, emit the module tree |
| `#[handler]` | attribute | Emit the marker type and the route shim |
| `ApiSchema` | derive | Register a documented body/response and its patches |
| `QueryParams` | derive | Query parameter table, parts schema, hooks |
| `PathParams` | derive | Path capture names, parts schema, hooks |
| `ApiResponse` | derive | `IntoResponse` plus documented responses for an enum (generic enums supported) |
| `ApiError` | derive | `ApiResponse` with a 4xx/5xx-only error contract (generic enums supported) |
| `Multipart` | derive | Multipart field table, extraction and body schema |

`Multipart` is compiled only with the `multipart` feature; the rest are always present.
`roundhouse-macros` is a normal dependency. There is no `macros` feature to turn off.

## Expansion anatomy

```text
routes! { prefix = "/api", "/notes" => { get(list), post(create) } }
    │
    ├─ syn parses module options, entries, groups and spreads
    ├─ flatten groups into leaf paths
    ├─ check the table (see below)
    └─ emit, recursively:
           const MODULE: Module
           fn __roundhouse_fragment() -> Fragment
           per entry:
             type __RoundhouseHandlerN = <marker type>;
             const _: () = { /* success + capture assertions */ };
             fn __roundhouse_route_N() -> Box<dyn Any + Send + Sync>
             fn __roundhouse_layer_N(..)                 (optional)
             fn __roundhouse_entry_doc_N(..)             (optional)
             const __ROUNDHOUSE_ROUTE_N: RouteRecord
           per group: a hidden const child Module
```

`__roundhouse_route_N` builds the `MethodRouter` at runtime and boxes it, because the concrete
state type differs per handler. `resolve`/`assemble` downcast it back to the handler's state.
The `RouteRecord` carries all the static metadata — path, method, options, function pointers —
so the build can work without the macro being present.

The marker alias is a diagnostic device. If the handler lacks `#[handler]`, the alias points at
a type that does not exist, producing one "type not found" error at the handler rather than
interpreting the marker path as a module and cascading.

`#[cfg(..)]` and other outer attributes on an entry are copied to every generated item for that
entry, and to every entry in a group. They are not interpreted by the macro.

The macro never parses a function body. `#[handler]` reads the signature and doc attributes only;
routing metadata cannot be inferred from what the function does.

## Doc tags

`#[handler]` also reads explicit doc tags:

- `@summary:` and `@description:` fill `HandlerDocs.summary` / `.description`;
- `@responses:` fills `HandlerDocs.declared_responses` and `DECLARED_SUCCESS`;
- `@responses(override):` sets the declared-response override.

The tag parser lives in `doc_tags.rs` and is shared with the `ApiResponse` derive and the
parameter derives. Untagged prose is ignored for OpenAPI. See [Doc tags](doc-tags.md) for the
grammar and precedence.

## `#[handler]`

The attribute takes no arguments and leaves the function untouched. It emits:

- a marker type implementing `HandlerMeta` (`State`, `PATH_NAMES`, `REQUIRED_AUTH`,
  `INFERRED_SUCCESS`, `docs`/`docs_with`);
- an associated `__roundhouse_shim::<P>` on the marker type, which extracts the handler's
  arguments plus a `ValidationContext` and the policy's auth extractor, validates, and calls
  the function.

Because the shim is a separate function, route policy (`validate`, `security`) is applied
without changing the handler's signature, and a direct call to the handler behaves exactly as
written. See [Documentation](documentation.md) for the trait-level details.

The shim's extractor classification is still syntactic, for wiring only: whether to wrap an
argument in `Mapped`, argument order (the auth extractor goes before the body), path names for
`Path<T>`, and state discovery. All documentation and validation work is delegated to
`ExtractArg`.

## Compile-time checks

One invocation can check its own tokens. These fail `cargo check`:

| Check | Error shape |
|---|---|
| Path syntax | `syn::Error` at the path span |
| Duplicate route in the table | Combined error pointing at both entries |
| `any` conflicting with a method | Combined error pointing at both entries |
| Duplicate explicit/derived operation id | Combined error pointing at both declarations |
| No success response | Const assertion naming the method, path and fix |
| `Path<T>` misses a capture | Const assertion naming the handler and template |
| Missing `#[handler]` | Missing marker type at the handler |
| Unknown option, non-type security, removed attribute | `syn::Error` at the offending token |

The two const assertions exist because a type-level error cannot format a useful message. A
`const` block can call `prefix::path_names_match` or inspect `INFERRED_SUCCESS` and `panic!`
with a string built from the entry's literals. The panic is evaluated during type checking, so
it is still a `cargo check` failure, not a runtime one.

`tests/ui/*.rs` + `tests/ui/*.stderr` pin these messages with `trybuild`: duplicate routes,
duplicate ids, missing handlers, path mismatches, security state mismatches, removed attributes
and an undeclared success.

## What cannot be checked at expansion time

A proc macro sees one invocation. It cannot know:

- the prefix a `...module` spread will contribute;
- whether two separate tables collide;
- whether a handler's state matches the app's state;
- whether the composed document is valid.

Those are build-time checks in [Routing](routing.md) and [Documentation](documentation.md). The
split is deliberate: do not move a check earlier than the information exists, and do not invent
a registry to make a proc macro see its siblings.

## Shared shape parsing

`serde_shape.rs` is one parser for the subset of serde attributes the derives need:
`rename`, `rename_all`, `rename_all_fields`, `tag`, `content`, `untagged`, `skip`,
`skip_deserializing`, `default`, `flatten` and friends. `ApiSchema`, `QueryParams` and
`PathParams` all use it, so the body document, query parameters and path captures cannot
disagree about a wire name.

The derives keep their own rules on top:

- `PathParams` rejects `skip`, `default` and `flatten`: a path capture is always present and
  path names are flat.
- `QueryParams` documents `Vec<T>` as an exploded form array and expects
  `RepeatedQuery<T>` to read it.
- `ApiSchema` splits attributes: `#[schema(..)]` for what utoipa understands,
  `#[roundhouse(..)]` for `unique_items` and `validate_with`. `#[schema(skip_validation)]` is
  rejected: a declared constraint is published and enforced; opting out is a route policy
  (`validate = false` or `validate_body = false`), not a field property.

`proc-macro-crate` resolves the dependency name, so `rh = { package = "roundhouse", .. }` works.
`examples/renamed` is the regression test.

## Constraints

- **Module scope only.** `routes!` emits items, so it cannot be invoked inside a function.
- **One `routes!` per module.** It owns `MODULE` and `__roundhouse_fragment()`.
- **Free functions, no generics on handlers.** `#[handler]` rejects generic handlers and cannot
  be applied to a method in an `impl` block. Response and error enums may be generic.
- **The handler must be reachable by path.** A local closure or a function alias without the
  marker is not a handler.
- **`doc` and `layer` are expressions.** They are type-checked in the emitted context: `doc`
  must coerce to `fn(&mut HandlerDocs)`, `layer` must be a tower layer over the relevant
  router.
- **`any` is served, not documented.** There is no honest single operation for it.
- **Two layers, one API.** `routes!` is the routing DSL: paths, methods and policy. Doc tags are
  the prose and response-metadata layer. There is no parallel builder to keep in sync.

## Why the macro is mandatory

The previous `macros` feature suggested a macro-free workflow, but the only public route
constructors were macro internals. Removing the feature made the situation honest: runtime
capabilities are optional, the authoring model is not.

The trait refactor and the macro are complements, not alternatives. `ExtractArg`, `ApiSchema`
and `ApiResponse` moved semantics out of the expansion, so the macro is now mostly parsing,
validation and wiring. That is the part that benefits from a separate library crate.

## Future

- **`roundhouse-macros-core`.** Move `serde_shape`, `util`, route parsing/validation/expansion
  and the derive expansion into a normal library crate; keep `roundhouse-macros` as the
  `proc-macro = true` entry point. This would allow unit tests of the parser and expander
  without `trybuild`, and make the expansion reusable by tooling. Under consideration; not
  implemented.
- **Richer diagnostics.** More checks can move from "build error" to "compile error" only when
  the information is local to one invocation. Cross-table checks stay at build time.
- **No no-macro builder.** It would weaken the guarantees that are the reason to use roundhouse.
