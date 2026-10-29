//! Migrate the utoipa-axum source shape to roundhouse.
//!
//! The transformation is deliberately local: `#[utoipa::path(..)]` becomes `#[handler]`, the
//! metadata in that attribute becomes a `routes!` entry in the same module, and utoipa-axum's
//! `.routes(routes!(..))` call is removed from the router expression. The one-time router
//! construction still needs an `App::new(..)`/`mount!` rewrite, but every handler moves
//! mechanically.

use quote::quote;
use syn::punctuated::Punctuated;
use syn::visit_mut::{self, VisitMut};
use syn::{Attribute, Expr, ExprLit, Ident, Item, ItemFn, ItemMacro, Lit, LitStr, Meta, Token};

/// The result of migrating one file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Migration {
    pub source: String,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
struct Entry {
    method: Ident,
    path: String,
    handler: Ident,
    operation_id: Option<String>,
    summary: Option<String>,
    description: Option<String>,
    tags: Vec<String>,
}

/// Rewrite one Rust source file. The returned source is parseable Rust.
pub fn migrate(source: &str) -> Result<Migration, syn::Error> {
    let mut file: syn::File = syn::parse_str(source)?;
    let mut warnings = Vec::new();

    RemoveUtoipaRoutes.visit_file_mut(&mut file);

    let entries = migrate_items(&mut file.items, "", &mut warnings)?;
    if !entries.is_empty() {
        add_import(&mut file.items);
        file.items.push(syn::Item::Macro(routes_item(&entries)?));
    }

    Ok(Migration {
        source: prettyplease::unparse(&file),
        warnings,
    })
}

#[allow(clippy::ptr_arg)]
fn migrate_items(
    items: &mut Vec<Item>,
    module: &str,
    warnings: &mut Vec<String>,
) -> Result<Vec<Entry>, syn::Error> {
    let mut entries = Vec::new();
    for item in items.iter_mut() {
        match item {
            Item::Fn(function) => {
                if let Some(entry) = migrate_function(function, module, warnings)? {
                    entries.push(entry);
                }
            }
            Item::Mod(module_item) => {
                let Some((_, children)) = &mut module_item.content else {
                    continue;
                };
                let name = module_item.ident.to_string();
                let path = if module.is_empty() {
                    name
                } else {
                    format!("{module}::{name}")
                };
                let nested = migrate_items(children, &path, warnings)?;
                if !nested.is_empty() {
                    add_import(children);
                    children.push(syn::Item::Macro(routes_item(&nested)?));
                }
            }
            _ => {}
        }
    }
    Ok(entries)
}

fn migrate_function(
    function: &mut ItemFn,
    module: &str,
    warnings: &mut Vec<String>,
) -> Result<Option<Entry>, syn::Error> {
    let Some(index) = function.attrs.iter().position(is_utoipa_path) else {
        return Ok(None);
    };

    let attribute = function.attrs[index].clone();
    let handler = function.sig.ident.clone();
    let where_ = if module.is_empty() {
        handler.to_string()
    } else {
        format!("{module}::{handler}")
    };

    let mut method = None;
    let mut path = None;
    let mut context_path = None;
    let mut operation_id = None;
    let mut summary = None;
    let mut description = None;
    let mut tags = Vec::new();
    let mut skipped = Vec::new();

    let metas = attribute.parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)?;
    for meta in metas {
        match &meta {
            Meta::Path(value) => {
                let name = last_ident(value);
                if is_method(&name) {
                    method = Some(Ident::new(&name, proc_macro2::Span::call_site()));
                }
            }
            Meta::NameValue(value) => {
                let name = last_ident(&value.path);
                let text = string_value(&value.value);
                match name.as_str() {
                    "path" => path = text,
                    "context_path" => context_path = text,
                    "operation_id" => operation_id = text,
                    "summary" => summary = text,
                    "description" => description = text,
                    "tag" => {
                        if let Some(text) = text {
                            tags.push(text);
                        }
                    }
                    "tags" => tags.extend(string_array(&value.value)),
                    "security" | "params" | "request_body" | "responses" => {
                        skipped.push(name);
                    }
                    _ => {}
                }
            }
            Meta::List(value) => {
                let name = last_ident(&value.path);
                if matches!(
                    name.as_str(),
                    "security" | "params" | "request_body" | "responses"
                ) {
                    skipped.push(name);
                }
            }
        }
    }

    let Some(method) = method else {
        warnings.push(format!(
            "{where_}: #[utoipa::path] has no HTTP method; left unchanged"
        ));
        return Ok(None);
    };
    let Some(path) = path else {
        warnings.push(format!(
            "{where_}: #[utoipa::path] has no `path = \"...\"`; left unchanged"
        ));
        return Ok(None);
    };

    let path = match context_path {
        Some(context) => join_paths(&context, &path),
        None => path,
    };

    if !skipped.is_empty() {
        skipped.sort();
        skipped.dedup();
        warnings.push(format!(
            "{where_}: not migrated automatically: {}",
            skipped.join(", ")
        ));
    }

    function.attrs[index] = syn::parse_quote!(#[handler]);

    Ok(Some(Entry {
        method,
        path,
        handler,
        operation_id,
        summary,
        description,
        tags,
    }))
}

