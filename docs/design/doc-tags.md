# Doc tags

OpenAPI prose and response declarations live in doc comments, but only when they are marked with
an explicit tag. Untagged prose is for rustdoc; it never reaches the document.

This keeps the route table lean:

- `routes!` says which handler serves which method and path, and carries routing policy.
- Doc tags say what the operation is and which responses it declares.
- The handler signature and response types say what the wire shape is.

## Syntax

Tags start a line after optional indentation. A tag starts a section; following lines belong to
it until the next recognized tag.

```rust
/// @summary: Create a note.
/// @description: Persists the note and returns the stored representation.
/// @responses:
///   201: Note
///   409: Problem
#[handler]
pub async fn create(...) -> Result<Json<Note>, NoteError> { ... }
```

Recognized tags:

| Tag | Where | Meaning |
|---|---|---|
| `@summary:` | handler | OpenAPI operation summary |
| `@description:` | handler, response variant, response body type, query/path field | OpenAPI description |
| `@responses:` | handler | Response declarations |
| `@responses(override):` | handler | Response declarations that may replace inferred responses |

Summary lines are joined with spaces. Description lines preserve paragraphs. A duplicate tag in
one doc block is a compile error, and a tag in the wrong context is a compile error.

Untagged text is ignored for OpenAPI. This is deliberate: internal rustdoc prose should not leak
into a public document.

## Precedence

Operation prose:

1. `doc = |docs| ..` hook on the entry (it can set `docs.summary` / `docs.description`),
2. the handler's `@summary:` / `@description:`,
3. nothing.

Response descriptions:

1. `#[status(description = "…")]`,
2. the response variant's `@description:`,
3. the response body type's `@description:`,
4. the canonical HTTP reason phrase.

Parameter descriptions:

1. `#[schema(description = "…")]`,
2. the field's `@description:`,
3. no description.

## `@responses:`

Each line is one of:

```text
201: Note
201: Json<Note>
204
default: Problem
```

- `status: Type` declares a JSON response body.
- `Json<T>` is accepted as a convenience and unwrapped to `T` for the schema.
- A bare `status` declares an empty response.
- `default: Type` maps to OpenAPI's `default` response.
- `Problem` and `ValidationErrors` declare a problem response body.
- Duplicate statuses are compile errors.
- The type text is parsed with `syn::parse_str::<syn::Type>` and resolves in the handler's module.
- Rust-analyzer cannot check types inside a doc string; a typo is a macro-expansion error.

An `@responses:` declaration adds or refines responses. A declaration that changes the shape of
an inferred response is `BuildError::ConflictingResponse` unless the block uses
`@responses(override):`; the operation is then marked `x-roundhouse-overridden-response`.

Response declarations are handler-local. Per-mount differences use a distinct handler; entry-level
`@responses:` is not part of this release.

## Rustdoc visibility

Handler doc tags are ordinary `///` comments, so they appear in `cargo doc`. Entry-level doc tags
are not used because rustdoc warns about doc comments attached to expressions inside a macro
invocation. If prose should stay out of rustdoc, keep it in the handler's implementation details
and use a `doc =` hook for the operation override.

## Constraints

- **Tags only.** There is no plain-doc fallback; publishing prose is always an explicit choice.
- **Handler docs are the response source.** The route table does not carry `responses(..)`.
- **Status codes stay explicit.** Doc comments describe; they do not infer an HTTP status.
- **Types in `@responses:` are JSON bodies.** Non-JSON responses are inferred from the return
  type (`File`, `Sse`, `Attachment`, …).
