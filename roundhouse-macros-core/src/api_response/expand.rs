use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::punctuated::Punctuated;
use syn::{Data, DeriveInput, Result, Token, Type};

use super::attrs::{FieldInfo, VariantAttrs, bindings, collect_fields};

fn schema_expr(ty: &Type) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::__private::ProbeSchema as _;
        (&#krate::__private::SchemaProbe::<#ty>::new()).probe_schema()
    })
}

fn register_expr(ty: &Type) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::__private::ProbeSchema as _;
        (&#krate::__private::SchemaProbe::<#ty>::new()).probe_register(context)
    })
}

pub fn expand(input: DeriveInput, error_only: bool) -> Result<TokenStream> {
    let krate = crate::util::krate();
    let ident = input.ident.clone();
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &ident,
            "ApiResponse/ApiError can only be derived for an enum",
        ));
    };
    let type_params: Vec<syn::Ident> = input
        .generics
        .type_params()
        .map(|param| param.ident.clone())
        .collect();

    let mut arms = Vec::new();
    let mut docs = Vec::new();
    let mut registrations = Vec::new();
    let mut success_statuses: Vec<u16> = Vec::new();
    let mut runtime_bounds = Vec::new();
    let mut doc_bounds = Vec::new();

    for variant in &data.variants {
        let tags = crate::doc_tags::parse(
            &variant.attrs,
            crate::doc_tags::AllowedTags::DESCRIPTION_ONLY,
        )?;
        let attrs = variant
            .attrs
            .iter()
            .find(|attr| attr.path().is_ident("status") || attr.path().is_ident("response"))
            .ok_or_else(|| {
                syn::Error::new_spanned(
                    &variant.ident,
                    "every variant needs #[status(..)]; a response you can produce but cannot describe is a bug",
                )
            })?;
        let attrs: VariantAttrs = if attrs.path().is_ident("response") {
            // Accept `#[response(status = 201)]` as an equivalent spelling.
            let mut tokens = TokenStream::new();
            let parsed =
                attrs.parse_args_with(Punctuated::<syn::Meta, Token![,]>::parse_terminated)?;
            for meta in parsed {
                if let syn::Meta::NameValue(value) = meta
                    && value.path.is_ident("status")
                    && let syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Int(int),
                        ..
                    }) = value.value
                {
                    let status = int.base10_parse::<u16>()?;
                    tokens.extend(quote!(#status));
                }
            }
            syn::parse2(tokens)?
        } else {
            attrs.parse_args()?
        };

        if error_only && attrs.status < 400 {
            return Err(syn::Error::new_spanned(
                &variant.ident,
                "ApiError statuses must be 4xx or 5xx",
            ));
        }

        let fields = collect_fields(&variant.fields)?;
        collect_bounds(&fields, &type_params, &mut runtime_bounds, &mut doc_bounds);
        let body = fields.iter().find(|field| field.body);
        let headers: Vec<&FieldInfo> = fields
            .iter()
            .filter(|field| field.header.is_some())
            .collect();
        let location = fields.iter().find(|field| field.location);
        let from_validation = fields.iter().any(|field| {
            field.from && crate::util::outer_ident(&field.ty).as_deref() == Some("ValidationErrors")
        });

        let bindings = bindings(&variant.fields, &variant.ident, &fields);
        let status = attrs.status;
        if (200..400).contains(&status) && !success_statuses.contains(&status) {
            success_statuses.push(status);
        }
        // Response descriptions fall back to the body type's `ApiSchema::description()` before
        // the canonical reason phrase. A variant-level description always wins.
        let explicit_description = attrs.description.clone().or(tags.description);
        let canonical = canonical_reason(status);
        let canonical_lit = syn::LitStr::new(canonical, variant.ident.span());
        let description = match explicit_description {
            Some(description) => {
                let description = syn::LitStr::new(&description, variant.ident.span());
                quote!(#description)
            }
            None => match body {
                Some(field)
                    if !matches!(
                        crate::util::outer_ident(&field.ty).as_deref(),
                        Some("Problem" | "ValidationErrors")
                    ) =>
                {
                    let doc_ty = if crate::util::outer_ident(&field.ty).as_deref() == Some("Json") {
                        crate::util::single_generic(&field.ty).unwrap_or_else(|| field.ty.clone())
                    } else {
                        field.ty.clone()
                    };
                    quote!({
                        use #krate::__private::ProbeSchema as _;
                        (&#krate::__private::SchemaProbe::<#doc_ty>::new())
                            .probe_description()
                            .unwrap_or(#canonical_lit)
                    })
                }
                _ => quote!(#canonical_lit),
            },
        };
        // `problem = "slug"` names the RFC 9457 `type`; it does not replace the human title.
        // The title stays the variant description (or the canonical reason).
        let problem_title = description.clone();

        let response_body = if from_validation {
            let field = fields
                .iter()
                .find(|field| field.from)
                .expect("from_validation implies a field");
            let access = &field.access;
            quote! {
                let mut response = #krate::axum::response::IntoResponse::into_response(
                    #krate::Problem::from_validation_with_status(
                        #access,
                        status,
                    )
                );
            }
        } else if let Some(field) = body {
            let access = &field.access;
            quote! {
                let mut response = #krate::axum::response::IntoResponse::into_response(#access);
                *response.status_mut() = status;
            }
        } else if status >= 400 {
            let type_uri = attrs
                .problem
                .as_ref()
                .map(|problem| format!("https://roundhouse.rs/problems/{problem}"))
                .unwrap_or_else(|| "about:blank".to_string());
            let detail = if error_only {
                quote!(problem.detail = Some(_roundhouse_detail.clone());)
            } else {
                quote!()
            };
            quote! {
                let mut problem = #krate::Problem::new(status, #problem_title);
                problem.type_uri = #type_uri.to_string();
                #detail
                let mut response = #krate::axum::response::IntoResponse::into_response(problem);
            }
        } else {
            quote! {
                let mut response = status.into_response();
            }
        };

        let mut header_statements = TokenStream::new();
        for header in &headers {
            let (name, append) = header.header.clone().unwrap();
            let access = &header.access;
            let method = if append {
                quote!(headers.append)
            } else {
                quote!(headers.insert)
            };
            let convert = if header.optional {
                quote! {
                    if let Some(value) = #access {
                        match ::std::convert::TryInto::<#krate::axum::http::HeaderValue>::try_into(value) {
                            Ok(value) => { #method(name, value); }
                            Err(_) => {
                                return #krate::Problem::internal(
                                    "response header could not be converted"
                                ).into_response();
                            }
                        }
                    }
                }
            } else {
                quote! {
                    match ::std::convert::TryInto::<#krate::axum::http::HeaderValue>::try_into(#access) {
                        Ok(value) => { #method(name, value); }
                        Err(_) => {
                            return #krate::Problem::internal(
                                "response header could not be converted"
                            ).into_response();
                        }
                    }
                }
            };
            header_statements.extend(quote! {
                {
                    let name = #krate::axum::http::HeaderName::from_bytes(#name.as_bytes())
                        .expect("valid response header name");
                    let headers = response.headers_mut();
                    #convert
                }
            });
        }

        if let Some(location) = location {
            let access = &location.access;
            header_statements.extend(quote! {
                match ::std::convert::TryInto::<#krate::axum::http::HeaderValue>::try_into(#access) {
                    Ok(value) => {
                        response.headers_mut().insert(
                            #krate::axum::http::header::LOCATION,
                            value,
                        );
                    }
                    Err(_) => {
                        return #krate::Problem::internal(
                            "Location header could not be converted"
                        ).into_response();
                    }
                }
            });
        }

        arms.push(quote! {
            #bindings => {
                let status = #krate::axum::http::StatusCode::from_u16(#status)
                    .unwrap_or(#krate::axum::http::StatusCode::INTERNAL_SERVER_ERROR);
                #response_body
                #header_statements
                response
            }
        });

        // Documentation.
        let mut header_docs = Vec::new();
        if location.is_some() {
            header_docs.push(quote! {
                #krate::doc::HeaderDoc {
                    name: "Location".to_string(),
                    schema: #krate::doc::schema::string_with_format(
                        #krate::utoipa::openapi::schema::SchemaFormat::Custom(
                            "uri".to_string()
                        )
                    ),
                }
            });
        }
        for header in &headers {
            let (name, _append) = header.header.clone().unwrap();
            let ty = &header.ty;
            let schema = schema_expr(ty);
            registrations.push(register_expr(ty));
            header_docs.push(quote! {
                #krate::doc::HeaderDoc {
                    name: #name.to_string(),
                    schema: #schema,
                }
            });
        }

        if let Some(field) = body {
            let ty = &field.ty;
            let kind = crate::util::outer_ident(ty);
            if kind.as_deref() == Some("Json") {
                let inner = crate::util::single_generic(ty)
                    .ok_or_else(|| syn::Error::new_spanned(ty, "expected Json<T>"))?;
                let inner_schema = schema_expr(&inner);
                registrations.push(register_expr(&inner));
                docs.push(quote! {
                    responses.push(#krate::doc::ResponseDoc {
                        status: #status,
                        description: #description.to_string(),
                        content_type: ::core::option::Option::Some("application/json".to_string()),
                        schema: ::core::option::Option::Some(#inner_schema),
                        headers: ::std::vec![#(#header_docs),*],
                    });
                });
            } else if matches!(kind.as_deref(), Some("Problem") | Some("ValidationErrors")) {
                docs.push(quote! {
                    responses.push(#krate::doc::ResponseDoc::problem(#status, #problem_title));
                });
            } else {
                let ty_schema = schema_expr(ty);
                registrations.push(register_expr(ty));
                docs.push(quote! {
                    responses.push(#krate::doc::ResponseDoc {
                        status: #status,
                        description: #description.to_string(),
                        content_type: ::core::option::Option::Some("application/json".to_string()),
                        schema: ::core::option::Option::Some(#ty_schema),
                        headers: ::std::vec![#(#header_docs),*],
                    });
                });
            }
        } else if status >= 400 || from_validation {
            docs.push(quote! {
                responses.push(#krate::doc::ResponseDoc::problem(#status, #problem_title));
            });
        } else {
            docs.push(quote! {
                responses.push(#krate::doc::ResponseDoc {
                    status: #status,
                    description: #description.to_string(),
                    content_type: ::core::option::Option::None,
                    schema: ::core::option::Option::None,
                    headers: ::std::vec![#(#header_docs),*],
                });
            });
        }
    }

    let mut runtime_generics = input.generics.clone();
    runtime_generics
        .make_where_clause()
        .predicates
        .extend(runtime_bounds);
    let mut doc_generics = input.generics.clone();
    doc_generics
        .make_where_clause()
        .predicates
        .extend(doc_bounds);
    let (_, ty_generics, _) = input.generics.split_for_impl();
    let (runtime_impl_generics, _, runtime_where) = runtime_generics.split_for_impl();
    let (doc_impl_generics, _, doc_where) = doc_generics.split_for_impl();
    let (error_impl_generics, _, error_where) = input.generics.split_for_impl();

    let docs_fn = format_ident!("__roundhouse_responses_{}", ident);
    let docs_call = if type_params.is_empty() {
        quote!(#docs_fn(context))
    } else {
        quote!(#docs_fn::#ty_generics(context))
    };
    let detail_prefix = if error_only {
        quote! {
            let _roundhouse_detail = ::std::string::ToString::to_string(&self);
        }
    } else {
        quote!()
    };
    let error_assert = if error_only {
        quote! {
            const _: fn() = || {
                fn assert_error<T: ::std::error::Error + ?Sized>() {}
                fn check #error_impl_generics () #error_where {
                    assert_error::<#ident #ty_generics>();
                }
            };
        }
    } else {
        quote!()
    };

    Ok(quote! {
        #error_assert

        impl #runtime_impl_generics #krate::axum::response::IntoResponse for #ident #ty_generics #runtime_where {
            fn into_response(self) -> #krate::axum::response::Response {
                use #krate::axum::response::IntoResponse;
                #detail_prefix
                match self {
                    #(#arms),*
                }
            }
        }

        #[allow(non_snake_case)]
        fn #docs_fn #doc_impl_generics (
            context: &mut dyn #krate::doc::SchemaContext,
        ) -> ::std::vec::Vec<#krate::doc::ResponseDoc> #doc_where {
            let mut responses = ::std::vec::Vec::new();
            #(#registrations)*
            #(#docs)*
            responses
        }

        impl #doc_impl_generics #krate::doc::ApiResponse for #ident #ty_generics #doc_where {
            const SUCCESS: &'static [u16] = &[#(#success_statuses),*];

            fn responses(
                context: &mut dyn #krate::doc::SchemaContext,
            ) -> ::std::vec::Vec<#krate::doc::ResponseDoc> {
                #docs_call
            }
        }

    })
}

