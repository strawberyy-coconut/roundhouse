use proc_macro2::TokenStream;
use quote::quote;
use syn::{ReturnType, Type};

use crate::util::{outer_ident, single_generic};

use super::signature::generic_types;

/// The success statuses the return type guarantees, or `None` when the macro cannot classify it
/// (a custom `ApiResponse` type registers its responses at build time). `Some(&[])` means the
/// macro knows there is no success response, so a route entry must declare one.
pub(super) fn inferred_successes(output: &ReturnType) -> Option<Vec<u16>> {
    let ReturnType::Type(_, ty) = output else {
        return Some(vec![204]);
    };
    let types = generic_types(ty);
    if outer_ident(ty).as_deref() == Some("Result") && types.len() == 2 {
        return ok_successes(&types[0]);
    }
    ok_successes(ty)
}

pub(super) fn ok_successes(ty: &Type) -> Option<Vec<u16>> {
    if let Type::Tuple(tuple) = ty {
        if tuple.elems.is_empty() {
            return Some(vec![200]);
        }
        if tuple.elems.len() >= 2 {
            let dynamic = tuple
                .elems
                .iter()
                .take(tuple.elems.len() - 1)
                .any(|element| outer_ident(element).as_deref() == Some("StatusCode"));
            let body = tuple.elems.last().expect("non-empty tuple");
            if dynamic {
                return Some(vec![0]);
            }
            return ok_successes(body);
        }
    }

    match outer_ident(ty).as_deref() {
        Some(
            "Json" | "Attachment" | "File" | "Sse" | "Html" | "String" | "Vec" | "Bytes" | "Body",
        ) => Some(vec![200]),
        Some("Created") => Some(vec![201]),
        Some("NoContent") => Some(vec![204]),
        Some("StatusCode") => Some(vec![0]),
        Some("Option" | "Infallible") => Some(Vec::new()),
        _ => {
            if let Type::Reference(reference) = ty
                && matches!(outer_ident(&reference.elem).as_deref(), Some("str"))
            {
                return Some(vec![200]);
            }
            if let Type::ImplTrait(_) = ty {
                return Some(Vec::new());
            }
            None
        }
    }
}

/// Document the handler's return type through its [`ApiResponse`](roundhouse::doc::ApiResponse)
/// implementation.
///
/// A type without one contributes no inferred response. `Json<T>` and `Vec<T>` additionally get
/// the historical "a response exists but its schema is invisible" marker; every other unknown
/// return type is expected to declare its responses in `routes!`.
pub(super) fn response_statements(output: &ReturnType) -> syn::Result<Vec<TokenStream>> {
    let krate = crate::util::krate();
    let ReturnType::Type(_, ty) = output else {
        return Ok(vec![quote! {
            docs.responses.push(#krate::doc::ResponseDoc::new(204, "No Content"));
        }]);
    };

    // An `impl Trait` return type cannot be named in a turbofish. `Sse<impl Stream>` still has a
    // fixed documented shape, so keep that one; every other hidden type declares its responses.
    if contains_impl_trait(ty) {
        return Ok(match outer_ident(ty).as_deref() {
            Some("Sse") => vec![quote! {
                docs.responses.push(#krate::attachment::sse_response(200));
            }],
            Some("File") => vec![quote! {
                docs.responses.push(#krate::attachment::file_response(200));
            }],
            _ => Vec::new(),
        });
    }

    // `Result<T, E>` is the one shape the macro still splits: the success type and the error type
    // are documented independently, so an undocumented `E` is a diagnostic rather than a missing
    // success response.
    let generic = generic_types(ty);
    if outer_ident(ty).as_deref() == Some("Result") && generic.len() == 2 {
        let ok = &generic[0];
        let err = &generic[1];
        let ok_statement = if is_vec_u8(ok) {
            quote! {
                docs.responses.push(#krate::doc::bytes_response(200));
            }
        } else {
            quote! {
                match {
                    use #krate::__private::ProbeResponses as _;
                    (&#krate::__private::ResponseProbe::<#ok>::new()).probe_responses(context)
                } {
                    ::core::option::Option::Some(__rh_responses) => {
                        docs.responses.extend(__rh_responses);
                    }
                    ::core::option::Option::None => {}
                }
            }
        };
        return Ok(vec![quote! {
            #ok_statement
            match {
                use #krate::__private::ProbeResponses as _;
                (&#krate::__private::ResponseProbe::<#err>::new()).probe_responses(context)
            } {
                ::core::option::Option::Some(__rh_responses) => {
                    docs.responses.extend(__rh_responses);
                }
                ::core::option::Option::None => {
                    docs.undocumented_error = ::core::option::Option::Some(
                        ::core::any::type_name::<#err>(),
                    );
                }
            }
        }]);
    }

    // `Vec<u8>` is the one response whose runtime representation is bytes rather than JSON. The
    // generic `ApiResponse for Vec<T>` impl would otherwise document it as an integer array.
    if is_vec_u8(ty) {
        return Ok(vec![quote! {
            docs.responses.push(#krate::doc::bytes_response(200));
        }]);
    }

    let unknown = match outer_ident(ty).as_deref() {
        Some("Json" | "Vec") => quote! {
            docs.unverified_response = true;
            docs.responses.push(#krate::doc::ResponseDoc::new(200, "OK"));
        },
        _ => quote!(),
    };

    Ok(vec![quote! {
        match {
            use #krate::__private::ProbeResponses as _;
            (&#krate::__private::ResponseProbe::<#ty>::new()).probe_responses(context)
        } {
            ::core::option::Option::Some(__rh_responses) => {
                docs.responses.extend(__rh_responses);
            }
            ::core::option::Option::None => {
                #unknown
            }
        }
    }])
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
