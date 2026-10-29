use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{FnArg, ItemFn, ReturnType, Token, Type};

use crate::util::{outer_ident, single_generic};

pub fn expand(item: ItemFn) -> TokenStream {
    let krate = crate::util::krate();
    if !item.sig.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &item.sig.generics,
            "roundhouse handlers cannot be generic; use a concrete signature",
        )
        .to_compile_error();
    }
    let name = item.sig.ident.clone();
    let marker_type = format_ident!("{}_ROUNDHOUSE_MISSING_HANDLER_TYPE", name);
    let marker_value = format_ident!("{}_ROUNDHOUSE_MISSING_HANDLER", name);
    let vis = item.vis.clone();

    let mut state_type: Option<Type> = None;
    let mut path_type: Option<Type> = None;
    let mut statements: Vec<TokenStream> = Vec::new();
    let mut body_count = 0usize;

    for argument in &item.sig.inputs {
        let FnArg::Typed(argument) = argument else {
            continue;
        };
        let (ty, handled) = match result_ok_type(&argument.ty) {
            Some(ok) => (ok, true),
            None => ((*argument.ty).clone(), false),
        };
        if let Some((auth, scopes, optional)) = auth_types(&ty) {
            if state_type.is_none() {
                state_type = Some(syn::parse_quote!(
                    <#auth as #krate::auth::AuthScheme>::State
                ));
            }
            let scopes = match scopes {
                Some(scopes) => quote!(#scopes),
                None => quote!(#krate::auth::NoScopes),
            };
            statements.push(quote! {
                docs.schemes.push(#krate::auth::auth_scheme::<#auth>());
            });
            if optional {
                statements.push(quote! {
                    docs.optional_auth.push(
                        #krate::auth::auth_doc::<#auth, #scopes>()
                    );
                });
            } else {
                statements.push(quote! {
                    docs.required_auth.push(
                        #krate::auth::auth_doc::<#auth, #scopes>()
                    );
                    docs.failures.extend(
                        #krate::doc::auth_failures::<#auth>()
                    );
                });
            }
            continue;
        }

        let Some(kind) = outer_ident(&ty) else {
            continue;
        };

        match kind.as_str() {
            "Path" if is_wrapper_type(&ty) => {
                if let Some(inner) = single_generic(&ty) {
                    path_type = Some(inner.clone());
                    statements.push(quote! {
                        docs.params.extend(#krate::doc::path_params::<#inner>());
                        docs.path_schema = ::core::option::Option::Some(
                            <#inner as #krate::doc::PathNames>::scalar_schema,
                        );
                    });
                }
                if !handled {
                    statements.push(quote! {
                        docs.failures.extend(#krate::doc::path_failures());
                    });
                }
            }
            "Query" if is_wrapper_type(&ty) => {
                if let Some(inner) = single_generic(&ty) {
                    statements.push(quote! {
                        docs.params.extend(
                            #krate::doc::stored_query_params_for::<#inner>(components)
                        );
                    });
                }
                if !handled {
                    statements.push(quote! {
                        docs.failures.extend(#krate::doc::query_failures());
                    });
                }
            }
            "Json" => {
                body_count += 1;
                if is_wrapper_type(&ty) {
                    if let Some(inner) = single_generic(&ty) {
                        statements.push(quote! {
                            docs.body = ::core::option::Option::Some(
                                #krate::doc::json_body::<#inner>(components)
                            );
                            docs.body_type = ::core::option::Option::Some(
                                || ::core::any::TypeId::of::<#inner>()
                            );
                        });
                    }
                    if !handled {
                        statements.push(quote! {
                            docs.failures.extend(#krate::doc::json_failures());
                        });
                    }
                } else {
                    statements.push(quote! { docs.unverified = true; });
                }
            }
            "Multipart" => {
                body_count += 1;
                if is_wrapper_type(&ty) {
                    if let Some(inner) = single_generic(&ty) {
                        statements.push(quote! {
                            docs.body = ::core::option::Option::Some(
                                #krate::doc::multipart_body::<#inner>(components)
                            );
                            docs.body_type = ::core::option::Option::Some(
                                || ::core::any::TypeId::of::<#inner>()
                            );
                        });
                    }
                    if !handled {
                        statements.push(quote! {
                            docs.failures.extend(#krate::doc::multipart_failures());
                        });
                    }
                } else {
                    statements.push(quote! { docs.unverified = true; });
                }
            }
            "Form" => {
                body_count += 1;
                if is_wrapper_type(&ty) {
                    if let Some(inner) = single_generic(&ty) {
                        statements.push(quote! {
                            docs.body = ::core::option::Option::Some(
                                #krate::doc::form_body::<#inner>(components)
                            );
                            docs.body_type = ::core::option::Option::Some(
                                || ::core::any::TypeId::of::<#inner>()
                            );
                        });
                    }
                    if !handled {
                        statements.push(quote! {
                            docs.failures.extend(#krate::doc::form_failures());
                        });
                    }
                } else {
                    statements.push(quote! { docs.unverified = true; });
                }
            }
            "State" => {
                if let Some(inner) = single_generic(&ty) {
                    state_type = Some(inner);
                }
            }
            "Bytes" | "Body" => {
                body_count += 1;
                statements.push(quote! {
                    docs.body = ::core::option::Option::Some(#krate::doc::binary_body());
                });
            }
            "Extension" if !handled && is_wrapper_type(&ty) => {
                statements.push(quote! {
                    docs.failures.extend(#krate::doc::extension_failures());
                });
            }
            _ => {}
        }
    }

    if body_count > 1 {
        return syn::Error::new(
            item.sig.ident.span(),
            "a handler may read the request body at most once (Json/Form/Multipart)",
        )
        .to_compile_error();
    }

    let response_statements = match response_statements(&item.sig.output) {
        Ok(statements) => statements,
        Err(error) => return error.to_compile_error(),
    };
    statements.extend(response_statements);

    let path_names = match &path_type {
        Some(ty) => quote!(<#ty as #krate::doc::PathNames>::NAMES),
        None => quote!(&[]),
    };
    let state_ty = match &state_type {
        Some(ty) => quote!(#ty),
        None => quote!(()),
    };
    let has_state = state_type.is_some();

    quote! {
        #item

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #vis struct #marker_type;

        #[doc(hidden)]
        #[allow(non_upper_case_globals)]
        #vis const #marker_value: #marker_type = #marker_type;

        impl #krate::doc::HandlerMeta for #marker_type {
            type State = #state_ty;
            const HAS_STATE: bool = #has_state;
            const PATH_NAMES: &'static [&'static str] = #path_names;

            fn docs(
                &self,
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) -> #krate::doc::HandlerDocs {
                let mut docs = #krate::doc::HandlerDocs::default();
                #(#statements)*
                docs
            }
        }
    }
}

fn auth_types(ty: &Type) -> Option<(Type, Option<Type>, bool)> {
    let (auth_ty, optional) = if outer_ident(ty).as_deref() == Some("Option") {
        (single_generic(ty)?, true)
    } else {
        (ty.clone(), false)
    };
    if outer_ident(&auth_ty).as_deref() != Some("Auth") || !is_wrapper_type(&auth_ty) {
        return None;
    }
    let types = generic_types(&auth_ty);
    let auth = types.first()?.clone();
    let scopes = types.get(1).cloned();
    Some((auth, scopes, optional))
}

fn is_wrapper_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let segments = &type_path.path.segments;
    segments.len() == 1
        || segments
            .first()
            .is_some_and(|segment| segment.ident == "roundhouse")
}

fn result_ok_type(ty: &Type) -> Option<Type> {
    if outer_ident(ty).as_deref() != Some("Result") {
        return None;
    }
    generic_types(ty).into_iter().next()
}

fn generic_types(ty: &Type) -> Vec<Type> {
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
            syn::GenericArgument::Type(ty) => Some(ty.clone()),
            _ => None,
        })
        .collect()
}

fn response_statements(output: &ReturnType) -> syn::Result<Vec<TokenStream>> {
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
            docs.responses.extend(#krate::doc::stored_responses_for::<#err>(components));
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

fn ok_statements(ty: &Type, status: u16) -> syn::Result<Vec<TokenStream>> {
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
        Some("Json") if !is_wrapper_type(ty) => Ok(vec![quote! {
            docs.unverified = true;
            docs.responses.push(#krate::doc::ResponseDoc::new(#status_lit, "OK"));
        }]),
        Some("Json") => {
            let inner = single_generic(ty).ok_or_else(|| {
                syn::Error::new_spanned(ty, "expected `Json<T>` as the response body")
            })?;
            Ok(vec![quote! {
                docs.responses.extend(
                    #krate::doc::json_response::<#inner>(components, #status_lit)
                );
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
            docs.unverified = true;
            docs.responses.push(#krate::doc::ResponseDoc::new(#status_lit, "OK"));
        }]),
        Some("Html") => {
            let inner = single_generic(ty).ok_or_else(|| {
                syn::Error::new_spanned(ty, "expected `Html<T>` as the response body")
            })?;
            Ok(vec![quote! {
                docs.responses.push(#krate::doc::text_response::<#inner>(
                    components,
                    #status_lit,
                    "text/html",
                ));
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
                    docs.responses.extend(
                        #krate::doc::json_response::<#inner>(components, #status_lit)
                    );
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
                docs.responses.extend(
                    #krate::doc::stored_responses_for::<#ty>(components)
                );
            }])
        }
    }
}

#[allow(dead_code)]
fn _punctuated_marker(_: Punctuated<Token![,], Token![,]>) {}
