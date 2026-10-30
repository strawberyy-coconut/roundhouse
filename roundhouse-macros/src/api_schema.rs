use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Expr, Fields, LitStr, Result, Type};

use crate::serde_shape::{
    SerdeContainer, container as serde_container, field as serde_field, field_name,
};

#[derive(Default, Clone)]
pub(crate) struct Constraints {
    min_length: Option<Expr>,
    max_length: Option<Expr>,
    pattern: Option<LitStr>,
    minimum: Option<Expr>,
    maximum: Option<Expr>,
    exclusive_minimum: Option<Expr>,
    exclusive_maximum: Option<Expr>,
    min_items: Option<Expr>,
    max_items: Option<Expr>,
    unique_items: bool,
    format: Option<LitStr>,
    example: Option<Expr>,
    validate_with: Option<syn::Path>,
}

#[derive(Default, Clone)]
pub(crate) struct SchemaAttrs {
    pub(crate) constraints: Constraints,
}

pub(crate) fn schema_attrs(attrs: &[syn::Attribute]) -> Result<SchemaAttrs> {
    let mut schema = SchemaAttrs::default();
    for attr in attrs {
        if attr.path().is_ident("roundhouse") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("unique_items") {
                    schema.constraints.unique_items = true;
                } else if meta.path.is_ident("validate_with") {
                    schema.constraints.validate_with = Some(meta.value()?.parse()?);
                } else {
                    return Err(meta.error(
                        "unknown `roundhouse` attribute; expected `unique_items` or `validate_with`",
                    ));
                }
                Ok(())
            })?;
        } else if attr.path().is_ident("schema") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("min_length") {
                    schema.constraints.min_length = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("max_length") {
                    schema.constraints.max_length = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("pattern") {
                    schema.constraints.pattern = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("minimum") {
                    schema.constraints.minimum = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("maximum") {
                    schema.constraints.maximum = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("exclusive_minimum") {
                    schema.constraints.exclusive_minimum = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("exclusive_maximum") {
                    schema.constraints.exclusive_maximum = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("min_items") {
                    schema.constraints.min_items = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("max_items") {
                    schema.constraints.max_items = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("format") {
                    schema.constraints.format = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("example") {
                    schema.constraints.example = Some(meta.value()?.parse()?);
                } else if meta.path.is_ident("skip_validation") {
                    return Err(meta.error(
                        "`#[schema(skip_validation)]` was removed: a constraint is either published \
                         and enforced, or not declared. Use `validate = false` on the routes! entry \
                         to opt the whole body out.",
                    ));
                } else if meta.path.is_ident("unique_items") {
                    return Err(meta.error(
                        "`unique_items` is a roundhouse constraint; write \
                         `#[roundhouse(unique_items)]` instead",
                    ));
                } else if meta.path.is_ident("validate_with") {
                    return Err(meta.error(
                        "`validate_with` is a roundhouse constraint; write \
                         `#[roundhouse(validate_with = path)]` instead",
                    ));
                } else if meta.path.is_ident("name") {
                    return Err(meta.error(
                        "component names come from utoipa; use `#[schema(as = path::Name)]` \
                         instead of `#[schema(name = ...)]`",
                    ));
                } else if meta.path.is_ident("via") {
                    return Err(meta.error(
                        "`via = utoipa` is no longer needed: derive `utoipa::ToSchema` alongside \
                         `ApiSchema`",
                    ));
                } else if meta.input.peek(syn::token::Paren) {
                    // A utoipa attribute with a nested group, such as `extensions(...)`.
                    let content;
                    syn::parenthesized!(content in meta.input);
                    let _ = content;
                } else if meta.input.peek(syn::Token![=]) {
                    // An attribute owned by utoipa (`as`, `value_type`, `example`, ...).
                    // Consume its value so `parse_nested_meta` can move to the next attribute.
                    let _: Expr = meta.value()?.parse()?;
                }
                Ok(())
            })?;
        }
    }
    Ok(schema)
}

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let krate = crate::util::krate();
    let ident = input.ident.clone();
    let container_serde = serde_container(&input.attrs)?;
    let _container_schema = schema_attrs(&input.attrs)?;

    let mut generics = input.generics.clone();
    for param in &mut generics.params {
        if let syn::GenericParam::Type(param) = param {
            param.bounds.push(syn::parse_quote!(#krate::doc::ApiSchema));
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

    let component_record = quote! {
        #krate::inventory::submit! {
            #krate::registry::ComponentTypeRecord::__new(
                || ::core::any::TypeId::of::<#ident #ty_generics>(),
                || <#ident #ty_generics as #krate::utoipa::ToSchema>::name().into_owned(),
            )
        }
    };

    Ok(quote! {
        #component_record

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
    let krate = crate::util::krate();
    let named = named_fields(fields)?;
    let mut register_calls = Vec::new();
    let mut patches = Vec::new();
    let mut validations = Vec::new();

    for (index, field) in named.iter().enumerate() {
        let ty = &field.ty;
        let field_serde = serde_field(&field.attrs)?;
        let field_schema = schema_attrs(&field.attrs)?;
        register_calls.push(quote! {
            <#ty as #krate::doc::ApiSchema>::register(components);
        });
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
    let krate = crate::util::krate();
    let mut register_calls = Vec::new();
    let mut patches = Vec::new();
    let mut validation_arms = Vec::new();

    for (variant_index, variant) in data.variants.iter().enumerate() {
        for field in variant.fields.iter() {
            let ty = &field.ty;
            register_calls.push(quote! {
                <#ty as #krate::doc::ApiSchema>::register(components);
            });
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
                validations.push(quote! {
                    <#ty as #krate::doc::ApiSchema>::validate_at(
                        #binding,
                        #pointer,
                        errors,
                    );
                });
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

pub(crate) fn field_schema_expr(ty: &Type, constraints: &Constraints) -> TokenStream {
    let krate = crate::util::krate();
    let mut calls = TokenStream::new();
    if let Some(value) = &constraints.min_length {
        calls.extend(
            quote! { schema = #krate::doc::schema::with_min_length(schema, (#value) as usize); },
        );
    }
    if let Some(value) = &constraints.max_length {
        calls.extend(
            quote! { schema = #krate::doc::schema::with_max_length(schema, (#value) as usize); },
        );
    }
    if let Some(value) = &constraints.pattern {
        calls.extend(quote! { schema = #krate::doc::schema::with_pattern(schema, #value); });
    }
    if let Some(value) = &constraints.minimum {
        calls.extend(
            quote! { schema = #krate::doc::schema::with_minimum(schema, (#value) as f64); },
        );
    }
    if let Some(value) = &constraints.maximum {
        calls.extend(
            quote! { schema = #krate::doc::schema::with_maximum(schema, (#value) as f64); },
        );
    }
    if let Some(value) = &constraints.exclusive_minimum {
        calls.extend(quote! { schema = #krate::doc::schema::with_exclusive_minimum(schema, (#value) as f64); });
    }
    if let Some(value) = &constraints.exclusive_maximum {
        calls.extend(quote! { schema = #krate::doc::schema::with_exclusive_maximum(schema, (#value) as f64); });
    }
    if let Some(value) = &constraints.min_items {
        calls.extend(
            quote! { schema = #krate::doc::schema::with_min_items(schema, (#value) as usize); },
        );
    }
    if let Some(value) = &constraints.max_items {
        calls.extend(
            quote! { schema = #krate::doc::schema::with_max_items(schema, (#value) as usize); },
        );
    }
    if constraints.unique_items {
        calls.extend(quote! { schema = #krate::doc::schema::with_unique_items(schema); });
    }
    if let Some(value) = &constraints.format {
        calls.extend(quote! { schema = #krate::doc::schema::with_format(schema, #value); });
    }
    if let Some(value) = &constraints.example {
        calls.extend(quote! {
            schema = #krate::doc::schema::with_example(
                schema,
                #krate::serde_json::json!(#value),
            );
        });
    }
    if constraints.validate_with.is_some() {
        calls.extend(quote! {
            schema = #krate::doc::schema::with_extension(
                schema,
                "x-validate-with",
                #krate::serde_json::json!(true),
            );
        });
    }

    let base = base_schema_expr(ty);
    if calls.is_empty() {
        base
    } else {
        quote! {
            {
                let mut schema = #base;
                #calls
                schema
            }
        }
    }
}

fn base_schema_expr(ty: &Type) -> TokenStream {
    let krate = crate::util::krate();
    match crate::util::is_option(ty) {
        Some(inner) if is_vec_u8(&inner) => quote! {
            #krate::doc::schema::nullable(#krate::doc::schema::bytes_schema())
        },
        Some(inner) => {
            let inner = base_schema_expr(&inner);
            quote! {
                #krate::doc::schema::nullable(#inner)
            }
        }
        None if is_vec_u8(ty) => quote! {
            #krate::doc::schema::bytes_schema()
        },
        None => quote!(<#ty as #krate::doc::ApiSchema>::schema()),
    }
}

fn is_vec_u8(ty: &Type) -> bool {
    crate::util::is_vec(ty)
        .and_then(|inner| crate::util::outer_ident(&inner))
        .as_deref()
        == Some("u8")
}

pub(crate) fn field_validation(
    index: usize,
    field_access: TokenStream,
    ty: &Type,
    property_name: &str,
    constraints: &Constraints,
    flatten: bool,
    pointer_base: TokenStream,
) -> TokenStream {
    let krate = crate::util::krate();
    let pointer_ident = format_ident!("__roundhouse_pointer_{index}");
    let pointer = LitStr::new(property_name, proc_macro2::Span::call_site());
    let pointer_init = if flatten {
        quote!(#pointer_base.to_string())
    } else {
        quote!(#krate::validate::pointer(#pointer_base, #pointer))
    };

    let optional = crate::util::is_option(ty).is_some();

    let custom = match &constraints.validate_with {
        Some(path) => quote! {
            if let ::core::result::Result::Err(mut custom) = #path(value) {
                errors.append_at(&#pointer_ident, custom);
            }
        },
        None => quote!(),
    };

    if optional {
        quote! {
            let #pointer_ident = #pointer_init;
            if let Some(value) = &#field_access {
                #custom
                #krate::doc::ApiSchema::validate_at(
                    value,
                    &#pointer_ident,
                    errors,
                );
            }
        }
    } else {
        quote! {
            let #pointer_ident = #pointer_init;
            {
                let value = &#field_access;
                #custom
            }
            #krate::doc::ApiSchema::validate_at(
                &#field_access,
                &#pointer_ident,
                errors,
            );
        }
    }
}
