use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Expr, Ident, LitInt, LitStr, Path, Result, Token, Type};

use crate::util::{parse_security_types, parse_u16};

#[derive(Clone, Copy, Default)]
pub(super) enum LayerMode {
    #[default]
    Full,
    Route,
}

#[derive(Default)]
pub(super) struct ModuleOptions {
    pub(super) prefix: Option<LitStr>,
    pub(super) tags: Vec<LitStr>,
    pub(super) security: Vec<Type>,
    pub(super) layer: Option<Expr>,
    pub(super) layer_mode: LayerMode,
    pub(super) hidden: bool,
    pub(super) doc: Option<Expr>,
}

pub(super) struct Entry {
    pub(super) attrs: Vec<Attribute>,
    pub(super) method: Ident,
    pub(super) path: LitStr,
    pub(super) assert_path: Option<LitStr>,
    pub(super) handler: Path,
    pub(super) summary: Option<LitStr>,
    pub(super) description: Option<LitStr>,
    pub(super) operation_id: Option<LitStr>,
    pub(super) tags: Vec<LitStr>,
    pub(super) security: Vec<Type>,
    pub(super) responses: Vec<(u16, Option<Type>)>,
    pub(super) layer: Option<Expr>,
    pub(super) doc: Option<Expr>,
    pub(super) hidden: bool,
    pub(super) absolute: bool,
    pub(super) validate: bool,
    pub(super) deprecated: bool,
    pub(super) public: bool,
    pub(super) response_override: bool,
}

impl Entry {
    fn new(attrs: Vec<Attribute>, method: Ident, path: LitStr, handler: Path) -> Self {
        Self {
            attrs,
            method,
            path,
            assert_path: None,
            handler,
            summary: None,
            description: None,
            operation_id: None,
            tags: Vec::new(),
            security: Vec::new(),
            responses: Vec::new(),
            layer: None,
            doc: None,
            hidden: false,
            absolute: false,
            validate: true,
            deprecated: false,
            public: false,
            response_override: false,
        }
    }

    /// A top-level entry: `get("/path", handler, option = ..)` or `get "/path" => handler`.
    pub(super) fn parse_flat(input: ParseStream<'_>) -> Result<Self> {
        let attrs = Attribute::parse_outer(input)?;
        let method: Ident = input.parse()?;

        if input.peek(syn::token::Paren) {
            // `get("/path", handler, option = ..)`
            let content;
            syn::parenthesized!(content in input);
            let path: LitStr = content.parse()?;
            content.parse::<Token![,]>()?;
            let handler: Path = content.parse()?;
            let mut entry = Entry::new(attrs, method, path, handler);
            parse_entry_options(&content, &mut entry, false)?;
            Ok(entry)
        } else {
            // `get "/path" => handler, option = ..`
            let path: LitStr = input.parse()?;
            input.parse::<Token![=>]>()?;
            let handler: Path = input.parse()?;
            let mut entry = Entry::new(attrs, method, path, handler);
            parse_entry_options(input, &mut entry, true)?;
            Ok(entry)
        }
    }

    /// An entry inside `"/path" => { get(handler, option = ..) }`: the group supplies the path.
    pub(super) fn parse_grouped(input: ParseStream<'_>, path: LitStr) -> Result<Self> {
        let attrs = Attribute::parse_outer(input)?;
        let method: Ident = input.parse()?;
        let content;
        syn::parenthesized!(content in input);
        let handler: Path = content.parse()?;
        let mut entry = Entry::new(attrs, method, path, handler);
        parse_entry_options(&content, &mut entry, false)?;
        Ok(entry)
    }
}

#[derive(Default)]
pub(super) struct Body {
    pub(super) entries: Vec<Entry>,
    pub(super) groups: Vec<Group>,
    pub(super) spreads: Vec<Path>,
}

impl Body {
    pub(super) fn parse(input: ParseStream<'_>) -> Result<Self> {
        Self::parse_body(input, None)
    }

    fn parse_shared(input: ParseStream<'_>, path: LitStr) -> Result<Self> {
        Self::parse_body(input, Some(path))
    }

