use std::any::TypeId;
use std::collections::{BTreeSet, HashMap};

use crate::auth::AuthGuard;
use crate::build_error::BuildError;
use crate::prefix;
use crate::registry::{Method, Module, RouteRecord};

use super::built::ResolvedRoute;

pub(super) fn resolve_tree(
    roots: &[&'static Module],
    strict: bool,
    global_layer_docs: &[fn() -> crate::doc::layer::LayerDoc],
    expected_state: Option<TypeId>,
    expected_state_name: &'static str,
    merges: usize,
    diagnostics: &mut Vec<crate::build_error::Diagnostic>,
) -> Result<Vec<ResolvedRoute>, BuildError> {
    let mut state = ResolveState {
        expected_state,
        expected_state_name,
        global_layer_docs,
        seen: HashMap::new(),
        resolved: Vec::new(),
    };

    let mut seen_roots: BTreeSet<*const Module> = BTreeSet::new();
    for root in roots {
        if !seen_roots.insert(*root as *const Module) {
            continue;
        }
        let inherited = Inherited::default();
        walk_module(root, inherited, strict, diagnostics, &mut state)?;
    }

    let mut resolved = state.resolved;
    resolved.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then_with(|| a.method.cmp(&b.method))
            .then_with(|| a.operation_id.cmp(&b.operation_id))
    });

    // `try_build` is meant to be total: probe the same matchit tree axum will build so a
    // shape conflict (e.g. `/{a}` next to `/{*rest}`) is a `BuildError`, not a panic during
    // assembly. One path may legitimately carry several methods, so insert each path once.
    let mut probe = matchit::Router::<()>::new();
    let mut previous_path: Option<&str> = None;
    for route in &resolved {
        if previous_path == Some(route.path.as_str()) {
            continue;
        }
        previous_path = Some(route.path.as_str());
        // A prefix and leaf that are valid alone can still combine into a template the router
        // cannot serve (for example more captures than matchit can name). Re-check the full
        // path before handing it to matchit, which panics past its own limit.
        if let Err(reason) = prefix::validate_template(route.path.as_str()) {
            return Err(BuildError::InvalidTemplate {
                reason,
                source: route.record.source_ref(),
            });
        }
        if let Err(error) = probe.insert(route.path.as_str(), ()) {
            return Err(BuildError::RouteConflict {
                path: route.path.clone(),
                reason: error.to_string(),
                source: route.record.source_ref(),
            });
        }
    }

    let mut operations: HashMap<&str, &'static RouteRecord> = HashMap::new();
    for route in &resolved {
        if let Some(previous) = operations.insert(route.operation_id.as_str(), route.record) {
            return Err(BuildError::DuplicateOperationId {
                operation_id: route.operation_id.clone(),
                first: previous.source_ref(),
                second: route.record.source_ref(),
            });
        }
    }

    if resolved.is_empty() && merges == 0 {
        return Err(BuildError::EmptyApp);
    }

    Ok(resolved)
}

struct ResolveState<'a> {
    expected_state: Option<TypeId>,
    expected_state_name: &'static str,
    global_layer_docs: &'a [fn() -> crate::doc::layer::LayerDoc],
    seen: HashMap<(Method, String), &'static RouteRecord>,
    resolved: Vec<ResolvedRoute>,
}

#[derive(Default, Clone)]
struct Inherited {
    prefix: String,
    tags: Vec<&'static str>,
    guards: Vec<AuthGuard>,
    hidden: bool,
    has_layer: bool,
    layer_docs: Vec<crate::doc::layer::LayerDoc>,
}