fn routes_item(entries: &[Entry]) -> Result<ItemMacro, syn::Error> {
    let entries = entries.iter().map(|entry| {
        let method = &entry.method;
        let path = LitStr::new(&entry.path, proc_macro2::Span::call_site());
        let handler = &entry.handler;
        let mut extras = Vec::new();
        if let Some(value) = &entry.operation_id {
            let value = LitStr::new(value, proc_macro2::Span::call_site());
            extras.push(quote!(operation_id = #value));
        }
        if let Some(value) = &entry.summary {
            let value = LitStr::new(value, proc_macro2::Span::call_site());
            extras.push(quote!(summary = #value));
        }
        if let Some(value) = &entry.description {
            let value = LitStr::new(value, proc_macro2::Span::call_site());
            extras.push(quote!(description = #value));
        }
        if !entry.tags.is_empty() {
            let values = entry
                .tags
                .iter()
                .map(|tag| LitStr::new(tag, proc_macro2::Span::call_site()));
            extras.push(quote!(tags = [#(#values),*]));
        }
        quote!(#method(#path, #handler #(, #extras)*),)
    });
    syn::parse2(quote! {
        routes! {
            #(#entries)*
        }
    })
}

fn add_import(items: &mut Vec<Item>) {
    let already = items.iter().any(|item| {
        let Item::Use(item) = item else {
            return false;
        };
        let text = quote!(#item).to_string();
        text.contains("roundhouse") && text.contains("prelude")
    });
    if !already {
        let import: Item = syn::parse_quote!(
            use roundhouse::prelude::*;
        );
        items.insert(0, import);
    }
}

fn is_utoipa_path(attribute: &Attribute) -> bool {
    let segments = &attribute.path().segments;
    segments
        .last()
        .is_some_and(|segment| segment.ident == "path")
        && segments.len() >= 2
}

fn last_ident(path: &syn::Path) -> String {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_default()
}

fn is_method(name: &str) -> bool {
    matches!(
        name,
        "get" | "post" | "put" | "patch" | "delete" | "head" | "options" | "trace"
    )
}

fn string_value(expression: &Expr) -> Option<String> {
    let Expr::Lit(ExprLit {
        lit: Lit::Str(value),
        ..
    }) = expression
    else {
        return None;
    };
    Some(value.value())
}

fn string_array(expression: &Expr) -> Vec<String> {
    let Expr::Array(array) = expression else {
        return Vec::new();
    };
    array.elems.iter().filter_map(string_value).collect()
}

fn join_paths(prefix: &str, path: &str) -> String {
    let mut pieces: Vec<&str> = Vec::new();
    for piece in prefix.split('/').chain(path.split('/')) {
        if !piece.is_empty() {
            pieces.push(piece);
        }
    }
    if pieces.is_empty() {
        "/".to_string()
    } else {
        format!("/{}", pieces.join("/"))
    }
}

/// Remove utoipa-axum's `.routes(routes!(..))` from a router expression.
struct RemoveUtoipaRoutes;

impl VisitMut for RemoveUtoipaRoutes {
    fn visit_expr_mut(&mut self, expression: &mut Expr) {
        if let Expr::MethodCall(call) = expression
            && call.method == "routes"
            && call.args.len() == 1
            && is_routes_macro(&call.args[0])
        {
            *expression = (*call.receiver).clone();
            return;
        }
        visit_mut::visit_expr_mut(self, expression);
    }
}

fn is_routes_macro(expression: &Expr) -> bool {
    let Expr::Macro(mac) = expression else {
        return false;
    };
    mac.mac
        .path
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "routes")
}

/// Whether the output still contains an utoipa-axum marker. Used by the CLI's `--check` mode.
pub fn looks_unmigrated(source: &str) -> bool {
    source.contains("utoipa::path") || source.contains("utoipa_axum")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_an_annotated_handler_and_its_metadata() {
        let source = r#"
use utoipa_axum::{router::OpenApiRouter, routes};

#[utoipa::path(
    get,
    path = "/pets/{id}",
    operation_id = "get_pet",
    summary = "Read a pet",
    description = "One pet by id",
    tag = "pets",
)]
pub async fn show() -> &'static str { "pet" }

pub fn router() -> OpenApiRouter {
    OpenApiRouter::new().routes(routes!(show))
}
"#;
        let migration = migrate(source).expect("valid source");
        let output = migration.source;
        assert!(output.contains("#[handler]"), "{output}");
        assert!(!output.contains("#[utoipa::path"), "{output}");
        assert!(!output.contains("routes!(show)"), "{output}");
        assert!(output.contains("routes! {"), "{output}");
        assert!(output.contains("get(\"/pets/{id}\", show"), "{output}");
        assert!(output.contains("operation_id = \"get_pet\""), "{output}");
        assert!(output.contains("summary"), "{output}");
        assert!(output.contains("\"Read a pet\""), "{output}");
        assert!(output.contains("description"), "{output}");
        assert!(output.contains("\"One pet by id\""), "{output}");
        assert!(output.contains("tags = [\"pets\"]"), "{output}");
        syn::parse_file(&output).expect("output parses");
    }

    #[test]
    fn joins_context_path_and_path() {
        let source = r#"
#[utoipa::path(get, context_path = "/api", path = "/pets")]
pub async fn list() -> &'static str { "pets" }
"#;
        let output = migrate(source).unwrap().source;
        assert!(output.contains("get(\"/api/pets\", list"), "{output}");
    }

    #[test]
    fn handles_nested_modules() {
        let source = r#"
mod pets {
    #[utoipa::path(get, path = "/pets")]
    pub async fn list() -> &'static str { "pets" }
}
"#;
        let output = migrate(source).unwrap().source;
        assert!(output.contains("mod pets {"), "{output}");
        assert!(output.contains("get(\"/pets\", list)"), "{output}");
        syn::parse_file(&output).expect("output parses");
    }

    #[test]
    fn is_idempotent_once_the_attributes_are_gone() {
        let source = r#"
#[utoipa::path(get, path = "/pets")]
pub async fn list() -> &'static str { "pets" }
"#;
        let once = migrate(source).unwrap().source;
        let twice = migrate(&once).unwrap().source;
        assert_eq!(once, twice);
    }
}