fn collect_bounds(
    fields: &[FieldInfo],
    type_params: &[syn::Ident],
    runtime: &mut Vec<syn::WherePredicate>,
    docs: &mut Vec<syn::WherePredicate>,
) {
    let krate = crate::util::krate();
    for field in fields {
        let ty = &field.ty;
        if field.body {
            let doc_ty = if crate::util::outer_ident(ty).as_deref() == Some("Json") {
                crate::util::single_generic(ty).unwrap_or_else(|| ty.clone())
            } else {
                ty.clone()
            };
            if mentions_generic(&doc_ty, type_params) {
                docs.push(syn::parse2(quote!(#doc_ty: #krate::doc::ApiSchema)).expect("bound"));
            }
            if mentions_generic(ty, type_params) {
                runtime.push(
                    syn::parse2(quote!(#ty: #krate::axum::response::IntoResponse)).expect("bound"),
                );
            }
        }
        if field.header.is_some() {
            if mentions_generic(ty, type_params) {
                docs.push(syn::parse2(quote!(#ty: #krate::doc::ApiSchema)).expect("bound"));
            }
            let runtime_ty = if field.optional {
                crate::util::single_generic(ty).unwrap_or_else(|| ty.clone())
            } else {
                ty.clone()
            };
            if mentions_generic(&runtime_ty, type_params) {
                runtime.push(
                    syn::parse2(quote!(
                        #runtime_ty: ::std::convert::TryInto<#krate::axum::http::HeaderValue>
                    ))
                    .expect("bound"),
                );
            }
        }
        if field.location {
            if mentions_generic(ty, type_params) {
                docs.push(syn::parse2(quote!(#ty: #krate::doc::ApiSchema)).expect("bound"));
            }
            let runtime_ty = if field.optional {
                crate::util::single_generic(ty).unwrap_or_else(|| ty.clone())
            } else {
                ty.clone()
            };
            if mentions_generic(&runtime_ty, type_params) {
                runtime.push(
                    syn::parse2(quote!(
                        #runtime_ty: ::std::convert::TryInto<#krate::axum::http::HeaderValue>
                    ))
                    .expect("bound"),
                );
            }
        }
    }
}

fn mentions_generic(ty: &Type, type_params: &[syn::Ident]) -> bool {
    match ty {
        Type::Path(path) => path.path.segments.iter().any(|segment| {
            type_params.contains(&segment.ident)
                || match &segment.arguments {
                    syn::PathArguments::AngleBracketed(arguments) => {
                        arguments.args.iter().any(|argument| match argument {
                            syn::GenericArgument::Type(ty) => mentions_generic(ty, type_params),
                            _ => false,
                        })
                    }
                    _ => false,
                }
        }),
        Type::Reference(reference) => mentions_generic(&reference.elem, type_params),
        Type::Array(array) => mentions_generic(&array.elem, type_params),
        Type::Slice(slice) => mentions_generic(&slice.elem, type_params),
        Type::Tuple(tuple) => tuple
            .elems
            .iter()
            .any(|element| mentions_generic(element, type_params)),
        Type::Paren(paren) => mentions_generic(&paren.elem, type_params),
        Type::Group(group) => mentions_generic(&group.elem, type_params),
        Type::Ptr(ptr) => mentions_generic(&ptr.elem, type_params),
        _ => false,
    }
}

fn canonical_reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        202 => "Accepted",
        204 => "No Content",
        301 => "Moved Permanently",
        302 => "Found",
        304 => "Not Modified",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        409 => "Conflict",
        410 => "Gone",
        413 => "Payload Too Large",
        415 => "Unsupported Media Type",
        422 => "Unprocessable Entity",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Response",
    }
}