fn walk_module(
    module: &'static Module,
    inherited: Inherited,
    strict: bool,
    diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    state: &mut ResolveState<'_>,
) -> Result<bool, BuildError> {
    if let Err(reason) = prefix::validate_prefix(module.prefix) {
        return Err(BuildError::InvalidPrefix {
            prefix: module.prefix.to_string(),
            reason,
            source: module.source_ref(),
        });
    }

    let parent_prefix = inherited.prefix.trim_end_matches('/');
    let child_prefix = module.prefix.trim_end_matches('/');
    if !parent_prefix.is_empty()
        && parent_prefix != "/"
        && !child_prefix.is_empty()
        && (child_prefix == parent_prefix || child_prefix.starts_with(&format!("{parent_prefix}/")))
    {
        crate::build_error::report(
            diagnostics,
            strict,
            crate::build_error::Diagnostic::new(
                module.source_ref(),
                format!(
                    "module prefix `{}` repeats the enclosing prefix `{}`; the resolved \
                     prefix is `{}`",
                    module.prefix,
                    inherited.prefix,
                    prefix::join(&inherited.prefix, module.prefix)
                ),
            ),
        )?;
    }

    if let Some(expected) = state.expected_state {
        for guard in module.security {
            if (guard.state)() != expected {
                return Err(BuildError::SecurityStateMismatch {
                    name: guard.name.to_string(),
                    expected: state.expected_state_name,
                    actual: (guard.state_name)(),
                    source: module.source_ref(),
                });
            }
        }
    }

    let mut context = Inherited {
        prefix: prefix::join(&inherited.prefix, module.prefix),
        tags: inherited.tags,
        guards: inherited.guards,
        hidden: inherited.hidden,
        has_layer: inherited.has_layer,
        layer_docs: inherited.layer_docs,
    };
    context.tags.extend(module.tags.iter().copied());
    context.guards.extend(module.security.iter().copied());
    context.hidden |= module.hidden;
    context.has_layer |= module.layer.is_some();
    if let Some(doc) = module.doc {
        context.layer_docs.push(doc());
    }

    let mut has_routes = false;
    for record in module.routes {
        has_routes |= emit_route(record, module, &context, strict, diagnostics, state)?;
    }
    for child in module.children {
        has_routes |= walk_module(child, context.clone(), strict, diagnostics, state)?;
    }

    if !has_routes {
        crate::build_error::report(
            diagnostics,
            strict,
            crate::build_error::Diagnostic::new(
                module.source_ref(),
                format!(
                    "module `{}` declares no routes and has no routes below it",
                    module.module_path
                ),
            ),
        )?;
    }

    Ok(has_routes)
}

