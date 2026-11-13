# Validation

roundhouse publishes one schema and executes it. The document emitter and the request enforcer
share the same components, refs and constraints, so a route cannot advertise a check it does
not run — or run a check the document does not describe.

## One artifact, two uses

`#[derive(ApiSchema)]` emits the shape and the roundhouse patches. The document builder
registers it as a component. At build time, when validation is enabled, the schema engine
compiles those components (plus inline body and parts schemas) into `jsonschema` validators.

There is no second, hand-written validation model. A constraint that appears in the document
came from the same attribute that produced the validator.

## Modes

`Validation` selects the enforcer:

| Setting | Behaviour |
|---|---|
| `Validation::Schema` | Execute the emitted schema, then the `validate_with` hooks |
| `Validation::Off` | Do not validate |

With the `schema-engine` feature (default), the app defaults to `Schema`. Without the feature
the default is `Off`; the document is still built and extractors still reject malformed input,
but the emitted schema is not executed. `App::validation(Validation::Off)` disables it for the
whole app.

Per-route opt-out is `validate = false` on a `routes!` entry. It is intentionally not an
attribute on the type: whether a request is enforced is a routing policy, not a property of the
payload.

```rust
routes! {
    "/import" => {
        post(import, validate = false, unverified = "opaque import payload"),
    },
}
```

`validate = false` disables both body and parts validation. The narrower policy is
`validate_body = false`: path, query and form values keep their schema, but the body is opaque.
This is the route policy for a large payload that should be deserialized but should not be
buffered into `serde_json::Value` and checked twice:

```rust
routes! {
    "/layouts" => {
        post(create_layout, validate_body = false),
    },
}
```

The body schema is still documented, but the raw JSON layer is not installed and the body schema
is not compiled into the engine. Malformed JSON, a wrong content type and a failed
deserialization still reject exactly as they do on a validated route.

## Request pipeline

```text
inject_validation_config
  ├─ inserts ValidationState { status, mode, what to skip }
  ├─ inserts the compiled engine (schema mode)
  └─ inserts multipart limits
        │
        ▼
auth guard (if any) → entry layer (if any)
        │
        ▼
raw JSON body validator (only when the route documents a JSON body)
        │
        ▼
axum extraction
  ├─ extractors that enforce themselves (RepeatedQuery, Multipart, Strict)
  └─ the remaining arguments
        │
        ▼
generated shim
  ├─ unwraps `Mapped<..>`
  ├─ asks each argument's ExtractArg::validate
  └─ calls the handler
```

### 1. Validation config

One middleware inserts a request-scoped `ValidationState` and, in schema mode, the compiled
`SchemaEngine`. Multipart limits are inserted alongside it. The value is copied, not mutated
through interior state, so a route's opt-out cannot leak into another request.

### 2. Raw JSON bodies

For a route whose documented body is `application/json` (or a `+json` media type), the build
installs a middleware that:

1. honours `Validation::Off`, the route's `validate = false` and its `validate_body = false`;
2. ignores non-JSON content types;
3. reads the body with the app's body limit (`413` if exceeded);
4. parses it as JSON (`400` if malformed);
5. validates the value with the engine by the body's `TypeId`;
6. rebuilds the request and marks `body_validated = true`.

Doing this before extraction is deliberate. The engine sees the bytes the client sent, plain
`axum::Json<T>` is validated without requiring `T: Serialize` for the extractor itself, and
malformed JSON has the same `Problem` shape as a constraint failure. The flag prevents a
`Strict<Json<T>>` later in the chain from validating the same bytes twice.

The cost is that the request body is buffered and parsed once by the validator, then parsed again
by axum's extractor. That is a deliberate trade-off: it keeps the `Serialize` bound off the
handler and validates before any extractor can consume the body. The body limit bounds the memory;
the default is 2 MiB and `App::max_body` raises or lowers it.

