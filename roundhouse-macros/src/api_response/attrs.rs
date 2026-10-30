use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Fields, Ident, LitInt, LitStr, Result, Token, Type};

use crate::util::is_option;

pub(super) struct VariantAttrs {
    pub(super) status: u16,
    pub(super) description: Option<String>,
    pub(super) problem: Option<String>,
}

impl Parse for VariantAttrs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut status = None;
        let mut description = None;
        let mut problem = None;

        if input.peek(LitInt) {
            let literal: LitInt = input.parse()?;
            status = Some(literal.base10_parse::<u16>()?);
        } else if input.peek(Ident) {
            let fork = input.fork();
            let key: Ident = fork.parse()?;
            if key == "status" && fork.peek(Token![=]) {
                let _: Ident = input.parse()?;
                input.parse::<Token![=]>()?;
                let literal: LitInt = input.parse()?;
                status = Some(literal.base10_parse::<u16>()?);
            }
        }

        while input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
            if input.is_empty() {
                break;
            }
            let key: Ident = input.parse()?;
            input.parse::<Token![=]>()?;
            match key.to_string().as_str() {
                "description" => {
                    let value: LitStr = input.parse()?;
                    description = Some(value.value());
                }
                "problem" => {
                    let value: LitStr = input.parse()?;
                    problem = Some(value.value());
                }
                "status" => {
                    let literal: LitInt = input.parse()?;
                    status = Some(literal.base10_parse::<u16>()?);
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!("unknown status attribute `{other}`"),
                    ));
                }
            }
        }

        let status = status.ok_or_else(|| {
            syn::Error::new(
                input.span(),
                "every variant needs #[status(..)]; a response you can produce but cannot describe is a bug",
            )
        })?;

        Ok(Self {
            status,
            description,
            problem,
        })
    }
}

#[derive(Clone)]
pub(super) struct FieldInfo {
    pub(super) access: TokenStream,
    pub(super) ty: Type,
    pub(super) optional: bool,
    pub(super) body: bool,
    pub(super) header: Option<(String, bool)>,
    pub(super) location: bool,
    pub(super) from: bool,
}

pub(super) fn collect_fields(fields: &Fields) -> Result<Vec<FieldInfo>> {
    let mut collected = Vec::new();
    match fields {
        Fields::Unit => {}
        Fields::Named(named) => {
            for field in &named.named {
                let ident = field
                    .ident
                    .as_ref()
                    .ok_or_else(|| syn::Error::new_spanned(field, "expected a named field"))?;
                collected.push(field_info(quote!(#ident), &field.ty, &field.attrs, false)?);
            }
        }
        Fields::Unnamed(unnamed) => {
            for (index, field) in unnamed.unnamed.iter().enumerate() {
                let access = format_ident!("field_{index}");
                collected.push(field_info(
                    quote!(#access),
                    &field.ty,
                    &field.attrs,
                    unnamed.unnamed.len() == 1,
                )?);
            }
        }
    }
    Ok(collected)
}

pub(super) fn field_info(
    access: TokenStream,
    ty: &Type,
    attrs: &[syn::Attribute],
    lone_unnamed: bool,
) -> Result<FieldInfo> {
    let mut body = false;
    let mut header = None;
    let mut location = false;
    let mut from = false;

    for attr in attrs {
        if attr.path().is_ident("body") {
            body = true;
        } else if attr.path().is_ident("location") {
            location = true;
        } else if attr.path().is_ident("from") {
            from = true;
        } else if attr.path().is_ident("header") {
            let (value, append) = attr.parse_args_with(|input: ParseStream<'_>| {
                let name: LitStr = input.parse()?;
                let mut append = false;
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                    let flag: Ident = input.parse()?;
                    if flag != "append" {
                        return Err(syn::Error::new(
                            flag.span(),
                            "expected `append` as the header option",
                        ));
                    }
                    append = true;
                }
                Ok((name.value(), append))
            })?;
            header = Some((value, append));
        }
    }

    if lone_unnamed && header.is_none() && !location && !from {
        body = true;
    }

    Ok(FieldInfo {
        access,
        ty: ty.clone(),
        optional: is_option(ty).is_some(),
        body,
        header,
        location,
        from,
    })
}

pub(super) fn bindings(fields: &Fields, variant: &Ident, infos: &[FieldInfo]) -> TokenStream {
    let used: Vec<bool> = infos
        .iter()
        .map(|info| info.body || info.header.is_some() || info.location || info.from)
        .collect();
    match fields {
        Fields::Unit => quote!(Self::#variant),
        Fields::Named(named) => {
            let idents: Vec<&Ident> = named
                .named
                .iter()
                .zip(used.iter())
                .filter(|(_, used)| **used)
                .filter_map(|(field, _)| field.ident.as_ref())
                .collect();
            if idents.is_empty() {
                quote!(Self::#variant { .. })
            } else {
                quote!(Self::#variant { #(#idents),*, .. })
            }
        }
        Fields::Unnamed(unnamed) => {
            let names: Vec<TokenStream> = (0..unnamed.unnamed.len())
                .map(|index| {
                    if used.get(index).copied().unwrap_or(false) {
                        let name = format_ident!("field_{index}");
                        quote!(#name)
                    } else {
                        quote!(_)
                    }
                })
                .collect();
            quote!(Self::#variant(#(#names),*))
        }
    }
}
