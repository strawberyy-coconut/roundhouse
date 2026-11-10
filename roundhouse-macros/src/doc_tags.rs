//! Explicit `@summary:`, `@description:` and `@responses:` doc tags.
//!
//! Proc macros cannot see ordinary `//` comments, so the tags live in `///` doc comments.
//! Untagged prose is deliberately ignored: publishing it to OpenAPI would make the document
//! depend on comments written for rustdoc.

use proc_macro2::Span;
use syn::{Attribute, Expr, ExprLit, Lit, Meta, Result};

/// Which tags a particular doc-comment site accepts.
#[derive(Clone, Copy)]
pub(crate) struct AllowedTags {
    pub(crate) summary: bool,
    pub(crate) description: bool,
    pub(crate) responses: bool,
}

impl AllowedTags {
    pub(crate) const HANDLER: Self = Self {
        summary: true,
        description: true,
        responses: true,
    };

    pub(crate) const DESCRIPTION_ONLY: Self = Self {
        summary: false,
        description: true,
        responses: false,
    };
}

/// One line of a tagged section, kept with its source span for diagnostics.
#[derive(Clone, Debug)]
pub(crate) struct DocLine {
    pub(crate) text: String,
    pub(crate) span: Span,
}

/// The body of a `@responses:` section, before its types are parsed.
#[derive(Default, Debug)]
pub(crate) struct ResponseSection {
    pub(crate) lines: Vec<DocLine>,
    pub(crate) override_existing: bool,
}

/// The tagged sections found in one doc block.
#[derive(Default, Debug)]
pub(crate) struct DocTags {
    pub(crate) summary: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) responses: Option<ResponseSection>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Summary,
    Description,
    Responses,
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

            let tag = if let Some(rest) = trimmed.strip_prefix("@summary:") {
                Some((Kind::Summary, rest, false))
            } else if let Some(rest) = trimmed.strip_prefix("@description:") {
                Some((Kind::Description, rest, false))
            } else if let Some(rest) = trimmed.strip_prefix("@responses(override):") {
                Some((Kind::Responses, rest, true))
            } else if let Some(rest) = trimmed.strip_prefix("@responses:") {
                Some((Kind::Responses, rest, false))
            } else {
                None
            };

            if let Some((kind, rest, override_existing)) = tag {
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
                    Kind::Responses if !allowed.responses => {
                        return Err(syn::Error::new(
                            span,
                            "`@responses:` is only valid on a handler; declare response \
                             descriptions on the response type instead",
                        ));
                    }
                    _ => {}
                }

                let already = match kind {
                    Kind::Summary => tags.summary.is_some(),
                    Kind::Description => tags.description.is_some(),
                    Kind::Responses => tags.responses.is_some(),
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
                    Kind::Responses => {
                        tags.responses = Some(ResponseSection {
                            lines: Vec::new(),
                            override_existing,
                        });
                        let rest = rest.trim();
                        if !rest.is_empty()
                            && let Some(section) = &mut tags.responses
                        {
                            section.lines.push(DocLine {
                                text: rest.to_string(),
                                span,
                            });
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
                Some(Kind::Responses) => {
                    let text = trimmed.trim();
                    if !text.is_empty()
                        && let Some(section) = &mut tags.responses
                    {
                        section.lines.push(DocLine {
                            text: text.to_string(),
                            span,
                        });
                    }
                }
                None => {}
            }
        }
    }

    if let Some(summary) = &mut tags.summary {
        *summary = summary_lines.join(" ");
        if summary.is_empty() {
            return Err(syn::Error::new(
                current_span,
                "`@summary:` needs some text",
            ));
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

    #[test]
    fn parses_response_lines_and_override() {
        let tags = parse(
            &attrs(quote! {
                /// @responses(override):
                ///   201: Json<Vec<User>>
                ///   204
                #[handler]
                fn create() {}
            }),
            AllowedTags::HANDLER,
        )
        .expect("tags parse");

        let section = tags.responses.expect("responses section");
        assert!(section.override_existing);
        assert_eq!(section.lines.len(), 2);
        assert_eq!(section.lines[0].text, "201: Json<Vec<User>>");
        assert_eq!(section.lines[1].text, "204");
    }
}