`validate = false` is not a silent performance switch: it marks the body unverified in the
document, and `App::strict()` rejects that marker unless the entry acknowledges it with
`unverified = "reason"`. `validate_body = false` is the precise alternative: it records
`x-roundhouse-body-validation: false`, does not compile a body validator, and is quiet under
`strict()` because the author made a body-only decision. `Strict<Json<T>>` honors both policies,
so it is not an escape hatch for large bodies.

A later optimization may skip the raw layer when the body schema has no enforceable keywords, or
offer a validate-after-extraction mode for `Serialize` body types. The latter would need an
explicit extractor or route mode so the `Serialize` bound is checked at compile time. Neither
changes the default until a benchmark justifies it.

### 3. The shim

The generated shim runs only when `ValidationContext::enabled()`:

```rust
if __rh_ctx.enabled() {
    // for each argument
    if let Err(problem) = ArgProbe::<T>::new().validate(&value, &__rh_ctx) {
        return problem.into_response();
    }
}
```

What that does depends on the argument's `ExtractArg` implementation. The shim is also where a
route's `validate = false` takes effect: `enabled()` is false, so neither the schema nor the
`validate_with` hooks run.

### 4. Extractors that own enforcement

Some extractors enforce their contract during extraction, before the shim sees the value:

- `RepeatedQuery<T>` parses with `serde_html_form`, validates the parts schema and runs hooks;
- `Multipart<F>` enforces the field table, required fields, per-part and total limits, and the
  accepted media types while reading parts;
- `Strict<Json<T>>`, `Strict<Form<T>>`, `Strict<Path<T>>`, `Strict<Query<T>>` and
  `Strict<Multipart<F>>` enforce during extraction and reject with `Problem`;
- `Result<Extractor, E>` leaves the rejection to the handler. The shim documents the successful
  shape but does not validate the value, because the handler may have already consumed it.

For `axum::Json<T>`, the shim re-runs only the `validate_with` hooks: the schema already ran in
the raw body layer. For `axum::Form<T>` the shim serialises the deserialised value and runs the
schema itself, because a form is not JSON on the wire.

### 5. `validate_with` hooks

Declarative constraints come from the schema. Anything the schema cannot express is a
`#[roundhouse(validate_with = path)]` function on `ApiSchema`, run after the engine. It receives
the value and a `ValidationErrors`, so it can report the same JSON Pointers as a schema error.
Hooks are skipped when validation is off or the route opted out.

## Enforcement matrix

| Argument | Schema enforcement | Hooks |
|---|---|---|
| `Json<T>` | Raw body layer before extraction | Shim |
| `Form<T>` | Shim, on the deserialised value | Shim |
| `Path<T>`, `Query<T>` | Shim, parts schema | Shim |
| `RepeatedQuery<T>` | Extractor | Extractor |
| `Multipart<F>` | Field table, required fields and byte/media limits during extraction; the emitted body schema is not executed | None |
| `Strict<Json<T>>`, `Strict<Form<T>>` | Extractor | Shim |
| `Strict<Path<T>>`, `Strict<Query<T>>` | Extractor | Extractor |
| `Result<Extractor, E>` | None — the handler owns the rejection | None |
| `Bytes`, `Body`, `String`, `Request` | No schema | None |
| `Extension<T>`, other parts | No schema; possibly an unverified-extractor diagnostic | None |

`validate = false` disables the schema and hook columns. `validate_body = false` disables only the
body rows; path and query rows still run. Parsing and extractor-owned contract checks still run:
an opted-out route still rejects malformed JSON, an invalid query string or a too-large multipart
part. Opting out is not an invitation to skip deserialisation.

## The schema engine

`SchemaEngine` is built from the final document at the end of `App::try_build`:

- named components are compiled once per `(TypeId, component name)`;
- inline body schemas are compiled per body `TypeId`, so composites (`Vec<T>`, `Option<T>` and
  maps) work without inventing a component;
- parts schemas are compiled separately, because the same Rust type can be a JSON body in one
  operation and a query struct in another.

