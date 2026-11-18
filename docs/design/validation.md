# Validation

Roundhouse publishes one schema and executes it. The contract build and the request enforcer
share the same components, refs and constraints, so a route cannot advertise a check it does not
run — or run a check the document does not describe.

## One contract, one engine

`#[roundhouse::schema]` emits the shape and the roundhouse patches. The contract build registers
it as a component and records request plans in `.roundhouse/contract.json`:

- the shared component schemas (`$defs`);
- body plans keyed by operation id, plus body schemas keyed by stable type name for extractors
  that enforce themselves;
- parts plans for path/query/form types, keyed by stable type name;
- the response table used by verification.

The production runtime reads that artifact, compiles the plans with `jsonschema`, and never
assembles or derives a document. There is no second, hand-written validation model: a constraint
that appears in the document came from the same attribute that produced the validator.

The contract build and the runtime are split so a production build does not carry utoipa, the
document builder or the response probes. See [Contract build](contract-build.md).

## Modes

`Validation` selects the enforcer:

| Setting | Behaviour |
|---|---|
| `Validation::Schema` | Execute the emitted schema, then the `validate_with` hooks. |
| `Validation::Off` | Do not validate. |

With the `validation` feature (default), the app defaults to `Schema`. Without the feature the
default is `Off`; the document is still built and extractors still reject malformed input, but the
emitted schema is not executed. `App::validation(Validation::Off)` disables it for the whole app.

A runtime contract is required for `Schema` mode. A `contract` build without
`App::contract(include_contract!())` fails with `BuildError::MissingContract`; use
`Validation::Off` for a router-only build. A build with the `build` feature without a bundle builds the
contract in-process for tests and checks.

Per-route policy is declared on the `routes!` entry:

```rust
routes! {
    "/import" => {
        post(import, validate = false, unverified = "opaque import payload"),
    },
    "/layouts" => {
        post(create_layout, validate_body = false),
    },
}
```

`validate = false` disables both body and parts validation. It is intentionally not an attribute
on the type: whether a request is enforced is a routing policy, not a property of the payload.
`validate_body = false` keeps path, query and form schema checks while treating the body as
opaque. The body schema is still documented, but the raw JSON layer is not installed and the body
schema is not compiled into the engine. Malformed JSON, a wrong content type and a failed
deserialization still reject exactly as they do on a validated route.

## Request pipeline

```text
inject_validation_config
  ├─ inserts ValidationState { status, mode, skip, skip_body, body_validated }
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
validation-policy marker (Off / PartsOnly)
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

1. honours `Validation::Off` and the route's `validate = false`;
2. ignores non-JSON content types;
3. reads the body with the app's body limit (`413` if exceeded; default 2 MiB);
4. parses it as JSON (`400` if malformed);
5. validates the value with the engine by operation id;
6. rebuilds the request and marks `body_validated = true`.

Doing this before extraction is deliberate. The engine sees the bytes the client sent, plain
`axum::Json<T>` is validated without requiring `T: Serialize` for the extractor itself, and
malformed JSON has the same `Problem` shape as a constraint failure. The flag prevents a later
validator from validating the same bytes twice.

The cost is that the request body is buffered and parsed once by the validator, then parsed again
by axum's extractor. That is a deliberate trade-off: it keeps the `Serialize` bound off the
handler and validates before any extractor can consume the body. `App::max_body` raises or lowers
the limit.

`validate = false` is not a silent performance switch: it marks the body unverified in the
document, and the default strict build rejects that marker unless the entry acknowledges it with
`unverified = "reason"`. `validate_body = false` is the precise alternative: it records
`x-roundhouse-body-validation: false`, does not compile a body validator, and is quiet under the
strict build because the author made a body-only decision. `Strict<Json<T>>` honors both policies,
so it is not an escape hatch for large bodies.

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

`SchemaEngine` is built from `.roundhouse/contract.json` during `App::build`:

- the shared `$defs` table is compiled once;
- body plans are keyed by operation id, with body schemas keyed by stable type name for
  extractors that enforce themselves (`Strict<Json<T>>`);
- parts plans are keyed by stable type name, because the same Rust type can be a JSON body in one
  operation and a query struct in another.

The contract builder writes those plans; it does not compile a separate runtime document. The
engine compiles JSON Schema draft 2020-12 with `jsonschema`, offline, with format assertions and
regex patterns enabled. References are already expressed as `#/$defs/..` in the contract artifact.

