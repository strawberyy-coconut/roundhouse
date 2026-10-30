use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::punctuated::Punctuated;
use syn::{Data, DeriveInput, Fields, Ident, LitInt, LitStr, Result, Token, Type};

use crate::util::is_option;

struct VariantAttrs {
    status: u16,
    description: Option<String>,
    problem: Option<String>,
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
struct FieldInfo {
    access: TokenStream,
    ty: Type,
    optional: bool,
    body: bool,
    header: Option<(String, bool)>,
    location: bool,
    from: bool,
}

fn schema_expr(ty: &Type) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::doc::ProbeSchemaFor as _;
        (&#krate::doc::SchemaFor::<#ty>::new()).probe_schema()
    })
}

fn register_expr(ty: &Type) -> TokenStream {
    let krate = crate::util::krate();
    quote!({
        use #krate::doc::ProbeSchemaFor as _;
        (&#krate::doc::SchemaFor::<#ty>::new()).probe_register(components)
    })
}

pub fn expand(input: DeriveInput, error_only: bool) -> Result<TokenStream> {
    let krate = crate::util::krate();
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "ApiResponse/ApiError do not support generic enums in v0.1",
        ));
    }
    let ident = input.ident.clone();
    let Data::Enum(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &ident,
            "ApiResponse/ApiError can only be derived for an enum",
        ));
    };

    let mut arms = Vec::new();
    let mut docs = Vec::new();
    let mut registrations = Vec::new();

    for variant in &data.variants {
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
            // Accept `#[response(status = 201)]` as a migration alias.
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
        let description = attrs
            .description
            .clone()
            .unwrap_or_else(|| canonical_reason(status).to_string());
        let problem_title = attrs.problem.clone().unwrap_or_else(|| description.clone());

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
        if let Some(location) = location {
            let required = !location.optional;
            header_docs.push(quote! {
                #krate::doc::HeaderDoc {
                    name: "Location".to_string(),
                    schema: #krate::doc::schema::string_with_format(
                        #krate::utoipa::openapi::schema::SchemaFormat::Custom(
                            "uri".to_string()
                        )
                    ),
                    required: #required,
                    append: false,
                }
            });
        }
        for header in &headers {
            let (name, append) = header.header.clone().unwrap();
            let ty = &header.ty;
            let required = !header.optional;
            let schema = schema_expr(ty);
            registrations.push(register_expr(ty));
            header_docs.push(quote! {
                #krate::doc::HeaderDoc {
                    name: #name.to_string(),
                    schema: #schema,
                    required: #required,
                    append: #append,
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

    let docs_fn = format_ident!("__roundhouse_responses_{}", ident);
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
                fn assert_error<T: ::std::error::Error>() {}
                assert_error::<#ident>();
            };
        }
    } else {
        quote!()
    };

    Ok(quote! {
        #error_assert

        impl #krate::axum::response::IntoResponse for #ident {
            fn into_response(self) -> #krate::axum::response::Response {
                use #krate::axum::response::IntoResponse;
                #detail_prefix
                match self {
                    #(#arms),*
                }
            }
        }

        #[allow(non_snake_case)]
        fn #docs_fn(
            components: &mut #krate::utoipa::openapi::schema::Components,
        ) -> ::std::vec::Vec<#krate::doc::ResponseDoc> {
            let mut responses = ::std::vec::Vec::new();
            #(#registrations)*
            #(#docs)*
            responses
        }

        impl #krate::doc::ApiResponse for #ident {
            fn responses(
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) -> ::std::vec::Vec<#krate::doc::ResponseDoc> {
                #docs_fn(components)
            }
        }

    })
}

fn collect_fields(fields: &Fields) -> Result<Vec<FieldInfo>> {
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

fn field_info(
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

fn bindings(fields: &Fields, variant: &Ident, infos: &[FieldInfo]) -> TokenStream {
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
