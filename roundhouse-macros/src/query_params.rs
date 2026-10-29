use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, Result};

use crate::util::is_option;

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "QueryParams does not support generic structs in v0.1",
        ));
    }
    let ident = input.ident;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &ident,
            "QueryParams can only be derived for a struct",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "QueryParams requires named fields",
        ));
    };

    let mut params = Vec::new();
    let mut validations = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let Some(field_ident) = &field.ident else {
            continue;
        };
        let mut name = field_ident.to_string();
        if let Some(stripped) = name.strip_prefix("r#") {
            name = stripped.to_string();
        }
        // Honor `#[serde(rename = "...")]` when present.
        for attr in &field.attrs {
            if attr.path().is_ident("serde") {
                let _ = attr.parse_nested_meta(|meta| {
                    if meta.path.is_ident("rename") {
                        let value: LitStr = meta.value()?.parse()?;
                        name = value.value();
                    }
                    Ok(())
                });
            }
        }
        let name_lit = LitStr::new(&name, field_ident.span());
        let ty = &field.ty;
        let required = is_option(ty).is_none();
        let schema = crate::api_schema::schema_attrs(&field.attrs)?;
        let schema_expr = crate::api_schema::field_schema_expr(ty, &schema.constraints);
        validations.push(crate::api_schema::field_validation(
            index,
            quote!(self.#field_ident),
            ty,
            &name,
            &schema.constraints,
            false,
            quote!(pointer),
        ));
        params.push(quote! {
            {
                <#ty as ::roundhouse::doc::ApiSchema>::register(components);
                ::roundhouse::doc::Param::new(
                    #name_lit,
                    ::roundhouse::doc::ParamIn::Query,
                    #schema_expr,
                ).required(#required)
            }
        });
    }

    let fn_name = quote::format_ident!("__roundhouse_query_params_{}", ident);
    Ok(quote! {
        #[allow(non_snake_case)]
        fn #fn_name(
            components: &mut ::roundhouse::utoipa::openapi::schema::Components,
        ) -> ::std::vec::Vec<::roundhouse::doc::Param> {
            ::std::vec![#(#params),*]
        }

        impl ::roundhouse::doc::QueryParams for #ident {
            fn params(
                components: &mut ::roundhouse::utoipa::openapi::schema::Components,
            ) -> ::std::vec::Vec<::roundhouse::doc::Param> {
                #fn_name(components)
            }

            fn validate_at(
                &self,
                pointer: &str,
                errors: &mut ::roundhouse::validate::ValidationErrors,
            ) {
                #(#validations)*
            }
        }

        ::roundhouse::inventory::submit! {
            ::roundhouse::registry::QueryDocRecord::__new(
                || ::core::any::type_name::<#ident>(),
                #fn_name,
            )
        }
    })
}
