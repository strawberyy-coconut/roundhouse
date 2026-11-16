//! `api!` parsing and expansion: the build-time contract configuration block.

use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Expr, Ident, LitInt, LitStr, Path, Result, Token, Type, braced, bracketed};

use crate::util;

pub fn expand(input: ApiInput) -> TokenStream {
    let krate = util::krate();
    let state = input
        .state
        .unwrap_or_else(|| syn::parse_quote!(crate::__RoundhouseAppState));
    let title = input.title.expect("title is required");
    let version = input.version.expect("version is required");
    let root = input.root.expect("root is required");
    let servers = input.servers;
    let registers = input.register;
    let defaults = input
        .response_defaults
        .iter()
        .map(|(status, text)| {
            let status = proc_macro2::Literal::u16_suffixed(*status);
            quote!((#status, #text))
        })
        .collect::<Vec<_>>();

    quote! {
        const _: () = assert!(
            #krate::__private::is_crate_root(module_path!()),
            "roundhouse::api! must be called at the crate root, not in a nested module",
        );

        #krate::__roundhouse_contract_code! {
            #[doc(hidden)]
            pub fn __roundhouse_contract() -> #krate::Contract<#state> {
                #krate::Contract::new(#krate::openapi::Info::new(#title, #version))
                    .include(&#root)
                    #(
                        .server(#krate::__private::utoipa::openapi::Server::new(#servers))
                    )*
                    #(.register::<#registers>())*
                    .response_defaults(&[#(#defaults),*])
            }
        }
    }
}

pub struct ApiInput {
    state: Option<Type>,
    title: Option<Expr>,
    version: Option<Expr>,
    root: Option<Path>,
    servers: Vec<Expr>,
    register: Vec<Type>,
    response_defaults: Vec<(u16, String)>,
}

impl Parse for ApiInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut api = Self {
            state: None,
            title: None,
            version: None,
            root: None,
            servers: Vec::new(),
            register: Vec::new(),
            response_defaults: Vec::new(),
        };

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "state" => {
                    if api.state.is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate `state` option"));
                    }
                    api.state = Some(input.parse()?);
                }
                "title" => {
                    if api.title.is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate `title` option"));
                    }
                    api.title = Some(input.parse()?);
                }
                "version" => {
                    if api.version.is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate `version` option"));
                    }
                    api.version = Some(input.parse()?);
                }
                "root" => {
                    if api.root.is_some() {
                        return Err(syn::Error::new(key.span(), "duplicate `root` option"));
                    }
                    api.root = Some(input.parse()?);
                }
                "servers" => {
                    let content;
                    bracketed!(content in input);
                    api.servers = Punctuated::<Expr, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
                }
                "register" => {
                    let content;
                    bracketed!(content in input);
                    api.register = Punctuated::<Type, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect();
                }
                "response_defaults" => {
                    let content;
                    braced!(content in input);
                    while !content.is_empty() {
                        let status: LitInt = content.parse()?;
                        let status = status.base10_parse::<u16>()?;
                        content.parse::<Token![=>]>()?;
                        let text: LitStr = content.parse()?;
                        api.response_defaults.push((status, text.value()));
                        if content.peek(Token![,]) {
                            content.parse::<Token![,]>()?;
                        } else {
                            break;
                        }
                    }
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown `api!` option `{other}`"),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else {
                break;
            }
        }

        if api.title.is_none() {
            return Err(input.error("`api!` needs `title = \"...\"`"));
        }
        if api.version.is_none() {
            return Err(input.error("`api!` needs `version = ...`"));
        }
        if api.root.is_none() {
            return Err(input.error("`api!` needs `root = module::MODULE`"));
        }
        Ok(api)
    }
}
