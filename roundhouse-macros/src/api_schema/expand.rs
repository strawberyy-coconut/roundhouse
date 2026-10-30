use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields, LitStr, Result, Type};

use crate::serde_shape::{
    SerdeContainer, container as serde_container, field as serde_field, field_name,
};

use super::attrs::{Constraints, schema_attrs};
use super::schema_expr::{field_validation, validate_expr};

fn register_expr(ty: &Type) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::doc::ProbeSchemaFor as _;
        (&#krate::doc::SchemaFor::<#ty>::new()).probe_register(components)
    })
}
pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let krate = crate::util::krate();
    let ident = input.ident.clone();
    let container_serde = serde_container(&input.attrs)?;
    let _container_schema = schema_attrs(&input.attrs)?;

    let mut generics = input.generics.clone();
    for param in &mut generics.params {
        if let syn::GenericParam::Type(param) = param {
            param
                .bounds
                .push(syn::parse_quote!(#krate::utoipa::ToSchema));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (register_calls, patches, validate_body) = match &input.data {
        Data::Struct(data) => struct_parts(&data.fields, &container_serde)?,
        Data::Enum(data) => enum_parts(data, &container_serde)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &ident,
                "ApiSchema cannot be derived for a union",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics #krate::doc::ApiSchema for #ident #ty_generics #where_clause {
            fn schema() -> #krate::utoipa::openapi::RefOr<
                #krate::utoipa::openapi::schema::Schema,
            > {
                #krate::doc::schema::r#ref(
                    <Self as #krate::utoipa::ToSchema>::name().into_owned(),
                )
            }

            fn register(
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) {
                let name = <Self as #krate::utoipa::ToSchema>::name().into_owned();
                if components.schemas.contains_key(&name) {
                    return;
                }
                // Break recursive registration: a field that points back at this type sees the
                // placeholder and stops before the real schema is inserted below.
                components.schemas.insert(
                    name.clone(),
                    #krate::doc::schema::any_value(),
                );
                #(#register_calls)*
                let mut schema = <Self as #krate::utoipa::PartialSchema>::schema();
                #(#patches)*
                components.schemas.insert(name, schema);
            }

            fn validate_at(
                &self,
                pointer: &str,
                errors: &mut #krate::validate::ValidationErrors,
            ) {
                #validate_body
            }
        }
    })
}

fn struct_parts(
    fields: &Fields,
    container: &SerdeContainer,
) -> Result<(Vec<TokenStream>, Vec<TokenStream>, TokenStream)> {
    let named = named_fields(fields)?;
    let mut register_calls = Vec::new();
    let mut patches = Vec::new();
    let mut validations = Vec::new();

    for (index, field) in named.iter().enumerate() {
        let ty = &field.ty;
        let field_serde = serde_field(&field.attrs)?;
        let field_schema = schema_attrs(&field.attrs)?;
        register_calls.push(register_expr(ty));
        if field_serde.hidden() {
            continue;
        }

        let property_name = field_name(field, &field_serde, container.rename_all)?;
        let access = match &field.ident {
            Some(ident) => quote!(self.#ident),
            None => quote!(self.0),
        };
        validations.push(field_validation(
            index,
            access,
            ty,
            &property_name,
            &field_schema.constraints,
            field_serde.flatten,
            quote!(pointer),
        ));

        if !field_serde.flatten {
            push_schema_patches(
                &mut patches,
                field,
                &property_name,
                &field_schema.constraints,
            );
        }
    }

    Ok((register_calls, patches, quote!(#(#validations)*)))
}

fn enum_parts(
    data: &syn::DataEnum,
    container: &SerdeContainer,
) -> Result<(Vec<TokenStream>, Vec<TokenStream>, TokenStream)> {
    let mut register_calls = Vec::new();
    let mut patches = Vec::new();
    let mut validation_arms = Vec::new();

    for (variant_index, variant) in data.variants.iter().enumerate() {
        for field in variant.fields.iter() {
            let ty = &field.ty;
            register_calls.push(register_expr(ty));
            if field.ident.is_some() {
                let field_serde = serde_field(&field.attrs)?;
                if field_serde.hidden() {
                    continue;
                }
                let field_schema = schema_attrs(&field.attrs)?;
                let property_name = field_name(field, &field_serde, container.rename_all)?;
                if !field_serde.flatten {
                    push_schema_patches(
                        &mut patches,
                        field,
                        &property_name,
                        &field_schema.constraints,
                    );
                }
            }
        }
        validation_arms.push(variant_validation(variant_index, variant, container)?);
    }

    let validate_body = if validation_arms.is_empty() {
        quote!()
    } else {
        quote! {
            match self {
                #(#validation_arms),*
            }
        }
    };
    Ok((register_calls, patches, validate_body))
}

fn push_schema_patches(
    patches: &mut Vec<TokenStream>,
    field: &syn::Field,
    property_name: &str,
    constraints: &Constraints,
) {
    let krate = crate::util::krate();
    let span = field
        .ident
        .as_ref()
        .map(|ident| ident.span())
        .unwrap_or_else(proc_macro2::Span::call_site);
    let name = LitStr::new(property_name, span);

    if constraints.unique_items {
        patches.push(quote! {
            #krate::doc::schema::patch_property(&mut schema, #name, &mut |property| {
                #krate::doc::schema::with_unique_items(property)
            });
        });
    }
    if constraints.validate_with.is_some() {
        patches.push(quote! {
            #krate::doc::schema::patch_property(&mut schema, #name, &mut |property| {
                #krate::doc::schema::with_extension(
                    property,
                    "x-validate-with",
                    #krate::serde_json::json!(true),
                )
            });
        });
    }
    if let Some(format) = &constraints.format {
        let format = LitStr::new(&format.value(), format.span());
        patches.push(quote! {
            #krate::doc::schema::patch_property(&mut schema, #name, &mut |property| {
                #krate::doc::schema::with_format(property, #format)
            });
        });
    }
}

