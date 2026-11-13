use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Attribute, Expr, Ident, LitStr, Path, Result, Token, Type};

use crate::util::parse_security_types;

/// The router state a table declares.
///
/// A type path is the normal form. `state = infer` restores the pre-0.3 behaviour: each entry
/// derives its state from its handler or its single auth scheme.
pub(super) enum StateOption {
    Type(Box<Type>),
    Infer,
}

#[derive(Clone, Copy, Default)]
pub(super) enum LayerMode {
    #[default]
    Full,
    Route,
}

#[derive(Default)]
pub(super) struct ModuleOptions {
    pub(super) prefix: Option<LitStr>,
    pub(super) state: Option<StateOption>,
    pub(super) tags: Vec<LitStr>,
    pub(super) security: Vec<Type>,
    pub(super) layer: Option<Expr>,
    pub(super) layer_mode: LayerMode,
    pub(super) hidden: bool,
    /// Clear inherited security guards for this module and its children.
    pub(super) anonymous: bool,
    pub(super) doc: Option<Expr>,
}

pub(super) struct Entry {
    pub(super) attrs: Vec<Attribute>,
    pub(super) method: Ident,
    pub(super) path: LitStr,
    pub(super) assert_path: Option<LitStr>,
    pub(super) handler: Path,
    pub(super) operation_id: Option<LitStr>,
    pub(super) tags: Vec<LitStr>,
    pub(super) security: Vec<Type>,
    pub(super) layer: Option<Expr>,
    pub(super) doc: Option<Expr>,
    /// `prose = include_str!(..)`: an operation summary/description from one string.
    pub(super) prose: Option<Expr>,
    pub(super) hidden: bool,
    pub(super) absolute: bool,
    pub(super) validate: bool,
    pub(super) validate_body: bool,
    /// Spans of explicit `validate` / `validate_body` values, for the contradictory-pair error.
    pub(super) validate_span: Option<proc_macro2::Span>,
    pub(super) validate_body_span: Option<proc_macro2::Span>,
    /// An explicit acknowledgement of an intentional unverified body/response/extractor.
    pub(super) unverified: Option<LitStr>,
    pub(super) deprecated: bool,
    /// Clear inherited security guards for this entry.
    pub(super) anonymous: bool,
}

impl Entry {
    fn new(attrs: Vec<Attribute>, method: Ident, path: LitStr, handler: Path) -> Self {
        Self {
            attrs,
            method,
            path,
            assert_path: None,
            handler,
            operation_id: None,
            tags: Vec::new(),
            security: Vec::new(),
            layer: None,
            doc: None,
            prose: None,
            hidden: false,
            absolute: false,
            validate: true,
            validate_body: true,
            validate_span: None,
            validate_body_span: None,
            unverified: None,
            deprecated: false,
            anonymous: false,
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
        parse_entry_options(&content, &mut entry)?;
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
                    // `"/notes" [tags = ..] { "/" => { get(list) } }` is a prefix group.
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
            } else if let Some(path) = &shared_path {
                let mut entry = Entry::parse_grouped(input, path.clone())?;
                entry.attrs.splice(0..0, attrs);
                body.entries.push(entry);
            } else {
                return Err(syn::Error::new(
                    input.span(),
                    "route entries must be inside a path group; write `\"/path\" => { get(handler) }`",
                ));
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
        let is_flag = name == "hidden" || name == "anonymous";
        let is_key = matches!(
            name.as_str(),
            "prefix" | "state" | "tags" | "security" | "layer" | "layer_mode" | "doc"
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
    if options.anonymous && !options.security.is_empty() {
        return Err(syn::Error::new(
            input.span(),
            "`anonymous` and `security` cannot be combined in one module; remove one",
        ));
    }
    Ok(options)
}

fn parse_entry_options(content: ParseStream<'_>, entry: &mut Entry) -> Result<()> {
    while content.peek(Token![,]) {
        content.parse::<Token![,]>()?;
        if content.is_empty() {
            break;
        }
        let key: Ident = content.call(Ident::parse_any)?;
        parse_entry_key(content, entry, key)?;
    }
    if !entry.validate
        && entry.validate_body
        && let (Some(off), Some(on)) = (entry.validate_span, entry.validate_body_span)
    {
        let mut error = syn::Error::new(
            on,
            "`validate_body = true` has no effect when `validate = false`; remove it",
        );
        error.combine(syn::Error::new(off, "this disables the whole route"));
        return Err(error);
    }
    if entry.anonymous && !entry.security.is_empty() {
        return Err(syn::Error::new(
            entry.path.span(),
            "`anonymous` and `security` cannot be combined on one entry; remove one",
        ));
    }
    Ok(())
}

fn parse_entry_key(content: ParseStream<'_>, entry: &mut Entry, key: Ident) -> Result<()> {
    match key.to_string().as_str() {
        "summary" => {
            return Err(syn::Error::new(
                key.span(),
                "operation summaries come from the handler's `/// @summary:` doc tag \
                 (or a `doc =` hook for a per-route override)",
            ));
        }
        "description" => {
            return Err(syn::Error::new(
                key.span(),
                "operation descriptions come from the handler's `/// @description:` doc tag \
                 (or a `doc =` hook for a per-route override)",
            ));
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
            return Err(syn::Error::new(
                key.span(),
                "response declarations live on the handler; write `/// @responses:` in its doc \
                 block",
            ));
        }
        "layer" => {
            content.parse::<Token![=]>()?;
            entry.layer = Some(content.parse()?);
        }
        "doc" => {
            content.parse::<Token![=]>()?;
            entry.doc = Some(content.parse()?);
        }
        "prose" => {
            content.parse::<Token![=]>()?;
            entry.prose = Some(content.parse()?);
        }
        "hidden" => entry.hidden = true,
        "anonymous" => entry.anonymous = true,
        "absolute" => entry.absolute = true,
        "deprecated" => entry.deprecated = true,
        "override" => {
            return Err(syn::Error::new(
                key.span(),
                "replace `override` with `@responses(override):` on the handler",
            ));
        }
        "validate" => {
            content.parse::<Token![=]>()?;
            let value: syn::LitBool = content.parse()?;
            entry.validate = value.value;
            entry.validate_span = Some(value.span());
        }
        "validate_body" => {
            content.parse::<Token![=]>()?;
            let value: syn::LitBool = content.parse()?;
            entry.validate_body = value.value;
            entry.validate_body_span = Some(value.span());
        }
        "unverified" => {
            content.parse::<Token![=]>()?;
            entry.unverified = Some(content.parse()?);
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
    if options.anonymous && !options.security.is_empty() {
        return Err(syn::Error::new(
            input.span(),
            "`anonymous` and `security` cannot be combined in one module; remove one",
        ));
    }
    Ok(options)
}

fn parse_module_key(input: ParseStream<'_>, options: &mut ModuleOptions, key: Ident) -> Result<()> {
    match key.to_string().as_str() {
        "prefix" => {
            input.parse::<Token![=]>()?;
            options.prefix = Some(input.parse()?);
        }
        "state" => {
            input.parse::<Token![=]>()?;
            let ty: Type = input.parse()?;
            options.state = Some(match &ty {
                Type::Path(path) if path.qself.is_none() && path.path.is_ident("infer") => {
                    StateOption::Infer
                }
                _ => StateOption::Type(Box::new(ty)),
            });
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
        "anonymous" => options.anonymous = true,
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
