use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, Result};

use crate::serde_shape::{container as serde_container, field as serde_field, field_name};
use crate::util::{is_option, is_vec};

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let krate = crate::util::krate();
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "QueryParams does not support generic structs in v0.2",
        ));
    }
    let ident = input.ident;
    let container = serde_container(&input.attrs)?;
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
    let mut value_inserts = Vec::new();
    for (index, field) in fields.named.iter().enumerate() {
        let Some(field_ident) = &field.ident else {
            continue;
        };
        let field_serde = serde_field(&field.attrs)?;
        if field_serde.skipped_on_input() {
            continue;
        }
        let ty = &field.ty;
        let schema = crate::api_schema::schema_attrs(&field.attrs)?;

        if field_serde.flatten {
            // Serde flattens the inner keys into this struct's own namespace. The inner
            // `QueryParams` impl owns both its parameter list and its validation pass.
            params.push(quote! {
                params.extend(<#ty as #krate::doc::QueryParams>::params(components));
            });
            validations.push(flatten_validation(field_ident, ty));
            value_inserts.push(flatten_value(field_ident, ty));
            continue;
        }

        let name = field_name(field, &field_serde, container.rename_all)?;
        let name_lit = LitStr::new(&name, field_ident.span());
        let required = is_option(ty).is_none() && !field_serde.default && !container.default;
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
        value_inserts.push(quote! {
            map.insert(
                #name_lit.to_string(),
                #krate::serde_json::to_value(&self.#field_ident)
                    .unwrap_or(#krate::serde_json::Value::Null),
            );
        });
        let array = is_vec(ty).is_some()
            || is_option(ty)
                .as_ref()
                .is_some_and(|inner| is_vec(inner).is_some());
        let style = if array {
            quote! {
                .style(#krate::utoipa::openapi::path::ParameterStyle::Form)
                .explode(true)
            }
        } else {
            quote!()
        };
        params.push(quote! {
            params.push(
                #krate::doc::Param::new(
                    #name_lit,
                    #krate::doc::ParamIn::Query,
                    #schema_expr,
                )
                .required(#required)
                #style
            );
        });
    }

    let fn_name = quote::format_ident!("__roundhouse_query_params_{}", ident);
    Ok(quote! {
        #[allow(non_snake_case)]
        fn #fn_name(
            components: &mut #krate::utoipa::openapi::schema::Components,
        ) -> ::std::vec::Vec<#krate::doc::Param> {
            let mut params = ::std::vec::Vec::new();
            #(#params)*
            params
        }

        impl #krate::doc::QueryParams for #ident {
            fn params(
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) -> ::std::vec::Vec<#krate::doc::Param> {
                #fn_name(components)
            }

            fn validate_at(
                &self,
                pointer: &str,
                errors: &mut #krate::validate::ValidationErrors,
            ) {
                #(#validations)*
            }
        }

        impl #krate::doc::PartsSchema for #ident {
            fn parts_schema(
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) -> #krate::utoipa::openapi::RefOr<#krate::utoipa::openapi::schema::Schema> {
                #krate::doc::schema::object_from_params(
                    <Self as #krate::doc::QueryParams>::params(components),
                )
            }
        }

        impl #krate::doc::PartsValue for #ident {
            fn parts_value(&self) -> #krate::serde_json::Value {
                let mut map = #krate::serde_json::Map::new();
                #(#value_inserts)*
                #krate::serde_json::Value::Object(map)
            }
        }

    })
}

fn flatten_value(field_ident: &syn::Ident, ty: &syn::Type) -> TokenStream {
    let krate = crate::util::krate();
    if is_option(ty).is_some() {
        quote! {
            if let ::core::option::Option::Some(value) = &self.#field_ident {
                if let #krate::serde_json::Value::Object(inner) =
                    #krate::doc::PartsValue::parts_value(value)
                {
                    map.extend(inner);
                }
            }
        }
    } else {
        quote! {
            if let #krate::serde_json::Value::Object(inner) =
                #krate::doc::PartsValue::parts_value(&self.#field_ident)
            {
                map.extend(inner);
            }
        }
    }
}

fn flatten_validation(field_ident: &syn::Ident, ty: &syn::Type) -> TokenStream {
    let krate = crate::util::krate();
    if is_option(ty).is_some() {
        quote! {
            if let ::core::option::Option::Some(value) = &self.#field_ident {
                #krate::doc::QueryParams::validate_at(value, pointer, errors);
            }
        }
    } else {
        quote! {
            #krate::doc::QueryParams::validate_at(&self.#field_ident, pointer, errors);
        }
    }
}
