use syn::Type;

use crate::util::{outer_ident, single_generic};

pub(super) fn auth_types(ty: &Type) -> Option<(Type, Option<Type>, bool)> {
    let (auth_ty, optional) = if outer_ident(ty).as_deref() == Some("Option") {
        (single_generic(ty)?, true)
    } else {
        (ty.clone(), false)
    };
    if outer_ident(&auth_ty).as_deref() != Some("Auth") || !is_wrapper_type(&auth_ty) {
        return None;
    }
    let types = generic_types(&auth_ty);
    let auth = types.first()?.clone();
    let scopes = types.get(1).cloned();
    Some((auth, scopes, optional))
}

pub(super) fn is_wrapper_type(ty: &Type) -> bool {
    let Type::Path(type_path) = ty else {
        return false;
    };
    let segments = &type_path.path.segments;
    segments.len() == 1
        || segments
            .first()
            .is_some_and(|segment| segment.ident == "roundhouse")
}

pub(super) fn result_ok_type(ty: &Type) -> Option<Type> {
    if outer_ident(ty).as_deref() != Some("Result") {
        return None;
    }
    generic_types(ty).into_iter().next()
}

/// One classification of a handler argument.
///
/// The shim's wrapping, the document's body marker and the extractor order all read this, so the
/// list of known extractors lives here once.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum ExtractorKind {
    Json,
    Form,
    Multipart,
    Query,
    Path,
    Extension,
    Auth,
    State,
    Bytes,
    Body,
    /// Another parts extractor, or a repeated-key parser.
    Parts,
    /// An extractor the framework does not know. Treated as a body: the conservative choice for
    /// ordering, and never wrapped or claimed in the document.
    Other,
}

impl ExtractorKind {
    pub(super) fn is_body(self) -> bool {
        matches!(
            self,
            Self::Json | Self::Form | Self::Multipart | Self::Bytes | Self::Body
        )
    }

    /// Whether the argument is extracted from the request head. An unknown extractor is not.
    pub(super) fn is_parts(self) -> bool {
        !self.is_body() && self != Self::Other
    }

    /// Whether the shim maps this extractor's rejection to `Problem`. `RepeatedQuery`, `Auth` and
    /// `Multipart` already reject with one, so they are not listed.
    pub(super) fn maps_rejection(self) -> bool {
        matches!(
            self,
            Self::Json | Self::Form | Self::Query | Self::Path | Self::Extension
        )
    }
}

/// Parts extractors with no dedicated [`ExtractorKind`] variant.
const OTHER_PARTS: &[&str] = &[
    "RepeatedQuery",
    "HeaderMap",
    "Method",
    "Uri",
    "OriginalUri",
    "MatchedPath",
    "Parts",
    "ConnectInfo",
    "TypedHeader",
];

/// Strip `Option<..>` and an explicit `Strict<..>` so classification sees the extractor.
///
/// `Mapped<..>` is deliberately not stripped: a mapping-only extractor is not claimed for the
/// document or the raw body layer.
pub(super) fn strip_wrappers(ty: &Type) -> Type {
    let mut ty = ty.clone();
    loop {
        let name = outer_ident(&ty);
        if matches!(name.as_deref(), Some("Option" | "Strict"))
            && let Some(inner) = single_generic(&ty)
        {
            ty = inner;
            continue;
        }
        return ty;
    }
}

pub(super) fn classify(ty: &Type) -> ExtractorKind {
    let ty = strip_wrappers(ty);
    match outer_ident(&ty).as_deref() {
        Some("Json") => ExtractorKind::Json,
        Some("Form") => ExtractorKind::Form,
        Some("Multipart") => ExtractorKind::Multipart,
        Some("Query") => ExtractorKind::Query,
        Some("Path") => ExtractorKind::Path,
        Some("Extension") => ExtractorKind::Extension,
        Some("Auth") => ExtractorKind::Auth,
        Some("State") => ExtractorKind::State,
        Some("Bytes") => ExtractorKind::Bytes,
        Some("Body") => ExtractorKind::Body,
        Some(name) if OTHER_PARTS.contains(&name) => ExtractorKind::Parts,
        _ => ExtractorKind::Other,
    }
}

/// Whether the handler names `Strict` itself. The shim leaves that extractor alone: it enforces.
pub(super) fn is_explicit_strict(ty: &Type) -> bool {
    outer_ident(ty).as_deref() == Some("Strict")
}

/// Whether this argument is a parts extractor. The injected auth extractor is inserted before the
/// first argument that is not one, so it always precedes the body.
pub(super) fn is_parts_arg(ty: &Type) -> bool {
    let ty = result_ok_type(ty).unwrap_or_else(|| ty.clone());
    classify(&ty).is_parts()
}

pub(super) fn generic_types(ty: &Type) -> Vec<Type> {
    let Type::Path(type_path) = ty else {
        return Vec::new();
    };
    let Some(segment) = type_path.path.segments.last() else {
        return Vec::new();
    };
    let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return Vec::new();
    };
    arguments
        .args
        .iter()
        .filter_map(|argument| match argument {
            syn::GenericArgument::Type(ty) => Some(ty.clone()),
            _ => None,
        })
        .collect()
}
