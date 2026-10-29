# Roundhouse v0.2 — Independent Source Review

| | |
|---|---|
| Repository | `axum-roundhouse` |
| Version | 0.2.0 |
| Commit | `81d81ce` (working tree clean) |
| Review date | 2026-10-28 |
| Scope | Workspace source, examples, integration tests, CI, README |
| Method | Static source review. No build or test execution; the review environment had no Rust toolchain and no container runtime. |
| Author | Independent review (AI-assisted) |

**Verdict:** a well-engineered, unusually coherent 0.x framework built on a strong idea. The idea is not yet true in the code: the document can still state things the router does not do, in at least six High-severity places. Fix those, add a document-validity/enforcement-parity test, and this becomes genuinely recommendable for contract-first JSON APIs.

---

## 1. Executive summary

Roundhouse's thesis is that a route declaration produces both the router and the OpenAPI document, so the two cannot drift. The implementation takes that seriously: documentation is derived from the same types the extractors use (`roundhouse::Json<T>` requires `T: ApiSchema`), routes inherit prefixes, tags, security and layers from the Rust module tree, and a substantial `BuildError` catalogue turns several classes of drift into build failures. The test suite is above the norm for a 0.x project: a golden document, a hand-written-vs-discovered differential test, a generated-vs-JSON-Schema conformance harness, trybuild, and per-feature integration tests.

The problem is that the invariant is not yet true. This review records **23 findings: 6 High, 10 Medium, 7 Low**. Each High finding is a case where the document asserts something the router does not do, or fails to assert something it does:

- `Auth<A, Scopes>` publishes OAuth scopes in the security requirement and never checks them. The repository's own test authenticates a non-privileged token against a scoped route and expects `200`.
- `mount!(security = [...])` publishes a security scheme whose *kind is guessed from the string*; nothing enforces it.
- `Problem`, the framework's default error type, has no `ApiResponse` implementation, so handler-returned 404/409/500 responses are absent from the document.
- A scalar `Path<T>` — the most idiomatic axum form — produces no path parameter, yielding a spec-invalid document; the compile-time name check silently skips it.
- `(StatusCode, Json<T>)` is documented as `200` regardless of the status code actually returned.
- `try_build()` still panics on a state mismatch, and a typo'd `scope(...)` produces an empty application with no error.

None of these is technically hard to fix. They are not exotic edge cases either; they are the framework's core promise. The rest of the review covers the architectural bet (link-time discovery), API and packaging issues, and a prioritised remediation plan.

No memory-safety, parser or remote-DoS findings were identified in the code read. The High findings are contract and authorization-correctness issues, not runtime unsafety.

---

## 2. Scope, method and limitations

### Reviewed

- Entire `src/` facade: registry, prefix resolution, router assembly, OpenAPI derivation, response validation, auth, extractors, `Problem`, CLI binary.
- All four proc-macro modules (`routes!`, `mount!`, `#[handler]`, `ApiSchema`/`ApiResponse`/`PathParams`/`QueryParams`/`Multipart`), some skimmed.
- `roundhouse-schema` (schema derivation, validation, engine) and `roundhouse-multipart`.
- All seven examples and their tests, skimmed for API usage and claims.
- Workspace tests, the golden document, CI workflow, manifests and README.

### Not executed

`cargo test`, `cargo clippy`, `cargo doc`, the examples, MSRV builds, benchmarks. The review environment had no `cargo`/`rustc` and no podman for the dev container. Findings are based on code paths and, where noted, on known axum behaviour. Appendix B lists the questions that a test run should settle.

### Conventions

- **Severity** — High: violates the core invariant or produces security-relevant misinformation; fix before a 1.0 or a public "safe to rely on" claim. Medium: a real defect or limitation with a workaround. Low: diagnostics, packaging, polish.
- **Effort** — S: under a day; M: 1–3 days; L: over 3 days, for one experienced maintainer.
- Line references are to commit `81d81ce`.

---

## 3. Scorecard

| Dimension | Assessment | Basis |
|---|---|---|
| Core architecture | **Strong** | Type-driven derivation, module-tree mounts, clean crate split, no unsafe. |
| Contract integrity | **Weak (today)** | Six High findings where the document and router disagree; several unmarked opt-outs. |
| Build diagnostics | **Good** | Strong `BuildError` catalogue and source locations; state mismatch still panics, warnings are tracing-only. |
| Developer experience | **Good, with ceremony** | Excellent route locality; path params require one-field structs, `operationId` collisions require manual overrides, scopes/security annotations behave unlike they read. |
| Test discipline | **Strong** | Golden document, differential test, conformance harness, trybuild; missing document-validity and enforcement-parity tests. |
| Packaging and release hygiene | **Weak** | Declared MSRV is wrong, `jsonschema` is unconditional, README claims drift, no MSRV job, placeholder repository URL. |
| Positioning | **Weak** | "Metaframework" promises a scope the project explicitly excludes; no comparison or quickstart. |

