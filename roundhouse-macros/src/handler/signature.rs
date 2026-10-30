use quote::format_ident;
use syn::{Ident, Type};

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

/// Classify the body argument for the route's raw JSON validator.
///
/// `Result<Extractor, E>` owns its rejection, so it has no automatic body validation. `Json` is
/// the only kind the router validates before extraction; form and multipart stay in the shim.
pub(super) fn body_meta_for(ty: &Type) -> Option<(Ident, Type)> {
    if result_ok_type(ty).is_some() {
        return None;
    }
    let ty = if outer_ident(ty).as_deref() == Some("Option") {
        single_generic(ty)?
    } else {
        ty.clone()
    };
    let ty = if outer_ident(&ty).as_deref() == Some("Strict") {
        single_generic(&ty)?
    } else {
        ty
    };
    match outer_ident(&ty).as_deref() {
        Some("Json") => single_generic(&ty).map(|inner| (format_ident!("Json"), inner)),
        Some("Form") => single_generic(&ty).map(|inner| (format_ident!("Form"), inner)),
        Some("Multipart") => single_generic(&ty).map(|inner| (format_ident!("Multipart"), inner)),
        Some("Bytes" | "Body") => Some((format_ident!("Raw"), ty.clone())),
        _ => None,
    }
}

/// Whether this argument is a parts extractor. The injected auth extractor is inserted before the
/// first argument that is not one, so it always precedes the body. An unknown extractor is
/// treated as a body: that is the conservative choice, and ordering among parts does not matter.
pub(super) fn is_parts_arg(ty: &Type) -> bool {
    let ty = result_ok_type(ty).unwrap_or_else(|| ty.clone());
    let ty = if outer_ident(&ty).as_deref() == Some("Option") {
        single_generic(&ty).unwrap_or(ty)
    } else {
        ty
    };
    matches!(
        outer_ident(&ty).as_deref(),
        Some(
            "State"
                | "Path"
                | "Query"
                | "RepeatedQuery"
                | "Extension"
                | "Auth"
                | "HeaderMap"
                | "Method"
                | "Uri"
                | "OriginalUri"
                | "MatchedPath"
                | "Parts"
                | "ConnectInfo"
                | "TypedHeader"
        )
    )
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
