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
///
/// `{{` and `}}` are matchit's escaped literal braces and are not captures.
pub fn captures(template: &str) -> Vec<String> {
    capture_specs(template)
        .into_iter()
        .map(|(name, _)| name)
        .collect()
}

/// True when the template contains a `{*name}` catch-all capture.
pub fn has_catch_all(template: &str) -> bool {
    capture_specs(template)
        .into_iter()
        .any(|(_, catch_all)| catch_all)
}

/// Captures with their catch-all flag, mirroring matchit's brace escaping.
fn capture_specs(template: &str) -> Vec<(String, bool)> {
    let chars = unescaped_chars(template);
    let mut specs = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == (b'{', false) {
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != (b'}', false) {
                end += 1;
            }
            if end >= chars.len() {
                break;
            }
            let name: Vec<u8> = chars[start..end].iter().map(|(byte, _)| *byte).collect();
            let name = String::from_utf8_lossy(&name).into_owned();
            let catch_all = name.starts_with('*');
            let name = name.strip_prefix('*').unwrap_or(&name).to_string();
            specs.push((name, catch_all));
            index = end + 1;
            continue;
        }
        index += 1;
    }
    specs
}

/// matchit unescapes `{{` / `}}` before it looks for captures. Do the same so a literal brace is
/// never mistaken for a parameter (and a real capture is never hidden by an escaped brace).
fn unescaped_chars(template: &str) -> Vec<(u8, bool)> {
    let bytes = template.as_bytes();
    let mut chars = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if (byte == b'{' || byte == b'}') && bytes.get(index + 1) == Some(&byte) {
            chars.push((byte, true));
            index += 2;
        } else {
            chars.push((byte, false));
            index += 1;
        }
    }
    chars
}

/// Compile-time check that a `Path<T>` extractor covers every capture in the leaf template.
///
/// Called from `routes!` as a `const` item. An empty `NAMES` (a scalar path param, or no `Path`
/// extractor at all) skips the check: the framework cannot recover a name from the type.
///
/// The check is a subset check, not an equality check. A mount prefix can contribute captures the
/// route table cannot see at compile time, so `Path<T>` is allowed to declare extra names; the
/// build validates those against the full resolved path. A capture that is in the leaf template
/// but missing from `Path<T>` is still a compile error.
pub const fn assert_path_names(template: &str, names: &[&str]) {
    if !path_names_match(template, names) {
        panic!("roundhouse: Path<T> does not declare every capture in the route's path template");
    }
}

/// The pure predicate behind [`assert_path_names`].
///
/// `routes!` uses it directly so the compile-time message can name the handler and the resolved
/// template; a const function cannot format a dynamic message.
pub const fn path_names_match(template: &str, names: &[&str]) -> bool {
    if names.is_empty() {
        return true;
    }

    let bytes = template.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        // matchit's escaped literal braces are not captures.
        if index + 1 < bytes.len()
            && ((bytes[index] == b'{' && bytes[index + 1] == b'{')
                || (bytes[index] == b'}' && bytes[index + 1] == b'}'))
        {
            index += 2;
            continue;
        }
        if bytes[index] == b'{' {
            let start = index + 1;
            let mut end = start;
            while end < bytes.len() && bytes[end] != b'}' {
                end += 1;
            }
            if end >= bytes.len() {
                break;
            }
            let mut capture_start = start;
            if capture_start < end && bytes[capture_start] == b'*' {
                capture_start += 1;
            }
            let capture_len = end - capture_start;

            let mut declared_index = 0;
            let mut found = false;
            while declared_index < names.len() {
                let declared = names[declared_index].as_bytes();
                if declared.len() == capture_len {
                    let mut byte_index = 0;
                    let mut same = true;
                    while byte_index < capture_len {
                        if declared[byte_index] != bytes[capture_start + byte_index] {
                            same = false;
                            break;
                        }
                        byte_index += 1;
                    }
                    if same {
                        found = true;
                        break;
                    }
                }
                declared_index += 1;
            }
            if !found {
                return false;
            }

            index = end + 1;
            continue;
        }
        index += 1;
    }
    true
}

/// Normalise captures to `{}` and wildcards to `{*}`, so two templates that `matchit` treats as
/// the same route shape compare equal.
pub fn normalise_template(template: &str) -> String {
    let chars = unescaped_chars(template);
    let mut out = String::with_capacity(template.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == (b'{', false) {
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != (b'}', false) {
                end += 1;
            }
            if end < chars.len() {
                if chars[start..end].iter().any(|(byte, _)| *byte == b'*') {
                    out.push_str("{*}");
                } else {
                    out.push_str("{}");
                }
                index = end + 1;
                continue;
            }
        }
        out.push(chars[index].0 as char);
        index += 1;
    }
    out
}

