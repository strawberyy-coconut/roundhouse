//! Explicit `@summary:` and `@description:` doc tags.
//!
//! Proc macros cannot see ordinary `//` comments, so the tags live in `///` doc comments.
//! Untagged prose is deliberately ignored: publishing it to OpenAPI would make the document
//! depend on comments written for rustdoc. Structured response, path and failure metadata lives
//! on the handler attribute instead.

use proc_macro2::Span;
use syn::{Attribute, Expr, ExprLit, Lit, Meta, Result};

/// Which tags a particular doc-comment site accepts.
#[derive(Clone, Copy)]
pub(crate) struct AllowedTags {
    pub(crate) summary: bool,
    pub(crate) description: bool,
}

impl AllowedTags {
    pub(crate) const HANDLER: Self = Self {
        summary: true,
        description: true,
    };

    pub(crate) const DESCRIPTION_ONLY: Self = Self {
        summary: false,
        description: true,
    };
}

/// The tagged sections found in one doc block.
#[derive(Default, Debug)]
pub(crate) struct DocTags {
    pub(crate) summary: Option<String>,
    pub(crate) description: Option<String>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Summary,
    Description,
}

pub(crate) fn parse(attrs: &[Attribute], allowed: AllowedTags) -> Result<DocTags> {
    let mut tags = DocTags::default();
    let mut current: Option<Kind> = None;
    let mut current_span = Span::call_site();
    let mut summary_lines: Vec<String> = Vec::new();
    let mut description_lines: Vec<String> = Vec::new();

    for attr in attrs {
        let Some((value, span)) = doc_value(attr) else {
            continue;
        };
        for raw in value.split('\n') {
            let line = raw.strip_prefix(' ').unwrap_or(raw);
            let line = line.trim_end_matches('\r');
            let trimmed = line.trim_start();

            // `@summary:` is a compact section: a blank line ends it, so following rustdoc
            // prose is not swallowed. Descriptions keep blank lines as paragraphs.
            if trimmed.is_empty() {
                match current {
                    Some(Kind::Description) => description_lines.push(String::new()),
                    Some(Kind::Summary) => current = None,
                    None => {}
                }
                continue;
            }

            let tag = if let Some(rest) = trimmed.strip_prefix("@summary:") {
                Some((Kind::Summary, rest))
            } else {
                trimmed
                    .strip_prefix("@description:")
                    .map(|rest| (Kind::Description, rest))
            };

            if let Some((kind, rest)) = tag {
                match kind {
                    Kind::Summary if !allowed.summary => {
                        return Err(syn::Error::new(
                            span,
                            "`@summary:` is only valid on a handler or a route entry",
                        ));
                    }
                    Kind::Description if !allowed.description => {
                        return Err(syn::Error::new(
                            span,
                            "`@description:` is only valid on a handler, route entry, \
                             response, parameter or documented type",
                        ));
                    }
                    _ => {}
                }

                let already = match kind {
                    Kind::Summary => tags.summary.is_some(),
                    Kind::Description => tags.description.is_some(),
                };
                if already {
                    return Err(syn::Error::new(
                        span,
                        "duplicate doc tag; each tag may appear once per doc block",
                    ));
                }

                current = Some(kind);
                current_span = span;
                match kind {
                    Kind::Summary => {
                        tags.summary = Some(String::new());
                        let rest = rest.trim();
                        if !rest.is_empty() {
                            summary_lines.push(rest.to_string());
                        }
                    }
                    Kind::Description => {
                        tags.description = Some(String::new());
                        let rest = rest.trim();
                        if !rest.is_empty() {
                            description_lines.push(rest.to_string());
                        }
                    }
                }
                continue;
            }

            match current {
                Some(Kind::Summary) => {
                    let text = trimmed.trim();
                    if !text.is_empty() {
                        summary_lines.push(text.to_string());
                    }
                }
                Some(Kind::Description) => {
                    description_lines.push(trimmed.to_string());
                }
                None => {}
            }
        }
    }

    if let Some(summary) = &mut tags.summary {
        *summary = summary_lines.join(" ");
        if summary.is_empty() {
            return Err(syn::Error::new(current_span, "`@summary:` needs some text"));
        }
    }
    if let Some(description) = &mut tags.description {
        while description_lines
            .last()
            .is_some_and(|line| line.trim().is_empty())
        {
            description_lines.pop();
        }
        while description_lines
            .first()
            .is_some_and(|line| line.trim().is_empty())
        {
            description_lines.remove(0);
        }
        *description = description_lines.join("\n");
        if description.is_empty() {
            return Err(syn::Error::new(
                current_span,
                "`@description:` needs some text",
            ));
        }
    }

    Ok(tags)
}

fn doc_value(attr: &Attribute) -> Option<(String, Span)> {
    let Meta::NameValue(name_value) = &attr.meta else {
        return None;
    };
    if !name_value.path.is_ident("doc") {
        return None;
    }
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = &name_value.value
    else {
        return None;
    };
    Some((value.value(), value.span()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use quote::quote;

    fn attrs(tokens: proc_macro2::TokenStream) -> Vec<Attribute> {
        let item: syn::ItemFn = syn::parse2(tokens).expect("valid fixture");
        item.attrs
    }

    #[test]
    fn parses_summary_and_description_sections() {
        let tags = parse(
            &attrs(quote! {
                /// @summary: Create a note.
                /// @description: Persists the note.
                ///
                /// Returns the stored note.
                #[handler]
                fn create() {}
            }),
            AllowedTags::HANDLER,
        )
        .expect("tags parse");

        assert_eq!(tags.summary.as_deref(), Some("Create a note."));
        assert_eq!(
            tags.description.as_deref(),
            Some("Persists the note.\n\nReturns the stored note.")
        );
    }

    #[test]
    fn summary_stops_at_a_blank_line() {
        let tags = parse(
            &attrs(quote! {
                /// @summary: Create a note.
                ///
                /// Internal rustdoc prose that must not become the summary.
                #[handler]
                fn create() {}
            }),
            AllowedTags::HANDLER,
        )
        .expect("tags parse");

        assert_eq!(tags.summary.as_deref(), Some("Create a note."));
    }

    #[test]
    fn ignores_untagged_prose() {
        let tags = parse(
            &attrs(quote! {
                /// Internal notes only.
                #[handler]
                fn create() {}
            }),
            AllowedTags::HANDLER,
        )
        .expect("tags parse");

        assert!(tags.summary.is_none());
        assert!(tags.description.is_none());
    }

    #[test]
    fn rejects_duplicate_tags() {
        let error = parse(
            &attrs(quote! {
                /// @summary: One.
                /// @summary: Two.
                #[handler]
                fn create() {}
            }),
            AllowedTags::HANDLER,
        )
        .expect_err("duplicate tags fail");

        assert!(error.to_string().contains("duplicate"));
    }

    #[test]
    fn rejects_wrong_context() {
        let error = parse(
            &attrs(quote! {
                /// @summary: Not here.
                fn filter() {}
            }),
            AllowedTags::DESCRIPTION_ONLY,
        )
        .expect_err("summary is not allowed here");

        assert!(error.to_string().contains("only valid"));
    }
}
