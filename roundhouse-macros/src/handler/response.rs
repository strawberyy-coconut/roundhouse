use proc_macro2::TokenStream;
use quote::quote;
use syn::punctuated::Punctuated;
use syn::{ReturnType, Token, Type};

use crate::util::{outer_ident, single_generic};

use super::signature::{generic_types, is_wrapper_type};

/// The success statuses the return type guarantees, or `None` when the macro cannot classify it
/// (a custom `ApiResponse` type registers its responses at build time). `Some(&[])` means the
/// macro knows there is no success response, which is a compile error unless the entry declares
/// one.
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

pub(super) fn response_statements(output: &ReturnType) -> syn::Result<Vec<TokenStream>> {
    let krate = crate::util::krate();
    let ReturnType::Type(_, ty) = output else {
        return Ok(vec![quote! {
            docs.responses.push(#krate::doc::ResponseDoc::new(204, "No Content"));
        }]);
    };

    let types = generic_types(ty);
    if outer_ident(ty).as_deref() == Some("Result") && types.len() == 2 {
        let ok = &types[0];
        let err = &types[1];
        let mut statements = ok_statements(ok, 200)?;
        statements.push(quote! {
            let __before = docs.responses.len();
            docs.responses.extend({
                use #krate::doc::ProbeResponses as _;
                (&#krate::doc::ResponseProbe::<#err>::new()).probe_responses(components)
            });
            if docs.responses.len() == __before {
                docs.undocumented_error = ::core::option::Option::Some(
                    ::core::any::type_name::<#err>(),
                );
            }
        });
        return Ok(statements);
    }

    ok_statements(ty, 200)
}

pub(super) fn ok_statements(ty: &Type, status: u16) -> syn::Result<Vec<TokenStream>> {
    let krate = crate::util::krate();
    if let Type::Tuple(tuple) = ty {
        if tuple.elems.is_empty() {
            // Axum responds `200 OK` for `()`, so say that rather than promising a status the
            // runtime does not produce.
            return Ok(vec![quote! {
                docs.responses.push(#krate::doc::ResponseDoc::new(200, "OK"));
            }]);
        }
        if tuple.elems.len() >= 2 {
            // `(StatusCode, Json<T>)` chooses its status at runtime. Documenting `200` would be a
            // lie for the usual `CREATED`/`NO_CONTENT` spellings, so document `default` instead.
            let dynamic = tuple
                .elems
                .iter()
                .take(tuple.elems.len().saturating_sub(1))
                .any(|element| outer_ident(element).as_deref() == Some("StatusCode"));
            let body = tuple.elems.last().unwrap();
            if dynamic {
                let mut statements = ok_statements(body, 0)?;
                if statements.is_empty() {
                    statements.push(quote! {
                        docs.responses.push(#krate::doc::ResponseDoc::default_response());
                    });
                }
                return Ok(statements);
            }
            return ok_statements(body, status);
        }
    }

    let status_lit = proc_macro2::Literal::u16_unsuffixed(status);
    let kind = outer_ident(ty);
    match kind.as_deref() {
        Some("Json") => {
            let inner = single_generic(ty).ok_or_else(|| {
                syn::Error::new_spanned(ty, "expected `Json<T>` as the response body")
            })?;
            Ok(vec![quote! {
                {
                    use #krate::doc::ProbeJsonResponse as _;
                    let mut __responses = (&#krate::doc::JsonResponseProbe::<#inner>::new())
                        .probe_json_response(components, #status_lit);
                    if __responses.is_empty() {
                        use #krate::doc::ProbeToSchemaJsonResponse as _;
                        __responses = (&#krate::doc::ToSchemaJsonResponseProbe::<#inner>::new())
                            .probe_to_schema_json_response(components, #status_lit);
                    }
                    if __responses.is_empty() {
                        docs.unverified_response = true;
                        docs.responses.push(#krate::doc::ResponseDoc::new(#status_lit, "OK"));
                    } else {
                        docs.responses.extend(__responses);
                    }
                }
            }])
        }
        Some("Attachment") if is_wrapper_type(ty) => {
            let inner = single_generic(ty)
                .ok_or_else(|| syn::Error::new_spanned(ty, "expected Attachment<T>"))?;
            Ok(vec![quote! {
                docs.responses.push(#krate::attachment::attachment_response::<#inner>(#status_lit));
            }])
        }
        Some("File") if is_wrapper_type(ty) => Ok(vec![quote! {
            docs.responses.push(#krate::attachment::file_response(#status_lit));
        }]),
        Some("Sse") if is_wrapper_type(ty) => Ok(vec![quote! {
            docs.responses.push(#krate::attachment::sse_response(#status_lit));
        }]),
        Some("Html") if !is_wrapper_type(ty) => Ok(vec![quote! {
            docs.unverified_response = true;
            docs.responses.push(#krate::doc::ResponseDoc::new(#status_lit, "OK"));
        }]),
        Some("Html") => {
            let inner = single_generic(ty).ok_or_else(|| {
                syn::Error::new_spanned(ty, "expected `Html<T>` as the response body")
            })?;
            Ok(vec![quote! {
                docs.responses.push({
                    use #krate::doc::ProbeTextResponse as _;
                    (&#krate::doc::TextResponseProbe::<#inner>::new())
                        .probe_text_response(components, #status_lit, "text/html")
                });
            }])
        }
        Some("StatusCode") => Ok(vec![quote! {
            docs.responses.push(#krate::doc::ResponseDoc::default_response());
        }]),
        Some("String") => Ok(vec![quote! {
            docs.responses.push(#krate::doc::plain_text_response(#status_lit));
        }]),
        Some("Vec") => {
            if single_generic(ty)
                .is_some_and(|inner| matches!(outer_ident(&inner).as_deref(), Some("u8")))
            {
                Ok(vec![quote! {
                    docs.responses.push(#krate::doc::bytes_response(#status_lit));
                }])
            } else {
                let inner = single_generic(ty).unwrap();
                Ok(vec![quote! {
                    {
                        use #krate::doc::ProbeJsonResponse as _;
                        let mut __responses = (&#krate::doc::JsonResponseProbe::<#inner>::new())
                            .probe_json_response(components, #status_lit);
                        if __responses.is_empty() {
                            use #krate::doc::ProbeToSchemaJsonResponse as _;
                            __responses = (&#krate::doc::ToSchemaJsonResponseProbe::<#inner>::new())
                                .probe_to_schema_json_response(components, #status_lit);
                        }
                        if __responses.is_empty() {
                            docs.unverified_response = true;
                            docs.responses.push(#krate::doc::ResponseDoc::new(#status_lit, "OK"));
                        } else {
                            docs.responses.extend(__responses);
                        }
                    }
                }])
            }
        }
        Some("Bytes") | Some("Body") => Ok(vec![quote! {
            docs.responses.push(#krate::doc::bytes_response(#status_lit));
        }]),
        Some("Option") => Ok(Vec::new()),
        Some("Infallible") => Ok(Vec::new()),
        _ => {
            if let Type::Reference(reference) = ty
                && matches!(outer_ident(&reference.elem).as_deref(), Some("str"))
            {
                return Ok(vec![quote! {
                    docs.responses.push(#krate::doc::plain_text_response(#status_lit));
                }]);
            }
            if let Type::ImplTrait(_) = ty {
                return Ok(Vec::new());
            }
            Ok(vec![quote! {
                docs.responses.extend({
                    use #krate::doc::ProbeResponses as _;
                    (&#krate::doc::ResponseProbe::<#ty>::new()).probe_responses(components)
                });
            }])
        }
    }
}

#[allow(dead_code)]
fn _punctuated_marker(_: Punctuated<Token![,], Token![,]>) {}