/// Reject templates axum's router cannot serve, with a migration hint for axum 0.7 syntax.
pub fn validate_template(template: &str) -> Result<(), String> {
    if template.split('/').any(|segment| segment.starts_with(':')) {
        return Err(format!(
            "path `{template}` uses axum 0.7 syntax; use `{{id}}` for a capture and `{{*rest}}` for a wildcard"
        ));
    }

    // matchit is the tree axum uses. Probing it here turns every template shape axum would panic
    // on (`{}`, `{id`, `/{bar}-foo`, a non-final catch-all) into a build error instead.
    let path = join("", template);
    // matchit names parameters `a`..=`z` and panics after 25 normal captures. Guard the limit
    // rather than let a pathological template unwind inside `try_build`.
    let named = capture_specs(&path)
        .into_iter()
        .filter(|(_, catch_all)| !catch_all)
        .count();
    if named > 25 {
        return Err(format!(
            "path `{template}` has more than 25 captures, which the router cannot represent"
        ));
    }
    let mut probe = matchit::Router::<()>::new();
    probe
        .insert(path.as_str(), ())
        .map_err(|error| format!("path `{template}` cannot be served: {error}"))?;
    Ok(())
}

/// Validate a `mount!` prefix. Captures are allowed; catch-alls are not.
///
/// A catch-all is only valid at the end of a full route. In a mount prefix it would have to
/// swallow the whole subtree, and any child path would make the full template invalid.
pub fn validate_mount_prefix(prefix: &str) -> Result<(), String> {
    if prefix.is_empty() {
        return Ok(());
    }
    validate_template(prefix)?;
    if has_catch_all(prefix) {
        return Err(format!(
            "mount prefix `{prefix}` ends in a catch-all; declare `{{*rest}}` in a leaf route instead"
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
    fn const_check_allows_mount_captures_and_reordering() {
        // A mount capture is invisible to the macro, so extra names are allowed.
        const _: () = assert_path_names("/{issue_id}", &["project_id", "issue_id"]);
        // Serde matches by name, so field order must not matter.
        const _: () = assert_path_names("/{issue_id}", &["issue_id", "project_id"]);
        // A leaf `/` has no captures of its own; the mount supplies them.
        const _: () = assert_path_names("/", &["project_id"]);
        const _: () = assert_path_names("/{id}", &[]);
        // Escaped literal braces are not captures.
        const _: () = assert_path_names("/{{literal}}", &["anything"]);
    }

    #[test]
    fn escaped_braces_are_not_captures() {
        assert_eq!(captures("/{{literal}}"), Vec::<String>::new());
        assert_eq!(captures("/files/{{name}}/{id}"), ["id"]);
        assert!(!has_catch_all("/{{*not-a-catch-all}}"));
        assert!(has_catch_all("/files/{*rest}"));
    }

    #[test]
    fn templates_are_normalised() {
        assert_eq!(normalise_template("/blog/{id}"), "/blog/{}");
        assert_eq!(normalise_template("/blog/{post_id}"), "/blog/{}");
        assert_eq!(normalise_template("/files/{*rest}"), "/files/{*}");
        assert_eq!(normalise_template("/healthz"), "/healthz");
        // Escaped literal braces are static text, not captures.
        assert_eq!(normalise_template("/{{a}}"), "/{a}");
        assert_eq!(normalise_template("/{{b}}"), "/{b}");
    }

    #[test]
    fn old_syntax_is_rejected() {
        assert!(validate_template("/blog/:id").is_err());
        assert!(validate_template("/blog/{id}").is_ok());
    }

    #[test]
    fn malformed_templates_are_rejected() {
        for template in ["/{}", "/{id", "/{id}}", "/{bar}-foo", "/{*rest}/tail"] {
            assert!(
                validate_template(template).is_err(),
                "`{template}` should be rejected"
            );
        }
        for template in ["/", "/blog", "/foo-{bar}", "/files/{*rest}", "/{{literal}}"] {
            assert!(
                validate_template(template).is_ok(),
                "`{template}` should be accepted"
            );
        }
    }

    #[test]
    fn mount_prefixes_allow_captures_but_not_catch_alls() {
        assert!(validate_mount_prefix("").is_ok());
        assert!(validate_mount_prefix("/{project_id}/issues").is_ok());
        assert!(validate_mount_prefix("/{*rest}").is_err());
        assert!(validate_mount_prefix("/:id").is_err());
        assert!(validate_mount_prefix("/{bar}-foo").is_err());
    }

    #[test]
    fn more_captures_than_matchit_can_name_is_an_error() {
        let template: String = (0..26).map(|index| format!("/{{p{index}}}")).collect();
        assert!(validate_template(&template).is_err());
    }
}