fn variant_validation(
    variant_index: usize,
    variant: &syn::Variant,
    container: &SerdeContainer,
) -> Result<TokenStream> {
    let krate = crate::util::krate();
    let variant_ident = &variant.ident;
    let variant_serde = serde_field(&variant.attrs)?;
    let variant_name = match &variant_serde.rename {
        Some(name) => name.clone(),
        None => container
            .rename_all
            .map(|rule| rule.apply(&variant.ident.to_string()))
            .unwrap_or_else(|| variant.ident.to_string()),
    };

    let (pointer_base, pointer_setup) = if container.untagged {
        (quote!(pointer), quote!())
    } else if let Some(tag) = &container.tag {
        if let Some(content) = &container.content {
            let content_lit = LitStr::new(content, variant.ident.span());
            (
                quote!(__roundhouse_variant_pointer),
                quote! {
                    let __roundhouse_variant_pointer_store =
                        #krate::validate::pointer(pointer, #content_lit);
                    let __roundhouse_variant_pointer: &str =
                        &__roundhouse_variant_pointer_store;
                },
            )
        } else {
            let _ = tag;
            (quote!(pointer), quote!())
        }
    } else {
        let variant_lit = LitStr::new(&variant_name, variant.ident.span());
        (
            quote!(__roundhouse_variant_pointer),
            quote! {
                let __roundhouse_variant_pointer_store =
                    #krate::validate::pointer(pointer, #variant_lit);
                let __roundhouse_variant_pointer: &str =
                    &__roundhouse_variant_pointer_store;
            },
        )
    };

    let mut validations = Vec::new();
    let pattern = match &variant.fields {
        Fields::Unit => quote!(Self::#variant_ident),
        Fields::Named(named) => {
            let mut used = Vec::new();
            for (field_index, field) in named.named.iter().enumerate() {
                let Some(ident) = &field.ident else { continue };
                let field_serde = serde_field(&field.attrs)?;
                let field_schema = schema_attrs(&field.attrs)?;
                if field_serde.hidden() {
                    continue;
                }
                let property_name = field_name(field, &field_serde, container.rename_all)?;
                let ty = &field.ty;
                validations.push(field_validation(
                    variant_index * 100 + field_index,
                    quote!(#ident),
                    ty,
                    &property_name,
                    &field_schema.constraints,
                    false,
                    pointer_base.clone(),
                ));
                used.push(ident.clone());
            }
            if used.is_empty() {
                quote!(Self::#variant_ident { .. })
            } else {
                quote!(Self::#variant_ident { #(#used),*, .. })
            }
        }
        Fields::Unnamed(unnamed) => {
            let mut bindings = Vec::new();
            for (field_index, field) in unnamed.unnamed.iter().enumerate() {
                let binding = format_ident!("__field_{field_index}");
                let field_serde = serde_field(&field.attrs)?;
                if field_serde.hidden() {
                    bindings.push(quote!(_));
                    continue;
                }
                bindings.push(quote!(#binding));
                let ty = &field.ty;
                let pointer = if unnamed.unnamed.len() == 1 {
                    quote!(#pointer_base)
                } else {
                    quote! {
                        &#krate::validate::pointer(
                            #pointer_base,
                            &#field_index.to_string(),
                        )
                    }
                };
                validations.push(validate_expr(ty, quote!(#binding), pointer));
            }
            quote!(Self::#variant_ident(#(#bindings),*))
        }
    };

    if validations.is_empty() {
        Ok(quote!(#pattern => {}))
    } else {
        Ok(quote! {
            #pattern => {
                #pointer_setup
                #(#validations)*
            }
        })
    }
}

fn named_fields(fields: &Fields) -> Result<Vec<&syn::Field>> {
    match fields {
        Fields::Named(named) => Ok(named.named.iter().collect()),
        Fields::Unit => Ok(Vec::new()),
        Fields::Unnamed(_) => Err(syn::Error::new_spanned(
            fields,
            "ApiSchema requires named struct fields, or derive it on an enum",
        )),
    }
}