Overall: **promising, not yet contract-safe**. The architecture is worth keeping; the integrity work is the critical path.

---

## 4. What works

**The derivation mechanism is the right one.** `roundhouse::Json<T>` cannot be used unless `T: ApiSchema` (`src/extract.rs:92-142`), so the document sees every body the router can accept. `#[handler]` reads the signature and emits a marker type (`roundhouse-macros/src/handler.rs:66-173`), and `routes!` const-asserts that `Path<T>` names match the template (`src/prefix.rs:73-126`). This is structurally stronger than attaching `#[utoipa::path]` next to a handler: annotation drift is the default failure mode of OpenAPI-in-Rust, and Roundhouse avoids it for the common shapes.

**Mount inheritance is the real differentiator.** `examples/blog` composes `/api/blog` → `/posts` → `/` from three `mount!` declarations with no `router()` function; tags, security and layers inherit once. `aide` and `utoipa-axum` give annotated routes, not a tree. This is the feature worth building the project around.

**The error catalogue is disciplined.** Normalised `{id}`/`{post_id}` duplicate detection (`src/prefix.rs:130-152`), operationId conflicts, undeclared success responses, unverified-body markers, `strict()`, a trybuild guard, a golden document, and a hand-written-vs-discovered differential (`examples/notes/tests/fidelity.rs`). Add the schema conformance harness (`tests/engine.rs`), response validation, streaming `TempFile`, and zero unsafe, and the engineering quality is well above typical 0.x.

**The auth abstraction is better than most.** `AuthScheme` is pure metadata; `Authenticate` receives `&Self::State` so token lookup uses real application state rather than request extensions; `Auth<A>::State` pins a handler's state type like `State<T>` (`src/auth.rs:64-89`). The scope handling is the one part of this design that does not carry through (F-01).

**The multipart and response-validation work is real engineering.** Streaming to an already-unlinked `TempFile`, per-field and global limits, a part-count bound, and response validation against the same compiled schema are not demo features.

---

## 5. Findings register

| ID | Severity | Area | Finding | Evidence |
|---|---|---|---|---|
| F-01 | **High** | Auth / contract | Scopes are documented but never enforced | `src/auth.rs:120-145,196-207`; `src/doc/build.rs:156-168`; `examples/auth/tests/auth.rs:36-76` |
| F-02 | **High** | Auth / contract | Mount/table `security` is doc-only and invents scheme kinds | `src/router.rs:478-480`; `src/doc/build.rs:160-168,337-351`; `tests/surface.rs:599` |
| F-03 | **High** | Errors / contract | `Problem` as the handler error type documents no responses | `src/doc/mod.rs:388-396`; `src/problem.rs:108-120`; `roundhouse-macros/src/handler.rs:288-295`; `src/lib.rs:13` |
| F-04 | **High** | Path params / validity | Scalar/foreign `Path<T>` yields no parameter object → invalid OpenAPI | `roundhouse-schema/src/schema.rs:55-62,447-463`; `roundhouse-macros/src/handler.rs:71-97`; `src/prefix.rs:73-126` |
| F-05 | **High** | Responses / contract | `(StatusCode, Json<T>)` documents `200`, ignores the status | `roundhouse-macros/src/handler.rs:301-314` |
| F-06 | **High** | Build API | `try_build` panics on state mismatch; empty scope builds silently | `src/router.rs:410-421,754-799`; `tests/state_mismatch.rs:19-23` |
| F-07 | Medium | Responses / contract | `responses(200 = X)` can override an inferred body with a different one | `src/doc/build.rs:134-137`; `tests/honesty.rs:243-255` |
| F-08 | Medium | Validation | Schema-mode engine silently skips composite bodies | `roundhouse-schema/src/engine.rs:64-68`; `src/router.rs:252-262` |
| F-09 | Medium | Packaging | Declared MSRV 1.85, but let-chains require 1.88 | `Cargo.toml:23`; `src/router.rs:278,291-292,309-310,742-744` |
| F-10 | Medium | Packaging / compile time | `jsonschema` is unconditional; two validators are maintained | `roundhouse-schema/Cargo.toml:14`; `roundhouse-schema/src/engine.rs` |
| F-11 | Medium | API stability | Registry structs expose ~20 `pub` fields each | `src/registry.rs:93-115,132-143` |
| F-12 | Medium | Macros | Expansions hardcode `::roundhouse`; renaming the dependency breaks | `roundhouse-macros/src/*.rs` (all expansions) |
| F-13 | Medium | CLI / ops | Document generation boots the app and prints to stdout; CLI shells out to cargo | `src/router.rs:367-408`; `src/bin/roundhouse.rs:65-68` |
| F-14 | Medium | Diagnostics | Build warnings use `tracing`; silent without a subscriber | `src/router.rs:420-431`; `src/doc/build.rs:38-60,327-331` |
| F-15 | Medium | DX | Default `operationId` is the handler name; collides across modules | `roundhouse-macros/src/routes.rs:278-282` |
| F-16 | Medium | Auth | `Option<Auth<A>>` maps invalid credentials to anonymous, not 401 | `src/auth.rs:147-168`; `examples/auth/src/routes/protected.rs:43` |
| F-17 | Low | Docs UI | `docs()` no-ops without `openapi()`; UI loads unpkg without SRI | `src/router.rs:173-182,289-302,679-697` |
| F-18 | Low | Packaging / docs | README and manifest drift (bytes, feature names, URL, publish flags) | `README.md:179-182`; `roundhouse-schema/src/schema.rs:245`; `Cargo.toml:22,31-38` |
| F-19 | Low | Macro API | `mount!(state = ...)` is accepted and silently ignored | `roundhouse-macros/src/mount.rs:85-89` |
| F-20 | Low | DX | Missing `#[handler]` produces a raw "cannot find value" error | `roundhouse-macros/src/handler.rs:204-208`; `roundhouse-macros/src/routes.rs:377` |
| F-21 | Low | Contract | Missing `Extension` documents a `Problem` 500; runtime is axum's rejection | `roundhouse-macros/src/handler.rs:166-170`; `src/doc/mod.rs:292-294` |
| F-22 | Low | Contract | `validate = false` / `DocumentRejections::None` create unmarked divergence | `src/router.rs:190-194,792-794` |
| F-23 | Low | Diagnostics | `SchemeKindConflict` prints the same source twice | `src/doc/build.rs:361-379` |

