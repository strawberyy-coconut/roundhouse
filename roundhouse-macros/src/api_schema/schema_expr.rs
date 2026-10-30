use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{LitStr, Type};

use super::attrs::Constraints;

pub(super) fn validate_expr(ty: &Type, value: TokenStream, pointer: TokenStream) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::doc::ProbeValidateFor as _;
        (&#krate::doc::ValidateFor::<#ty>::new())
            .probe_validate_for(#value, &#pointer, errors)
    })
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
        None => {
            let krate = crate::util::krate();
            quote!({
                use #krate::doc::ProbeSchemaFor as _;
                (&#krate::doc::SchemaFor::<#ty>::new()).probe_schema()
            })
        }
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
        let inner = crate::util::is_option(ty).unwrap_or_else(|| ty.clone());
        let validate = validate_expr(&inner, quote!(value), quote!(#pointer_ident));
        quote! {
            let #pointer_ident = #pointer_init;
            if let Some(value) = &#field_access {
                #custom
                #validate
            }
        }
    } else {
        let validate = validate_expr(ty, quote!(&#field_access), quote!(#pointer_ident));
        quote! {
            let #pointer_ident = #pointer_init;
            {
                let value = &#field_access;
                #custom
            }
            #validate
        }
    }
}
