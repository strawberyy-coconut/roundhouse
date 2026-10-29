use proc_macro2::TokenStream;
use quote::quote;
use syn::parse::{Parse, ParseStream};
use syn::{Expr, Ident, LitStr, Result, Token, Type};

use crate::util::{auth_slice, parse_security_types, string_slice};

pub struct MountInput {
    prefix: Option<LitStr>,
    tags: Vec<LitStr>,
    security: Vec<Type>,
    layer: Option<Expr>,
    layer_mode: LayerMode,
    hidden: bool,
    doc: Option<Expr>,
}

#[derive(Clone, Copy)]
enum LayerMode {
    Full,
    Route,
}

impl Parse for MountInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut prefix = None;
        if input.peek(LitStr) {
            prefix = Some(input.parse()?);
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        } else if input.peek(Ident) {
            // `mount!(None)` is accepted as the explicit spelling of a group mount.
            let fork = input.fork();
            let ident: Ident = fork.parse()?;
            if ident == "None" {
                input.parse::<Ident>()?;
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
            }
        }

        let mut tags = Vec::new();
        let mut security = Vec::new();
        let mut layer = None;
        let mut layer_mode = LayerMode::Full;
        let mut hidden = false;
        let mut doc = None;

        while !input.is_empty() {
            let key: Ident = input.parse()?;
            match key.to_string().as_str() {
                "tags" => {
                    input.parse::<Token![=]>()?;
                    tags = parse_string_list(input)?;
                }
                "security" => {
                    input.parse::<Token![=]>()?;
                    security = parse_security_types(input)?;
                }
                "layer" => {
                    input.parse::<Token![=]>()?;
                    layer = Some(input.parse()?);
                }
                "layer_mode" => {
                    input.parse::<Token![=]>()?;
                    let mode: Ident = input.parse()?;
                    layer_mode = match mode.to_string().as_str() {
                        "full" => LayerMode::Full,
                        "route" => LayerMode::Route,
                        other => {
                            return Err(syn::Error::new(
                                mode.span(),
                                format!("expected `full` or `route`, found `{other}`"),
                            ));
                        }
                    };
                }
                "hidden" => hidden = true,
                "doc" => {
                    input.parse::<Token![=]>()?;
                    doc = Some(input.parse()?);
                }
                // Stateless routes are inferred now; a `state` key here would be silently
                // ignored, which is exactly the kind of drift the project rejects.
                "state" => {
                    return Err(syn::Error::new(
                        key.span(),
                        "mount! no longer takes `state`: routes infer their state from \
                         `State<T>`/`Auth<A>` arguments",
                    ));
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown mount! attribute `{other}`"),
                    ));
                }
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            } else {
                break;
            }
        }

        Ok(Self {
            prefix,
            tags,
            security,
            layer,
            layer_mode,
            hidden,
            doc,
        })
    }
}

fn parse_string_list(input: ParseStream<'_>) -> Result<Vec<LitStr>> {
    let content;
    syn::bracketed!(content in input);
    let values = content.parse_terminated(|input| input.parse::<LitStr>(), Token![,])?;
    Ok(values.into_iter().collect())
}

pub fn expand(input: MountInput) -> TokenStream {
    let krate = crate::util::krate();
    let prefix = input
        .prefix
        .as_ref()
        .map(|lit| lit.value())
        .unwrap_or_default();
    let prefix_lit = LitStr::new(&prefix, proc_macro2::Span::call_site());
    let tags = string_slice(&input.tags);
    let security = auth_slice(&input.security);
    let hidden = input.hidden;
    let line = proc_macro2::Span::call_site().start().line as u32;

    let doc_value = match &input.doc {
        Some(expr) => {
            quote! {
                fn __roundhouse_mount_doc() -> #krate::doc::layer::LayerDoc {
                    #expr
                }
                const __ROUNDHOUSE_DOC: ::core::option::Option<
                    fn() -> #krate::doc::layer::LayerDoc,
                > = ::core::option::Option::Some(__roundhouse_mount_doc);
            }
        }
        None => quote! {
            const __ROUNDHOUSE_DOC: ::core::option::Option<
                fn() -> #krate::doc::layer::LayerDoc,
            > = ::core::option::Option::None;
        },
    };

    let layer_value = if input.layer.is_some() {
        quote!(::core::option::Option::Some(__roundhouse_mount_layer))
    } else {
        quote!(::core::option::Option::None)
    };

    let layer = match &input.layer {
        Some(expr) => {
            let body = match input.layer_mode {
                LayerMode::Full => quote! { router.layer(#expr) },
                LayerMode::Route => quote! { router.route_layer(#expr) },
            };
            quote! {
                fn __roundhouse_mount_layer(
                    router: #krate::axum::Router,
                ) -> #krate::axum::Router {
                    #body
                }
            }
        }
        None => quote! {},
    };

    quote! {
        // A second `mount!` in this module collides with this handle: the right compile error.
        /// The module's route table, for `App::scope`, `App::include` and `App::mount`.
        pub const MOUNT: #krate::MountHandle =
            #krate::MountHandle::new(module_path!(), #prefix_lit);

        #[allow(non_upper_case_globals)]
        const _: () = {
            #layer
            #doc_value

            #krate::inventory::submit! {
                #krate::registry::MountRecord::__new(
                    module_path!(),
                    file!(),
                    #line,
                    #prefix_lit,
                    #tags,
                    #security,
                    #hidden,
                    #layer_value,
                    __ROUNDHOUSE_DOC,
                )
            }
        };
    }
}