---

## 6. High-severity findings

### F-01 — Scopes are documented but not enforced

`AuthScopes` carries only a `SCOPES` constant (`src/auth.rs:94-99`). `Auth<A, Scopes>`'s `FromRequestParts` stores `PhantomData<Scopes>` and performs no scope check (`src/auth.rs:120-145`), while `auth_doc` adds `Scopes::SCOPES` to the OpenAPI security requirement (`src/doc/build.rs:156-168,353-359`). `examples/auth/tests/auth.rs:36-76` sends `Bearer reader` to `/scoped` and asserts `200`, while the document requires `read:posts`.

**Impact.** An OpenAPI security requirement with scopes is an authorization contract. Gateways that generate policy from the spec, client generators, contract tests, and human readers all interpret `read:posts` as a requirement. The application enforces only authentication. If a deployment relies on the document for authorization decisions, this is an authorization bypass on paper; even without that, consumers are misled. The README's "adds scope metadata" is honest internally but the wire format is not metadata — it is a claim.

**Recommendation.** Add a scope decision to the auth model, for example a default method on `AuthScheme`:

```rust
fn authorize(_identity: &Self::Identity, scopes: &[&str]) -> Result<(), Problem> {
    if scopes.is_empty() { Ok(()) } else { Err(/* forbidden */) }
}
```

Call it from `Auth::from_request_parts` after `authenticate`, mapping failures to `403` with the scheme's documented failure response. Schemes that intend to use scopes implement it. If you want documentation-only scopes, remove them from `security` and publish `x-roundhouse-scopes` instead, plus a build flag that makes the intent explicit. Do not ship a default that documents a requirement no code checks.

**Test.** Unauthenticated → 401; valid token without the scope → 403; valid token with the scope → 200; the document's scopes appear only when the scheme opts into enforcement.

**Effort:** M.

### F-02 — Mount/table `security` is doc-only and invents scheme kinds

`mount!` collects `security = [...]` names (`src/router.rs:478-480`), `resolve` carries them to `ResolvedRoute`, and the document builder adds them as required security requirements (`src/doc/build.rs:160-168`). Nothing in assembly enforces them. Worse, `default_security_scheme` guesses the kind: the name `bearer` becomes HTTP bearer, every other string becomes an API-key header named after the string (`src/doc/build.rs:337-351`). `tests/surface.rs:599` only asserts that the requirement reaches the document.

**Impact.** `mount!(security = ["oauth"])` publishes an `apiKey` header scheme called `oauth` that no layer checks. This is fabricated security metadata, which is worse than omitting it. It also creates a false sense of coverage: a team can annotate a subtree, see green docs, and ship an unauthenticated API.

**Recommendation.** Remove the guessing. Require that every name in `security` be registered by an `Auth<A>` or by a `layer_documented(auth::<A>())` in the same subtree, and fail the build otherwise. If a layer is the enforcer, `layer_documented` already gives the build a way to see it; use that as the only accepted proof. If an escape hatch is needed, mark the operation `x-roundhouse-unverified-security` and make `strict()` reject it.

