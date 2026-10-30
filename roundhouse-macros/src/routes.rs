use proc_macro2::TokenStream;
use quote::{format_ident, quote, quote_spanned};
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::spanned::Spanned;
use syn::{Attribute, Expr, Ident, LitInt, LitStr, Path, Result, Token, Type};

use crate::util::{
    auth_slice, last_segment, marker_type, parse_security_types, parse_u16, string_slice,
};

#[derive(Clone, Copy, Default)]
enum LayerMode {
    #[default]
    Full,
    Route,
}

#[derive(Default)]
struct ModuleOptions {
    prefix: Option<LitStr>,
    tags: Vec<LitStr>,
    security: Vec<Type>,
    layer: Option<Expr>,
    layer_mode: LayerMode,
    hidden: bool,
    doc: Option<Expr>,
}

struct Entry {
    attrs: Vec<Attribute>,
    method: Ident,
    path: LitStr,
    assert_path: Option<LitStr>,
    handler: Path,
    summary: Option<LitStr>,
    description: Option<LitStr>,
    operation_id: Option<LitStr>,
    tags: Vec<LitStr>,
    security: Vec<Type>,
    responses: Vec<(u16, Option<Type>)>,
    layer: Option<Expr>,
    doc: Option<Expr>,
    hidden: bool,
    absolute: bool,
    validate: bool,
    deprecated: bool,
    public: bool,
    response_override: bool,
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
    fn parse_flat(input: ParseStream<'_>) -> Result<Self> {
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
    fn parse_grouped(input: ParseStream<'_>, path: LitStr) -> Result<Self> {
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
struct Body {
    entries: Vec<Entry>,
    groups: Vec<Group>,
    spreads: Vec<Path>,
}

impl Body {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
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

struct Group {
    path: LitStr,
    options: ModuleOptions,
    /// `"/path" => { get(handler) }`: the group supplies the full entry path.
    shared_path: bool,
    body: Body,
}

pub struct RoutesInput {
    options: ModuleOptions,
    body: Body,
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

pub fn expand(input: RoutesInput) -> TokenStream {
    let mut flattened = Vec::new();
    flatten(&input.body, "", &mut flattened);
    if let Err(error) = validate_entries(&flattened) {
        return error.to_compile_error();
    }

    let krate = crate::util::krate();
    let mut ids = 0usize;
    let mut definitions = TokenStream::new();
    emit_module(
        &format_ident!("MODULE"),
        true,
        input.options.prefix.as_ref(),
        &input.options,
        &input.body,
        &mut ids,
        &mut definitions,
    );

    quote! {
        #definitions

        /// The explicit fragment this `routes!` table contributes.
        #[doc(hidden)]
        pub fn __roundhouse_fragment() -> #krate::Fragment {
            #krate::Fragment::new(&MODULE)
        }
    }
}

fn emit_module(
    name: &Ident,
    public: bool,
    prefix: Option<&LitStr>,
    options: &ModuleOptions,
    body: &Body,
    ids: &mut usize,
    definitions: &mut TokenStream,
) {
    let krate = crate::util::krate();
    let module_id = *ids;
    *ids += 1;

    let mut routes = Vec::new();
    for entry in &body.entries {
        let id = *ids;
        *ids += 1;
        let ExpandedEntry { defs, record } = expand_entry(entry, id);
        definitions.extend(defs);
        let attrs = &entry.attrs;
        routes.push(quote!(#(#attrs)* #record));
    }

    let mut children = Vec::new();
    for (index, group) in body.groups.iter().enumerate() {
        let child_name = format_ident!("__ROUNDHOUSE_GROUP_{module_id}_{index}");
        let prefix = if group.shared_path {
            None
        } else {
            Some(&group.path)
        };
        emit_module(
            &child_name,
            false,
            prefix,
            &group.options,
            &group.body,
            ids,
            definitions,
        );
        children.push(quote!(&#child_name));
    }
    for spread in &body.spreads {
        children.push(quote!(&#spread::MODULE));
    }

    let prefix_value = prefix.map(|prefix| prefix.value()).unwrap_or_default();
    let prefix_lit = LitStr::new(&prefix_value, proc_macro2::Span::call_site());
    let tags = string_slice(&options.tags);
    let security = auth_slice(&options.security);
    let hidden = options.hidden;

    let layer_fn = format_ident!("__roundhouse_module_layer_{module_id}");
    let layer_value = if options.layer.is_some() {
        quote!(::core::option::Option::Some(#layer_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    if let Some(expr) = &options.layer {
        let body = match options.layer_mode {
            LayerMode::Full => quote!(router.layer(#expr)),
            LayerMode::Route => quote!(router.route_layer(#expr)),
        };
        definitions.extend(quote! {
            fn #layer_fn(
                router: #krate::axum::Router,
            ) -> #krate::axum::Router {
                #body
            }
        });
    }

    let doc_fn = format_ident!("__roundhouse_module_doc_{module_id}");
    let doc_value = if options.doc.is_some() {
        quote!(::core::option::Option::Some(#doc_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    if let Some(expr) = &options.doc {
        definitions.extend(quote! {
            fn #doc_fn() -> #krate::doc::layer::LayerDoc {
                #expr
            }
        });
    }

    let line = match prefix {
        Some(prefix) => {
            let line = prefix.span().start().line as u32;
            quote!(#line)
        }
        None => quote!(line!()),
    };
    let visibility = if public { quote!(pub) } else { quote!() };

    definitions.extend(quote! {
        #visibility const #name: #krate::registry::Module = #krate::registry::Module::__new(
            module_path!(),
            file!(),
            #line,
            #prefix_lit,
            #tags,
            #security,
            #hidden,
            #layer_value,
            #doc_value,
            &[#(#routes),*],
            &[#(#children),*],
        );
    });
}

struct ExpandedEntry {
    defs: TokenStream,
    record: Ident,
}

fn flatten<'a>(body: &'a Body, prefix: &str, out: &mut Vec<(String, &'a Entry)>) {
    for entry in &body.entries {
        let path = join_paths(prefix, &entry.path.value());
        out.push((path, entry));
    }
    for group in &body.groups {
        if group.shared_path {
            flatten(&group.body, prefix, out);
        } else {
            let prefix = join_paths(prefix, &group.path.value());
            flatten(&group.body, &prefix, out);
        }
    }
}

fn join_paths(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_string()
    } else if path == "/" {
        prefix.to_string()
    } else {
        format!(
            "{}/{}",
            prefix.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

/// Table-local checks. These are the checks a single `routes!` invocation can make from the
/// tokens alone: two entries that resolve to the same method and template, and two entries whose
/// operation ids collide. Cross-table collisions still need the resolved route table.
fn validate_entries(entries: &[(String, &Entry)]) -> Result<()> {
    use std::collections::HashMap;

    let mut routes: HashMap<(String, String), &Entry> = HashMap::new();
    let mut explicit_ids: HashMap<String, proc_macro2::Span> = HashMap::new();
    let mut derived_ids: HashMap<(String, String), &Entry> = HashMap::new();

    for (full_path, entry) in entries {
        let method = entry.method.to_string();
        let path = normalise_path(full_path);

        if let Some(first) = routes.get(&(method.clone(), path.clone())) {
            return Err(duplicate_route_error(first, entry, &method, &path));
        }
        if let Some((first_method, first)) =
            routes
                .iter()
                .find_map(|((existing, existing_path), first)| {
                    (existing_path == &path
                        && (existing == "any" || method == "any")
                        && existing != &method)
                        .then_some((existing, *first))
                })
        {
            let mut error = syn::Error::new(
                entry.method.span(),
                format!(
                    "route `{method} {path}` conflicts with `{first_method} {path}`; an `any` \
                     entry covers every method"
                ),
            );
            error.combine(syn::Error::new(
                first.method.span(),
                "the conflicting entry",
            ));
            return Err(error);
        }
        routes.insert((method.clone(), path.clone()), entry);

        match &entry.operation_id {
            Some(id) => {
                if let Some(first_span) = explicit_ids.insert(id.value(), id.span()) {
                    let mut error = syn::Error::new(
                        id.span(),
                        format!(
                            "duplicate operationId `{}` in this routes! table",
                            id.value()
                        ),
                    );
                    error.combine(syn::Error::new(first_span, "first declared here"));
                    return Err(error);
                }
            }
            None => {
                // The derived id is `method + module path + handler`; within one table the module
                // path is shared, so the method and handler name are enough to detect a collision.
                let handler = last_segment(&entry.handler);
                let key = (method.clone(), handler.clone());
                if let Some(first) = derived_ids.insert(key, entry) {
                    let mut error = syn::Error::new(
                        entry.handler.span(),
                        format!(
                            "two entries derive the same operationId `{}_{}`",
                            method.to_ascii_lowercase(),
                            handler
                        ),
                    );
                    error.combine(syn::Error::new(
                        first.handler.span(),
                        "the first entry; set an explicit `operation_id = \"...\"` on one of them",
                    ));
                    return Err(error);
                }
            }
        }
    }

    Ok(())
}

fn duplicate_route_error(first: &Entry, second: &Entry, method: &str, path: &str) -> syn::Error {
    let mut error = syn::Error::new(
        second.method.span(),
        format!("duplicate route `{method} {path}` in this routes! table"),
    );
    error.combine(syn::Error::new(
        first.method.span(),
        format!("first declared as `{method} {path}`"),
    ));
    error
}

/// Normalise a template the way the router does: captures collapse to `{}` / `{*}`, escaped
/// braces are kept, and repeated slashes are collapsed.
fn normalise_path(path: &str) -> String {
    let chars: Vec<char> = path.chars().collect();
    let mut out = String::with_capacity(path.len());
    let mut index = 0;
    while index < chars.len() {
        if index + 1 < chars.len()
            && ((chars[index] == '{' && chars[index + 1] == '{')
                || (chars[index] == '}' && chars[index + 1] == '}'))
        {
            out.push(chars[index]);
            index += 2;
            continue;
        }
        if chars[index] == '{' {
            let start = index + 1;
            let mut end = start;
            while end < chars.len() && chars[end] != '}' {
                end += 1;
            }
            if end < chars.len() {
                if chars[start..end].contains(&'*') {
                    out.push_str("{*}");
                } else {
                    out.push_str("{}");
                }
                index = end + 1;
                continue;
            }
        }
        out.push(chars[index]);
        index += 1;
    }

    let mut collapsed = String::with_capacity(out.len());
    let mut last_slash = false;
    for ch in out.chars() {
        if ch == '/' {
            if !last_slash {
                collapsed.push('/');
            }
            last_slash = true;
        } else {
            collapsed.push(ch);
            last_slash = false;
        }
    }
    if collapsed.is_empty() {
        "/".to_string()
    } else {
        collapsed
    }
}

fn method_variant(method: &Ident) -> Result<TokenStream> {
    let krate = crate::util::krate();
    Ok(match method.to_string().as_str() {
        "get" => quote!(#krate::registry::Method::Get),
        "post" => quote!(#krate::registry::Method::Post),
        "put" => quote!(#krate::registry::Method::Put),
        "patch" => quote!(#krate::registry::Method::Patch),
        "delete" => quote!(#krate::registry::Method::Delete),
        "head" => quote!(#krate::registry::Method::Head),
        "options" => quote!(#krate::registry::Method::Options),
        "trace" => quote!(#krate::registry::Method::Trace),
        "any" => quote!(#krate::registry::Method::Any),
        other => {
            return Err(syn::Error::new(
                method.span(),
                format!(
                    "unknown verb `{other}`; expected get, post, put, patch, delete, head, options, trace or any"
                ),
            ));
        }
    })
}

fn expand_entry(entry: &Entry, index: usize) -> ExpandedEntry {
    let krate = crate::util::krate();
    if !entry.path.value().starts_with('/') {
        return ExpandedEntry {
            defs: syn::Error::new(
                entry.path.span(),
                "route paths must start with `/` (use `/` for a module's own path)",
            )
            .to_compile_error(),
            record: format_ident!("__ROUNDHOUSE_ROUTE_{index}"),
        };
    }
    if entry
        .path
        .value()
        .split('/')
        .any(|segment| segment.starts_with(':'))
    {
        return ExpandedEntry {
            defs: syn::Error::new(
                entry.path.span(),
                "axum 0.7 path syntax (`:id`) is not supported; use `{id}` for a capture and `{*rest}` for a wildcard",
            )
            .to_compile_error(),
            record: format_ident!("__ROUNDHOUSE_ROUTE_{index}"),
        };
    }

    let attrs = &entry.attrs;
    let method_ident = format_ident!("{}", entry.method);
    let method = match method_variant(&entry.method) {
        Ok(method) => method,
        Err(error) => {
            return ExpandedEntry {
                defs: error.to_compile_error(),
                record: format_ident!("__ROUNDHOUSE_ROUTE_{index}"),
            };
        }
    };
    let path = &entry.path;
    let assert_path = option_lit(&entry.assert_path);
    let handler = &entry.handler;
    let marker_type = marker_type(handler);
    let single_scheme = match entry.security.as_slice() {
        [scheme] => Some(scheme),
        _ => None,
    };
    let shim_state = match single_scheme {
        Some(scheme) => quote!(<#scheme as #krate::auth::AuthScheme>::State),
        None => quote!(<#marker_type as #krate::doc::HandlerMeta>::State),
    };
    let policy = match (single_scheme, entry.validate) {
        (Some(scheme), true) => quote!(#krate::shim::AuthedValidated::<#scheme>),
        (Some(scheme), false) => quote!(#krate::shim::Authed::<#scheme>),
        (None, true) => quote!(#krate::shim::Validated::<#shim_state>),
        (None, false) => quote!(#krate::shim::Plain::<#shim_state>),
    };
    let state = match single_scheme {
        Some(_) => quote!(
            || ::core::option::Option::Some(
                ::core::any::TypeId::of::<#shim_state>(),
            )
        ),
        None => quote!(|| #krate::doc::handler_state::<#marker_type>()),
    };
    let state_name = match single_scheme {
        Some(_) => quote!(|| ::core::any::type_name::<#shim_state>()),
        None => quote!(
            || ::core::any::type_name::<
                <#marker_type as #krate::doc::HandlerMeta>::State,
            >()
        ),
    };
    let handler_alias = format_ident!("__RoundhouseHandler{index}");
    // Through an alias so a missing `#[handler]` reports one "type not found" error rather than
    // also interpreting the marker as a module path.
    let route_handler = quote!(#handler_alias::__roundhouse_shim::<#policy>);
    let shim_auth = match single_scheme {
        Some(scheme) => quote!(&[<#scheme as #krate::auth::AuthScheme>::NAME]),
        None => quote!(&[]),
    };
    let operation_id = option_lit(&entry.operation_id);
    let handler_name = LitStr::new(&last_segment(handler), entry.path.span());
    let assertion_message = LitStr::new(
        &format!(
            "roundhouse: Path<T> in handler `{}` does not declare every capture in `{}`",
            last_segment(handler),
            entry.path.value().replace('{', "{{").replace('}', "}}"),
        ),
        entry.path.span(),
    );
    let summary = option_lit(&entry.summary);
    let description = option_lit(&entry.description);
    let tags = string_slice(&entry.tags);
    let security = auth_slice(&entry.security);
    let required_auth = quote!(<#marker_type as #krate::doc::HandlerMeta>::REQUIRED_AUTH);
    let hidden = entry.hidden;
    let absolute = entry.absolute;
    let validate = entry.validate;
    let deprecated = entry.deprecated;
    let public = entry.public;
    let response_override = entry.response_override;
    let declared_success = entry
        .responses
        .iter()
        .any(|(status, _)| *status == 0 || (200..400).contains(status));
    let no_success_message = LitStr::new(
        &format!(
            "roundhouse: `{} {}` has no success response; return a concrete response type or \
             declare `responses(..)` in the table",
            method_ident,
            entry.path.value().replace('{', "{{").replace('}', "}}")
        ),
        entry.path.span(),
    );
    let success_assert = quote_spanned! { entry.path.span()=>
        const _: () = {
            if !#declared_success {
                if let ::core::option::Option::Some(__statuses) =
                    <#handler_alias as #krate::doc::HandlerMeta>::INFERRED_SUCCESS
                {
                    assert!(!__statuses.is_empty(), #no_success_message);
                }
            }
        };
    };
    let responses = entry.responses.iter().map(|(status, body)| {
        let status = proc_macro2::Literal::u16_suffixed(*status);
        match body {
            Some(ty) => quote! {
                __responses.push(#krate::doc::json_response_doc::<#ty>(components, #status));
            },
            None => quote! {
                __responses.push(#krate::doc::ResponseDoc::new(
                    #status,
                    #krate::doc::canonical_reason(#status),
                ));
            },
        }
    });
    let line = entry.path.span().start().line as u32;

    let route_fn = format_ident!("__roundhouse_route_{index}");
    let responses_fn = format_ident!("__roundhouse_responses_{index}");
    let layer_fn = format_ident!("__roundhouse_layer_{index}");
    let doc_fn = format_ident!("__roundhouse_entry_doc_{index}");
    let record_name = format_ident!("__ROUNDHOUSE_ROUTE_{index}");

    let layer_value = if entry.layer.is_some() {
        quote!(::core::option::Option::Some(#layer_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    let layer = match &entry.layer {
        Some(expr) => quote! {
            fn #layer_fn(
                method_router: #krate::axum::routing::MethodRouter,
            ) -> #krate::axum::routing::MethodRouter {
                method_router.layer(#expr)
            }
        },
        None => quote! {},
    };
    let doc_value = if entry.doc.is_some() {
        quote!(::core::option::Option::Some(#doc_fn))
    } else {
        quote!(::core::option::Option::None)
    };
    let doc = match &entry.doc {
        Some(expr) => quote! {
            fn #doc_fn(docs: &mut #krate::doc::HandlerDocs) {
                let hook: fn(&mut #krate::doc::HandlerDocs) = #expr;
                hook(docs)
            }
        },
        None => quote! {},
    };

    let defs = quote! {
        #(#attrs)*
        #[allow(non_camel_case_types)]
        type #handler_alias = #marker_type;

        #(#attrs)*
        #[allow(non_upper_case_globals)]
        const _: () = {
            #success_assert

            const _: () = {
                assert!(
                    #krate::prefix::path_names_match(
                        #path,
                        <#marker_type as #krate::doc::HandlerMeta>::PATH_NAMES,
                    ),
                    #assertion_message,
                );
            };
        };

        #(#attrs)*
        fn #route_fn()
            -> ::std::boxed::Box<dyn ::core::any::Any + ::core::marker::Send + ::core::marker::Sync>
        {
            let method_router: #krate::axum::routing::MethodRouter<#shim_state> =
                #krate::axum::routing::#method_ident(#route_handler);
            ::std::boxed::Box::new(method_router)
        }

        #(#attrs)*
        #layer
        #(#attrs)*
        #doc

        #(#attrs)*
        fn #responses_fn(
            components: &mut #krate::utoipa::openapi::schema::Components,
        ) -> ::std::vec::Vec<#krate::doc::ResponseDoc> {
            let mut __responses = ::std::vec::Vec::new();
            #(#responses)*
            __responses
        }

        #(#attrs)*
        #[allow(non_upper_case_globals)]
        const #record_name: #krate::registry::RouteRecord = #krate::registry::RouteRecord::__new(
            module_path!(),
            file!(),
            #line,
            #method,
            #path,
            #assert_path,
            #operation_id,
            #handler_name,
            #summary,
            #description,
            #tags,
            #security,
            #required_auth,
            #shim_auth,
            #hidden,
            #absolute,
            #validate,
            #deprecated,
            #public,
            #response_override,
            #responses_fn,
            #state,
            #state_name,
            #route_fn,
            #layer_value,
            <#marker_type as #krate::doc::HandlerMeta>::docs_with,
            <#marker_type as #krate::doc::HandlerMeta>::body,
            #doc_value,
        );
    };

    ExpandedEntry {
        defs,
        record: record_name,
    }
}

fn option_lit(value: &Option<LitStr>) -> TokenStream {
    match value {
        Some(lit) => quote!(::core::option::Option::Some(#lit)),
        None => quote!(::core::option::Option::None),
    }
}
