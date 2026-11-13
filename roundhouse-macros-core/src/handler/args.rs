//! Typed metadata for `#[handler(..)]`.
//!
//! Structured response and path metadata lives here. Doc comments remain the home for prose
//! (`@summary:` and `@description:`), so a typo in a response type is an ordinary syntax or type
//! error rather than a line in a doc string.

use proc_macro2::Span;
use syn::ext::IdentExt;
use syn::parse::{Parse, ParseStream};
use syn::{Ident, LitInt, LitStr, Result, Token, Type};

use crate::util::{outer_ident, single_generic};

use super::ResponseBody;

#[derive(Default)]
pub(crate) struct HandlerArgs {
    pub(super) responses: Vec<DeclaredResponse>,
    pub(super) override_existing: bool,
    pub(super) response_descriptions: Vec<ResponseDescription>,
    pub(super) paths: Vec<PathDescription>,
}

#[derive(Clone)]
pub(super) struct DeclaredResponse {
    pub(super) status: u16,
    pub(super) body: ResponseBody,
    pub(super) span: Span,
}

#[derive(Clone)]
pub(super) struct ResponseDescription {
    pub(super) status: u16,
    pub(super) description: String,
    pub(super) span: Span,
}

#[derive(Clone)]
pub(super) struct PathDescription {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) span: Span,
}

impl Parse for HandlerArgs {
    fn parse(input: ParseStream<'_>) -> Result<Self> {
        let mut args = Self::default();
        while !input.is_empty() {
            let key: Ident = input.parse()?;
            let content;
            syn::parenthesized!(content in input);
            match key.to_string().as_str() {
                "response" => {
                    let (status, body) = parse_response(&content)?;
                    args.responses.push(DeclaredResponse {
                        status,
                        body,
                        span: key.span(),
                    });
                }
                "override_response" => {
                    args.override_existing = true;
                    let (status, body) = parse_response(&content)?;
                    args.responses.push(DeclaredResponse {
                        status,
                        body,
                        span: key.span(),
                    });
                }
                "default_response" => {
                    let ty: Type = content.parse()?;
                    let body = response_body_from_type(ty, key.span())?;
                    args.responses.push(DeclaredResponse {
                        status: 0,
                        body,
                        span: key.span(),
                    });
                }
                "response_description" => {
                    let status = parse_status(&content)?;
                    content.parse::<Token![,]>()?;
                    let description: LitStr = content.parse()?;
                    args.response_descriptions.push(ResponseDescription {
                        status,
                        description: description.value(),
                        span: description.span(),
                    });
                }
                "path" => {
                    let name = if content.peek(LitStr) {
                        content.parse::<LitStr>()?.value()
                    } else {
                        content.call(Ident::parse_any)?.to_string()
                    };
                    content.parse::<Token![,]>()?;
                    let description: LitStr = content.parse()?;
                    args.paths.push(PathDescription {
                        name,
                        description: description.value(),
                        span: description.span(),
                    });
                }
                other => {
                    return Err(syn::Error::new(
                        key.span(),
                        format!(
                            "unknown `#[handler(..)]` option `{other}`; expected `response`, \
                             `override_response`, `default_response`, `response_description` \
                             or `path`"
                        ),
                    ));
                }
            }
            if input.peek(Token![,]) {
                input.parse::<Token![,]>()?;
            }
        }
        Ok(args)
    }
}

fn parse_response(input: ParseStream<'_>) -> Result<(u16, ResponseBody)> {
    let status = parse_status(input)?;
    if input.is_empty() {
        return Ok((status, ResponseBody::Empty));
    }
    input.parse::<Token![,]>()?;
    let ty: Type = input.parse()?;
    let span = ty_span(&ty);
    Ok((status, response_body_from_type(ty, span)?))
}

fn parse_status(input: ParseStream<'_>) -> Result<u16> {
    if input.peek(LitInt) {
        let literal: LitInt = input.parse()?;
        let status = literal.base10_parse::<u16>()?;
        if !(100..=599).contains(&status) {
            return Err(syn::Error::new(
                literal.span(),
                format!("`{status}` is not an HTTP status code"),
            ));
        }
        return Ok(status);
    }
    let ident: Ident = input.call(Ident::parse_any)?;
    if ident == "default" {
        return Ok(0);
    }
    Err(syn::Error::new(
        ident.span(),
        format!("expected an HTTP status or `default`, found `{ident}`"),
    ))
}

fn response_body_from_type(ty: Type, span: Span) -> Result<ResponseBody> {
    Ok(match outer_ident(&ty).as_deref() {
        Some("Problem" | "ValidationErrors") => ResponseBody::Problem,
        Some("Json") => {
            let inner = single_generic(&ty).ok_or_else(|| {
                syn::Error::new(span, "`Json<T>` needs exactly one type argument")
            })?;
            ResponseBody::Json(Box::new(inner))
        }
        _ => ResponseBody::Json(Box::new(ty)),
    })
}

fn ty_span(ty: &Type) -> Span {
    use syn::spanned::Spanned;
    ty.span()
}