**Effort:** M.

### F-03 — `Problem` as the handler error type documents no responses

`ApiResponse` is implemented only by the derive macro (`src/doc/mod.rs:388-396`; `roundhouse-macros/src/api_response.rs:417-430`). `Problem` implements `IntoResponse` but not `ApiResponse` (`src/problem.rs:108-120`). For `Result<T, E>`, `#[handler]` documents the OK half and queries `stored_responses_for::<E>` (`roundhouse-macros/src/handler.rs:288-295`); for `Problem`, that lookup returns empty. The crate-level example uses `Result<Json<Post>, Problem>` (`src/lib.rs:13`).

**Impact.** A handler that returns `404`/`409`/`500` as `Problem` documents only its success status (plus extractor failures, if any). `strict()` does not catch this because a success response exists. This is the framework's default error type and the most natural thing for a new user to write.

**Recommendation.** Implement `ApiResponse for Problem`, documenting a `default` response (`application/problem+json`, the existing problem schema) and, if you want precision, `400`/`500` entries. Alternatively special-case `Problem` in `#[handler]`. Also warn when a `Result` error type yields zero documented responses; the current silence is the opposite of the project's stated philosophy.

**Test.** A route returning `Result<Json<T>, Problem>` with a `404` branch must show `404` (or `default`) in the document.

**Effort:** S.

### F-04 — Scalar or foreign `Path<T>` produces an invalid document

`PathNames::params()` defaults to empty (`roundhouse-schema/src/schema.rs:55-62`), and the scalar impls are empty (`:447-463`). `#[handler]` documents `Path<T>` by calling `T::params()` (`roundhouse-macros/src/handler.rs:71-83`). The compile-time check is skipped when `PATH_NAMES` is empty (`src/prefix.rs:73-77`), and `captures()` exists but is only used in tests (`:49-71`). Consequently:

```rust
routes! { get("/users/{id}", show) }
#[handler]
async fn show(Path(id): Path<u64>) -> Json<User> { ... }
```

produces a path item for `/users/{id}` with no `parameter` object. OpenAPI 3.1 path templating requires each template expression to correspond to a path parameter, so the document is invalid; Swagger UI flags it and client generators often fail. `axum::extract::Path`/`Query` are silently skipped as well, because `is_wrapper_type` only accepts single-segment or `roundhouse::` paths (`roundhouse-macros/src/handler.rs:242-251`); the foreign-body case is at least marked unverified, the foreign-path case is not. `Option<Json<T>>` and other extractors outside the closed list are likewise invisible.

A related hole: `mount!("/orgs/{org}")` is accepted (the prefix is just a string), and its capture is never documented.

**Impact.** The most idiomatic axum spelling produces a spec-invalid document, while the framework's workaround — a one-field `PostId` struct with `#[derive(PathParams)]` for every path parameter — is neither enforced nor documented as required. This is the kind of thing a client-generating consumer discovers immediately.

