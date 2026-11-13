//! `#[handler]` expansion: signature metadata, extractor docs and the route shim.

mod args;
mod response;
mod signature;

use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{FnArg, ItemFn, LitStr, Type};

use crate::doc_tags::{AllowedTags, DocLine, DocTags};
use crate::util::{outer_ident, single_generic};

pub(crate) use self::args::HandlerArgs;
use self::response::{classify_return, inferred_successes, response_statements};
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

pub fn expand(item: ItemFn, args: HandlerArgs) -> TokenStream {
    let krate = crate::util::krate();
    if !item.sig.generics.params.is_empty() {
        return syn::Error::new_spanned(
            &item.sig.generics,
            "roundhouse handlers cannot be generic; use a concrete signature",
        )
        .to_compile_error();
    }
    let tags = match crate::doc_tags::parse(&item.attrs, AllowedTags::HANDLER) {
        Ok(tags) => tags,
        Err(error) => return error.to_compile_error(),
    };
    let (declared_responses, declared_success) = match declared_response_statements(&tags, &args) {
        Ok(statements) => statements,
        Err(error) => return error.to_compile_error(),
    };
    let response_descriptions = match response_description_statements(&tags, &args) {
        Ok(statements) => statements,
        Err(error) => return error.to_compile_error(),
    };
    let declared_success = match declared_success {
        Some(statuses) => {
            let statuses = statuses
                .iter()
                .map(|status| proc_macro2::Literal::u16_suffixed(*status));
            quote!(::core::option::Option::Some(&[#(#statuses),*]))
        }
        None => quote!(::core::option::Option::None),
    };

    let name = item.sig.ident.clone();
    let marker_type = format_ident!("{}_missing_roundhouse_handler_attribute", name);
    let vis = item.vis.clone();

    let mut state_args: Vec<Type> = Vec::new();
    let mut auth_state: Option<Type> = None;
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
            if auth_state.is_none() {
                auth_state = Some(syn::parse_quote!(
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
                if let Some(state) = single_generic(&strip_wrappers(ty)) {
                    state_args.push(state);
                }
            }
            _ => {}
        }

        // Everything else is the extractor's own business. The probe resolves `ExtractArg` at the
        // concrete type and falls back to the unverified-extractor marker for types the framework
        // does not know.
        statements.push(quote! {
            {
                use #krate::__private::ProbeArg as _;
                docs.merge((&#krate::__private::ArgProbe::<#ty>::new()).document(context));
            }
        });
        verify_checks.push(quote! {
            {
                use #krate::__private::ProbeArg as _;
                let __rh_probe = &#krate::__private::ArgProbe::<#ty>::new();
                if __rh_probe.body_arg() && !__rh_probe.verified() {
                    __rh_all_verified = false;
                }
            }
        });
    }

    if let Some(summary) = &tags.summary {
        let summary = syn::LitStr::new(summary, item.sig.ident.span());
        statements.push(quote! {
            docs.set_summary(#summary.to_string());
        });
    }
    if let Some(description) = &tags.description {
        let description = syn::LitStr::new(description, item.sig.ident.span());
        statements.push(quote! {
            docs.set_description(#description.to_string());
        });
    }
    statements.extend(declared_responses);
    statements.extend(response_descriptions);
    match path_statements(&tags, &args) {
        Ok(path_statements) => statements.extend(path_statements),
        Err(error) => return error.to_compile_error(),
    }

    let return_shape = classify_return(&item.sig.output);
    let response_statements = match response_statements(&return_shape) {
        Ok(statements) => statements,
        Err(error) => return error.to_compile_error(),
    };
    statements.extend(response_statements);

    let path_names = match &path_type {
        Some(ty) => quote!(<#ty as #krate::doc::Params>::NAMES),
        None => quote!(&[]),
    };

    // The declared `State<T>` arguments are only a default: `routes!` may supply the router state
    // with `state = ...`. A single handler with several distinct substates cannot infer which one
    // is the app state, so the first one is the fallback and the table state is the supported way
    // to disambiguate.
    let default_state = state_args.first().cloned().or(auth_state);
    let state_ty = match &default_state {
        Some(ty) => quote!(#ty),
        None => quote!(()),
    };
    let has_state = default_state.is_some();

    let mut substate_bounds: Vec<TokenStream> = Vec::new();
    let mut seen_substates: Vec<String> = Vec::new();
    for state in &state_args {
        let text = quote!(#state).to_string();
        if seen_substates.contains(&text) {
            continue;
        }
        seen_substates.push(text);
        substate_bounds.push(quote!(#state: #krate::__private::SubstateOf<S>));
    }

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
            {
                use #krate::__private::ProbeArg as _;
                let __rh_probe = &#krate::__private::ArgProbe::<#ty>::new();
                if __rh_ctx.arg_enabled(__rh_probe.body_arg())
                    && let ::core::result::Result::Err(__rh_problem) =
                        __rh_probe.validate(&#ident, &__rh_ctx)
                {
                    return #krate::axum::response::IntoResponse::into_response(__rh_problem);
                }
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
    let inferred_success = match inferred_successes(&return_shape) {
        Some(statuses) => {
            let statuses = statuses
                .iter()
                .map(|status| proc_macro2::Literal::u16_suffixed(*status));
            quote!(::core::option::Option::Some(&[#(#statuses),*]))
        }
        None => quote!(::core::option::Option::None),
    };
    // The shim is generic over the router state. `routes!` chooses `S` from the table's
    // `state = ...` option, or falls back to `HandlerSpec::State`. A handler's `State<T>`
    // arguments then use axum's `FromRef` contract through `SubstateOf`.
    quote! {
        #item

        impl #marker_type {
            /// The generated route entry point. `routes!` instantiates the policy and state.
            #[doc(hidden)]
            #[allow(dead_code, clippy::too_many_arguments)]
            #vis async fn __roundhouse_shim<P, S>(
                __rh_ctx: #krate::__private::ValidationContext,
                __rh_state: #krate::axum::extract::State<S>,
                #(#shim_params),*
            ) -> #krate::axum::response::Response
            where
                P: #krate::__private::ShimPolicyFor<S>,
                S: ::core::clone::Clone + ::core::marker::Send + ::core::marker::Sync + 'static,
                #(#substate_bounds,)*
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

        impl #krate::doc::HandlerSpec for #marker_type {
            type State = #state_ty;
            const HAS_STATE: bool = #has_state;
            const PATH_NAMES: &'static [&'static str] = #path_names;
            const REQUIRED_AUTH: &'static [&'static str] = &[#(#required_auth),*];
            const INFERRED_SUCCESS: ::core::option::Option<&'static [u16]> = #inferred_success;
            const DECLARED_SUCCESS: ::core::option::Option<&'static [u16]> = #declared_success;
        }

        impl #krate::doc::DocSource for #marker_type {
            fn operation(
                context: &mut #krate::doc::DocContext<'_>,
            ) -> #krate::doc::OperationDoc {
                let mut docs = #krate::doc::OperationDoc::default();
                #(#statements)*
                let mut __rh_all_verified = true;
                #(#verify_checks)*
                docs.verification.set_body_verified(__rh_all_verified);
                docs
            }
        }
    }
}

/// Expand `#[handler(path(..))]` and `@path:` into path-description statements.
fn path_statements(tags: &DocTags, args: &HandlerArgs) -> syn::Result<Vec<TokenStream>> {
    let mut descriptions: Vec<(String, String, proc_macro2::Span)> = Vec::new();
    if let Some(lines) = &tags.path {
        for line in lines {
            let Some((name, description)) = line.text.split_once(':') else {
                return Err(syn::Error::new(
                    line.span,
                    "`@path:` lines must be `name: description`",
                ));
            };
            descriptions.push((
                name.trim().to_string(),
                description.trim().to_string(),
                line.span,
            ));
        }
    }
    for path in &args.paths {
        descriptions.push((path.name.clone(), path.description.clone(), path.span));
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut statements = Vec::new();
    for (name, description, span) in descriptions {
        if name.is_empty() {
            return Err(syn::Error::new(
                span,
                "a path description needs a parameter name",
            ));
        }
        if description.is_empty() {
            return Err(syn::Error::new(
                span,
                "a path description needs a description after the name",
            ));
        }
        if !seen.insert(name.clone()) {
            return Err(syn::Error::new(
                span,
                format!(
                    "`{name}` is described more than once; typed `#[handler(path(..))]` and \
                     `@path:` cannot both describe it"
                ),
            ));
        }
        let name_lit = LitStr::new(&name, span);
        let description_lit = LitStr::new(&description, span);
        statements.push(quote! {
            docs.push_path_description(
                #name_lit.to_string(),
                #description_lit.to_string(),
            );
        });
    }
    Ok(statements)
}

/// Expand `#[handler(response(..))]` and `@responses:` into declared-response statements.
fn declared_response_statements(
    tags: &DocTags,
    args: &HandlerArgs,
) -> syn::Result<(Vec<TokenStream>, Option<Vec<u16>>)> {
    let krate = crate::util::krate();
    let mut declarations = Vec::new();
    let mut override_existing = args.override_existing;

    if let Some(section) = &tags.responses {
        if section.lines.is_empty() {
            return Err(syn::Error::new(
                proc_macro2::Span::call_site(),
                "`@responses:` needs at least one `status[: Type]` line",
            ));
        }
        override_existing |= section.override_existing;
        for line in &section.lines {
            let (status, body) = response_line(line)?;
            declarations.push(self::args::DeclaredResponse {
                status,
                body,
                span: line.span,
            });
        }
    }
    declarations.extend(args.responses.iter().cloned());

    if declarations.is_empty() {
        return Ok((Vec::new(), None));
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut success = Vec::new();
    let mut statements = Vec::new();
    if override_existing {
        statements.push(quote! {
            docs.set_declared_response_override();
        });
    }

    for declaration in &declarations {
        let status_u16 = declaration.status;
        if !seen.insert(status_u16) {
            return Err(syn::Error::new(
                declaration.span,
                format!(
                    "status {status_u16} is declared more than once; typed \
                     `#[handler(response(..))]` and `@responses:` cannot both declare it"
                ),
            ));
        }
        if status_u16 == 0 || (200..400).contains(&status_u16) {
            success.push(status_u16);
        }
        let status = proc_macro2::Literal::u16_suffixed(status_u16);

        statements.push(match &declaration.body {
            ResponseBody::Empty => quote! {
                docs.push_declared_response(#krate::doc::ResponseDoc::new(
                    #status,
                    #krate::doc::canonical_reason(#status),
                ));
            },
            ResponseBody::Problem => quote! {
                docs.push_declared_response(#krate::doc::ResponseDoc::problem(
                    #status,
                    #krate::doc::canonical_reason(#status),
                ));
            },
            ResponseBody::Json(ty) => quote! {
                docs.push_declared_response({
                    use #krate::__private::ProbeJsonResponseDoc as _;
                    (&#krate::__private::JsonResponseDocProbe::<#ty>::new())
                        .probe_json_response_doc(context, #status)
                });
            },
        });
    }

    Ok((statements, Some(success)))
}

/// Expand `#[handler(response_description(..))]` and `@response-descriptions:` into overrides.
fn response_description_statements(
    tags: &DocTags,
    args: &HandlerArgs,
) -> syn::Result<Vec<TokenStream>> {
    let mut descriptions: Vec<(u16, String, proc_macro2::Span)> = Vec::new();
    if let Some(lines) = &tags.response_descriptions {
        for line in lines {
            let Some((status, description)) = line.text.split_once(':') else {
                return Err(syn::Error::new(
                    line.span,
                    "`@response-descriptions:` lines must be `status: description`",
                ));
            };
            let status = status.trim();
            let status = if status.eq_ignore_ascii_case("default") {
                0
            } else {
                status.parse::<u16>().map_err(|_| {
                    syn::Error::new(
                        line.span,
                        format!("`{status}` is not an HTTP status or `default`"),
                    )
                })?
            };
            if status != 0 && !(100..=599).contains(&status) {
                return Err(syn::Error::new(
                    line.span,
                    format!("`{status}` is not an HTTP status code"),
                ));
            }
            descriptions.push((status, description.trim().to_string(), line.span));
        }
    }
    for description in &args.response_descriptions {
        descriptions.push((
            description.status,
            description.description.clone(),
            description.span,
        ));
    }

    let mut seen = std::collections::BTreeSet::new();
    let mut statements = Vec::new();
    for (status, description, span) in descriptions {
        if !seen.insert(status) {
            return Err(syn::Error::new(
                span,
                format!(
                    "status {status} is described more than once; typed \
                     `#[handler(response_description(..))]` and `@response-descriptions:` \
                     cannot both describe it"
                ),
            ));
        }
        if description.is_empty() {
            return Err(syn::Error::new(
                span,
                "a response description needs text after the status",
            ));
        }
        let description = syn::LitStr::new(&description, span);
        let status = proc_macro2::Literal::u16_suffixed(status);
        statements.push(quote! {
            docs.push_response_description(#status, #description.to_string());
        });
    }
    Ok(statements)
}

#[derive(Clone)]
enum ResponseBody {
    Empty,
    Problem,
    Json(Box<Type>),
}

fn response_line(line: &DocLine) -> syn::Result<(u16, ResponseBody)> {
    let (status_text, body_text) = match line.text.split_once(':') {
        Some((status, body)) => (status.trim(), body.trim()),
        None => (line.text.trim(), ""),
    };
    let status = if status_text.eq_ignore_ascii_case("default") {
        0
    } else {
        status_text.parse::<u16>().map_err(|_| {
            syn::Error::new(
                line.span,
                format!("`{status_text}` is not an HTTP status or `default`"),
            )
        })?
    };
    if status != 0 && !(100..=599).contains(&status) {
        return Err(syn::Error::new(
            line.span,
            format!("`{status}` is not an HTTP status code"),
        ));
    }

    if body_text.is_empty() {
        return Ok((status, ResponseBody::Empty));
    }
    let ty: Type = syn::parse_str(body_text).map_err(|error| {
        syn::Error::new(
            line.span,
            format!("invalid response type `{body_text}`: {error}"),
        )
    })?;
    let body = match outer_ident(&ty).as_deref() {
        Some("Problem" | "ValidationErrors") => ResponseBody::Problem,
        Some("Json") => {
            let inner = single_generic(&ty).ok_or_else(|| {
                syn::Error::new(
                    line.span,
                    "`Json<T>` needs exactly one type argument in `@responses:`",
                )
            })?;
            ResponseBody::Json(Box::new(inner))
        }
        _ => ResponseBody::Json(Box::new(ty)),
    };
    Ok((status, body))
}