The engine compiles JSON Schema draft 2020-12 with `jsonschema`, offline, with format
assertions and regex patterns enabled. References are rewritten from
`#/components/schemas/..` to `#/$defs/..` for execution.

Every violation is collected, not short-circuited. For a tagged enum (`oneOf`) or `anyOf`, the
engine reports the branch with the fewest failures, so the client sees `/title` and
`min_length` rather than a root-level union code. The translation reads the structured
`ValidationErrorKind`, not the engine's message text, so `code` and `limit` stay stable.

A schema that cannot compile is `BuildError::SchemaCompilation`, not a runtime surprise. The
same applies to response validation, which compiles at build time.

## Errors

Every validation failure becomes an RFC 9457 `Problem`:

```json
{
  "type": "about:blank",
  "title": "Bad Request",
  "status": 400,
  "errors": [
    { "pointer": "/title", "code": "min_length", "message": "must be at least 3 character(s)", "limit": 3 }
  ]
}
```

- `pointer` is a JSON Pointer into the body or the parts value.
- `code` is the schema keyword (`min_length`, `pattern`, `required`, ...).
- `limit` carries the violating operand when there is one.
- The status defaults to `400`; `App::validation_status(StatusCode::UNPROCESSABLE_ENTITY)` is a
  common alternative.

Malformed JSON and a wrong `Content-Type` use the same body shape. Axum's rejection would
answer `422` for some malformed bodies; roundhouse normalises those to `400` so the document's
promise stays true.

## Opting out and honesty

`validate = false` does not pretend the body was verified. The operation is marked:

```json
{
  "x-roundhouse-validation": false,
  "x-roundhouse-unverified-body": true
}
```

`strict()` rejects a route whose body is unverified unless the entry acknowledges it with
`unverified = "reason"`. This keeps the default loud for accidental gaps while making an explicit
decision quiet. The acknowledgement suppresses the diagnostic; it does not change the extensions,
so document consumers can still see that the body was not verified.

`validate_body = false` is different: it is a precise body policy and records
`x-roundhouse-body-validation: false`. It does not need an acknowledgement and is quiet under
`strict()`. The `x-roundhouse-unverified-body` marker remains because the body genuinely was not
checked.

## Response validation

`App::validate_responses()` is an opt-in debug/test layer. It compiles each documented JSON
response schema once and checks the actual response body and content type:

- a mismatch is a `500 Problem` naming the expected content type or the failing pointer;
- non-JSON documented responses are left alone;
- the body is buffered up to 32 MiB.

It requires `schema-engine`; without the feature `try_build` returns
`BuildError::SchemaCompilation`. It is not intended for production traffic.

## Limits

- Only JSON request bodies are validated from raw bytes. Forms are validated after
  deserialisation; multipart is enforced by its extractor.
- A JSON body is currently parsed twice when schema mode is on: once for validation and once by
  the extractor. For large payloads, use `validate_body = false` (or `validate = false` /
  `Validation::Off` deliberately); do not add a silent size threshold, because it would change
  the contract based on the request size.
- Body and parts validation share one `jsonschema` engine. A feature that kept only parts
  validation would still depend on the engine, so it would not reduce dependency weight. The
  useful split is compile-time work: opted-out body schemas are not compiled.
- Streaming JSON Schema validation is not planned: `required`, `additionalProperties`, `oneOf` and
  most format checks need the whole value.
- Response validation is JSON-only and needs the body in memory.
- Constraints are declarative. Cross-field, database or async rules belong in `validate_with`
  or the handler, and are not inferred from the document.
- `schema-engine` off means no declarative enforcement. The code still compiles and the document
  is still honest about its shape, but the checks are not executed.

## Future

- The engine could validate multipart text fields from the emitted schema, but the current
  extractor already enforces the field table and the limits at the byte level. Adding a second
  pass needs a clear story for which errors win.
- Streaming request validation is not planned: a JSON schema check needs the whole value, and
  buffering is bounded by the body limit.