**Recommendation.** The route table already knows the template, so the document pass can synthesise parameters for captures not covered by an extractor (using the scalar's schema and `required: true`) and fail the build if names disagree. Alternatively, reject scalar/foreign `Path` with a targeted error that tells the user to derive `PathParams`. Add a build check that every capture in every resolved path is documented and unique. Reject captures in mount prefixes, or implement inherited parameters.

**Effort:** S–M.

### F-05 — `(StatusCode, Json<T>)` documents `200`

`ok_statements` treats any tuple of two or more elements as "the last element is the body" (`roundhouse-macros/src/handler.rs:301-314`) and never inspects the leading `StatusCode`. A handler returning `(StatusCode::CREATED, Json(post))` is documented as `200` with the post schema; `(StatusCode::NO_CONTENT, Json(...))` is documented as `200`. This is a standard axum response shape.

**Impact.** The document states a success status the server does not return. Clients and contract tests break; response validation (if enabled) will not catch it because the documented status is 200 and the actual status has no documented entry.

**Recommendation.** If the first tuple element is `StatusCode`, document a `default` response (the same treatment as `-> StatusCode`) unless the table declares `responses(..)`. Handle `(StatusCode, HeaderMap, T)` as well. A declarative alternative is to require `responses(..)` for status-bearing tuples and fail the build otherwise.

**Test.** Return `201`; assert the document has `default` or `201`, never `200` alone.

**Effort:** S.

### F-06 — The fallible build is not total

`App::try_build` calls `assemble`, which calls `build_method_router`; a state mismatch panics there (`src/router.rs:754-799`), so `try_build` can unwind. `tests/state_mismatch.rs:19-23` asserts the panic. Separately, `resolve` warns for a `mount!` with no routes beneath it (`:420-431`), but there is no check that a `scope(..)` matched anything or that the resolved route set is non-empty. `app!(state)` uses `module_path!()`, so calling it in `main.rs` instead of the library root yields an app with zero routes that starts successfully and 404s everything.

**Impact.** The README's central claim — "`App::try_build()` returns a `BuildError` instead of panicking" — is false for the most common wiring error. And a typo in a scope string is indistinguishable from a working empty app. Both undermine "loud by default" at exactly the moment a new user needs it.

**Recommendation.** Move the state check into `resolve`/`assemble` as a `BuildError::StateMismatch { route, expected, actual }`. Add `BuildError::EmptyScope` (a scope that is an ancestor of no route) and `BuildError::EmptyApp` (no routes resolved) — or at least warnings promoted by `strict()`. Ensure no panic path exists in `try_build`; a `catch_unwind` test over negative fixtures is a cheap guard.

**Effort:** S.

---

## 7. Medium-severity findings

**F-07 — Declared responses can contradict inferred ones.** `routes!` responses are inserted after inferred responses, overwriting them (`src/doc/build.rs:134-137`), and `tests/honesty.rs:243-255` asserts that a handler returning `OtherPayload` is documented as `Payload`. For `impl IntoResponse` the declaration is the only way to document, so the feature is needed; but when an inferred response exists and differs, the framework is lying by design. Restrict overrides to additive statuses, or require an explicit `override` flag that emits `x-roundhouse-overridden` and warns under `strict()`.

**F-08 — Schema-mode engine skips composite bodies.** The engine keys validators by `TypeId` (`roundhouse-schema/src/engine.rs:64-68`) and is populated only from `ComponentTypeRecord`s emitted by the derive (`src/router.rs:252-262`). `Json<Vec<Note>>`, `Json<Option<T>>`, maps and tuples have no such record, so `validate::<T>` returns `None` and the JSON-Schema execution is skipped. The generated pass still runs afterward (`src/extract.rs:122-130`), so requests are not unvalidated, but the advertised "validate against the document" is not what executes for those shapes, and the two engines can diverge on formats/keywords the generated checks do not implement. Compile the body's inline schema for the operation instead of looking up a component by `TypeId`, or return an error when no validator is found.

**F-09 — MSRV is wrong.** `Cargo.toml:23` declares `rust-version = "1.85"`, but the code uses let-chains (`if serve_openapi && let (...)`) at `src/router.rs:278,291-292,309-310,742-744`, `src/extract.rs:118,297` and `src/doc/downgrade.rs:29`; let-chains stabilised in 1.88. Cargo does not detect this, so a 1.85 user gets syntax errors. Bump to 1.88 (and align the satellite crates) or rewrite the chains, and add an MSRV CI job that actually compiles.

**F-10 — `jsonschema` is unconditional, and two validators are maintained.** `roundhouse-schema/Cargo.toml:14` makes `jsonschema` a non-optional dependency, so every user compiles it even with the default `Validation::Generated` and no response validation. Feature-gate it (`schema-engine`) and make the feature required by `Validation::Schema` and `validate_responses`. Longer term, two implementations of the same contract are a permanent tax; the conformance harness is the right mitigation, but one canonical engine would remove the class of divergence entirely.

**F-11 — Registry structs are public API.** `RouteRecord` and `MountRecord` expose ~20 `pub` fields each (`src/registry.rs:93-115,132-143`). Every future field is a semver break, and users can construct invalid records. Keep the fields accessible to macro expansions but stabilise only constructors: a `#[doc(hidden)] pub mod __private` with builder functions, or `#[non_exhaustive]` plus accessors. Do this before 1.0; it is much cheaper now.

**F-12 — Macro expansions hardcode `::roundhouse`.** All expansions refer to `::roundhouse::...`, so renaming the dependency (`roundhouse = { package = "roundhouse", ... }` under another key, or a vendored fork) breaks every macro with confusing "unresolved crate" errors. Use `proc-macro-crate` to resolve the actual name, or document "do not rename this dependency" prominently. This is a small change with a large support-burden payoff.

**F-13 — Document generation boots the app and mixes channels.** `App::serve` scans `std::env::args()` for `--openapi`/`--routes` and prints the document with `println!` (`src/router.rs:367-408`); the `roundhouse` binary spawns `cargo run -p <pkg> -- --openapi` (`src/bin/roundhouse.rs:65-68`). State construction therefore happens before any doc dump, so DB pools, env validation and migrations become prerequisites for a CI diff; the examples initialise `tracing` to stdout by default, so one startup log line corrupts the JSON. Split `serve` from `document()`, put logs on stderr, add `--output <path>`, and consider a build-time registry dump so the "document is a build artifact" claim is true.

**F-14 — Build warnings are silent without a subscriber.** Unused mounts (`src/router.rs:420-431`), `any` operations (`src/doc/build.rs:38-44`), unverified bodies (`:54-59`) and conflicting success bodies (`:327-331`) are `tracing::warn!`. A plain binary that has not installed a subscriber prints nothing. Collect diagnostics into `Built` (`Vec<Diagnostic>` with source), print them to stderr in `build()`/`serve()`, and let `strict()` promote them to errors. "Loud by default" should not depend on the user's logging setup.

**F-15 — `operationId` defaults to the handler name.** `last_segment(handler)` (`roundhouse-macros/src/routes.rs:278-282`) makes `list`, `show`, `create` and `health` collide across modules; the error then asks the user to set `operation_id` manually. The examples set it explicitly five times. Derive from method + path (`get_blog_posts_id`), sanitised, and let the manual override handle exceptions. This removes a recurring papercut without changing semantics.

**F-16 — `Option<Auth<A>>` treats invalid credentials as anonymous.** `OptionalFromRequestParts` maps every error from `authenticate` to `Ok(None)` (`src/auth.rs:147-168`). An expired or malformed token on an "optional auth" route returns `200` with anonymous content, hiding revocation and auth failures. Distinguish absent credentials (None) from present-but-invalid (401). This may be intentional, but it should be a documented, testable choice rather than a blanket `Err(_) => None`.

---

## 8. Low-severity findings

| ID | Finding | Fix |
|---|---|---|
| F-17 | `docs()` sets `docs_route` but does nothing unless `openapi()` was called (`src/router.rs:173-182,289-302`); the UI loads Swagger from unpkg with no integrity (`:679-697`). | Make `docs()` imply `openapi()`, or error; vendor the UI (`utoipa-swagger-ui`) for offline/production. |
| F-18 | README says `bytes` is an optional feature; it is unconditional (`roundhouse-schema/src/schema.rs:245`). `rust_decimal` is called a feature but the feature is `decimal`. The repository URL looks like a placeholder; examples are publishable by default. | Correct the README, set `publish = false` on examples, verify the URL, add a quickstart and a "when not to use this" section. |
| F-19 | `mount!(state = ...)` is accepted and deliberately ignored (`roundhouse-macros/src/mount.rs:85-89`). Silently ignoring an attribute is the opposite of the project's philosophy. | Remove it, or emit a deprecation warning/error. |
| F-20 | A handler listed in `routes!` without `#[handler]` fails with "cannot find value `x_ROUNDHOUSE_HANDLER`" (`roundhouse-macros/src/routes.rs:377`). | Rename the marker so the missing item names the problem, or emit a targeted `compile_error!` via a helper trait. |
| F-21 | A missing `Extension<T>` documents a `Problem` 500 (`roundhouse-macros/src/handler.rs:166-170`), but the runtime uses axum's rejection, which is not a `Problem`. | Wrap `Extension` like the other extractors, or stop documenting a body the runtime does not produce. |
| F-22 | `validate = false` still publishes constraints, and `DocumentRejections::None` removes documented failures (`src/router.rs:190-194,792-794`). | Mark the operation (`x-roundhouse-validation: off`, `x-roundhouse-rejections: none`) so deliberate divergence is visible. |
| F-23 | `SchemeKindConflict` reports `first` and `second` as the same source (`src/doc/build.rs:361-379`), because the map stores only the scheme. | Store the first `Source` alongside the scheme; the diagnostic then points at both declarations. |

Minor observations, not worth an ID: `component_name` counts every registered type globally (`src/doc/mod.rs:169-187`), so an out-of-scope type can force module-qualified names in an unrelated app; the full document is built even when neither OpenAPI nor response validation is enabled (`src/router.rs:246-270`), so doc errors can block an app that never serves docs; `stored_*_for` looks up by `type_name` strings (`src/doc/mod.rs:317-347`), which is fine internally but brittle if the representation changes.

---

## 9. Cross-cutting analysis

### 9.1 Three kinds of drift, only one of which is treated as a bug

The codebase implicitly recognises three categories, but only the first is enforced:

1. **Enforcement gaps** — the document claims a requirement the router does not check. Examples: F-01 (scopes), F-02 (mount security), F-04 (path parameters, in the other direction), F-05 (status), F-03 (error responses).
2. **Derivation gaps** — the router does something the document cannot see. Examples: F-04 (foreign extractors), F-16 (optional auth behaviour), F-08 (composite bodies in schema mode), F-21 (Extension rejection).
3. **Marked opt-outs** — `hidden`, `absolute`, `validate = false`, `responses(..)`, `DocumentRejections::None`. These are legitimate, but they are currently invisible in the document except for two extensions (`x-roundhouse-unverified-body`, `x-roundhouse-public`).

The framework needs an explicit, testable invariant instead of a slogan. A workable one:

> For every operation O: every documented security requirement is enforced by an extractor or a documented layer; every path capture is documented exactly once; every success status the router returns in the example suites is documented (checked by response validation); and every deliberate exception carries an `x-roundhouse-*` marker.

Then encode it as a test that walks `Built::routes()` and the document. That test is the product.

### 9.2 The discovery bet

Link-time `inventory` plus `module_path!()` scoping is a genuine architectural bet, and the rationale — locality — is sound. A leaf module declares its route beside its handler; a subtree declares its prefix once; no wiring function. That is the best part of the DX, and I would keep it.

The costs are structural, though:

- **Invisible control flow.** The route tree exists only at runtime and in `roundhouse routes`. No IDE navigation, no type-checked composition, no way to see the tree in review except by running the CLI.
- **Discovery failures are silent.** Empty scope, empty app, and (until fixed) state mismatch. The failure mode of magic must be a diagnostic, not an empty router.
- **Doc generation is runtime.** The document is produced by booting the app, so environment and side effects leak into CI. "Build artifact" is aspirational until there is a non-booting path (build script, codegen, or a registry-only dump binary).
- **The registry is public API.** Twenty fields per record is a long-term semver anchor; hide it now.
- **The table is not a value.** `routes!` expands to side effects; you cannot unit-test or compose a subtree, and the only seam is `scope`/`include`. A `routes!` that also yields a `RouteTable` value would give an escape hatch without giving up locality — worth considering before 1.0.

### 9.3 Positioning

"Metaframework" sets expectations the project deliberately does not meet: scaffolding, a dev loop with reload, configuration conventions, templates, sessions/data, deploy. In Rust the label also connotes full-stack (Leptos, Dioxus, Tuono). The README's "Out of scope" section is honest, but it contradicts the title.

The project has a sharper, defensible position: **an OpenAPI-first axum framework where the document cannot drift**. The competitive set is `aide`, `utoipa-axum`, `poem-openapi`/`okapi`, and `salvo`'s OpenAPI support. Roundhouse's differentiators are the module-tree mount inheritance, runtime constraint enforcement from the same declaration, a single `Problem` shape, and build failures instead of omissions. That is enough to win a real niche — teams with JSON APIs, generated clients and CI-checked contracts — without pretending to be Rails.

Two consequences: write the comparison (especially "why not aide/utoipa-axum?", which is the first question a Rust developer asks), and make the integrity claim literally true before marketing it. The scopes test is the kind of thing a skeptical reviewer will find in ten minutes and then discount the rest of the README.

---

## 10. Remediation plan

### P0 — restore the invariant (4–6 days)

| IDs | Outcome |
|---|---|
| F-01, F-02 | Security requirements are enforced or rejected at build time; scopes have a runtime decision. |
| F-03, F-05 | Default error type and status-bearing tuples are documented correctly. |
| F-04 | Every path capture is documented or the build fails; scalar/foreign `Path` no longer emits an invalid document. |
| F-06 | `try_build` is total; empty scope/app are build errors. |

**Exit criteria**

- `try_build` returns `Err` for every negative fixture; no panic path remains.
- A document-validity test passes for all seven examples (every capture has a parameter, every parameter is used, operations are unique, security schemes are referenced and complete).
- For every operation with a security requirement, an unauthenticated request fails; for every scope, a token lacking it fails.
- A `Result<_, Problem>` route documents its 404 (or `default`).
- `(StatusCode, Json<T>)` never documents 200 when the runtime returns 201/204.

### P1 — semantics, tooling, packaging (1–1.5 weeks)

F-07 (override policy), F-08 (inline body schema), F-09 (MSRV + CI), F-10 (feature gate), F-11 (registry API), F-12 (crate name resolution), F-13 (CLI split), F-14 (diagnostics), F-15 (`operationId`), F-16 (optional auth semantics).

### P2 — polish (2–3 days)

F-17 through F-23, plus the README quickstart/comparison, `publish = false` on examples, a CHANGELOG, and `cargo deny`/`cargo semver-checks` once published.

---

## 11. Test and CI additions

1. **`document_is_valid_openapi`** — validate the generated document against the OpenAPI 3.1 meta-schema (or a maintained validator). Catches F-04, F-05 and future schema mistakes.
2. **`every_capture_is_documented`** — walk `Built::routes()`, extract `{...}` captures, compare with the document's path parameters. Catches F-04, including mount captures.
3. **`documented_security_is_enforced`** — for every operation with a `security` requirement, issue an unauthenticated request and assert 401/403; with scopes, issue a token without the scope and assert 403. Catches F-01 and F-02. This is the single most valuable test in the list.
4. **`documented_status_matches_runtime`** — table-driven over the examples' routes; run `validate_responses()` in the example test suites. Catches F-03, F-05 and F-07 drift.
5. **`try_build_never_panics`** — `catch_unwind` over negative fixtures: state mismatch, empty scope, scalar `Path`, mount capture, invalid template.
6. **`error_types_are_documented`** — every handler's `E` in `Result<T, E>` must yield at least one documented response, or a build warning (an error under `strict()`).
7. **MSRV job** (`cargo +1.88 test`) and **`cargo deny`**; `cargo doc -D warnings` is already in CI.
8. **Doc snapshots for both 3.1 and 3.0** — the 3.0 downgrade currently has no conformance check.

---

## 12. Recommendation

**Today.** Use Roundhouse for an internal JSON API if you use derived `ApiError` types (not `Problem`), avoid `Auth<A, Scopes>` and mount-level `security`, and define `PathParams` structs for every path parameter. Under those constraints it is a pleasant, coherent framework, and the module-tree mounts are better than the alternatives.

**Do not** treat the generated document as an authorization contract yet, and do not rely on `try_build` to be non-panicking.

**After P0.** I would recommend it for contract-first teams: the wedge is real, and the implementation is close. The remaining gap is mostly a week of correctness work plus the validity/parity tests, not a redesign.

**On the label.** Either drop "metaframework" and call it an OpenAPI-first axum framework, or commit to the missing metaframework pieces (scaffolding, dev loop, configuration, a persistence story). The current README claims the first and excludes the second; the mismatch costs credibility with exactly the audience that would appreciate the engineering.

---

## Appendix A — Files reviewed

`Cargo.toml`, `Cargo.lock` (dependencies only), `README.md`, `.github/workflows/ci.yml`, `.cargo/config.toml`, `compose.dev.yaml`, `Containerfile.dev`.

`src/`: `lib.rs`, `registry.rs`, `prefix.rs`, `router.rs`, `auth.rs`, `problem.rs`, `extract.rs`, `attachment.rs`, `build_error.rs`, `bin/roundhouse.rs`, `doc/{mod,build,layer,response,downgrade}.rs`.

`roundhouse-macros/src/`: `lib.rs`, `routes.rs`, `mount.rs`, `handler.rs`, `api_schema.rs` (partial), `api_response.rs` (partial), `path_params.rs`, `query_params.rs`, `multipart.rs` (partial), `util.rs`.

`roundhouse-schema/src/`: `lib.rs`, `schema.rs`, `validate.rs`, `engine.rs`.

`roundhouse-multipart/src/lib.rs` (skimmed).

`tests/`: `honesty.rs`, `state_mismatch.rs`, `engine.rs`, `surface.rs` (partial), `spine.rs` (partial), `conflicts.rs` (partial), `openapi.golden.json` (structure and path parameters), `ui/`.

`examples/`: all seven crates, with closer reading of `blog`, `auth`, `notes` and `layers`; README claims cross-checked against source.

## Appendix B — Open questions that need a test run

1. **405 response shape under nested mounts.** The suite asserts the status but not the body/content type. Is a method-not-allowed response a `Problem` like the 404 fallback, or axum's empty 405? If the latter, the "one parser" claim has an exception.
2. **`route_layer` on an ancestor mount.** Does a `layer_mode = route` on a parent wrap descendant routes? The tests cover the module's own routes and a 404, not a child route. If it wraps children, the docs should say so; if not, the naming is misleading.
3. **`MatchedPath` in nested routers.** `validate_responses` keys on `MatchedPath` (`src/doc/response.rs:132-141`). If nested routers report only the inner path, response validation silently skips every mounted route. Confirm with a test.
4. **Inventory under LTO / `-Z build-std` / wasm.** The registry relies on linker-section retention; the project should test the release profile it recommends.
5. **Compile-time and runtime cost.** No measurements are committed. `jsonschema` is unconditional; the macro expansion cost is unknown. A simple `cargo build -p notes-example` timing before/after feature-gating would guide F-10.
6. **3.0 downgrade fidelity.** `openapi_30` is documented as best-effort; there is no test that a 3.0 consumer accepts the output.

## Appendix C — Suggested extension conventions

If the deliberate divergences stay, give each one a marker so a consumer can detect it mechanically:

| Extension | Meaning |
|---|---|
| `x-roundhouse-unverified-body` | A body extractor the document cannot verify (existing). |
| `x-roundhouse-public` | Intentionally unauthenticated (existing). |
| `x-roundhouse-overridden-response` | A declared response replaces an inferred one. |
| `x-roundhouse-validation: off` | Runtime constraints are not enforced for this operation. |
| `x-roundhouse-rejections: none` | Extractor failures are not documented. |
| `x-roundhouse-unverified-security` | Security is declared without an enforcer (a bug marker, not a feature). |
| `x-roundhouse-scopes` | Scopes are published but not enforced by the application. |

The rule to encode in tests: **no operation may carry a security, parameter, response or validation claim without either an enforcement proof or one of these markers.**
