use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::{Ident, LitStr, Result};

use crate::util::{auth_slice, last_segment, marker_type, string_slice};

use super::parse::{Body, Entry, LayerMode, ModuleOptions, RoutesInput};
use super::validate::{flatten, validate_entries};

pub fn expand(input: RoutesInput) -> TokenStream {
    let mut flattened = Vec::new();
    flatten(&input.body, "", &mut flattened);
    if let Err(error) = validate_entries(&flattened) {
        return error.to_compile_error();
    }

    let krate = crate::util::krate();
    let mut ids = 0usize;
    let mut definitions = TokenStream::new();
    emit_module(
        &format_ident!("MODULE"),
        true,
        input.options.prefix.as_ref(),
        &input.options,
        &input.body,
        &mut ids,
        &mut definitions,
    );

    quote! {
        #definitions

        /// The explicit fragment this `routes!` table contributes.
        #[doc(hidden)]
        pub fn __roundhouse_fragment() -> #krate::Fragment {
            #krate::Fragment::new(&MODULE)
        }
    }
}

pub(super) fn emit_module(
    name: &Ident,
    public: bool,
    prefix: Option<&LitStr>,
    options: &ModuleOptions,
    body: &Body,
    ids: &mut usize,
    definitions: &mut TokenStream,
) {
    let krate = crate::util::krate();
    let module_id = *ids;
    *ids += 1;

    let mut routes = Vec::new();
    for entry in &body.entries {
        let id = *ids;
        *ids += 1;
        let ExpandedEntry { defs, record } = expand_entry(entry, id);
        definitions.extend(defs);
        let attrs = &entry.attrs;
        routes.push(quote!(#(#attrs)* #record));
    }

    let mut children = Vec::new();
    for (index, group) in body.groups.iter().enumerate() {
        let child_name = format_ident!("__ROUNDHOUSE_GROUP_{module_id}_{index}");
        let prefix = if group.shared_path {
            None
        } else {
            Some(&group.path)
        };
        emit_module(
            &child_name,
            false,
            prefix,
            &group.options,
            &group.body,
            ids,
            definitions,
        );
        children.push(quote!(&#child_name));
    }
    for spread in &body.spreads {
        children.push(quote!(&#spread::MODULE));
    }

    let prefix_value = prefix.map(|prefix| prefix.value()).unwrap_or_default();
    let prefix_lit = LitStr::new(&prefix_value, proc_macro2::Span::call_site());
    let tags = string_slice(&options.tags);
    let security = auth_slice(&options.security);
    let hidden = options.hidden;

    let layer_fn = format_ident!("__roundhouse_module_layer_{module_id}");
    let layer_value = if options.layer.is_some() {
        quote!(::core::option::Option::Some(#layer_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    if let Some(expr) = &options.layer {
        let body = match options.layer_mode {
            LayerMode::Full => quote!(router.layer(#expr)),
            LayerMode::Route => quote!(router.route_layer(#expr)),
        };
        definitions.extend(quote! {
            fn #layer_fn(
                router: #krate::axum::Router,
            ) -> #krate::axum::Router {
                #body
            }
        });
    }

    let doc_fn = format_ident!("__roundhouse_module_doc_{module_id}");
    let doc_value = if options.doc.is_some() {
        quote!(::core::option::Option::Some(#doc_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    if let Some(expr) = &options.doc {
        definitions.extend(quote! {
            fn #doc_fn() -> #krate::doc::layer::LayerDoc {
                #expr
            }
        });
    }

    let line = match prefix {
        Some(prefix) => {
            let line = prefix.span().start().line as u32;
            quote!(#line)
        }
        None => quote!(line!()),
    };
    let visibility = if public { quote!(pub) } else { quote!() };

    definitions.extend(quote! {
        #visibility const #name: #krate::registry::Module = #krate::registry::Module::__new(
            module_path!(),
            file!(),
            #line,
            #prefix_lit,
            #tags,
            #security,
            #hidden,
            #layer_value,
            #doc_value,
            &[#(#routes),*],
            &[#(#children),*],
        );
    });
}

pub(super) struct ExpandedEntry {
    pub(super) defs: TokenStream,
    pub(super) record: Ident,
}
pub(super) fn method_variant(method: &Ident) -> Result<TokenStream> {
    let krate = crate::util::krate();
    Ok(match method.to_string().as_str() {
        "get" => quote!(#krate::registry::Method::Get),
        "post" => quote!(#krate::registry::Method::Post),
        "put" => quote!(#krate::registry::Method::Put),
        "patch" => quote!(#krate::registry::Method::Patch),
        "delete" => quote!(#krate::registry::Method::Delete),
        "head" => quote!(#krate::registry::Method::Head),
        "options" => quote!(#krate::registry::Method::Options),
        "trace" => quote!(#krate::registry::Method::Trace),
        "any" => quote!(#krate::registry::Method::Any),
        other => {
            return Err(syn::Error::new(
                method.span(),
                format!(
                    "unknown verb `{other}`; expected get, post, put, patch, delete, head, options, trace or any"
                ),
            ));
        }
    })
}

pub(super) fn expand_entry(entry: &Entry, index: usize) -> ExpandedEntry {
    let krate = crate::util::krate();
    if !entry.path.value().starts_with('/') {
        return ExpandedEntry {
            defs: syn::Error::new(
                entry.path.span(),
                "route paths must start with `/` (use `/` for a module's own path)",
            )
            .to_compile_error(),
            record: format_ident!("__ROUNDHOUSE_ROUTE_{index}"),
        };
    }
    if entry
        .path
        .value()
        .split('/')
        .any(|segment| segment.starts_with(':'))
    {
        return ExpandedEntry {
            defs: syn::Error::new(
                entry.path.span(),
                "axum 0.7 path syntax (`:id`) is not supported; use `{id}` for a capture and `{*rest}` for a wildcard",
            )
            .to_compile_error(),
            record: format_ident!("__ROUNDHOUSE_ROUTE_{index}"),
        };
    }

    let attrs = &entry.attrs;
    let method_ident = format_ident!("{}", entry.method);
    let method = match method_variant(&entry.method) {
        Ok(method) => method,
        Err(error) => {
            return ExpandedEntry {
                defs: error.to_compile_error(),
                record: format_ident!("__ROUNDHOUSE_ROUTE_{index}"),
            };
        }
    };
    let path = &entry.path;
    let assert_path = option_lit(&entry.assert_path);
    let handler = &entry.handler;
    let marker_type = marker_type(handler);
    let single_scheme = match entry.security.as_slice() {
        [scheme] => Some(scheme),
        _ => None,
    };
    let shim_state = match single_scheme {
        Some(scheme) => quote!(<#scheme as #krate::auth::AuthScheme>::State),
        None => quote!(<#marker_type as #krate::doc::HandlerMeta>::State),
    };
    let policy = match (single_scheme, entry.validate) {
        (Some(scheme), true) => quote!(#krate::shim::AuthedValidated::<#scheme>),
        (Some(scheme), false) => quote!(#krate::shim::Authed::<#scheme>),
        (None, true) => quote!(#krate::shim::Validated::<#shim_state>),
        (None, false) => quote!(#krate::shim::Plain::<#shim_state>),
    };
    let state = match single_scheme {
        Some(_) => quote!(
            || ::core::option::Option::Some(
                ::core::any::TypeId::of::<#shim_state>(),
            )
        ),
        None => quote!(|| #krate::doc::handler_state::<#marker_type>()),
    };
    let state_name = match single_scheme {
        Some(_) => quote!(|| ::core::any::type_name::<#shim_state>()),
        None => quote!(
            || ::core::any::type_name::<
                <#marker_type as #krate::doc::HandlerMeta>::State,
            >()
        ),
    };
    let handler_alias = format_ident!("__RoundhouseHandler{index}");
    // Through an alias so a missing `#[handler]` reports one "type not found" error rather than
    // also interpreting the marker as a module path.
    let route_handler = quote!(#handler_alias::__roundhouse_shim::<#policy>);
    let shim_auth = match single_scheme {
        Some(scheme) => quote!(&[<#scheme as #krate::auth::AuthScheme>::NAME]),
        None => quote!(&[]),
    };
    let operation_id = option_lit(&entry.operation_id);
    let handler_name = LitStr::new(&last_segment(handler), entry.path.span());
    let assertion_message = LitStr::new(
        &format!(
            "roundhouse: Path<T> in handler `{}` does not declare every capture in `{}`",
            last_segment(handler),
            entry.path.value().replace('{', "{{").replace('}', "}}"),
        ),
        entry.path.span(),
    );
    let summary = option_lit(&entry.summary);
    let description = option_lit(&entry.description);
    let tags = string_slice(&entry.tags);
    let security = auth_slice(&entry.security);
    let required_auth = quote!(<#marker_type as #krate::doc::HandlerMeta>::REQUIRED_AUTH);
    let hidden = entry.hidden;
    let absolute = entry.absolute;
    let validate = entry.validate;
    let deprecated = entry.deprecated;
    let public = entry.public;
    let response_override = entry.response_override;
    let declared_success = entry
        .responses
        .iter()
        .any(|(status, _)| *status == 0 || (200..400).contains(status));
    let no_success_message = LitStr::new(
        &format!(
            "roundhouse: `{} {}` has no success response; return a concrete response type or \
             declare `responses(..)` in the table",
            method_ident,
            entry.path.value().replace('{', "{{").replace('}', "}}")
        ),
        entry.path.span(),
    );
    let success_assert = quote_spanned! { entry.path.span()=>
        const _: () = {
            if !#declared_success {
                if let ::core::option::Option::Some(__statuses) =
                    <#handler_alias as #krate::doc::HandlerMeta>::INFERRED_SUCCESS
                {
                    assert!(!__statuses.is_empty(), #no_success_message);
                }
            }
        };
    };
    let responses = entry.responses.iter().map(|(status, body)| {
        let status = proc_macro2::Literal::u16_suffixed(*status);
        match body {
            Some(ty) => quote! {
                __responses.push({
                    use #krate::doc::ProbeJsonResponseDoc as _;
                    (&#krate::doc::JsonResponseDocProbe::<#ty>::new())
                        .probe_json_response_doc(components, #status)
                });
            },
            None => quote! {
                __responses.push(#krate::doc::ResponseDoc::new(
                    #status,
                    #krate::doc::canonical_reason(#status),
                ));
            },
        }
    });
    let line = entry.path.span().start().line as u32;

    let route_fn = format_ident!("__roundhouse_route_{index}");
    let responses_fn = format_ident!("__roundhouse_responses_{index}");
    let layer_fn = format_ident!("__roundhouse_layer_{index}");
    let doc_fn = format_ident!("__roundhouse_entry_doc_{index}");
    let record_name = format_ident!("__ROUNDHOUSE_ROUTE_{index}");

    let layer_value = if entry.layer.is_some() {
        quote!(::core::option::Option::Some(#layer_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    let layer = match &entry.layer {
        Some(expr) => quote! {
            fn #layer_fn(
                method_router: #krate::axum::routing::MethodRouter,
            ) -> #krate::axum::routing::MethodRouter {
                method_router.layer(#expr)
            }
        },
        None => quote! {},
    };
    let doc_value = if entry.doc.is_some() {
        quote!(::core::option::Option::Some(#doc_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    let doc = match &entry.doc {
        Some(expr) => quote! {
            fn #doc_fn(docs: &mut #krate::doc::HandlerDocs) {
                let hook: fn(&mut #krate::doc::HandlerDocs) = #expr;
                hook(docs)
            }
        },
        None => quote! {},
    };

    let defs = quote! {
        #(#attrs)*
        #[allow(non_camel_case_types)]
        type #handler_alias = #marker_type;

        #(#attrs)*
        #[allow(non_upper_case_globals)]
        const _: () = {
            #success_assert

            const _: () = {
                assert!(
                    #krate::prefix::path_names_match(
                        #path,
                        <#marker_type as #krate::doc::HandlerMeta>::PATH_NAMES,
                    ),
                    #assertion_message,
                );
            };
        };

        #(#attrs)*
        fn #route_fn()
            -> ::std::boxed::Box<dyn ::core::any::Any + ::core::marker::Send + ::core::marker::Sync>
        {
            let method_router: #krate::axum::routing::MethodRouter<#shim_state> =
                #krate::axum::routing::#method_ident(#route_handler);
            ::std::boxed::Box::new(method_router)
        }

        #(#attrs)*
        #layer
        #(#attrs)*
        #doc

        #(#attrs)*
        fn #responses_fn(
            components: &mut #krate::utoipa::openapi::schema::Components,
        ) -> ::std::vec::Vec<#krate::doc::ResponseDoc> {
            let mut __responses = ::std::vec::Vec::new();
            #(#responses)*
            __responses
        }

        #(#attrs)*
        #[allow(non_upper_case_globals)]
        const #record_name: #krate::registry::RouteRecord = #krate::registry::RouteRecord::__new(
            module_path!(),
            file!(),
            #line,
            #method,
            #path,
            #assert_path,
            #operation_id,
            #handler_name,
            #summary,
            #description,
            #tags,
            #security,
            #required_auth,
            #shim_auth,
            #hidden,
            #absolute,
            #validate,
            #deprecated,
            #public,
            #response_override,
            #responses_fn,
            #state,
            #state_name,
            #route_fn,
            #layer_value,
            <#marker_type as #krate::doc::HandlerMeta>::docs_with,
            #doc_value,
        );
    };

    ExpandedEntry {
        defs,
        record: record_name,
    }
}

pub(super) fn option_lit(value: &Option<LitStr>) -> TokenStream {
    match value {
        Some(lit) => quote!(::core::option::Option::Some(#lit)),
        None => quote!(::core::option::Option::None),
    }
}