    fn parse_body(input: ParseStream<'_>, shared_path: Option<LitStr>) -> Result<Self> {
        let mut body = Body::default();

        while !input.is_empty() {
            let attrs = Attribute::parse_outer(input)?;

            if input.peek(Token![...]) {
                input.parse::<Token![...]>()?;
                body.spreads.push(input.parse::<Path>()?);
                if input.peek(Token![,]) {
                    input.parse::<Token![,]>()?;
                }
                continue;
            }

            if input.peek(LitStr) {
                let path: LitStr = input.parse()?;
                let shared_path = input.peek(Token![=>]);
                let options = if shared_path {
                    input.parse::<Token![=>]>()?;
                    ModuleOptions::default()
                } else {
                    parse_module_options(input)?
                };
                let content;
                syn::braced!(content in input);
                let group_body = if shared_path {
                    // `"/{id}" => { get(show) }` is one path shared by several methods.
                    Body::parse_shared(&content, path.clone())?
                } else {
                    // `"/notes" [tags = ..] { get "/" => list }` is a prefix group.
                    Body::parse(&content)?
                };
                let mut group = Group {
                    path,
                    options,
                    shared_path,
                    body: group_body,
                };
                // In a group, attributes written before the path apply to every entry inside.
                for entry in &mut group.body.entries {
                    entry.attrs.splice(0..0, attrs.clone());
                }
                body.groups.push(group);
            } else {
                let mut entry = match &shared_path {
                    Some(path) => Entry::parse_grouped(input, path.clone())?,
                    None => Entry::parse_flat(input)?,
                };
                entry.attrs.splice(0..0, attrs);
                body.entries.push(entry);
            }

            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }

        Ok(body)
    }
}

pub(super) struct Group {
    pub(super) path: LitStr,
    pub(super) options: ModuleOptions,
    /// `"/path" => { get(handler) }`: the group supplies the full entry path.
    pub(super) shared_path: bool,
    pub(super) body: Body,
}

pub struct RoutesInput {
    pub(super) options: ModuleOptions,
    pub(super) body: Body,
}

impl Parse for RoutesInput {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        // Root options come before the first entry:
        //
        //     routes! {
        //         prefix = "/api",
        //         tags = ["api"],
        //         "/notes" [tags = ["notes"]] { ... }
        //     }
        //
        // A bare `"/api" { ... }` is the same tree with an anonymous root; group options live in
        // the brackets.
        let options = parse_root_options(input)?;
        let body = Body::parse(input)?;
        Ok(Self { options, body })
    }
}