fn emit_route(
    record: &'static RouteRecord,
    module: &'static Module,
    context: &Inherited,
    strict: bool,
    diagnostics: &mut Vec<crate::build_error::Diagnostic>,
    state: &mut ResolveState<'_>,
) -> Result<bool, BuildError> {
    if let Some(expected) = state.expected_state
        && let Some(actual) = (record.state)()
        && actual != expected
    {
        return Err(BuildError::StateMismatch {
            operation_id: record
                .operation_id
                .unwrap_or(record.handler_name)
                .to_string(),
            expected: (record.state_name)(),
            actual: state.expected_state_name,
            source: record.source_ref(),
        });
    }
    if let Err(reason) = prefix::validate_template(record.path) {
        return Err(BuildError::InvalidTemplate {
            reason,
            source: record.source_ref(),
        });
    }
    if let Some(expected) = state.expected_state {
        for guard in record.security {
            if (guard.state)() != expected {
                return Err(BuildError::SecurityStateMismatch {
                    name: guard.name.to_string(),
                    expected: state.expected_state_name,
                    actual: (guard.state_name)(),
                    source: record.source_ref(),
                });
            }
        }
    }

    if record.absolute {
        let mut ignored = Vec::new();
        if !context.prefix.is_empty() && context.prefix != "/" {
            ignored.push("prefix");
        }
        if !context.tags.is_empty() {
            ignored.push("tags");
        }
        if !context.guards.is_empty() {
            ignored.push("security");
        }
        if context.hidden {
            ignored.push("hidden");
        }
        if context.has_layer {
            ignored.push("layers");
        }
        if !context.layer_docs.is_empty() {
            ignored.push("layer documentation");
        }
        if !ignored.is_empty() {
            crate::build_error::report(
                diagnostics,
                strict,
                crate::build_error::Diagnostic::new(
                    record.source_ref(),
                    format!(
                        "absolute route `{}` ignores its enclosing module's {}; declare the \
                         policy on the entry instead",
                        record.operation_id.unwrap_or(record.handler_name),
                        ignored.join(", ")
                    ),
                ),
            )?;
        }
    }

    let (path, tags, auth, hidden, mut layer_docs) = if record.absolute {
        (
            prefix::join("", record.path),
            Vec::new(),
            dedup_guards(record.security.iter().copied()),
            record.hidden,
            Vec::new(),
        )
    } else {
        (
            prefix::join(&context.prefix, record.path),
            context.tags.clone(),
            dedup_guards(
                context
                    .guards
                    .iter()
                    .copied()
                    .chain(record.security.iter().copied()),
            ),
            context.hidden || record.hidden,
            context.layer_docs.clone(),
        )
    };
    layer_docs.extend(state.global_layer_docs.iter().map(|doc| doc()));

    if let Some(expected) = record.assert_path
        && path != expected
    {
        return Err(BuildError::PathAssertion {
            expected: expected.to_string(),
            actual: path,
            source: record.source_ref(),
        });
    }

    let normalised = prefix::normalise_template(&path);
    let mut captures = prefix::captures(&path);
    let total = captures.len();
    captures.sort();
    captures.dedup();
    if captures.len() != total {
        return Err(BuildError::InvalidTemplate {
            reason: format!("`{path}` declares the same capture name more than once"),
            source: record.source_ref(),
        });
    }

    let operation_id = record
        .operation_id
        .map(str::to_string)
        .unwrap_or_else(|| default_operation_id(record));
    let key = (record.method, normalised.clone());
    if let Some(previous) = state.seen.get(&key) {
        return Err(BuildError::DuplicateRoute {
            method: record.method,
            path,
            first: previous.source_ref(),
            second: record.source_ref(),
        });
    }
    for ((existing_method, existing_path), previous) in &state.seen {
        if *existing_path == normalised
            && (*existing_method == Method::Any || record.method == Method::Any)
            && *existing_method != record.method
        {
            return Err(BuildError::DuplicateRoute {
                method: record.method,
                path,
                first: previous.source_ref(),
                second: record.source_ref(),
            });
        }
    }
    state.seen.insert(key, record);

    // Keep the module in the resolved route for callers that inspect the tree; it is not used
    // for assembly, which walks `Module::routes` directly.
    let _ = module;
    state.resolved.push(ResolvedRoute {
        method: record.method,
        path,
        operation_id,
        module_path: record.module_path,
        record,
        tags,
        auth,
        hidden,
        layer_docs,
    });
    Ok(true)
}

/// `GET blog::posts::show` → `get_blog_posts_show`.
///
/// The id follows the declaration, not the URL, so moving a route under a different prefix does
/// not rename every generated client method. An explicit `operation_id` always wins.
fn default_operation_id(record: &RouteRecord) -> String {
    let mut parts = vec![record.method.as_str().to_ascii_lowercase()];
    for segment in record.module_path.split("::") {
        let segment = sanitize(segment);
        if !segment.is_empty() {
            parts.push(segment);
        }
    }
    let handler = sanitize(record.handler_name);
    if parts.last().is_none_or(|last| *last != handler) {
        parts.push(handler);
    }
    parts.join("_")
}

fn sanitize(input: &str) -> String {
    let mut out = String::new();
    let mut last_underscore = false;
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    out.trim_matches('_').to_string()
}

/// Inherited guards plus the entry's own, with one guard per scheme name.
///
/// A handler that takes `Auth<A>` skips the guard for `A` at assembly time, so a scheme that is
/// declared twice is authenticated once.
fn dedup_guards(guards: impl Iterator<Item = AuthGuard>) -> Vec<AuthGuard> {
    let mut deduped: Vec<AuthGuard> = Vec::new();
    for guard in guards {
        if !deduped.iter().any(|existing| existing.name == guard.name) {
            deduped.push(guard);
        }
    }
    deduped
}
