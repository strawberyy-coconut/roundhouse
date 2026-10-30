use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, Result};

use crate::serde_shape::{container as serde_container, field as serde_field, field_name};

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let krate = crate::util::krate();
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "PathParams does not support generic structs in v0.2",
        ));
    }
    let ident = input.ident;
    let container = serde_container(&input.attrs)?;
    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &ident,
            "PathParams can only be derived for a struct",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "PathParams requires named fields",
        ));
    };

    let mut names = Vec::new();
    let mut params = Vec::new();
    let mut validations = Vec::new();
    let mut value_inserts = Vec::new();

    for (index, field) in fields.named.iter().enumerate() {
        let Some(field_ident) = &field.ident else {
            continue;
        };
        let field_serde = serde_field(&field.attrs)?;
        if field_serde.skipped_on_input() {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(skip)]` is not valid on a path parameter: a path capture is always \
                 present and must be deserialized",
            ));
        }
        if field_serde.default || container.default {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(default)]` is not valid on a path parameter: a path capture is always \
                 present",
            ));
        }
        if field_serde.flatten {
            return Err(syn::Error::new_spanned(
                field,
                "`#[serde(flatten)]` is not valid on a path parameter: path captures are flat",
            ));
        }
        let name = field_name(field, &field_serde, container.rename_all)?;
        let name_lit = LitStr::new(&name, field_ident.span());
        let ty = &field.ty;
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
        names.push(name_lit.clone());
        value_inserts.push(quote! {
            map.insert(
                #name_lit.to_string(),
                #krate::serde_json::to_value(&self.#field_ident)
                    .unwrap_or(#krate::serde_json::Value::Null),
            );
        });
        params.push(quote! {
            #krate::doc::Param::new(
                #name_lit,
                #krate::doc::ParamIn::Path,
                #schema_expr,
            )
        });
    }

    Ok(quote! {
        impl #krate::doc::PathNames for #ident {
            const NAMES: &'static [&'static str] = &[#(#names),*];

            fn params() -> ::std::vec::Vec<#krate::doc::Param> {
                ::std::vec![#(#params),*]
            }
        }

        impl #krate::doc::PathValidate for #ident {
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
                _components: &mut #krate::utoipa::openapi::schema::Components,
            ) -> #krate::utoipa::openapi::RefOr<#krate::utoipa::openapi::schema::Schema> {
                #krate::doc::schema::object_from_params(
                    <Self as #krate::doc::PathNames>::params(),
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
