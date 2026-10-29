use proc_macro2::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Expr, Fields, Ident, LitStr, Result, Token, Type};

use crate::util::{outer_ident, single_generic};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    File,
    OptionalFile,
    Files,
    OptionalFiles,
    TempFile,
    OptionalTempFile,
    TempFiles,
    OptionalTempFiles,
    Text,
    OptionalText,
    Texts,
    OptionalTexts,
}

impl Kind {
    fn is_file(self) -> bool {
        matches!(
            self,
            Kind::File
                | Kind::OptionalFile
                | Kind::Files
                | Kind::OptionalFiles
                | Kind::TempFile
                | Kind::OptionalTempFile
                | Kind::TempFiles
                | Kind::OptionalTempFiles
        )
    }
}

struct FieldSpec {
    ident: Ident,
    name: String,
    ty: Type,
    kind: Kind,
    max_bytes: Option<Expr>,
    accept: Vec<LitStr>,
}

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    if !input.generics.params.is_empty() {
        return Err(syn::Error::new_spanned(
            &input.generics,
            "Multipart does not support generic structs in v0.1",
        ));
    }
    let ident = input.ident;
    let deny_unknown = container_deny_unknown(&input.attrs);

    let Data::Struct(data) = &input.data else {
        return Err(syn::Error::new_spanned(
            &ident,
            "Multipart can only be derived for a struct",
        ));
    };
    let Fields::Named(fields) = &data.fields else {
        return Err(syn::Error::new_spanned(
            &data.fields,
            "Multipart requires named fields",
        ));
    };

    let mut specs = Vec::new();
    for field in &fields.named {
        let Some(field_ident) = field.ident.clone() else {
            continue;
        };
        let mut name = field_ident.to_string();
        if let Some(stripped) = name.strip_prefix("r#") {
            name = stripped.to_string();
        }
        if let Some(rename) = serde_rename(&field.attrs)? {
            name = rename;
        }
        let (max_bytes, accept) = file_attrs(&field.attrs)?;
        let kind = classify(&field.ty)?;
        specs.push(FieldSpec {
            ident: field_ident,
            name,
            ty: field.ty.clone(),
            kind,
            max_bytes,
            accept,
        });
    }

    let mut bindings = Vec::new();
    let mut properties = Vec::new();
    let mut required = Vec::new();
    let mut registers = Vec::new();
    let mut constructor_fields = Vec::new();
    let mut field_specs = Vec::new();

    for spec in &specs {
        let field = &spec.ident;
        let name = &spec.name;
        let ty = &spec.ty;
        let max = match &spec.max_bytes {
            Some(expr) => quote!(::core::option::Option::Some((#expr) as usize)),
            None => quote!(::core::option::Option::None),
        };
        let accept_literals = &spec.accept;
        let accept = if spec.accept.is_empty() {
            quote!(&[])
        } else {
            quote!(&[#(#accept_literals),*])
        };
        let required_field = matches!(spec.kind, Kind::File | Kind::TempFile | Kind::Text);
        let kind = match spec.kind {
            Kind::File => quote!(::roundhouse::multipart::FieldKind::File),
            Kind::OptionalFile => quote!(::roundhouse::multipart::FieldKind::File),
            Kind::Files => quote!(::roundhouse::multipart::FieldKind::FileList),
            Kind::OptionalFiles => quote!(::roundhouse::multipart::FieldKind::FileList),
            Kind::TempFile => quote!(::roundhouse::multipart::FieldKind::TempFile),
            Kind::OptionalTempFile => quote!(::roundhouse::multipart::FieldKind::TempFile),
            Kind::TempFiles => quote!(::roundhouse::multipart::FieldKind::TempFileList),
            Kind::OptionalTempFiles => quote!(::roundhouse::multipart::FieldKind::TempFileList),
            Kind::Text => quote!(::roundhouse::multipart::FieldKind::Text),
            Kind::OptionalText => quote!(::roundhouse::multipart::FieldKind::Text),
            Kind::Texts => quote!(::roundhouse::multipart::FieldKind::TextList),
            Kind::OptionalTexts => quote!(::roundhouse::multipart::FieldKind::TextList),
        };
        field_specs.push(quote! {
            ::roundhouse::multipart::FieldSpec::new(#name, #kind)
                .required(#required_field)
                .max_bytes(#max)
                .accept(#accept)
        });

        let constructor = if matches!(
            spec.kind,
            Kind::OptionalFile
                | Kind::OptionalTempFile
                | Kind::OptionalFiles
                | Kind::OptionalTempFiles
                | Kind::OptionalText
                | Kind::OptionalTexts
                | Kind::Files
                | Kind::TempFiles
                | Kind::Texts
        ) {
            quote!(#field)
        } else {
            quote!(#field: #field.expect("required multipart field was validated by the extractor"))
        };
        constructor_fields.push(constructor);

        let binding = match spec.kind {
            Kind::File => quote! {
                let #field = ::roundhouse::multipart::require(
                    fields.take_files(#name).into_iter().next(),
                    #name,
                    &mut errors,
                );
            },
            Kind::OptionalFile => quote! {
                let #field = fields.take_files(#name).into_iter().next();
            },
            Kind::Files => quote! {
                let #field: ::std::vec::Vec<::roundhouse::multipart::FilePart> =
                    fields.take_files(#name);
            },
            Kind::OptionalFiles => quote! {
                let __values = fields.take_files(#name);
                let #field: ::core::option::Option<
                    ::std::vec::Vec<::roundhouse::multipart::FilePart>,
                > = if __values.is_empty() {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(__values)
                };
            },
            Kind::TempFile => quote! {
                let #field = ::roundhouse::multipart::require(
                    fields.take_temp_files(#name).into_iter().next(),
                    #name,
                    &mut errors,
                );
            },
            Kind::OptionalTempFile => quote! {
                let #field = fields.take_temp_files(#name).into_iter().next();
            },
            Kind::TempFiles => quote! {
                let #field: ::std::vec::Vec<::roundhouse::multipart::TempFile> =
                    fields.take_temp_files(#name);
            },
            Kind::OptionalTempFiles => quote! {
                let __values = fields.take_temp_files(#name);
                let #field: ::core::option::Option<
                    ::std::vec::Vec<::roundhouse::multipart::TempFile>,
                > = if __values.is_empty() {
                    ::core::option::Option::None
                } else {
                    ::core::option::Option::Some(__values)
                };
            },
            Kind::Text => quote! {
                let #field = ::roundhouse::multipart::require(
                    fields
                        .take_texts(#name)
                        .into_iter()
                        .next()
                        .and_then(|bytes| ::roundhouse::multipart::parse_text::<#ty>(
                            bytes, #name, &mut errors,
                        )),
                    #name,
                    &mut errors,
                );
            },
            Kind::OptionalText => {
                let target = text_target(&spec.kind, ty);
                quote! {
                    let #field = fields
                        .take_texts(#name)
                        .into_iter()
                        .next()
                        .and_then(|bytes| ::roundhouse::multipart::parse_text::<#target>(
                            bytes, #name, &mut errors,
                        ));
                }
            }
            Kind::Texts => {
                let target = text_target(&spec.kind, ty);
                quote! {
                    let #field: ::std::vec::Vec<#target> = fields
                        .take_texts(#name)
                        .into_iter()
                        .filter_map(|bytes| ::roundhouse::multipart::parse_text::<#target>(
                            bytes, #name, &mut errors,
                        ))
                        .collect();
                }
            }
            Kind::OptionalTexts => {
                let target = text_target(&spec.kind, ty);
                quote! {
                    let __values = fields.take_texts(#name);
                    let #field: ::core::option::Option<::std::vec::Vec<#target>> =
                        if __values.is_empty() {
                            ::core::option::Option::None
                        } else {
                            ::core::option::Option::Some(
                                __values
                                    .into_iter()
                                    .filter_map(|bytes| ::roundhouse::multipart::parse_text::<#target>(
                                        bytes, #name, &mut errors,
                                    ))
                                    .collect(),
                            )
                        };
                }
            }
        };
        bindings.push(binding);

        let mut extensions = TokenStream::new();
        if spec.kind.is_file() {
            if let Some(expr) = &spec.max_bytes {
                extensions.extend(quote! {
                    let schema = ::roundhouse::doc::schema::with_extension(
                        schema,
                        "x-max-bytes",
                        ::roundhouse::serde_json::json!((#expr) as usize),
                    );
                });
            }
            if !spec.accept.is_empty() {
                let accept_literals = &spec.accept;
                extensions.extend(quote! {
                    let schema = ::roundhouse::doc::schema::with_extension(
                        schema,
                        "x-accept",
                        ::roundhouse::serde_json::json!([#(#accept_literals),*]),
                    );
                });
            }
        }
        let base_schema = match spec.kind {
            Kind::File | Kind::TempFile => quote!(::roundhouse::multipart::file_schema()),
            Kind::OptionalFile | Kind::OptionalTempFile => {
                quote!(::roundhouse::multipart::optional_file_schema())
            }
            Kind::Files | Kind::TempFiles => quote!(::roundhouse::multipart::files_schema()),
            Kind::OptionalFiles | Kind::OptionalTempFiles => {
                quote!(::roundhouse::multipart::optional_files_schema())
            }
            Kind::Text | Kind::OptionalText | Kind::Texts | Kind::OptionalTexts => {
                quote!(<#ty as ::roundhouse::doc::ApiSchema>::schema())
            }
        };
        let schema = if extensions.is_empty() {
            base_schema
        } else {
            quote! {
                {
                    let schema = #base_schema;
                    #extensions
                    schema
                }
            }
        };
        properties.push(quote! {
            builder = builder.property(#name, #schema);
        });
        if required_field {
            required.push(quote! {
                builder = builder.required(#name);
            });
        }
        if matches!(
            spec.kind,
            Kind::Text | Kind::OptionalText | Kind::Texts | Kind::OptionalTexts
        ) {
            registers.push(quote! {
                <#ty as ::roundhouse::doc::ApiSchema>::register(components);
            });
        }
    }

    Ok(quote! {
        impl ::roundhouse::multipart::MultipartForm for #ident {
            fn fields() -> &'static [::roundhouse::multipart::FieldSpec] {
                const FIELDS: &[::roundhouse::multipart::FieldSpec] = &[#(#field_specs),*];
                FIELDS
            }

            fn from_fields(
                fields: &mut ::roundhouse::multipart::FieldValues,
            ) -> ::core::result::Result<Self, ::roundhouse::multipart::MultipartProblem> {
                let mut errors = ::roundhouse::validate::ValidationErrors::new();
                #(#bindings)*
                if !errors.is_empty() {
                    return ::core::result::Result::Err(
                        ::roundhouse::multipart::MultipartProblem::from_validation(errors),
                    );
                }
                ::core::result::Result::Ok(Self { #(#constructor_fields),* })
            }

            fn schema(
                components: &mut ::roundhouse::utoipa::openapi::schema::Components,
            ) -> ::roundhouse::utoipa::openapi::RefOr<
                ::roundhouse::utoipa::openapi::schema::Schema,
            > {
                #(#registers)*
                let mut builder = ::roundhouse::multipart::schema_object();
                #(#properties)*
                #(#required)*
                builder.into()
            }

            fn deny_unknown() -> bool {
                #deny_unknown
            }
        }
    })
}

fn classify(ty: &Type) -> Result<Kind> {
    let outer = outer_ident(ty);
    match outer.as_deref() {
        Some("FilePart") => Ok(Kind::File),
        Some("TempFile") => Ok(Kind::TempFile),
        Some("Option") => {
            let inner = single_generic(ty).ok_or_else(|| {
                syn::Error::new_spanned(ty, "expected Option<FilePart> or Option<T>")
            })?;
            match outer_ident(&inner).as_deref() {
                Some("FilePart") => Ok(Kind::OptionalFile),
                Some("TempFile") => Ok(Kind::OptionalTempFile),
                Some("Vec") => {
                    let inner = single_generic(&inner)
                        .ok_or_else(|| syn::Error::new_spanned(ty, "expected Option<Vec<T>>"))?;
                    match outer_ident(&inner).as_deref() {
                        Some("FilePart") => Ok(Kind::OptionalFiles),
                        Some("TempFile") => Ok(Kind::OptionalTempFiles),
                        _ => Ok(Kind::OptionalTexts),
                    }
                }
                _ => Ok(Kind::OptionalText),
            }
        }
        Some("Vec") => {
            let inner =
                single_generic(ty).ok_or_else(|| syn::Error::new_spanned(ty, "expected Vec<T>"))?;
            match outer_ident(&inner).as_deref() {
                Some("FilePart") => Ok(Kind::Files),
                Some("TempFile") => Ok(Kind::TempFiles),
                _ => Ok(Kind::Texts),
            }
        }
        _ => Ok(Kind::Text),
    }
}

fn text_target(kind: &Kind, ty: &Type) -> Type {
    match kind {
        Kind::OptionalText | Kind::Texts | Kind::OptionalTexts => {
            let inner = single_generic(ty).unwrap_or_else(|| ty.clone());
            if matches!(kind, Kind::OptionalTexts) {
                single_generic(&inner).unwrap_or(inner)
            } else {
                inner
            }
        }
        _ => ty.clone(),
    }
}

fn container_deny_unknown(attrs: &[syn::Attribute]) -> bool {
    let mut deny = false;
    for attr in attrs {
        if attr.path().is_ident("multipart") {
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("deny_unknown") {
                    deny = true;
                }
                Ok(())
            });
        }
    }
    deny
}

fn serde_rename(attrs: &[syn::Attribute]) -> Result<Option<String>> {
    let mut rename = None;
    for attr in attrs {
        if attr.path().is_ident("serde") {
            attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    let value: LitStr = meta.value()?.parse()?;
                    rename = Some(value.value());
                }
                Ok(())
            })?;
        }
    }
    Ok(rename)
}

fn file_attrs(attrs: &[syn::Attribute]) -> Result<(Option<Expr>, Vec<LitStr>)> {
    let mut max_bytes = None;
    let mut accept = Vec::new();
    for attr in attrs {
        if !attr.path().is_ident("file") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("max_bytes") {
                max_bytes = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("accept") {
                let list = meta.value()?;
                let content;
                syn::bracketed!(content in list);
                accept = content
                    .parse_terminated(|input| input.parse::<LitStr>(), Token![,])?
                    .into_iter()
                    .collect();
            }
            Ok(())
        })?;
    }
    Ok((max_bytes, accept))
}
