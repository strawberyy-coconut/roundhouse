//! Parsing and expansion for roundhouse's proc macros.
//!
//! This is a normal library crate so the parser and expander can be unit-tested without a proc
//! macro harness. [`roundhouse-macros`](https://docs.rs/roundhouse-macros) is the thin
//! `proc-macro = true` entry point.

use proc_macro2::TokenStream;
use syn::{DeriveInput, Item, ItemFn};

mod api_response;
mod api_schema;
mod doc_tags;
mod handler;
mod multipart;
mod path_params;
mod query_params;
mod routes;
mod serde_shape;
mod util;

pub fn expand_routes(input: TokenStream) -> TokenStream {
    match syn::parse2::<routes::RoutesInput>(input) {
        Ok(input) => routes::expand(input),
        Err(error) => error.to_compile_error(),
    }
}

pub fn expand_handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    let args = if attr.is_empty() {
        handler::HandlerArgs::default()
    } else {
        match syn::parse2::<handler::HandlerArgs>(attr) {
            Ok(args) => args,
            Err(error) => return error.to_compile_error(),
        }
    };
    match syn::parse2::<ItemFn>(item) {
        Ok(item) => handler::expand(item, args),
        Err(error) => error.to_compile_error(),
    }
}

pub fn expand_schema(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "`#[roundhouse::schema]` takes no arguments; schema options belong in \
             `#[schema(..)]`, `#[serde(..)]` or `#[roundhouse(..)]`",
        )
        .to_compile_error();
    }
    let item = match syn::parse2::<Item>(item) {
        Ok(item) => item,
        Err(error) => return error.to_compile_error(),
    };
    let attrs = match &item {
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        other => {
            return syn::Error::new_spanned(
                other,
                "`#[roundhouse::schema]` can only be applied to a struct or enum",
            )
            .to_compile_error();
        }
    };
    if has_derive(attrs, "ToSchema") || has_derive(attrs, "ApiSchema") {
        let message = "`#[roundhouse::schema]` already derives `ToSchema` and `ApiSchema`; \
                       remove the separate derive";
        if let Some(attr) = attrs.iter().find(|attr| attr.path().is_ident("derive")) {
            return syn::Error::new_spanned(attr, message).to_compile_error();
        }
        return syn::Error::new(proc_macro2::Span::call_site(), message).to_compile_error();
    }
    let krate = util::krate();
    quote::quote! {
        #[derive(#krate::ToSchema, #krate::ApiSchema)]
        #item
    }
}

pub fn expand_api_schema(input: TokenStream) -> TokenStream {
    parse_derive(input, api_schema::expand)
}

pub fn expand_query_params(input: TokenStream) -> TokenStream {
    parse_derive(input, query_params::expand)
}

pub fn expand_path_params(input: TokenStream) -> TokenStream {
    parse_derive(input, path_params::expand)
}

pub fn expand_multipart(input: TokenStream) -> TokenStream {
    parse_derive(input, multipart::expand)
}

pub fn expand_api_response(input: TokenStream, error_only: bool) -> TokenStream {
    parse_derive(input, |input| api_response::expand(input, error_only))
}

fn parse_derive(
    input: TokenStream,
    expand: impl FnOnce(DeriveInput) -> syn::Result<TokenStream>,
) -> TokenStream {
    match syn::parse2::<DeriveInput>(input) {
        Ok(input) => expand(input).unwrap_or_else(syn::Error::into_compile_error),
        Err(error) => error.to_compile_error(),
    }
}

fn has_derive(attrs: &[syn::Attribute], name: &str) -> bool {
    attrs
        .iter()
        .filter(|attr| attr.path().is_ident("derive"))
        .filter_map(|attr| {
            attr.parse_args_with(
                syn::punctuated::Punctuated::<syn::Path, syn::Token![,]>::parse_terminated,
            )
            .ok()
        })
        .flatten()
        .any(|path| {
            path.segments
                .last()
                .is_some_and(|segment| segment.ident == name)
        })
}
