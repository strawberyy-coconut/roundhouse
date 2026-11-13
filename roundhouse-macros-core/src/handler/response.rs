use proc_macro2::TokenStream;
use quote::quote;
use syn::{ReturnType, Type};

use crate::util::{outer_ident, single_generic};

/// The outer shape of a handler's return type.
///
/// This is the single classification used by both the compile-time success check
/// (`HandlerSpec::INFERRED_SUCCESS`) and the document statements. Keeping one value means the
/// macro cannot say a route has a `200` while the builder documents something else.
pub(super) enum ReturnShape<'a> {
    /// No `->`, or an explicit `()`.
    Unit,
    /// `Result<T, E>`.
    Result { ok: &'a Type, err: &'a Type },
    /// `(StatusCode, T)`: the status is chosen at runtime and documented as `default`.
    DynamicStatus { ty: &'a Type },
    /// A concrete outer type with a known status table.
    Known { kind: KnownKind, ty: &'a Type },
    /// `Option<T>`, `Infallible`, or a bare `impl Trait`: known to have no success response.
    NoSuccess { ty: &'a Type },
    /// The macro cannot classify it; probe `ApiResponse`.
    Unknown { ty: &'a Type },
}

/// A concrete return type with a statically known success status.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KnownKind {
    Json,
    Vec,
    Text,
    Bytes,
    Html,
    Attachment,
    File,
    Sse,
    Created,
    NoContent,
    StatusCode,
}

impl KnownKind {
    fn statuses(self) -> &'static [u16] {
        match self {
            KnownKind::Json
            | KnownKind::Vec
            | KnownKind::Text
            | KnownKind::Bytes
            | KnownKind::Html
            | KnownKind::Attachment
            | KnownKind::File
            | KnownKind::Sse => &[200],
            KnownKind::Created => &[201],
            KnownKind::NoContent => &[204],
            KnownKind::StatusCode => &[0],
        }
    }
}

/// Classify a handler's return type once.
pub(super) fn classify_return(output: &ReturnType) -> ReturnShape<'_> {
    match output {
        ReturnType::Default => ReturnShape::Unit,
        ReturnType::Type(_, ty) => classify_return_type(ty),
    }
}

/// Classify a type nested in a return position (a `Result` arm or a tuple body).
pub(super) fn classify_return_type(ty: &Type) -> ReturnShape<'_> {
    // `impl Trait` cannot be named in a turbofish, so it can never be probed. `Sse` and `File`
    // have a fixed documented shape; every other hidden type must be declared with
    // `#[handler(response(..))]`.
    if contains_impl_trait(ty) {
        return match outer_ident(ty).as_deref() {
            Some("Sse") => ReturnShape::Known {
                kind: KnownKind::Sse,
                ty,
            },
            Some("File") => ReturnShape::Known {
                kind: KnownKind::File,
                ty,
            },
            _ => ReturnShape::NoSuccess { ty },
        };
    }

    if let Some((ok, err)) = result_types(ty) {
        return ReturnShape::Result { ok, err };
    }

    if let Type::Tuple(tuple) = ty {
        if tuple.elems.is_empty() {
            return ReturnShape::Unit;
        }
        if tuple.elems.len() == 2 && outer_ident(&tuple.elems[0]).as_deref() == Some("StatusCode") {
            return ReturnShape::DynamicStatus { ty };
        }
        // Axum implements more tuple shapes than roundhouse can document (`HeaderMap` response
        // parts, for example). Claiming the last element's status would be a guess, so leave the
        // shape unknown and let the build require a declaration.
        return ReturnShape::Unknown { ty };
    }

    if let Type::Reference(reference) = ty
        && matches!(outer_ident(&reference.elem).as_deref(), Some("str"))
    {
        return ReturnShape::Known {
            kind: KnownKind::Text,
            ty,
        };
    }

    match outer_ident(ty).as_deref() {
        Some("Json") => ReturnShape::Known {
            kind: KnownKind::Json,
            ty,
        },
        Some("Vec") => ReturnShape::Known {
            kind: KnownKind::Vec,
            ty,
        },
        Some("String") => ReturnShape::Known {
            kind: KnownKind::Text,
            ty,
        },
        Some("Html") => ReturnShape::Known {
            kind: KnownKind::Html,
            ty,
        },
        Some("Attachment") => ReturnShape::Known {
            kind: KnownKind::Attachment,
            ty,
        },
        Some("File") => ReturnShape::Known {
            kind: KnownKind::File,
            ty,
        },
        Some("Sse") => ReturnShape::Known {
            kind: KnownKind::Sse,
            ty,
        },
        Some("Bytes" | "Body") => ReturnShape::Known {
            kind: KnownKind::Bytes,
            ty,
        },
        Some("Created") => ReturnShape::Known {
            kind: KnownKind::Created,
            ty,
        },
        Some("NoContent") => ReturnShape::Known {
            kind: KnownKind::NoContent,
            ty,
        },
        Some("StatusCode") => ReturnShape::Known {
            kind: KnownKind::StatusCode,
            ty,
        },
        Some("Option" | "Infallible") => ReturnShape::NoSuccess { ty },
        _ => ReturnShape::Unknown { ty },
    }
}

