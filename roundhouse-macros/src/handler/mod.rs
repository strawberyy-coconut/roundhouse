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
    let mut verify_checks: Vec<TokenStream> = Vec::new();

    for argument in &item.sig.inputs {
        let FnArg::Typed(argument) = argument else {
            continue;
        };
        let ty = &argument.ty;

        if let Some((auth, _scopes, optional)) = auth_types(ty) {
            if state_type.is_none() {
                state_type = Some(syn::parse_quote!(
                    <#auth as #krate::auth::AuthScheme>::State
                ));
            }
            if !optional {
                required_auth.push(quote!(
                    <#auth as #krate::auth::AuthScheme>::NAME
                ));
            }
        }

        // Compile-time metadata that cannot be recovered from a trait bound: the state type and
        // the `Path<T>` capture list. Both are best-effort and conservative; a path alias simply
        // leaves `PATH_NAMES` empty (the build still checks the document).
        match classify(ty) {
            ExtractorKind::Path if is_wrapper_type(ty) => {
                path_type = single_generic(&strip_wrappers(ty));
            }
            ExtractorKind::State => {
                state_type = single_generic(&strip_wrappers(ty));
            }
            _ => {}
        }

        // Everything else is the extractor's own business. The probe resolves `ExtractArg` at the
        // concrete type and falls back to the unverified-extractor marker for types the framework
        // does not know.
        statements.push(quote! {
            {
                use #krate::doc::ProbeArg as _;
                (&#krate::doc::ArgProbe::<#ty>::new()).document(components, &mut docs);
            }
        });
        verify_checks.push(quote! {
            {
                use #krate::doc::ProbeArg as _;
                let __rh_probe = &#krate::doc::ArgProbe::<#ty>::new();
                if __rh_probe.body_arg() && !__rh_probe.verified() {
                    __rh_all_verified = false;
                }
            }
        });
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
    // The auth extractor goes just before the body argument. A handler that also takes `Auth<A>`
    // runs first and leaves the `Authenticated` marker, so the shim's `AuthEnforce` skips a
    // second authentication.
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
                use #krate::doc::ProbeArg as _;
                (&#krate::doc::ArgProbe::<#ty>::new()).validate(&#ident, &__rh_ctx)
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
                if validate {
                    let mut __rh_all_verified = true;
                    #(#verify_checks)*
                    if __rh_all_verified {
                        docs.unverified = false;
                    }
                }
                docs
            }
        }
    }
}
