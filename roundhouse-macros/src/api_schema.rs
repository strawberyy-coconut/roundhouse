use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Expr, Fields, LitStr, Result, Type};

#[derive(Default, Clone)]
struct SerdeContainer {
    rename_all: Option<RenameRule>,
    tag: Option<String>,
    content: Option<String>,
    untagged: bool,
    deny_unknown_fields: bool,
}

#[derive(Default, Clone)]
struct SerdeField {
    rename: Option<String>,
    skip: bool,
    skip_serializing_if: bool,
    default: bool,
    flatten: bool,
}

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
    pub(crate) name: Option<String>,
    pub(crate) via_utoipa: bool,
    pub(crate) constraints: Constraints,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum RenameRule {
    Lower,
    Upper,
    Camel,
    Snake,
    Pascal,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl RenameRule {
    fn parse(value: &str) -> Result<Self> {
        Ok(match value {
            "lowercase" => RenameRule::Lower,
            "UPPERCASE" => RenameRule::Upper,
            "camelCase" => RenameRule::Camel,
            "snake_case" => RenameRule::Snake,
            "PascalCase" => RenameRule::Pascal,
            "SCREAMING_SNAKE_CASE" => RenameRule::ScreamingSnake,
            "kebab-case" => RenameRule::Kebab,
            "SCREAMING-KEBAB-CASE" => RenameRule::ScreamingKebab,
            other => {
                return Err(syn::Error::new(
                    proc_macro2::Span::call_site(),
                    format!("unsupported serde rename_all rule `{other}`"),
                ));
            }
        })
    }

    fn apply(self, name: &str) -> String {
        let words = split_words(name);
        match self {
            RenameRule::Lower => words.join("").to_lowercase(),
            RenameRule::Upper => words.join("").to_uppercase(),
            RenameRule::Snake => words.join("_").to_lowercase(),
            RenameRule::ScreamingSnake => words.join("_").to_uppercase(),
            RenameRule::Kebab => words.join("-").to_lowercase(),
            RenameRule::ScreamingKebab => words.join("-").to_uppercase(),
            RenameRule::Pascal => words
                .iter()
                .map(|word| capitalize(word))
                .collect::<String>(),
            RenameRule::Camel => words
                .iter()
                .enumerate()
                .map(|(index, word)| {
                    if index == 0 {
                        word.to_lowercase()
                    } else {
                        capitalize(word)
                    }
                })
                .collect::<String>(),
        }
    }
}

fn split_words(name: &str) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut previous_lower = false;
    for ch in name.chars() {
        if ch == '_' || ch == '-' || ch == ' ' {
            if !current.is_empty() {
                words.push(std::mem::take(&mut current));
            }
            previous_lower = false;
            continue;
        }
        if ch.is_uppercase() && previous_lower && !current.is_empty() {
            words.push(std::mem::take(&mut current));
        }
        previous_lower = ch.is_lowercase() || ch.is_ascii_digit();
        current.push(ch);
    }
    if !current.is_empty() {
        words.push(current);
    }
    if words.is_empty() {
        words.push(name.to_string());
    }
    words
}

fn capitalize(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + &chars.as_str().to_lowercase(),
        None => String::new(),
    }
}

fn serde_container(attrs: &[syn::Attribute]) -> Result<SerdeContainer> {
    let mut container = SerdeContainer::default();
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let value: LitStr = meta.value()?.parse()?;
                container.rename_all = Some(RenameRule::parse(&value.value())?);
            } else if meta.path.is_ident("tag") {
                let value: LitStr = meta.value()?.parse()?;
                container.tag = Some(value.value());
            } else if meta.path.is_ident("content") {
                let value: LitStr = meta.value()?.parse()?;
                container.content = Some(value.value());
            } else if meta.path.is_ident("untagged") {
                container.untagged = true;
            } else if meta.path.is_ident("deny_unknown_fields") {
                container.deny_unknown_fields = true;
            }
            Ok(())
        })?;
    }
    Ok(container)
}

fn serde_field(attrs: &[syn::Attribute]) -> Result<SerdeField> {
    let mut field = SerdeField::default();
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value: LitStr = meta.value()?.parse()?;
                field.rename = Some(value.value());
            } else if meta.path.is_ident("skip")
                || meta.path.is_ident("skip_serializing")
                || meta.path.is_ident("skip_deserializing")
            {
                field.skip = true;
            } else if meta.path.is_ident("skip_serializing_if") {
                let _: Expr = meta.value()?.parse()?;
                field.skip_serializing_if = true;
            } else if meta.path.is_ident("default") {
                field.default = true;
            } else if meta.path.is_ident("flatten") {
                field.flatten = true;
            }
            Ok(())
        })?;
    }
    Ok(field)
}

