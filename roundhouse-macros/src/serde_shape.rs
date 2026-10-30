//! The subset of serde's attribute language the derives need.
//!
//! `ApiSchema`, `QueryParams` and `PathParams` are three views of the same wire shape. Keeping
//! one parser here is what stops the body document from honoring `rename_all` while a query or
//! path document silently does not.

use syn::{Expr, LitStr, Result};

#[derive(Default, Clone)]
pub(crate) struct SerdeContainer {
    pub(crate) rename_all: Option<RenameRule>,
    /// `#[serde(rename_all_fields = "...")]`: the default field rule for struct variants.
    pub(crate) rename_all_fields: Option<RenameRule>,
    pub(crate) tag: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) untagged: bool,
    pub(crate) deny_unknown_fields: bool,
    /// `#[serde(default)]` at the container level: every field has a default.
    pub(crate) default: bool,
}

#[derive(Default, Clone)]
pub(crate) struct SerdeField {
    pub(crate) rename: Option<String>,
    /// `#[serde(skip)]`: neither serialized nor deserialized.
    pub(crate) skip: bool,
    pub(crate) skip_serializing: bool,
    pub(crate) skip_serializing_if: bool,
    pub(crate) skip_deserializing: bool,
    pub(crate) default: bool,
    pub(crate) flatten: bool,
}

impl SerdeField {
    /// Whether a document of the serialized shape should omit this field.
    pub(crate) fn hidden(&self) -> bool {
        self.skip || self.skip_serializing || self.skip_deserializing
    }

    /// Whether deserialization ignores this field entirely.
    pub(crate) fn skipped_on_input(&self) -> bool {
        self.skip || self.skip_deserializing
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenameRule {
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

    /// serde renames struct fields with slightly different rules than enum variants. In
    /// particular, `lowercase` and `snake_case` are identities for a field; they are not for a
    /// variant. Keep the two algorithms separate so a document cannot drift from the derive.
    pub(crate) fn apply_to_field(self, field: &str) -> String {
        match self {
            RenameRule::Lower | RenameRule::Snake => field.to_owned(),
            RenameRule::Upper => field.to_ascii_uppercase(),
            RenameRule::Pascal => {
                let mut pascal = String::new();
                let mut capitalize = true;
                for ch in field.chars() {
                    if ch == '_' {
                        capitalize = true;
                    } else if capitalize {
                        pascal.push(ch.to_ascii_uppercase());
                        capitalize = false;
                    } else {
                        pascal.push(ch);
                    }
                }
                pascal
            }
            RenameRule::Camel => {
                let pascal = RenameRule::Pascal.apply_to_field(field);
                match pascal.len() {
                    0 => pascal,
                    _ => pascal[..1].to_ascii_lowercase() + pascal.get(1..).unwrap_or_default(),
                }
            }
            RenameRule::ScreamingSnake => field.to_ascii_uppercase(),
            RenameRule::Kebab => field.replace('_', "-"),
            RenameRule::ScreamingKebab => RenameRule::ScreamingSnake
                .apply_to_field(field)
                .replace('_', "-"),
        }
    }

    pub(crate) fn apply_to_variant(self, variant: &str) -> String {
        match self {
            RenameRule::Lower => variant.to_ascii_lowercase(),
            RenameRule::Upper => variant.to_ascii_uppercase(),
            RenameRule::Camel => match variant.len() {
                0 => String::new(),
                _ => variant[..1].to_ascii_lowercase() + variant.get(1..).unwrap_or_default(),
            },
            RenameRule::Pascal => variant.to_owned(),
            RenameRule::Snake => {
                let mut snake = String::new();
                for (index, ch) in variant.char_indices() {
                    if index > 0 && ch.is_uppercase() {
                        snake.push('_');
                    }
                    snake.push(ch.to_ascii_lowercase());
                }
                snake
            }
            RenameRule::ScreamingSnake => RenameRule::Snake
                .apply_to_variant(variant)
                .to_ascii_uppercase(),
            RenameRule::Kebab => RenameRule::Snake
                .apply_to_variant(variant)
                .replace('_', "-"),
            RenameRule::ScreamingKebab => RenameRule::ScreamingSnake
                .apply_to_variant(variant)
                .replace('_', "-"),
        }
    }
}

pub(crate) fn container(attrs: &[syn::Attribute]) -> Result<SerdeContainer> {
    let mut container = SerdeContainer::default();
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename_all") {
                let value: LitStr = meta.value()?.parse()?;
                container.rename_all = Some(RenameRule::parse(&value.value())?);
            } else if meta.path.is_ident("rename_all_fields") {
                let value: LitStr = meta.value()?.parse()?;
                container.rename_all_fields = Some(RenameRule::parse(&value.value())?);
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
            } else if meta.path.is_ident("default") {
                container.default = true;
                consume_optional_value(&meta)?;
            }
            Ok(())
        })?;
    }
    Ok(container)
}

pub(crate) fn field(attrs: &[syn::Attribute]) -> Result<SerdeField> {
    let mut field = SerdeField::default();
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                let value: LitStr = meta.value()?.parse()?;
                field.rename = Some(value.value());
            } else if meta.path.is_ident("skip") {
                field.skip = true;
            } else if meta.path.is_ident("skip_serializing") {
                field.skip_serializing = true;
            } else if meta.path.is_ident("skip_deserializing") {
                field.skip_deserializing = true;
            } else if meta.path.is_ident("skip_serializing_if") {
                let _: Expr = meta.value()?.parse()?;
                field.skip_serializing_if = true;
            } else if meta.path.is_ident("default") {
                field.default = true;
                consume_optional_value(&meta)?;
            } else if meta.path.is_ident("flatten") {
                field.flatten = true;
            }
            Ok(())
        })?;
    }
    Ok(field)
}

/// `#[serde(default = "path")]` carries a value; plain `#[serde(default)]` does not.
fn consume_optional_value(meta: &syn::meta::ParseNestedMeta<'_>) -> Result<()> {
    if meta.input.peek(syn::Token![=]) {
        let _: Expr = meta.value()?.parse()?;
    }
    Ok(())
}

pub(crate) fn field_name(
    field: &syn::Field,
    serde: &SerdeField,
    rule: Option<RenameRule>,
) -> Result<String> {
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
        .map(|rule| rule.apply_to_field(raw))
        .unwrap_or_else(|| raw.to_string()))
}

/// Resolve an enum variant's wire name. A variant-level `rename` wins, then the container's
/// `rename_all` rule, then the Rust identifier.
pub(crate) fn variant_name(
    variant: &syn::Variant,
    serde: &SerdeField,
    rule: Option<RenameRule>,
) -> String {
    if let Some(rename) = &serde.rename {
        return rename.clone();
    }
    let raw = variant.ident.to_string();
    let raw = raw.strip_prefix("r#").unwrap_or(&raw);
    rule.map(|rule| rule.apply_to_variant(raw))
        .unwrap_or_else(|| raw.to_string())
}
