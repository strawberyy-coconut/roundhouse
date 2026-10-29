use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, LitStr, Result};

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "PathParams does not support generic structs in v0.1",
        ));
    }
    let ident = input.ident;
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

    for (index, field) in fields.named.iter().enumerate() {
        let Some(field_ident) = &field.ident else {
            continue;
        };
        let mut name = field_ident.to_string();
        if let Some(stripped) = name.strip_prefix("r#") {
            name = stripped.to_string();
        }
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
        params.push(quote! {
            ::roundhouse::doc::Param::new(
                #name_lit,
                ::roundhouse::doc::ParamIn::Path,
                #schema_expr,
            )
        });
    }

    Ok(quote! {
        impl ::roundhouse::doc::PathNames for #ident {
            const NAMES: &'static [&'static str] = &[#(#names),*];

            fn params() -> ::std::vec::Vec<::roundhouse::doc::Param> {
                ::std::vec![#(#params),*]
            }
        }

        impl ::roundhouse::doc::PathValidate for #ident {
            fn validate_at(
                &self,
                pointer: &str,
                errors: &mut ::roundhouse::validate::ValidationErrors,
            ) {
                #(#validations)*
            }
        }
    })
}
