use syn::Result;
use syn::spanned::Spanned;

use crate::util::last_segment;

use super::parse::{Body, Entry};

pub(super) fn flatten<'a>(body: &'a Body, prefix: &str, out: &mut Vec<(String, &'a Entry)>) {
    for entry in &body.entries {
        let path = join_paths(prefix, &entry.path.value());
        out.push((path, entry));
    }
    for group in &body.groups {
        if group.shared_path {
            flatten(&group.body, prefix, out);
        } else {
            let prefix = join_paths(prefix, &group.path.value());
            flatten(&group.body, &prefix, out);
        }
    }
}

pub(super) fn join_paths(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_string()
    } else if path == "/" {
        prefix.to_string()
    } else {
        format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

/// Table-local checks. These are the checks a single `routes!` invocation can make from the
/// tokens alone: two entries that resolve to the same method and template, and two entries whose
/// operation ids collide. Cross-table collisions still need the resolved route table.
pub(super) fn validate_entries(entries: &[(String, &Entry)]) -> Result<()> {
    use std::collections::HashMap;

    let mut routes: HashMap<(String, String), &Entry> = HashMap::new();
    let mut explicit_ids: HashMap<String, proc_macro2::Span> = HashMap::new();
    let mut derived_ids: HashMap<(String, String), &Entry> = HashMap::new();

    for (full_path, entry) in entries {
        let method = entry.method.to_string();
        let path = normalise_path(full_path);

        if let Some(first) = routes.get(&(method.clone(), path.clone())) {
            return Err(duplicate_route_error(first, entry, &method, &path));
        }
        if let Some((first_method, first)) =
            routes
                .iter()
                .find_map(|((existing, existing_path), first)| {
                    (existing_path == &path
                        && (existing == "any" || method == "any")
                        && existing != &method)
                        .then_some((existing, *first))
                })
        {
            let mut error = syn::Error::new(
                entry.method.span(),
                format!(
                    "route `{method} {path}` conflicts with `{first_method} {path}`; an `any` \
                     entry covers every method"
                ),
            );
            error.combine(syn::Error::new(
                first.method.span(),
                "the conflicting entry",
            ));
            return Err(error);
        }
        routes.insert((method.clone(), path.clone()), entry);

        match &entry.operation_id {
            Some(id) => {
                if let Some(first_span) = explicit_ids.insert(id.value(), id.span()) {
                    let mut error = syn::Error::new(
                        id.span(),
                        format!(
                            "duplicate operationId `{}` in this routes! table",
                            id.value()
                        ),
                    );
                    error.combine(syn::Error::new(first_span, "first declared here"));
                    return Err(error);
                }
            }
            None => {
                // The derived id is `method + module path + handler`; within one table the module
                // path is shared, so the method and handler name are enough to detect a collision.
                let handler = last_segment(&entry.handler);
                let key = (method.clone(), handler.clone());
                if let Some(first) = derived_ids.insert(key, entry) {
                    let mut error = syn::Error::new(
                        entry.handler.span(),
                        format!(
                            "two entries derive the same operationId `{}_{}`",
                            method.to_ascii_lowercase(),
                            handler
                        ),
                    );
                    error.combine(syn::Error::new(
                        first.handler.span(),
                        "the first entry; set an explicit `operation_id = \"...\"` on one of them",
                    ));
                    return Err(error);
                }
            }
        }
    }

    Ok(())
}

fn duplicate_route_error(first: &Entry, second: &Entry, method: &str, path: &str) -> syn::Error {
    let mut error = syn::Error::new(
        second.method.span(),
        format!("duplicate route `{method} {path}` in this routes! table"),
    );
    error.combine(syn::Error::new(
        first.method.span(),
        format!("first declared as `{method} {path}`"),
    ));
    error
}

/// Normalise a template the way the router does: captures collapse to `{}` / `{*}`, escaped
/// braces are kept, and repeated slashes are collapsed.
pub(super) fn normalise_path(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    let mut out = String::with_capacity(path.len());
    let mut index = 0;
    while index < chars.len() {
        if index + 1 < chars.len()
            && ((chars[index] == '{' && chars[index + 1] == '{')
                || (chars[index] == '}' && chars[index + 1] == '}'))
        {
            out.push(chars[index]);
            index += 2;
            continue;
        }
        if chars[index] == '{' {
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != '}' {
                end += 1;
            }
            if end < chars.len() {
                if chars[start..end].contains(&'*') {
                    out.push_str("{*}");
                } else {
                    out.push_str("{}");
                }
                index = end + 1;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }

    let mut collapsed = String::with_capacity(out.len());
    let mut last_slash = false;
    for ch in out.chars() {
        if ch == '/' {
            if !last_slash {
                collapsed.push('/');
            }
            last_slash = true;
        } else {
            collapsed.push(ch);
            last_slash = false;
        }
    }
    if collapsed.is_empty() {
        "/".to_string()
    } else {
        collapsed
    }
}
