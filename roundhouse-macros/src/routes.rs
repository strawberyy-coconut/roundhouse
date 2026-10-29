use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Expr, Ident, LitInt, LitStr, Path, Result, Token, Type};

use crate::util::{last_segment, marker_type, marker_value, parse_u16, string_slice};

struct Entry {
    attrs: Vec<Attribute>,
    method: Ident,
    path: LitStr,
    handler: Path,
    summary: Option<LitStr>,
    description: Option<LitStr>,
    operation_id: Option<LitStr>,
    tags: Vec<LitStr>,
    security: Vec<LitStr>,
    responses: Vec<(u16, Option<Type>)>,
    layer: Option<Expr>,
    hidden: bool,
    absolute: bool,
    validate: bool,
    deprecated: bool,
    public: bool,
    response_override: bool,
}

impl Parse for Entry {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let attrs = Attribute::parse_outer(input)?;
        let method: Ident = input.parse()?;
        let content;
        syn::parenthesized!(content in input);

        let path: LitStr = content.parse()?;
        content.parse::<Token![,]>()?;
        let handler: Path = content.parse()?;

        let mut entry = Entry {
            attrs,
            method,
            path,
            handler,
            summary: None,
            description: None,
            operation_id: None,
            tags: Vec::new(),
            security: Vec::new(),
            responses: Vec::new(),
            layer: None,
            hidden: false,
            absolute: false,
            validate: true,
            deprecated: false,
            public: false,
            response_override: false,
        };
        parse_attrs(&content, &mut entry)?;
        Ok(entry)
    }
}

fn parse_group_entry(input: ParseStream<'_>, path: LitStr) -> Result<Entry> {
    let attrs = Attribute::parse_outer(input)?;
    let method: Ident = input.parse()?;
    let content;
    syn::parenthesized!(content in input);

    let handler: Path = content.parse()?;
    let mut entry = Entry {
        attrs,
        method,
        path,
        handler,
        summary: None,
        description: None,
        operation_id: None,
        tags: Vec::new(),
        security: Vec::new(),
        responses: Vec::new(),
        layer: None,
        hidden: false,
        absolute: false,
        validate: true,
        deprecated: false,
        public: false,
        response_override: false,
    };
    parse_attrs(&content, &mut entry)?;
    Ok(entry)
}

fn parse_attrs(content: ParseStream<'_>, entry: &mut Entry) -> Result<()> {
    while content.peek(Token![,]) {
        content.parse::<Token![,]>()?;
        if content.is_empty() {
            break;
        }
        let key: Ident = content.call(Ident::parse_any)?;
        match key.to_string().as_str() {
            "summary" => {
                content.parse::<Token![=]>()?;
                entry.summary = Some(content.parse()?);
            }
            "description" => {
                content.parse::<Token![=]>()?;
                entry.description = Some(content.parse()?);
            }
            "operation_id" => {
                content.parse::<Token![=]>()?;
                entry.operation_id = Some(content.parse()?);
            }
            "tags" => {
                content.parse::<Token![=]>()?;
                entry.tags = parse_string_list(content)?;
            }
            "security" => {
                content.parse::<Token![=]>()?;
                entry.security = parse_string_list(content)?;
            }
            "responses" => {
                let responses;
                syn::parenthesized!(responses in content);
                entry.responses = parse_responses(&responses)?;
            }
            "layer" => {
                content.parse::<Token![=]>()?;
                entry.layer = Some(content.parse()?);
            }
            "hidden" => entry.hidden = true,
            "absolute" => entry.absolute = true,
            "public" => entry.public = true,
            "deprecated" => entry.deprecated = true,
            "override" => entry.response_override = true,
            "validate" => {
                content.parse::<Token![=]>()?;
                let value: syn::LitBool = content.parse()?;
                entry.validate = value.value;
            }
            other => {
                return Err(syn::Error::new(
                    key.span(),
                    format!("unknown routes! attribute `{other}`"),
                ));
            }
        }
    }
    Ok(())
}

fn parse_string_list(input: ParseStream<'_>) -> Result<Vec<LitStr>> {
    let content;
    syn::bracketed!(content in input);
    Ok(content
        .parse_terminated(|input| input.parse::<LitStr>(), Token![,])?
        .into_iter()
        .collect())
}

fn parse_responses(input: ParseStream<'_>) -> Result<Vec<(u16, Option<Type>)>> {
    let mut responses = Vec::new();
    while !input.is_empty() {
        let literal: LitInt = input.parse()?;
        let status = parse_u16(&literal)?;
        let body = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse::<Type>()?)
        } else {
            None
        };
        responses.push((status, body));
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(responses)
}

pub struct RoutesInput {
    entries: Vec<Entry>,
}