fn parse_root_options(input: ParseStream<'_>) -> Result<ModuleOptions> {
    let mut options = ModuleOptions::default();
    while input.peek(Ident) {
        let fork = input.fork();
        let key: Ident = fork.call(Ident::parse_any)?;
        let name = key.to_string();
        let is_flag = name == "hidden";
        let is_key = matches!(
            name.as_str(),
            "prefix" | "tags" | "security" | "layer" | "layer_mode" | "doc"
        );
        if !is_flag && !is_key {
            break;
        }
        if !is_flag && !fork.peek(Token![=]) {
            break;
        }

        let key: Ident = input.call(Ident::parse_any)?;
        parse_module_key(input, &mut options, key)?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(options)
}

const ENTRY_KEYS: &[&str] = &[
    "summary",
    "description",
    "operation_id",
    "path",
    "tags",
    "security",
    "responses",
    "layer",
    "doc",
    "hidden",
    "absolute",
    "public",
    "deprecated",
    "override",
    "validate",
];

fn parse_entry_options(
    content: ParseStream<'_>,
    entry: &mut Entry,
    stop_at_unknown: bool,
) -> Result<()> {
    while content.peek(Token![,]) {
        if stop_at_unknown {
            let fork = content.fork();
            fork.parse::<Token![,]>()?;
            if fork.is_empty() {
                content.parse::<Token![,]>()?;
                break;
            }
            let key: Ident = match fork.call(Ident::parse_any) {
                Ok(key) => key,
                Err(_) => break,
            };
            // A known option is ours. An unknown identifier followed by `=` is still an option
            // (report it as unknown); anything else starts the next entry.
            if !ENTRY_KEYS.contains(&key.to_string().as_str()) && !fork.peek(Token![=]) {
                break;
            }
        }

        content.parse::<Token![,]>()?;
        if content.is_empty() {
            break;
        }
        let key: Ident = content.call(Ident::parse_any)?;
        parse_entry_key(content, entry, key)?;
    }
    Ok(())
}

fn parse_entry_key(content: ParseStream<'_>, entry: &mut Entry, key: Ident) -> Result<()> {
    match key.to_string().as_str() {
        "summary" => {
            content.parse::<Token![=]>()?;
            entry.summary = Some(content.parse()?);
        }
        "description" => {
            content.parse::<Token![=]>()?;
            entry.description = Some(content.parse()?);
        }
        "operation_id" => {
            content.parse::<Token![=]>()?;
            entry.operation_id = Some(content.parse()?);
        }
        "path" => {
            content.parse::<Token![=]>()?;
            entry.assert_path = Some(content.parse()?);
        }
        "tags" => {
            content.parse::<Token![=]>()?;
            entry.tags = parse_string_list(content)?;
        }
        "security" => {
            content.parse::<Token![=]>()?;
            entry.security = parse_security_types(content)?;
        }
        "responses" => {
            let responses;
            syn::parenthesized!(responses in content);
            entry.responses = parse_responses(&responses)?;
        }
        "layer" => {
            content.parse::<Token![=]>()?;
            entry.layer = Some(content.parse()?);
        }
        "doc" => {
            content.parse::<Token![=]>()?;
            entry.doc = Some(content.parse()?);
        }
        "hidden" => entry.hidden = true,
        "absolute" => entry.absolute = true,
        "public" => entry.public = true,
        "deprecated" => entry.deprecated = true,
        "override" => entry.response_override = true,
        "validate" => {
            content.parse::<Token![=]>()?;
            let value: syn::LitBool = content.parse()?;
            entry.validate = value.value;
        }
        other => {
            return Err(syn::Error::new(
                key.span(),
                format!("unknown routes! attribute `{other}`"),
            ));
        }
    }
    Ok(())
}

/// Parse `[tags = .., security = .., layer = .., ...]` after a group's path.
fn parse_module_options(input: ParseStream<'_>) -> Result<ModuleOptions> {
    if input.peek(syn::token::Bracket) {
        let content;
        syn::bracketed!(content in input);
        let options = parse_module_options_into(&content, ModuleOptions::default())?;
        if let Some(prefix) = &options.prefix {
            return Err(syn::Error::new(
                prefix.span(),
                "a group's prefix is its path; remove `prefix = ..` from the brackets",
            ));
        }
        return Ok(options);
    }
    Ok(ModuleOptions::default())
}

fn parse_module_options_into(
    input: ParseStream<'_>,
    mut options: ModuleOptions,
) -> Result<ModuleOptions> {
    while !input.is_empty() {
        let key: Ident = input.parse()?;
        parse_module_key(input, &mut options, key)?;
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(options)
}

fn parse_module_key(input: ParseStream<'_>, options: &mut ModuleOptions, key: Ident) -> Result<()> {
    match key.to_string().as_str() {
        "prefix" => {
            input.parse::<Token![=]>()?;
            options.prefix = Some(input.parse()?);
        }
        "tags" => {
            input.parse::<Token![=]>()?;
            options.tags = parse_string_list(input)?;
        }
        "security" => {
            input.parse::<Token![=]>()?;
            options.security = parse_security_types(input)?;
        }
        "layer" => {
            input.parse::<Token![=]>()?;
            options.layer = Some(input.parse()?);
        }
        "layer_mode" => {
            input.parse::<Token![=]>()?;
            let mode: Ident = input.parse()?;
            options.layer_mode = match mode.to_string().as_str() {
                "full" => LayerMode::Full,
                "route" => LayerMode::Route,
                other => {
                    return Err(syn::Error::new(
                        mode.span(),
                        format!("expected `full` or `route`, found `{other}`"),
                    ));
                }
            };
        }
        "hidden" => options.hidden = true,
        "doc" => {
            input.parse::<Token![=]>()?;
            options.doc = Some(input.parse()?);
        }
        other => {
            return Err(syn::Error::new(
                key.span(),
                format!("unknown module attribute `{other}`"),
            ));
        }
    }
    Ok(())
}

fn parse_string_list(input: ParseStream<'_>) -> Result<Vec<LitStr>> {
    let content;
    syn::bracketed!(content in input);
    Ok(content
        .parse_terminated(|input| input.parse::<LitStr>(), Token![,])?
        .into_iter()
        .collect())
}

fn parse_responses(input: ParseStream<'_>) -> Result<Vec<(u16, Option<Type>)>> {
    let mut responses = Vec::new();
    while !input.is_empty() {
        let literal: LitInt = input.parse()?;
        let status = parse_u16(&literal)?;
        let body = if input.peek(Token![=]) {
            input.parse::<Token![=]>()?;
            Some(input.parse::<Type>()?)
        } else {
            None
        };
        responses.push((status, body));
        if input.peek(Token![,]) {
            input.parse::<Token![,]>()?;
        } else {
            break;
        }
    }
    Ok(responses)
}
