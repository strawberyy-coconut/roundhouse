mod api_response;
mod api_schema;
mod handler;
mod mount;
mod multipart;
mod path_params;
mod query_params;
mod routes;
mod util;

use proc_macro::TokenStream;
use syn::{DeriveInput, ItemFn, parse_macro_input};

#[proc_macro]
pub fn mount(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as mount::MountInput);
    mount::expand(input).into()
}

#[proc_macro]
pub fn routes(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as routes::RoutesInput);
    routes::expand(input).into()
}

#[proc_macro_attribute]
pub fn handler(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return syn::Error::new(
            proc_macro2::Span::call_site(),
            "`#[handler]` takes no arguments; routing metadata lives in `routes!`",
        )
        .to_compile_error()
        .into();
    }
    let item = parse_macro_input!(item as ItemFn);
    handler::expand(item).into()
}

#[proc_macro_derive(ApiSchema, attributes(schema, serde))]
pub fn derive_api_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    api_schema::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(QueryParams, attributes(serde, schema))]
pub fn derive_query_params(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    query_params::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(Multipart, attributes(multipart, file, serde))]
pub fn derive_multipart(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    multipart::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(PathParams, attributes(serde, schema))]
pub fn derive_path_params(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    path_params::expand(input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(
    ApiResponse,
    attributes(status, response, body, header, location, from)
)]
pub fn derive_api_response(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    api_response::expand(input, false)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

#[proc_macro_derive(ApiError, attributes(status, response, body, header, location, from))]
pub fn derive_api_error(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    api_response::expand(input, true)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
