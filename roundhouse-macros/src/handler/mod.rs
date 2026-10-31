//! `#[handler]` expansion: signature metadata, extractor docs and the route shim.

mod response;
mod signature;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, Type};

use crate::util::{outer_ident, single_generic};

use self::response::{inferred_successes, response_statements};
use self::signature::{
    ExtractorKind, auth_types, classify, is_explicit_strict, is_parts_arg, is_wrapper_type,
    result_ok_type, strip_wrappers,
};

/// How the shim extracts one handler argument.
enum ShimWrap {
    /// The handler's own extractor, unchanged.
    None,
    /// `Mapped<T>`: the extractor's rejection is a `Problem`.
    Mapped,
    /// `Option<Mapped<T>>` for an optional fallible extractor.
    OptionalMapped(Box<Type>),
}

fn shim_wrap(ty: &Type) -> ShimWrap {
    // `Result<Extractor, E>` owns its rejection; an explicit `Strict<..>` enforces in the extractor.
    if result_ok_type(ty).is_some() || is_explicit_strict(ty) {
        return ShimWrap::None;
    }
    if outer_ident(ty).as_deref() == Some("Option") {
        return match single_generic(ty) {
            Some(inner) if classify(&inner).maps_rejection() => {
                ShimWrap::OptionalMapped(Box::new(inner))
            }
            _ => ShimWrap::None,
        };
    }
    if classify(ty).maps_rejection() {
        ShimWrap::Mapped
    } else {
        ShimWrap::None
    }
}

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
        match classify(&ty) {
            ExtractorKind::Path if is_wrapper_type(&ty) => {
                path_type = single_generic(&strip_wrappers(&ty));
            }
            ExtractorKind::State => {
                state_type = single_generic(&strip_wrappers(&ty));
            }
            kind if kind.is_body() => {
                body_count += 1;
            }
            _ => {}
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
            if !#handled {
                use #krate::doc::ProbeJsonBody as _;
                (&&#krate::doc::JsonBodyProbe::<#ty>::new())
                    .probe_json_body(components, &mut docs);
                use #krate::doc::ProbeToSchemaBody as _;
                (&&#krate::doc::ToSchemaBodyProbe::<#ty>::new())
                    .probe_to_schema_body(components, &mut docs);
            }
            {
                use #krate::doc::ProbeForeignBody as _;
                (&&#krate::doc::ForeignBodyProbe::<#ty>::new())
                    .probe_foreign_body(components, &mut docs);
            }
            if !#handled {
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
    let mut shim_unwraps = Vec::new();
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
        // Route policy, applied by the shim rather than in the handler's own signature: the
        // extractor's rejection becomes a `Problem` without the handler naming `Strict`. An
        // unrecognized extractor keeps its own rejection, so this never guesses at a type.
        let wrap = shim_wrap(ty);
        shim_params.push(match &wrap {
            ShimWrap::None => quote!(#ident: #ty),
            ShimWrap::Mapped => quote!(#ident: #krate::extract::Mapped<#ty>),
            ShimWrap::OptionalMapped(inner) => {
                quote!(#ident: ::core::option::Option<#krate::extract::Mapped<#inner>>)
            }
        });
        shim_unwraps.push(match wrap {
            ShimWrap::None => quote!(),
            ShimWrap::Mapped => quote! {
                let #ident = #krate::extract::Mapped::into_inner(#ident);
            },
            ShimWrap::OptionalMapped(_) => quote! {
                let #ident = ::core::option::Option::map(
                    #ident,
                    #krate::extract::Mapped::into_inner,
                );
            },
        });
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
                #(#shim_unwraps)*
                if __rh_ctx.enabled() {
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
