use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{LitStr, Type};

use super::attrs::Constraints;

pub(super) fn validate_expr(ty: &Type, value: TokenStream, pointer: TokenStream) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::__private::ProbeSchema as _;
        (&#krate::__private::SchemaProbe::<#ty>::new())
            .probe_validate_at(#value, &#pointer, errors)
    })
}

/// Register the components referenced by a field type.
///
/// Parameter derives call this before they publish a `$ref`; without it a query or path field
/// whose type derives `ApiSchema` documents a component that the document never contains.
pub(crate) fn register_expr(ty: &Type) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::__private::ProbeSchema as _;
        (&#krate::__private::SchemaProbe::<#ty>::new()).probe_register(context)
    })
}
pub(crate) fn field_schema_expr(ty: &Type, constraints: &Constraints) -> TokenStream {
    let krate = crate::util::krate();
    let mut calls = TokenStream::new();
    if let Some(value) = &constraints.min_length {
        calls.extend(length_constraint(
            &krate,
            value,
            "min_length",
            quote!(with_min_length),
        ));
    }
    if let Some(value) = &constraints.max_length {
        calls.extend(length_constraint(
            &krate,
            value,
            "max_length",
            quote!(with_max_length),
        ));
    }
    if let Some(value) = &constraints.pattern {
        calls.extend(quote! { schema = #krate::doc::schema::with_pattern(schema, #value); });
    }
    if let Some(value) = &constraints.minimum {
        let call = if is_integer_literal(value) {
            quote!(#krate::doc::schema::with_minimum_value(
                schema,
                #krate::serde_json::json!(#value),
            ))
        } else {
            quote!(#krate::doc::schema::with_minimum(schema, (#value) as f64))
        };
        calls.extend(quote! { schema = #call; });
    }
    if let Some(value) = &constraints.maximum {
        let call = if is_integer_literal(value) {
            quote!(#krate::doc::schema::with_maximum_value(
                schema,
                #krate::serde_json::json!(#value),
            ))
        } else {
            quote!(#krate::doc::schema::with_maximum(schema, (#value) as f64))
        };
        calls.extend(quote! { schema = #call; });
    }
    if let Some(value) = &constraints.exclusive_minimum {
        let call = if is_integer_literal(value) {
            quote!(#krate::doc::schema::with_exclusive_minimum_value(
                schema,
                #krate::serde_json::json!(#value),
            ))
        } else {
            quote!(#krate::doc::schema::with_exclusive_minimum(schema, (#value) as f64))
        };
        calls.extend(quote! { schema = #call; });
    }
    if let Some(value) = &constraints.exclusive_maximum {
        let call = if is_integer_literal(value) {
            quote!(#krate::doc::schema::with_exclusive_maximum_value(
                schema,
                #krate::serde_json::json!(#value),
            ))
        } else {
            quote!(#krate::doc::schema::with_exclusive_maximum(schema, (#value) as f64))
        };
        calls.extend(quote! { schema = #call; });
    }
    if let Some(value) = &constraints.min_items {
        calls.extend(length_constraint(
            &krate,
            value,
            "min_items",
            quote!(with_min_items),
        ));
    }
    if let Some(value) = &constraints.max_items {
        calls.extend(length_constraint(
            &krate,
            value,
            "max_items",
            quote!(with_max_items),
        ));
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
                use #krate::__private::ProbeSchema as _;
                (&#krate::__private::SchemaProbe::<#ty>::new()).probe_schema()
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

/// Whether a constraint expression is an integer literal (possibly negated).
///
/// Those can be preserved as an exact JSON number. Other expressions keep the historical `f64`
/// conversion because their type is not known to the macro.
fn is_integer_literal(expr: &syn::Expr) -> bool {
    match expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Int(_),
            ..
        }) => true,
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) => is_integer_literal(expr),
        _ => false,
    }
}

fn is_negative_integer_literal(expr: &syn::Expr) -> bool {
    matches!(
        expr,
        syn::Expr::Unary(syn::ExprUnary {
            op: syn::UnOp::Neg(_),
            expr,
            ..
        }) if is_integer_literal(expr)
    )
}

fn length_constraint(
    krate: &TokenStream,
    value: &syn::Expr,
    name: &str,
    builder: TokenStream,
) -> TokenStream {
    if is_negative_integer_literal(value) {
        let message = format!("`{name}` must not be negative");
        let message = syn::LitStr::new(&message, proc_macro2::Span::call_site());
        quote!(compile_error!(#message);)
    } else {
        quote! {
            schema = #krate::doc::schema::#builder(schema, (#value) as usize);
        }
    }
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
