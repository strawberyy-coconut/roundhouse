use proc_macro2::TokenStream;
use quote::quote;
use syn::{ReturnType, Type};

use crate::util::{outer_ident, single_generic};

/// The outer shape of a handler's return type.
///
/// It drives the document statements; the contract build owns the success-response check.
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
    /// The macro cannot classify it; probe the built-in response shapes.
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
            // A bare `impl IntoResponse` has an unknown concrete response. Leave it unclassified
            // so the contract compiler decides whether to infer or reject the response.
            // infers the response (or rejects it when inference is not available).
            _ => ReturnShape::Unknown { ty },
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

/// Document the handler's return type through its built-in response shape
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
        ReturnShape::Unknown { ty } => {
            if contains_impl_trait(ty) {
                Vec::new()
            } else {
                vec![probe_only(&krate, ty)]
            }
        }
    })
}

fn ok_statements(krate: &TokenStream, ok: &Type) -> TokenStream {
    match classify_return_type(ok) {
        ReturnShape::DynamicStatus { ty } => dynamic_statements(krate, ty),
        ReturnShape::Known { kind, ty } => known_statements(krate, kind, ty),
        _ if contains_impl_trait(ok) => quote!(),
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

    fn kind(source: &str) -> KnownKind {
        let ty: Type = syn::parse_str(source).expect("valid type");
        match classify_return_type(&ty) {
            ReturnShape::Known { kind, .. } => kind,
            _ => panic!("`{source}` is not a known shape"),
        }
    }

    #[test]
    fn known_shapes_are_classified() {
        for (source, expected) in [
            ("Json<Note>", KnownKind::Json),
            ("Html<Note>", KnownKind::Html),
            ("String", KnownKind::Text),
            ("&'static str", KnownKind::Text),
            ("Vec<u8>", KnownKind::Vec),
            ("Bytes", KnownKind::Bytes),
            ("Body", KnownKind::Bytes),
            ("Attachment<Vec<u8>>", KnownKind::Attachment),
            ("File", KnownKind::File),
            ("Sse<EventStream>", KnownKind::Sse),
            ("Created<Note>", KnownKind::Created),
            ("NoContent", KnownKind::NoContent),
            ("StatusCode", KnownKind::StatusCode),
        ] {
            assert_eq!(kind(source), expected, "{source}");
        }
    }

    #[test]
    fn dynamic_status_is_default() {
        assert!(matches!(
            classify_return_type(&syn::parse_str::<Type>("(StatusCode, Json<Note>)").unwrap()),
            ReturnShape::DynamicStatus { .. }
        ));
    }

    #[test]
    fn unsupported_tuples_are_unknown() {
        for source in ["(String, String)", "(String, StatusCode)", "(Json<Note>,)"] {
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
    fn hidden_sse_keeps_its_shape() {
        assert_eq!(kind("Sse<impl Stream>"), KnownKind::Sse);
    }

    #[test]
    fn unknown_types_are_not_claimed() {
        for source in ["Problem", "CustomResponse"] {
            assert!(matches!(
                classify_return_type(&syn::parse_str::<Type>(source).unwrap()),
                ReturnShape::Unknown { .. }
            ));
        }
    }
}
