use syn::{Expr, LitStr, Result};

#[derive(Default, Clone)]
pub(crate) struct Constraints {
    pub(crate) min_length: Option<Expr>,
    pub(crate) max_length: Option<Expr>,
    pub(crate) pattern: Option<LitStr>,
    pub(crate) minimum: Option<Expr>,
    pub(crate) maximum: Option<Expr>,
    pub(crate) exclusive_minimum: Option<Expr>,
    pub(crate) exclusive_maximum: Option<Expr>,
    pub(crate) min_items: Option<Expr>,
    pub(crate) max_items: Option<Expr>,
    pub(crate) unique_items: bool,
    pub(crate) format: Option<LitStr>,
    pub(crate) example: Option<Expr>,
    pub(crate) validate_with: Option<syn::Path>,
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