impl Parse for RoutesInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut entries = Vec::new();

        while !input.is_empty() {
            let attrs = Attribute::parse_outer(input)?;

            if input.peek(LitStr) {
                // Grouped form: "/{id}" => { get(show), patch(update) }
                let path: LitStr = input.parse()?;
                input.parse::<Token![=>]>()?;
                let content;
                syn::braced!(content in input);
                while !content.is_empty() {
                    let mut entry = parse_group_entry(&content, path.clone())?;
                    entry.attrs.splice(0..0, attrs.clone());
                    entries.push(entry);
                    if content.peek(Token![,]) {
                        content.parse::<Token![,]>()?;
                    }
                }
            } else {
                let mut entry: Entry = input.parse()?;
                entry.attrs.splice(0..0, attrs);
                entries.push(entry);
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(Self { entries })
    }
}

pub fn expand(input: RoutesInput) -> TokenStream {
    let mut output = TokenStream::new();
    for entry in input.entries {
        let expanded = expand_entry(entry);
        output.extend(expanded);
    }
    output
}

fn method_variant(method: &Ident) -> Result<TokenStream> {
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

fn expand_entry(entry: Entry) -> TokenStream {
    let krate = crate::util::krate();
    if !entry.path.value().starts_with('/') {
        return syn::Error::new(
            entry.path.span(),
            "route paths must start with `/` (use `/` for a module's own path)",
        )
        .to_compile_error();
    }
    if entry
        .path
        .value()
        .split('/')
        .any(|segment| segment.starts_with(':'))
    {
        return syn::Error::new(
            entry.path.span(),
            "axum 0.7 path syntax (`:id`) is not supported; use `{id}` for a capture and `{*rest}` for a wildcard",
        )
        .to_compile_error();
    }
    let attrs = &entry.attrs;
    let method_ident = format_ident!("{}", entry.method);
    let method = match method_variant(&entry.method) {
        Ok(method) => method,
        Err(error) => return error.to_compile_error(),
    };
    let path = &entry.path;
    let handler = &entry.handler;
    let marker_type = marker_type(handler);
    let marker_value = marker_value(handler);
    let operation_id = entry
        .operation_id
        .as_ref()
        .map(|lit| LitStr::new(&lit.value(), lit.span()))
        .unwrap_or_else(|| LitStr::new(&last_segment(handler), entry.path.span()));
    let summary = option_lit(&entry.summary);
    let description = option_lit(&entry.description);
    let tags = string_slice(&entry.tags);
    let security = string_slice(&entry.security);
    let hidden = entry.hidden;
    let absolute = entry.absolute;
    let validate = entry.validate;
    let deprecated = entry.deprecated;
    let public = entry.public;
    let response_override = entry.response_override;
    let responses = entry.responses.iter().map(|(status, body)| {
        let status = proc_macro2::Literal::u16_suffixed(*status);
        match body {
            Some(ty) => quote! {
                __responses.push(#krate::doc::json_response_doc::<#ty>(components, #status));
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

    let layer_value = if entry.layer.is_some() {
        quote!(::core::option::Option::Some(__roundhouse_route_layer))
    } else {
        quote!(::core::option::Option::None)
    };

    let layer = match &entry.layer {
        Some(expr) => quote! {
            fn __roundhouse_route_layer(
                method_router: #krate::axum::routing::MethodRouter,
            ) -> #krate::axum::routing::MethodRouter {
                method_router.layer(#expr)
            }
        },
        None => quote! {},
    };

    quote! {
        #(#attrs)*
        #[allow(non_upper_case_globals)]
        const _: () = {
            const _: () = #krate::prefix::assert_path_names(
                #path,
                <#marker_type as #krate::doc::HandlerMeta>::PATH_NAMES,
            );

            fn __roundhouse_route()
                -> ::std::boxed::Box<dyn ::core::any::Any + ::core::marker::Send + ::core::marker::Sync>
            {
                let method_router: #krate::axum::routing::MethodRouter<
                    <#marker_type as #krate::doc::HandlerMeta>::State,
                > = #krate::axum::routing::#method_ident(#handler);
                ::std::boxed::Box::new(method_router)
            }

            #layer

            fn __roundhouse_responses(
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) -> ::std::vec::Vec<#krate::doc::ResponseDoc> {
                let mut __responses = ::std::vec::Vec::new();
                #(#responses)*
                __responses
            }

            #krate::inventory::submit! {
                #krate::registry::RouteRecord::__new(
                    module_path!(),
                    file!(),
                    #line,
                    #method,
                    #path,
                    #operation_id,
                    #summary,
                    #description,
                    #tags,
                    #security,
                    #hidden,
                    #absolute,
                    #validate,
                    #deprecated,
                    #public,
                    #response_override,
                    __roundhouse_responses,
                    || #krate::doc::handler_state::<#marker_type>(),
                    || ::core::any::type_name::<
                        <#marker_type as #krate::doc::HandlerMeta>::State,
                    >(),
                    __roundhouse_route,
                    #layer_value,
                    |components| <#marker_type as #krate::doc::HandlerMeta>::docs(&#marker_value, components),
                )
            }
        };
    }
}

fn option_lit(value: &Option<LitStr>) -> TokenStream {
    match value {
        Some(lit) => quote!(::core::option::Option::Some(#lit)),
        None => quote!(::core::option::Option::None),
    }
}