Every violation is collected, not short-circuited. For a tagged enum (`oneOf`) or `anyOf`, the
engine reports the branch with the fewest failures, so the client sees `/title` and
`min_length` rather than a root-level union code. The translation reads the structured
`ValidationErrorKind`, not the engine's message text, so `code` and `limit` stay stable.

A malformed or incompatible plan is `BuildError::SchemaCompilation`, not a runtime surprise.
Response verification compiles from the same contract response table.

## Errors

Every validation failure becomes an RFC 9457 `Problem`:

```json
{
  "type": "https://roundhouse.rs/problems/validation",
  "title": "Validation failed",
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

Malformed JSON and a wrong `Content-Type` use the same body shape. Axum's rejection would answer
`422` for some malformed bodies; roundhouse normalises those to `400` so the document's promise
stays true.

## Opting out and honesty

`validate = false` does not pretend the body was verified. The operation is marked:

```json
{
  "x-roundhouse-validation": false,
  "x-roundhouse-unverified-body": true
}
```

The strict build rejects a route whose body is unverified unless the entry acknowledges it with
`unverified = "reason"`. This keeps the default loud for accidental gaps while making an explicit
decision quiet. The acknowledgement suppresses the diagnostic; it does not change the extensions,
so document consumers can still see that the body was not verified.

`validate_body = false` is different: it is a precise body policy and records
`x-roundhouse-body-validation: false`. It does not need an acknowledgement and is quiet under
the strict build. The `x-roundhouse-unverified-body` marker remains because the body genuinely was
not checked.

Multipart text fields are enforced by the extractor's field table and byte/media limits, not by
the emitted body schema. A file field without `max_bytes` is bounded only by the app-wide body
limit, and the strict build reports it.

## Response verification

`App::verify_responses()` is an opt-in test layer driven by the contract response table. It
checks the actual status, content type and JSON body against `.roundhouse/contract.json`:

- a documented status is checked for content type and, when present, a JSON schema match;
- a status the operation does not document is a failure;
- routes absent from the contract are skipped;
- a mismatch is a `500 Problem` naming the operation and the failing constraint;
- the body is buffered up to 32 MiB.

`cargo roundhouse verify` runs `cargo test` with `ROUNDHOUSE_VERIFY=1`, which installs the same
middleware without a code change. It requires `contract` + `validation` and is not
intended for production traffic. The standalone `ContractResponseValidator` exposes the same
checks to tests that already have the response in hand.

## Limits

- Only JSON request bodies are validated from raw bytes. Forms are validated after
  deserialisation; multipart is enforced by its extractor.
- A JSON body is currently parsed twice when schema mode is on: once for validation and once by
  the extractor. For large payloads, use `validate_body = false` (or `validate = false` /
  `Validation::Off` deliberately); do not add a silent size threshold, because it would change the
  contract based on the request size.
- Body and parts validation share one `jsonschema` engine. A feature that kept only parts
  validation would still depend on the engine, so it would not reduce dependency weight. The
  useful split is compile-time work: opted-out body schemas are not compiled.
- Streaming JSON Schema validation is not planned: `required`, `additionalProperties`, `oneOf` and
  most format checks need the whole value.
- Response validation is JSON-only and needs the body in memory.
- Constraints are declarative. Cross-field, database or async rules belong in `validate_with` or
  the handler, and are not projected from the document.
- `jsonschema` is the only engine. `validator`, `garde`, `axum-valid` and `validify` are not
  supported: their rules are a runtime DSL, not a schema, and a bridge would make the document a
  lossy projection. `validate_with` is the escape hatch for non-schema rules.
- `validation` off means no declarative enforcement. The code still compiles and the document
  is still honest about its shape, but the checks are not executed.
