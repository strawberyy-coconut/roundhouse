//! Path joining, module ancestry, and duplicate detection.

/// Join a mount prefix and a leaf path.
///
/// Trailing `/` on the left and leading `/` on the right are trimmed, repeated `/` collapse, and
/// `{id}` / `{*rest}` segments are opaque. `"/blog" + "/"` is `/blog`, not `/blog/`.
pub fn join(left: &str, right: &str) -> String {
    let mut pieces: Vec<&str> = Vec::new();
    for piece in left.split('/').chain(right.split('/')) {
        if !piece.is_empty() {
            pieces.push(piece);
        }
    }
    if pieces.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", pieces.join("/"))
    }
}

/// Join an ordered list of prefixes and a leaf.
pub fn join_all<'a>(prefixes: impl IntoIterator<Item = &'a str>, leaf: &str) -> String {
    let mut path = String::new();
    for prefix in prefixes {
        path = join(&path, prefix);
    }
    join(&path, leaf)
}

/// True when `module` is `ancestor` or a descendant of it, matching on `::` boundaries.
pub fn is_ancestor(ancestor: &str, module: &str) -> bool {
    module == ancestor
        || module
            .strip_prefix(ancestor)
            .is_some_and(|rest| rest.starts_with("::"))
}

/// Scope matching. `app` owns `app` and `app::users`, but not `admin::users` or
/// `application::users`.
pub fn in_scope(scope: &str, module: &str) -> bool {
    is_ancestor(scope, module)
}

/// The number of `::`-separated segments in a module path.
pub fn depth(module: &str) -> usize {
    module.split("::").count()
}

/// Decode `{name}` and `{*name}` captures from a path template, in order.
pub fn captures(template: &str) -> Vec<String> {
    let mut names = Vec::new();
    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'{' {
            let start = index + 1;
            if let Some(offset) = template[start..].find('}') {
                let end = start + offset;
                let mut name = &template[start..end];
                if let Some(stripped) = name.strip_prefix('*') {
                    name = stripped;
                }
                names.push(name.to_string());
                index = end + 1;
                continue;
            }
        }
        index += 1;
    }
    names
}

/// Compile-time check that a `Path<T>` extractor's names equal the template's captures.
///
/// Called from `routes!` as a `const` item. An empty `NAMES` (a scalar path param, or no `Path`
/// extractor at all) skips the check: the framework cannot recover a name from the type.
pub const fn assert_path_names(template: &str, names: &[&str]) {
    if names.is_empty() {
        return;
    }

    let bytes = template.as_bytes();
    let mut index = 0;
    let mut name_index = 0;
    while index < bytes.len() {
        if bytes[index] == b'{' {
            let start = index + 1;
            let mut offset = 0;
            while start + offset < bytes.len() && bytes[start + offset] != b'}' {
                offset += 1;
            }
            if start + offset >= bytes.len() {
                break;
            }
            let mut capture_start = start;
            if capture_start < start + offset && bytes[capture_start] == b'*' {
                capture_start += 1;
            }
            let capture_len = start + offset - capture_start;

            if name_index >= names.len() {
                panic!("roundhouse: path template has more captures than Path<T> declares");
            }
            let declared = names[name_index].as_bytes();
            if declared.len() != capture_len {
                panic!("roundhouse: Path<T> capture names do not match the path template");
            }
            let mut i = 0;
            while i < capture_len {
                if declared[i] != bytes[capture_start + i] {
                    panic!("roundhouse: Path<T> capture names do not match the path template");
                }
                i += 1;
            }

            name_index += 1;
            index = start + offset + 1;
            continue;
        }
        index += 1;
    }

    if name_index != names.len() {
        panic!("roundhouse: Path<T> declares more names than the path template captures");
    }
}

/// Reject axum 0.7 `:id` syntax with a migration hint.
pub fn validate_template(template: &str) -> Result<(), String> {
    if template.split('/').any(|segment| segment.starts_with(':')) {
        return Err(format!(
            "path `{template}` uses axum 0.7 syntax; use `{{id}}` for a capture and `{{*rest}}` for a wildcard"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_rules() {
        assert_eq!(join("", "/x"), "/x");
        assert_eq!(join("/x", ""), "/x");
        assert_eq!(join("", ""), "/");
        assert_eq!(join("/blog", "/"), "/blog");
        assert_eq!(join("/blog/", "/{id}"), "/blog/{id}");
        assert_eq!(join("/a//b", "//c"), "/a/b/c");
        assert_eq!(join("/", "/"), "/");
    }

    #[test]
    fn ancestry_matches_on_segments() {
        assert!(is_ancestor("app", "app"));
        assert!(is_ancestor("app", "app::users"));
        assert!(!is_ancestor("app", "admin::users"));
        assert!(!is_ancestor("app", "application::users"));
    }

    #[test]
    fn captures_are_ordered() {
        assert_eq!(captures("/blog/{id}/files/{*rest}"), ["id", "rest"]);
        assert_eq!(captures("/healthz"), Vec::<String>::new());
    }

    #[test]
    fn const_check_accepts_matching_names() {
        const _: () = assert_path_names("/blog/{id}", &["id"]);
        const _: () = assert_path_names("/x/{a}/{*rest}", &["a", "rest"]);
    }

    #[test]
    fn old_syntax_is_rejected() {
        assert!(validate_template("/blog/:id").is_err());
        assert!(validate_template("/blog/{id}").is_ok());
    }
}