pub(crate) fn schema_attrs(attrs: &[syn::Attribute]) -> Result<SchemaAttrs> {
    let mut schema = SchemaAttrs::default();
    for attr in attrs {
        if !attr.path().is_ident("schema") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                let value: LitStr = meta.value()?.parse()?;
                schema.name = Some(value.value());
            } else if meta.path.is_ident("min_length") {
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
            } else if meta.path.is_ident("unique_items") {
                schema.constraints.unique_items = true;
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
            } else if meta.path.is_ident("validate_with") {
                schema.constraints.validate_with = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("via") {
                let value: syn::Path = meta.value()?.parse()?;
                if value
                    .segments
                    .last()
                    .is_some_and(|segment| segment.ident == "utoipa")
                {
                    schema.via_utoipa = true;
                }
            }
            Ok(())
        })?;
    }
    Ok(schema)
}

pub fn expand(input: DeriveInput) -> Result<TokenStream> {
    let krate = crate::util::krate();
    let ident = input.ident.clone();
    let container_serde = serde_container(&input.attrs)?;
    let container_schema = schema_attrs(&input.attrs)?;
    let explicit_name = container_schema.name.clone();
    let simple_lit = LitStr::new(&ident.to_string(), ident.span());
    let name_expr = match &explicit_name {
        Some(name) => {
            let lit = LitStr::new(name, ident.span());
            quote!(#lit.to_string())
        }
        None => quote! {
            #krate::doc::component_name(#simple_lit, module_path!())
        },
    };

    let mut generics = input.generics.clone();
    for param in &mut generics.params {
        if let syn::GenericParam::Type(param) = param {
            param
                .bounds
                .push(syn::parse_quote!(#krate::doc::ApiSchema));
        }
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let (register_body, validate_body) = match &input.data {
        Data::Struct(data) => struct_parts(&data.fields, &container_serde)?,
        Data::Enum(data) => enum_parts(data, &container_serde)?,
        Data::Union(_) => {
            return Err(syn::Error::new_spanned(
                &ident,
                "ApiSchema cannot be derived for a union",
            ));
        }
    };

    let (schema_body, register_block) = if container_schema.via_utoipa {
        (
            quote! {
                <Self as #krate::utoipa::PartialSchema>::schema()
            },
            quote! {
                let mut schemas = ::std::vec::Vec::new();
                <Self as #krate::utoipa::ToSchema>::schemas(&mut schemas);
                for (name, schema) in schemas {
                    components.schemas.entry(name).or_insert(schema);
                }
            },
        )
    } else {
        (
            quote! {
                {
                    let name = #name_expr;
                    #krate::doc::schema::r#ref(name)
                }
            },
            quote! {
                let name = #name_expr;
                if components.schemas.contains_key(&name) {
                    return;
                }
                components.schemas.insert(
                    name.clone(),
                    #krate::doc::schema::any_value(),
                );
                #register_body
            },
        )
    };

    let component_record = {
        let name = match &explicit_name {
            Some(name) => {
                let lit = LitStr::new(name, ident.span());
                quote!(::core::option::Option::Some(#lit))
            }
            None => quote!(::core::option::Option::None),
        };
        quote! {
            #krate::inventory::submit! {
                #krate::registry::ComponentTypeRecord::__new(
                    #simple_lit,
                    module_path!(),
                    || ::core::any::TypeId::of::<#ident #ty_generics>(),
                    #name,
                )
            }
        }
    };

    Ok(quote! {
        #component_record

        impl #impl_generics #krate::doc::ApiSchema for #ident #ty_generics #where_clause {
            fn schema() -> #krate::utoipa::openapi::RefOr<
                #krate::utoipa::openapi::schema::Schema,
            > {
                #schema_body
            }

            fn register(
                components: &mut #krate::utoipa::openapi::schema::Components,
            ) {
                #register_block
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

fn struct_parts(fields: &Fields, container: &SerdeContainer) -> Result<(TokenStream, TokenStream)> {
    let krate = crate::util::krate();
    let named = named_fields(fields)?;
    let mut register_calls = Vec::new();
    let mut validations = Vec::new();

    for (index, field) in named.iter().enumerate() {
        let ty = &field.ty;
        let field_serde = serde_field(&field.attrs)?;
        let field_schema = schema_attrs(&field.attrs)?;
        register_calls.push(quote! {
            <#ty as #krate::doc::ApiSchema>::register(components);
        });
        if field_serde.skip {
            continue;
        }
        {
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
        }
    }

    let object = object_expr(fields, container)?;
    let register_body = quote! {
        #(#register_calls)*
        {
            let base: #krate::utoipa::openapi::RefOr<
                #krate::utoipa::openapi::schema::Schema,
            > = #object;
            components.schemas.insert(name.clone(), base);
        }
    };

    Ok((register_body, quote!(#(#validations)*)))
}

fn enum_parts(
    data: &syn::DataEnum,
    container: &SerdeContainer,
) -> Result<(TokenStream, TokenStream)> {
    let krate = crate::util::krate();
    if container.tag.is_none()
        && !container.untagged
        && data
            .variants
            .iter()
            .all(|variant| matches!(variant.fields, Fields::Unit))
    {
        let mut values = Vec::new();
        for variant in &data.variants {
            let variant_serde = serde_field(&variant.attrs)?;
            let variant_name = match &variant_serde.rename {
                Some(name) => name.clone(),
                None => container
                    .rename_all
                    .map(|rule| rule.apply(&variant.ident.to_string()))
                    .unwrap_or_else(|| variant.ident.to_string()),
            };
            values.push(LitStr::new(&variant_name, variant.ident.span()));
        }
        let register_body = quote! {
            {
                let mut builder = #krate::doc::schema::object()
                    .schema_type(#krate::utoipa::openapi::schema::Type::String);
                builder = builder.enum_values(Some(::std::vec![
                    #(#krate::serde_json::json!(#values)),*
                ]));
                let base: #krate::utoipa::openapi::RefOr<
                    #krate::utoipa::openapi::schema::Schema,
                > = builder.into();
                components.schemas.insert(name.clone(), base);
            }
        };
        return Ok((register_body, quote!()));
    }

    let mut register_calls = Vec::new();
    let mut variants = Vec::new();
    let mut validation_arms = Vec::new();

    for (variant_index, variant) in data.variants.iter().enumerate() {
        for field in variant.fields.iter() {
            let ty = &field.ty;
            register_calls.push(quote! {
                <#ty as #krate::doc::ApiSchema>::register(components);
            });
        }
        let variant_serde = serde_field(&variant.attrs)?;
        let variant_name = match &variant_serde.rename {
            Some(name) => name.clone(),
            None => container
                .rename_all
                .map(|rule| rule.apply(&variant.ident.to_string()))
                .unwrap_or_else(|| variant.ident.to_string()),
        };
        let variant_lit = LitStr::new(&variant_name, variant.ident.span());
        let payload = variant_payload(&variant.fields, container)?;

        let schema = if container.untagged {
            payload
        } else if let Some(tag) = &container.tag {
            let tag_lit = LitStr::new(tag, variant.ident.span());
            if let Some(content) = &container.content {
                let content_lit = LitStr::new(content, variant.ident.span());
                if matches!(variant.fields, Fields::Unit) {
                    quote! {
                        #krate::doc::schema::object_with(
                            ::std::vec![(
                                #tag_lit.to_string(),
                                #krate::doc::schema::string_enum(#variant_lit),
                            )],
                            ::std::vec![#tag_lit.to_string()],
                        )
                    }
                } else {
                    quote! {
                        #krate::doc::schema::object_with(
                            ::std::vec![
                                (
                                    #tag_lit.to_string(),
                                    #krate::doc::schema::string_enum(#variant_lit),
                                ),
                                (#content_lit.to_string(), #payload),
                            ],
                            ::std::vec![
                                #tag_lit.to_string(),
                                #content_lit.to_string(),
                            ],
                        )
                    }
                }
            } else if matches!(variant.fields, Fields::Unit) {
                quote! {
                    #krate::doc::schema::object_with(
                        ::std::vec![(
                            #tag_lit.to_string(),
                            #krate::doc::schema::string_enum(#variant_lit),
                        )],
                        ::std::vec![#tag_lit.to_string()],
                    )
                }
            } else {
                quote! {
                    #krate::doc::schema::with_properties(
                        #payload,
                        ::std::vec![(
                            #tag_lit.to_string(),
                            #krate::doc::schema::string_enum(#variant_lit),
                        )],
                        ::std::vec![#tag_lit.to_string()],
                    )
                }
            }
        } else {
            quote! {
                #krate::doc::schema::object_with(
                    ::std::vec![(#variant_lit.to_string(), #payload)],
                    ::std::vec![#variant_lit.to_string()],
                )
            }
        };

        variants.push(quote! {
            {
                let schema: #krate::utoipa::openapi::RefOr<
                    #krate::utoipa::openapi::schema::Schema,
                > = #schema;
                schema
            }
        });
        validation_arms.push(variant_validation(variant_index, variant, container)?);
    }

    let discriminator = match (&container.tag, &container.content) {
        (Some(tag), None) => {
            let tag = LitStr::new(tag, proc_macro2::Span::call_site());
            quote! {
                one_of = one_of.discriminator(Some(
                    #krate::utoipa::openapi::schema::Discriminator::new(#tag)
                ));
            }
        }
        _ => quote!(),
    };

    let register_body = quote! {
        #(#register_calls)*
        {
            let mut one_of = #krate::utoipa::openapi::schema::OneOfBuilder::new();
            #(one_of = one_of.item(#variants);)*
            #discriminator
            let base: #krate::utoipa::openapi::RefOr<
                #krate::utoipa::openapi::schema::Schema,
            > = one_of.into();
            components.schemas.insert(name.clone(), base);
        }
    };

    let validate_body = if validation_arms.is_empty() {
        quote!()
    } else {
        quote! {
            match self {
                #(#validation_arms),*
            }
        }
    };
    Ok((register_body, validate_body))
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
                if field_serde.skip {
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
                if field_serde.skip {
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

fn object_expr(fields: &Fields, container: &SerdeContainer) -> Result<TokenStream> {
    object_expr_with(fields, container, Vec::new())
}

fn object_expr_with(
    fields: &Fields,
    container: &SerdeContainer,
    extra: Vec<(String, TokenStream)>,
) -> Result<TokenStream> {
    let krate = crate::util::krate();
    let named = named_fields(fields)?;
    let mut properties = Vec::new();
    let mut required = Vec::new();
    let mut flatten = Vec::new();

    for (name, schema) in extra {
        properties.push(quote! {
            builder = builder.property(#name, #schema);
        });
        required.push(quote! {
            builder = builder.required(#name);
        });
    }

    for field in named {
        let ty = &field.ty;
        let field_serde = serde_field(&field.attrs)?;
        let field_schema = schema_attrs(&field.attrs)?;
        if field_serde.skip {
            continue;
        }
        let name = field_name(field, &field_serde, container.rename_all)?;
        let schema_expr = field_schema_expr(ty, &field_schema.constraints);
        if field_serde.flatten {
            flatten.push(schema_expr);
            continue;
        }
        properties.push(quote! {
            builder = builder.property(#name, #schema_expr);
        });
        let optional = crate::util::is_option(ty).is_some();
        if !optional && !field_serde.default && !field_serde.skip_serializing_if {
            required.push(quote! {
                builder = builder.required(#name);
            });
        }
    }

    let deny = if container.deny_unknown_fields {
        quote! {
            builder = builder.additional_properties(Some(
                #krate::utoipa::openapi::schema::AdditionalProperties::FreeForm(false)
            ));
        }
    } else {
        quote!()
    };

    let base = if flatten.is_empty() {
        quote! {
            {
                let base: #krate::utoipa::openapi::RefOr<
                    #krate::utoipa::openapi::schema::Schema,
                > = builder.into();
                base
            }
        }
    } else {
        quote! {
            {
                let base: #krate::utoipa::openapi::RefOr<
                    #krate::utoipa::openapi::schema::Schema,
                > = builder.into();
                let mut all = #krate::utoipa::openapi::schema::AllOfBuilder::new();
                all = all.item(base);
                #(all = all.item(#flatten);)*
                let combined: #krate::utoipa::openapi::RefOr<
                    #krate::utoipa::openapi::schema::Schema,
                > = #krate::utoipa::openapi::RefOr::T(
                    #krate::utoipa::openapi::schema::Schema::AllOf(all.build())
                );
                combined
            }
        }
    };

    Ok(quote! {
        {
            let mut builder = #krate::doc::schema::object();
            #(#properties)*
            #(#required)*
            #deny
            #base
        }
    })
}

fn variant_payload(fields: &Fields, container: &SerdeContainer) -> Result<TokenStream> {
    let krate = crate::util::krate();
    match fields {
        Fields::Unit => Ok(quote! {
            {
                let schema: #krate::utoipa::openapi::RefOr<
                    #krate::utoipa::openapi::schema::Schema,
                > = #krate::utoipa::openapi::schema::Object::with_type(
                    #krate::utoipa::openapi::schema::Type::Null
                ).into();
                schema
            }
        }),
        Fields::Named(_) => object_expr(fields, container),
        Fields::Unnamed(unnamed) => {
            if unnamed.unnamed.len() == 1 {
                let ty = &unnamed.unnamed[0].ty;
                Ok(quote! {
                    <#ty as #krate::doc::ApiSchema>::schema()
                })
            } else {
                Ok(quote! {
                    {
                        let schema: #krate::utoipa::openapi::RefOr<
                            #krate::utoipa::openapi::schema::Schema,
                        > = #krate::utoipa::openapi::schema::Object::with_type(
                            #krate::utoipa::openapi::schema::Type::Array
                        ).into();
                        schema
                    }
                })
            }
        }
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

fn field_name(field: &syn::Field, serde: &SerdeField, rule: Option<RenameRule>) -> Result<String> {
    if let Some(rename) = &serde.rename {
        return Ok(rename.clone());
    }
    let ident = field
        .ident
        .as_ref()
        .ok_or_else(|| syn::Error::new_spanned(field, "expected a named field"))?;
    let raw = ident.to_string();
    let raw = raw.strip_prefix("r#").unwrap_or(&raw);
    Ok(rule
        .map(|rule| rule.apply(raw))
        .unwrap_or_else(|| raw.to_string()))
}

pub(crate) fn field_schema_expr(ty: &Type, constraints: &Constraints) -> TokenStream {
    let krate = crate::util::krate();
    let mut calls = TokenStream::new();
    if let Some(value) = &constraints.min_length {
        calls.extend(quote! { schema = #krate::doc::schema::with_min_length(schema, (#value) as usize); });
    }
    if let Some(value) = &constraints.max_length {
        calls.extend(quote! { schema = #krate::doc::schema::with_max_length(schema, (#value) as usize); });
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
        calls.extend(quote! { schema = #krate::doc::schema::with_min_items(schema, (#value) as usize); });
    }
    if let Some(value) = &constraints.max_items {
        calls.extend(quote! { schema = #krate::doc::schema::with_max_items(schema, (#value) as usize); });
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

    let (inner_ty, optional) = match crate::util::is_option(ty) {
        Some(inner) => (inner, true),
        None => (ty.clone(), false),
    };
    let is_vec = crate::util::is_vec(&inner_ty).is_some();

    let mut checks = TokenStream::new();
    if let Some(value) = &constraints.min_length {
        checks.extend(quote! {
            if value.chars().count() < (#value) as usize {
                errors.push(#krate::validate::ValidationError::new(
                    #pointer_ident.clone(),
                    "min_length",
                    format!("must be at least {} character(s)", #value),
                ).with_limit(#krate::serde_json::json!(#value)));
            }
        });
    }
    if let Some(value) = &constraints.max_length {
        checks.extend(quote! {
            if value.chars().count() > (#value) as usize {
                errors.push(#krate::validate::ValidationError::new(
                    #pointer_ident.clone(),
                    "max_length",
                    format!("must be at most {} character(s)", #value),
                ).with_limit(#krate::serde_json::json!(#value)));
            }
        });
    }
    if let Some(value) = &constraints.pattern {
        let static_ident = format_ident!("__ROUNDHOUSE_PATTERN_{index}");
        checks.extend(quote! {
            static #static_ident: ::std::sync::OnceLock<#krate::regex::Regex> =
                ::std::sync::OnceLock::new();
            let pattern = #static_ident.get_or_init(|| {
                #krate::regex::Regex::new(#value).expect("invalid #[schema(pattern)]")
            });
            if !pattern.is_match(value) {
                errors.push(#krate::validate::ValidationError::new(
                    #pointer_ident.clone(),
                    "pattern",
                    format!("must match the pattern `{}`", #value),
                ));
            }
        });
    }
    if let Some(value) = &constraints.minimum {
        checks.extend(quote! {
            if ((*value) as f64) < ((#value) as f64) {
                errors.push(#krate::validate::ValidationError::new(
                    #pointer_ident.clone(),
                    "minimum",
                    format!("must be greater than or equal to {}", #value),
                ).with_limit(#krate::serde_json::json!(#value)));
            }
        });
    }
    if let Some(value) = &constraints.maximum {
        checks.extend(quote! {
            if ((*value) as f64) > ((#value) as f64) {
                errors.push(#krate::validate::ValidationError::new(
                    #pointer_ident.clone(),
                    "maximum",
                    format!("must be less than or equal to {}", #value),
                ).with_limit(#krate::serde_json::json!(#value)));
            }
        });
    }
    if let Some(value) = &constraints.exclusive_minimum {
        checks.extend(quote! {
            if ((*value) as f64) <= ((#value) as f64) {
                errors.push(#krate::validate::ValidationError::new(
                    #pointer_ident.clone(),
                    "exclusive_minimum",
                    format!("must be greater than {}", #value),
                ).with_limit(#krate::serde_json::json!(#value)));
            }
        });
    }
    if let Some(value) = &constraints.exclusive_maximum {
        checks.extend(quote! {
            if ((*value) as f64) >= ((#value) as f64) {
                errors.push(#krate::validate::ValidationError::new(
                    #pointer_ident.clone(),
                    "exclusive_maximum",
                    format!("must be less than {}", #value),
                ).with_limit(#krate::serde_json::json!(#value)));
            }
        });
    }
    if let Some(format) = &constraints.format {
        let format_value = format.value();
        let check = match format_value.as_str() {
            "email" => quote! {
                {
                    let text: &str = ::std::convert::AsRef::<str>::as_ref(value);
                    let valid = match text.split_once('@') {
                        Some((local, domain)) => {
                            !local.is_empty()
                                && !domain.is_empty()
                                && !domain.starts_with('.')
                                && !domain.ends_with('.')
                                && domain.contains('.')
                        }
                        None => false,
                    };
                    if !valid {
                        errors.push(#krate::validate::ValidationError::new(
                            #pointer_ident.clone(),
                            "format",
                            "must be a valid email address",
                        ));
                    }
                }
            },
            "uuid" => quote! {
                {
                    let text: &str = ::std::convert::AsRef::<str>::as_ref(value);
                    let bytes = text.as_bytes();
                    let valid = bytes.len() == 36
                        && bytes[8] == b'-'
                        && bytes[13] == b'-'
                        && bytes[18] == b'-'
                        && bytes[23] == b'-'
                        && bytes
                            .iter()
                            .enumerate()
                            .all(|(index, byte)| {
                                matches!(index, 8 | 13 | 18 | 23) || byte.is_ascii_hexdigit()
                            });
                    if !valid {
                        errors.push(#krate::validate::ValidationError::new(
                            #pointer_ident.clone(),
                            "format",
                            "must be a valid UUID",
                        ));
                    }
                }
            },
            "url" | "uri" => quote! {
                {
                    let text: &str = ::std::convert::AsRef::<str>::as_ref(value);
                    if !(text.starts_with("http://") || text.starts_with("https://")) {
                        errors.push(#krate::validate::ValidationError::new(
                            #pointer_ident.clone(),
                            "format",
                            "must be a valid URL",
                        ));
                    }
                }
            },
            _ => quote!(),
        };
        checks.extend(check);
    }

    if is_vec {
        if let Some(value) = &constraints.min_items {
            checks.extend(quote! {
                if value.len() < (#value) as usize {
                    errors.push(#krate::validate::ValidationError::new(
                        #pointer_ident.clone(),
                        "min_items",
                        format!("must contain at least {} item(s)", #value),
                    ).with_limit(#krate::serde_json::json!(#value)));
                }
            });
        }
        if let Some(value) = &constraints.max_items {
            checks.extend(quote! {
                if value.len() > (#value) as usize {
                    errors.push(#krate::validate::ValidationError::new(
                        #pointer_ident.clone(),
                        "max_items",
                        format!("must contain at most {} item(s)", #value),
                    ).with_limit(#krate::serde_json::json!(#value)));
                }
            });
        }
        if constraints.unique_items {
            checks.extend(quote! {
                {
                    let mut seen = ::std::collections::HashSet::new();
                    for item in value.iter() {
                        if !seen.insert(item) {
                            errors.push(#krate::validate::ValidationError::new(
                                #pointer_ident.clone(),
                                "unique_items",
                                "items must be unique",
                            ));
                            break;
                        }
                    }
                }
            });
        }
    }

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
                #checks
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
                #checks
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
