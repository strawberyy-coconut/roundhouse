//! The subset of serde's attribute language the derives need.
//!
//! `ApiSchema`, `QueryParams` and `PathParams` are three views of the same wire shape. Keeping
//! one parser here is what stops the body document from honoring `rename_all` while a query or
//! path document silently does not.

use syn::{Expr, LitStr, Result};

#[derive(Default, Clone)]
pub(crate) struct SerdeContainer {
    pub(crate) rename_all: Option<RenameRule>,
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

    pub(crate) fn apply(self, name: &str) -> String {
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
        .map(|rule| rule.apply(raw))
        .unwrap_or_else(|| raw.to_string()))
}
