use roundhouse_path::{join, normalise_template};
use syn::Result;
use syn::spanned::Spanned;

use crate::util::last_segment;

use super::parse::{Body, Entry};

pub(super) fn flatten<'a>(body: &'a Body, prefix: &str, out: &mut Vec<(String, &'a Entry)>) {
    for entry in &body.entries {
        let path = join(prefix, &entry.path.value());
        out.push((path, entry));
    }
    for group in &body.groups {
        if group.shared_path {
            flatten(&group.body, prefix, out);
        } else {
            let prefix = join(prefix, &group.path.value());
            flatten(&group.body, &prefix, out);
        }
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
        let path = normalise_template(full_path);

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
