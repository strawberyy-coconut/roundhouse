# Doc tags

Doc comments carry **prose**. Structured response and path metadata lives in the handler's
attribute. That split is deliberate:

- `#[handler(..)]` is parsed as Rust: a malformed status or type is an ordinary syntax or type
  error, and the tokens are visible to tooling.
- `@summary:` and `@description:` stay in `///` comments because they are prose, should appear in
  `cargo doc`, and should not be duplicated in a second file.

```rust
/// @summary: Create a note.
/// @description: Persists the note and returns the stored representation.
#[handler(
    response(201, Note),
    response(409, Problem),
    response_description(201, "The stored note."),
)]
pub async fn create(...) -> ... { ... }
```

## Prose tags

Tags start a line after optional indentation. A tag starts a section; following lines belong to it
until the next recognized tag. `@summary:` also ends at a blank line, so following rustdoc prose is
not swallowed. `@description:` keeps blank lines as paragraphs.

| Tag | Where | Meaning |
|---|---|---|
| `@summary:` | handler, route entry | OpenAPI operation summary |
| `@description:` | handler, route entry, response variant, response body type, query/path field | OpenAPI description |

Summary lines are joined with spaces. Description lines preserve paragraphs. A duplicate tag in one
doc block is a compile error, and a tag in the wrong context is a compile error.

Untagged text is ignored for OpenAPI. This is deliberate: internal rustdoc prose should not leak
into a public document.

## Typed metadata

The handler attribute owns the structured half:

| Form | Meaning |
|---|---|
| `response(status, Type)` | A JSON response body |
| `response(status)` | An empty response |
| `default_response(Type)` | OpenAPI's `default` response |
| `override_response(status, Type)` | Replace an inferred response |
| `response_description(status, "text")` | Override one response description |
| `path(name, "text")` | Describe a path parameter with no field description |

`Json<T>` is unwrapped to `T` for the body schema. `Problem` and `ValidationErrors` are documented
as problem bodies. A typed declaration that changes the shape of an inferred response needs
`override_response`.

## Precedence

Operation prose:

1. `doc = |docs| ..` hook on the entry (it can call `docs.set_summary()` / `docs.set_description()`),
2. the handler's `@summary:` / `@description:`,
3. the entry's `prose = ..` (it only fills fields the handler left empty),
4. nothing.

Response descriptions:

1. `#[status(description = "…")]`,
2. `response_description(status, "…")`,
3. the response variant's `@description:`,
4. the response body type's `@description:`,
5. the canonical HTTP reason phrase.

Parameter descriptions:

1. `#[schema(description = "…")]`,
2. a typed `path(name, "…")` declaration,
3. the field's `@description:`,
4. no description.

## Deprecated compatibility tags

`@responses:`, `@responses(override):`, `@response-descriptions:` and `@path:` were the 0.2 route
to the same metadata. They still parse so a migration can be incremental, but **new code should
use the typed attribute**. Declaring the same status or path in both forms is a compile error, so
the two cannot silently disagree.

## `prose = include_str!(..)`

An entry option, not a doc tag, supplies both prose fields from one string:

```rust
routes! {
    "/notes/{id}" => {
        get(show, prose = include_str!("../../docs/show.md")),
    },
}
```

The first non-empty line is the summary; the remaining text is the description. Any `&'static str`
expression is accepted, including a const string. `prose` only fills fields the handler did not set
with `@summary:` / `@description:`; a `doc =` hook runs last and can override both.

## Rustdoc visibility

Handler prose tags are ordinary `///` comments, so they appear in `cargo doc`. Entry-level doc tags
are not used because rustdoc warns about doc comments attached to expressions inside a macro
invocation. If the public prose is a separate file, use `prose = include_str!(..)` on the entry.

## Constraints

- **Doc comments are prose only.** Structured metadata belongs on `#[handler(..)]`.
- **Tags only.** There is no plain-doc fallback; publishing prose is always an explicit choice.
- **Status codes stay explicit.** Doc comments describe; they do not infer an HTTP status.
- **Compatibility is temporary.** The structured doc tags exist for the 0.3 migration and are not
  the documented authoring form.