/// The success statuses the return shape guarantees, or `None` when the macro cannot classify it.
///
/// `Some(&[])` means the macro knows there is no success response, so a route entry must declare
/// one. `Some(&[0])` means the status is chosen at runtime and documented as `default`.
pub(super) fn inferred_successes(shape: &ReturnShape<'_>) -> Option<Vec<u16>> {
    match shape {
        ReturnShape::Unit => Some(vec![200]),
        ReturnShape::Result { ok, .. } => inferred_successes(&classify_return_type(ok)),
        ReturnShape::DynamicStatus { .. } => Some(vec![0]),
        ReturnShape::Known { kind, .. } => Some(kind.statuses().to_vec()),
        ReturnShape::NoSuccess { .. } => Some(Vec::new()),
        ReturnShape::Unknown { .. } => None,
    }
}

/// Document the handler's return type through its [`ApiResponse`](roundhouse::doc::ApiResponse)
/// implementation.
///
/// A type without one contributes no inferred response. A known shape whose probe returns
/// nothing is published with the bare status and `x-roundhouse-unverified-response`, so the
/// document still carries the status the handler can produce.
pub(super) fn response_statements(shape: &ReturnShape<'_>) -> syn::Result<Vec<TokenStream>> {
    let krate = crate::util::krate();
    Ok(match shape {
        ReturnShape::Unit => vec![quote! {
            docs.push_response(#krate::doc::ResponseDoc::new(200, "OK"));
        }],
        ReturnShape::Result { ok, err } => {
            let ok_statement = ok_statements(&krate, ok);
            let err_statement = quote! {
                match {
                    use #krate::__private::ProbeResponses as _;
                    (&#krate::__private::ResponseProbe::<#err>::new()).probe_responses(context)
                } {
                    ::core::option::Option::Some(__rh_responses) => {
                        docs.extend_responses(__rh_responses);
                    }
                    ::core::option::Option::None => {
                        docs.verification.set_undocumented_error(::core::any::type_name::<#err>());
                    }
                }
            };
            vec![quote! {
                #ok_statement
                #err_statement
            }]
        }
        ReturnShape::DynamicStatus { ty } => vec![dynamic_statements(&krate, ty)],
        ReturnShape::Known { kind, ty } => vec![known_statements(&krate, *kind, ty)],
        ReturnShape::NoSuccess { ty } => {
            // A NoSuccess shape that contains `impl Trait` cannot be named in the probe.
            if contains_impl_trait(ty) {
                Vec::new()
            } else {
                vec![probe_only(&krate, ty)]
            }
        }
        ReturnShape::Unknown { ty } => vec![probe_only(&krate, ty)],
    })
}

fn ok_statements(krate: &TokenStream, ok: &Type) -> TokenStream {
    match classify_return_type(ok) {
        ReturnShape::DynamicStatus { ty } => dynamic_statements(krate, ty),
        ReturnShape::Known { kind, ty } => known_statements(krate, kind, ty),
        _ => probe_only(krate, ok),
    }
}

fn dynamic_statements(krate: &TokenStream, ty: &Type) -> TokenStream {
    let fallback = quote! {
        docs.verification.mark_response();
        docs.push_response(#krate::doc::ResponseDoc::default_response());
    };
    probe_with_fallback(krate, ty, fallback)
}

fn known_statements(krate: &TokenStream, kind: KnownKind, ty: &Type) -> TokenStream {
    match kind {
        KnownKind::Vec if is_vec_u8(ty) => quote! {
            docs.push_response(#krate::doc::bytes_response(200));
        },
        KnownKind::File => quote! {
            docs.push_response(#krate::attachment::file_response(200));
        },
        KnownKind::Sse => quote! {
            docs.push_response(#krate::attachment::sse_response(200));
        },
        _ => {
            let fallback = fallback_statements(krate, kind.statuses());
            probe_with_fallback(krate, ty, fallback)
        }
    }
}

fn fallback_statements(krate: &TokenStream, statuses: &'static [u16]) -> TokenStream {
    let pushes = statuses.iter().map(|status| {
        if *status == 0 {
            quote! {
                docs.push_response(#krate::doc::ResponseDoc::default_response());
            }
        } else {
            let status = proc_macro2::Literal::u16_suffixed(*status);
            quote! {
                docs.push_response(
                    #krate::doc::ResponseDoc::new(#status, #krate::doc::canonical_reason(#status))
                );
            }
        }
    });
    quote! {
        docs.verification.mark_response();
        #(#pushes)*
    }
}

fn probe_only(krate: &TokenStream, ty: &Type) -> TokenStream {
    quote! {
        match {
            use #krate::__private::ProbeResponses as _;
            (&#krate::__private::ResponseProbe::<#ty>::new()).probe_responses(context)
        } {
            ::core::option::Option::Some(__rh_responses) => {
                docs.extend_responses(__rh_responses);
            }
            ::core::option::Option::None => {}
        }
    }
}

fn probe_with_fallback(krate: &TokenStream, ty: &Type, fallback: TokenStream) -> TokenStream {
    quote! {
        match {
            use #krate::__private::ProbeResponses as _;
            (&#krate::__private::ResponseProbe::<#ty>::new()).probe_responses(context)
        } {
            ::core::option::Option::Some(__rh_responses) => {
                docs.extend_responses(__rh_responses);
            }
            ::core::option::Option::None => {
                #fallback
            }
        }
    }
}

fn result_types(ty: &Type) -> Option<(&Type, &Type)> {
    if outer_ident(ty).as_deref() != Some("Result") {
        return None;
    }
    let types = generic_type_refs(ty);
    (types.len() == 2).then(|| (types[0], types[1]))
}

fn generic_type_refs(ty: &Type) -> Vec<&Type> {
    let Type::Path(type_path) = ty else {
        return Vec::new();
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Vec::new();
    };
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Vec::new();
    };
    arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty),
            _ => None,
        })
        .collect()
}

fn is_vec_u8(ty: &Type) -> bool {
    let Some(inner) = single_generic(ty) else {
        return false;
    };
    outer_ident(ty).as_deref() == Some("Vec") && outer_ident(&inner).as_deref() == Some("u8")
}

fn contains_impl_trait(ty: &Type) -> bool {
    match ty {
        Type::ImplTrait(_) => true,
        Type::Array(array) => contains_impl_trait(&array.elem),
        Type::Group(group) => contains_impl_trait(&group.elem),
        Type::Paren(paren) => contains_impl_trait(&paren.elem),
        Type::Ptr(ptr) => contains_impl_trait(&ptr.elem),
        Type::Reference(reference) => contains_impl_trait(&reference.elem),
        Type::Slice(slice) => contains_impl_trait(&slice.elem),
        Type::Tuple(tuple) => tuple.elems.iter().any(contains_impl_trait),
        Type::Path(path) => path.path.segments.iter().any(|segment| {
            if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                arguments.args.iter().any(|argument| match argument {
                    syn::GenericArgument::Type(ty) => contains_impl_trait(ty),
                    _ => false,
                })
            } else {
                false
            }
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn statuses(source: &str) -> Option<Vec<u16>> {
        let ty: Type = syn::parse_str(source).expect("valid type");
        inferred_successes(&classify_return_type(&ty))
    }

    fn kind(source: &str) -> KnownKind {
        let ty: Type = syn::parse_str(source).expect("valid type");
        match classify_return_type(&ty) {
            ReturnShape::Known { kind, .. } => kind,
            _ => panic!("`{source}` is not a known shape"),
        }
    }

    #[test]
    fn no_return_type_is_ok() {
        assert_eq!(
            inferred_successes(&classify_return(&ReturnType::Default)),
            Some(vec![200])
        );
    }

    #[test]
    fn explicit_unit_is_ok() {
        assert_eq!(statuses("()"), Some(vec![200]));
    }

    #[test]
    fn known_shapes_have_their_status() {
        for (source, expected) in [
            ("Json<Note>", (KnownKind::Json, 200)),
            ("Html<Note>", (KnownKind::Html, 200)),
            ("String", (KnownKind::Text, 200)),
            ("&'static str", (KnownKind::Text, 200)),
            ("Vec<u8>", (KnownKind::Vec, 200)),
            ("Bytes", (KnownKind::Bytes, 200)),
            ("Body", (KnownKind::Bytes, 200)),
            ("Attachment<Vec<u8>>", (KnownKind::Attachment, 200)),
            ("File", (KnownKind::File, 200)),
            ("Sse<EventStream>", (KnownKind::Sse, 200)),
            ("Created<Note>", (KnownKind::Created, 201)),
            ("NoContent", (KnownKind::NoContent, 204)),
            ("StatusCode", (KnownKind::StatusCode, 0)),
        ] {
            assert_eq!(kind(source), expected.0, "{source}");
            assert_eq!(statuses(source), Some(vec![expected.1]), "{source}");
        }
    }

    #[test]
    fn result_uses_the_ok_arm() {
        assert_eq!(statuses("Result<Json<Note>, E>"), Some(vec![200]));
        assert_eq!(statuses("Result<Created<Note>, E>"), Some(vec![201]));
        assert_eq!(statuses("Result<Option<Json<Note>>, E>"), Some(vec![]));
    }

    #[test]
    fn dynamic_status_is_default() {
        assert_eq!(statuses("(StatusCode, Json<Note>)"), Some(vec![0]));
        assert!(matches!(
            classify_return_type(&syn::parse_str::<Type>("(StatusCode, Json<Note>)").unwrap()),
            ReturnShape::DynamicStatus { .. }
        ));
    }

    #[test]
    fn unsupported_tuples_are_unknown() {
        for source in ["(String, String)", "(String, StatusCode)", "(Json<Note>,)"] {
            assert_eq!(statuses(source), None, "{source}");
            assert!(
                matches!(
                    classify_return_type(&syn::parse_str::<Type>(source).unwrap()),
                    ReturnShape::Unknown { .. }
                ),
                "{source}"
            );
        }
    }

    #[test]
    fn no_success_shapes() {
        for source in ["Option<Json<Note>>", "Infallible", "impl IntoResponse"] {
            assert_eq!(statuses(source), Some(vec![]), "{source}");
        }
    }

    #[test]
    fn hidden_sse_keeps_its_status() {
        assert_eq!(statuses("Sse<impl Stream>"), Some(vec![200]));
        assert_eq!(kind("Sse<impl Stream>"), KnownKind::Sse);
    }

    #[test]
    fn unknown_types_are_not_claimed() {
        for source in ["Problem", "CustomResponse"] {
            assert_eq!(statuses(source), None, "{source}");
        }
    }
}
