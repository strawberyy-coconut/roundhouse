# Doc tags

OpenAPI prose and response declarations live in doc comments, but only when they are marked with
an explicit tag. Untagged prose is for rustdoc; it never reaches the document.

This keeps the route table lean:

- `routes!` says which handler serves which method and path, and carries routing policy.
- Doc tags say what the operation is and which responses it declares.
- The handler signature and response types say what the wire shape is.

## Syntax

Tags start a line after optional indentation. A tag starts a section; following lines belong to
it until the next recognized tag. `@summary:`, `@responses:` and `@path:` also end at a blank
line, so following rustdoc prose is not swallowed. `@description:` keeps blank lines as
paragraphs.

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
| `@response-descriptions:` | handler | `status: description` prose overrides |
| `@path:` | handler | `name: description` lines for path parameters |

Summary lines are joined with spaces. Description lines preserve paragraphs. A duplicate tag in
one doc block is a compile error, and a tag in the wrong context is a compile error.

Untagged text is ignored for OpenAPI. This is deliberate: internal rustdoc prose should not leak
into a public document.

## Precedence

Operation prose:

1. `doc = |docs| ..` hook on the entry (it can set `docs.summary` / `docs.description`),
2. the handler's `@summary:` / `@description:`,
3. the entry's `prose = ..` (it only fills fields the handler left empty),
4. nothing.

Response descriptions can also be overridden per route with `@response-descriptions:`; those
overrides apply after inferred and `@responses:` shapes have merged.

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
- A bare `status` asserts that status as a success. If the handler already inferred a response
  for it, the inferred shape is kept; otherwise an empty response is added. This is how a
  `Result` alias keeps the compile-time success guarantee without repeating the body type:
  `@responses:` followed by `200`.
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

## `@response-descriptions:`

Overrides the prose of an individual response after inferred and declared shapes have merged:

```rust
/// @response-descriptions:
///   200: The stored note.
///   404: The note does not exist.
#[handler]
async fn show(...) -> ... { ... }
```

A status that the operation does not document is a build diagnostic. The tag does not change a
status or body shape; use `@responses:` for that.

## `prose = include_str!(..)`

An entry option, not a doc tag, supplies both fields from one string:

```rust
routes! {
    "/notes/{id}" => {
        get(show, prose = include_str!("../../docs/show.md")),
    },
}
```

The first non-empty line is the summary; the remaining text is the description. Any
`&'static str` expression is accepted, including a const string. `prose` only fills fields the
handler did not set with `@summary:` / `@description:`; a `doc =` hook runs last and can override
both. This is the supported way to keep public prose in one file instead of repeating it in a doc
tag block.

## `@path:`

A handler can describe path parameters that have no field to carry the description:

```rust
/// @path:
///   id: The note's identifier.
///   slug: The note's URL slug.
#[handler]
async fn show(Path(id): Path<String>) -> Json<Note> { ... }
```

A scalar `Path<T>` has no name of its own, so the tag supplies it. A matching `PathParams` field
`@description:` (or `#[schema(description = ...)]`) takes precedence. A `@path:` name that is not a
template capture is a build error.

## Rustdoc visibility

Handler doc tags are ordinary `///` comments, so they appear in `cargo doc`. Entry-level doc tags
are not used because rustdoc warns about doc comments attached to expressions inside a macro
invocation. If the public prose is a separate file, use `prose = include_str!(..)` on the entry;
it is not emitted into rustdoc. A `doc =` hook remains the escape hatch for computed prose.

## Constraints

- **Tags only.** There is no plain-doc fallback; publishing prose is always an explicit choice.
- **Handler docs are the response source.** The route table does not carry `responses(..)`.
- **Status codes stay explicit.** Doc comments describe; they do not infer an HTTP status.
- **Types in `@responses:` are JSON bodies.** Non-JSON responses are inferred from the return
  type (`File`, `Sse`, `Attachment`, …).
