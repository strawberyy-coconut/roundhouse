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
    let marker_type = format_ident!("{}_missing_roundhouse_handler_attribute", name);
    let vis = item.vis.clone();

    let mut state_type: Option<Type> = None;
    let mut path_type: Option<Type> = None;
    let mut statements: Vec<TokenStream> = Vec::new();
    let mut required_auth: Vec<TokenStream> = Vec::new();
    let mut body_count = 0usize;
    let mut body_type: Option<Type> = None;

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
                required_auth.push(quote!(
                    <#auth as #krate::auth::AuthScheme>::NAME
                ));
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

        // Compile-time metadata that cannot be recovered from the resolved value: the state
        // type and the `Path<T>` capture list. Both are best-effort and conservative; a path
        // alias simply leaves `PATH_NAMES` empty (the build still checks the document).
        if let Some(kind) = outer_ident(&ty) {
            match kind.as_str() {
                "Path" if is_wrapper_type(&ty) => {
                    if let Some(inner) = single_generic(&ty) {
                        path_type = Some(inner.clone());
                    }
                }
                "State" => {
                    if let Some(inner) = single_generic(&ty) {
                        state_type = Some(inner);
                    }
                }
                "Json" | "Form" | "Multipart" | "Bytes" | "Body" => {
                    body_count += 1;
                }
                _ => {}
            }
        }

        // Remember the body type so `docs_with` can clear the unverified-body marker when the
        // route's shim validates it. A `Result<Extractor, E>` owns its rejection and is never
        // cleared; `Option<Json<T>>` is unwrapped so the probe sees `Json<T>`.
        if !handled && body_type.is_none() && !is_parts_arg(&ty) {
            body_type = Some(if outer_ident(&ty).as_deref() == Some("Option") {
                single_generic(&ty).unwrap_or_else(|| ty.clone())
            } else {
                ty.clone()
            });
        }

        // Everything else is decided by the resolved type. This is what keeps an alias or a
        // re-export (`use axum::Json;`, `use roundhouse::Json as Body;`) from claiming the
        // documentation of a different extractor.
        let add_failures = !handled;
        statements.push(quote! {
            {
                use #krate::doc::ProbeExtractor as _;
                (&#krate::doc::ExtractProbe::<#ty>::new())
                    .probe_extractor(components, &mut docs, #add_failures);
            }
            {
                use #krate::doc::ProbeJsonBody as _;
                (&&#krate::doc::JsonBodyProbe::<#ty>::new())
                    .probe_json_body(components, &mut docs);
            }
            {
                use #krate::doc::ProbeForeignBody as _;
                (&&#krate::doc::ForeignBodyProbe::<#ty>::new())
                    .probe_foreign_body(components, &mut docs);
            }
            {
                use #krate::doc::ProbeParts as _;
                (&&#krate::doc::PartsProbe::<#ty>::new()).probe_parts(components, &mut docs);
            }
        });
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

    // The shim that routes! registers instead of the handler itself. It has the handler's own
    // extractors, plus the policy's auth extractor and a ValidationContext. The handler function
    // above is untouched, so direct calls still behave exactly as written.
    let mut shim_params = Vec::new();
    let mut shim_idents = Vec::new();
    let mut shim_checks = Vec::new();
    // The auth extractor goes just before the body argument, so a handler that also takes
    // `Auth<A>` runs first and leaves the `Authenticated` marker. The shim's `AuthEnforce` then
    // sees it and skips a second authentication.
    let mut auth_positioned = false;
    for (index, argument) in item.sig.inputs.iter().enumerate() {
        let FnArg::Typed(argument) = argument else {
            continue;
        };
        if !auth_positioned && !is_parts_arg(&argument.ty) {
            shim_params.push(quote!(__rh_auth: P::Auth));
            auth_positioned = true;
        }
        let ident = format_ident!("__rh_arg_{index}");
        let ty = &argument.ty;
        shim_params.push(quote!(#ident: #ty));
        shim_checks.push(quote! {
            if let ::core::result::Result::Err(__rh_problem) = {
                use #krate::shim::ValidateArg as _;
                (&#ident).validate_arg(&__rh_ctx)
            } {
                return #krate::axum::response::IntoResponse::into_response(__rh_problem);
            }
        });
        shim_idents.push(ident);
    }
    if !auth_positioned {
        shim_params.push(quote!(__rh_auth: P::Auth));
    }
    let shim_call = if item.sig.asyncness.is_some() {
        quote!(#name(#(#shim_idents),*).await)
    } else {
        quote!(#name(#(#shim_idents),*))
    };
    let body_validated = match &body_type {
        Some(ty) => quote!({
            use #krate::shim::ProbeBody as _;
            (&#krate::shim::BodyProbe::<#ty>::new()).probe_body()
        }),
        None => quote!(true),
    };
    let inferred_success = match inferred_successes(&item.sig.output) {
        Some(statuses) => {
            let statuses = statuses
                .iter()
                .map(|status| proc_macro2::Literal::u16_suffixed(*status));
            quote!(::core::option::Option::Some(&[#(#statuses),*]))
        }
        None => quote!(::core::option::Option::None),
    };
    // A handler that declares `State<S>` fixes the router state. For an authenticated route that
    // must equal the scheme's `AuthScheme::State`; `ShimPolicyFor` turns a mismatch into a
    // compile error with a targeted diagnostic. A stateless handler lets the policy provide the
    // state instead, so it can sit under any scheme.
    let (state_param, policy_bound) = if has_state {
        (
            quote!(__rh_state: #krate::axum::extract::State<#state_ty>),
            quote!(P: #krate::shim::ShimPolicyFor<#state_ty>),
        )
    } else {
        (
            quote!(__rh_state: #krate::axum::extract::State<P::State>),
            quote!(P: #krate::shim::ShimPolicy),
        )
    };

    quote! {
        #item

        impl #marker_type {
            /// The generated route entry point. `routes!` instantiates the policy.
            #[doc(hidden)]
            #[allow(dead_code, clippy::too_many_arguments)]
            #vis async fn __roundhouse_shim<P>(
                __rh_ctx: #krate::shim::ValidationContext,
                #state_param,
                #(#shim_params),*
            ) -> #krate::axum::response::Response
            where
                #policy_bound,
            {
                let _ = (&__rh_auth, &__rh_state);
                if P::VALIDATE && __rh_ctx.enabled() {
                    #(#shim_checks)*
                }
                let __rh_response = #shim_call;
                #krate::axum::response::IntoResponse::into_response(__rh_response)
            }
        }

        #[doc(hidden)]
        #[allow(non_camel_case_types)]
        #vis struct #marker_type;

        impl #krate::doc::HandlerMeta for #marker_type {
            type State = #state_ty;
            const HAS_STATE: bool = #has_state;
            const PATH_NAMES: &'static [&'static str] = #path_names;
            const REQUIRED_AUTH: &'static [&'static str] = &[#(#required_auth),*];
            const INFERRED_SUCCESS: ::core::option::Option<&'static [u16]> = #inferred_success;

            fn docs(
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) -> #krate::doc::HandlerDocs {
                let mut docs = #krate::doc::HandlerDocs::default();
                #(#statements)*
                docs
            }

            fn docs_with(
                components: &mut #krate::utoipa::openapi::schema::Components,
                validate: bool,
            ) -> #krate::doc::HandlerDocs {
                let mut docs = <Self as #krate::doc::HandlerMeta>::docs(components);
                if validate && #body_validated {
                    docs.unverified = false;
                }
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

/// Whether this argument is a parts extractor. The injected auth extractor is inserted before the
/// first argument that is not one, so it always precedes the body. An unknown extractor is
/// treated as a body: that is the conservative choice, and ordering among parts does not matter.
fn is_parts_arg(ty: &Type) -> bool {
    let ty = result_ok_type(ty).unwrap_or_else(|| ty.clone());
    let ty = if outer_ident(&ty).as_deref() == Some("Option") {
        single_generic(&ty).unwrap_or(ty)
    } else {
        ty
    };
    matches!(
        outer_ident(&ty).as_deref(),
        Some(
            "State"
                | "Path"
                | "Query"
                | "Extension"
                | "Auth"
                | "HeaderMap"
                | "Method"
                | "Uri"
                | "OriginalUri"
                | "MatchedPath"
                | "Parts"
                | "ConnectInfo"
                | "TypedHeader"
        )
    )
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

/// The success statuses the return type guarantees, or `None` when the macro cannot classify it
/// (a custom `ApiResponse` type registers its responses at build time). `Some(&[])` means the
/// macro knows there is no success response, which is a compile error unless the entry declares
/// one.
fn inferred_successes(output: &ReturnType) -> Option<Vec<u16>> {
    let ReturnType::Type(_, ty) = output else {
        return Some(vec![204]);
    };
    let types = generic_types(ty);
    if outer_ident(ty).as_deref() == Some("Result") && types.len() == 2 {
        return ok_successes(&types[0]);
    }
    ok_successes(ty)
}

fn ok_successes(ty: &Type) -> Option<Vec<u16>> {
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
        Some("Json") if !is_wrapper_type(ty) => {
            let inner = single_generic(ty).ok_or_else(|| {
                syn::Error::new_spanned(ty, "expected `Json<T>` as the response body")
            })?;
            Ok(vec![quote! {
                if let Some(__schema) = {
                    use #krate::doc::ForeignSchemaProbe as _;
                    (&#krate::doc::ForeignSchema::<#inner>::new())
                        .foreign_schema(components)
                } {
                    docs.responses.push(#krate::doc::ResponseDoc::json(
                        #status_lit,
                        __schema,
                    ));
                } else {
                    docs.responses.push(#krate::doc::ResponseDoc::new(
                        #status_lit,
                        "OK",
                    ));
                }
                docs.unverified_response = true;
            }])
        }
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
            docs.unverified_response = true;
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
