# Changelog

## 0.3.0

### Breaking

- Strict builds are the default. Unverified bodies, responses and extractors, undocumented
  `any` routes, empty modules and unacknowledged merged routers are build errors.
  `App::permissive()` restores warning-only builds.
- Removed the `public` route option. Use `anonymous` to clear inherited security.
- Removed the 0.2 `@responses:`, `@responses(override):`, `@response-descriptions:` and `@path:`
  doc tags. Use `#[handler(..)]`.
- Removed `App::router()` and the `HandlerDocs` alias. Use `App::into_router()` /
  `Built::router()` and `OperationDoc`.
- utoipa is now an implementation detail. The documented path is `#[roundhouse::schema]` with
  `#[roundhouse(..)]` constraints. utoipa names live in `roundhouse::interop`.
  `impl_api_schema!` is now `roundhouse::interop::impl_from_utoipa!`.
- `Info` and `OpenApi` no longer dereference to their utoipa values; use `as_utoipa()` or
  `into_utoipa()`.
- Prefix groups use `prefix "/path" [options] { .. }`. `"/path" [options] { .. }` is a compile
  error.
- `app_state!` outside the crate root is a compile error at the call site.
- `App::merge(router)` requires `App::merge_undocumented(router, reason)`.

### Added

- `#[roundhouse(..)]` constraints for lengths, patterns, numeric and collection bounds, formats,
  descriptions, examples, `unique_items`, `validate_with` and component names.
- `roundhouse::interop` for utoipa types and generated `ToSchema` adapters.
- `App::permissive()` and `App::merge_undocumented()`.
- A dependency-free throughput benchmark (`cargo bench -p roundhouse --bench throughput`).
- Documentation for the new authoring surface and the 0.3 migration path.

### Changed

- Table-local duplicate checks skip `#[cfg(..)]`-gated entries; collisions that survive `cfg`
  are caught by the resolved route table.
- `#[roundhouse(..)]` constraints are patched into the published schema, so the document and the
  validator cannot disagree.
