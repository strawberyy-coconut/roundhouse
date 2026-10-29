use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::{LitInt, LitStr, Path};

/// The path to the `roundhouse` crate as the *expanding* crate names it.
///
/// Expansions must not hardcode `::roundhouse`: a renamed dependency
/// (`roundhouse = { package = "roundhouse", ... }` under another key) would fail to resolve
/// every generated item.
pub fn krate() -> TokenStream {
    use proc_macro_crate::{FoundCrate, crate_name};

    match crate_name("roundhouse") {
        Ok(FoundCrate::Itself) => quote!(crate),
        Ok(FoundCrate::Name(name)) => {
            let ident = proc_macro2::Ident::new(&name, proc_macro2::Span::call_site());
            quote!(::#ident)
        }
        // Standalone use of the macros crate (tests, doctests): keep the conventional name.
        Err(_) => quote!(::roundhouse),
    }
}

/// Append a suffix to the final segment: `handlers::show` → `handlers::show_SUFFIX`.
pub fn append_to_path(path: &Path, suffix: &str) -> Path {
    let mut path = path.clone();
    if let Some(last) = path.segments.last_mut() {
        last.ident = format_ident!("{}{}", last.ident, suffix, span = last.ident.span());
    }
    path
}

pub fn marker_type(path: &Path) -> Path {
    append_to_path(path, "_missing_roundhouse_handler_attribute")
}

pub fn last_segment(path: &Path) -> String {
    path.segments
        .last()
        .map(|segment| segment.ident.to_string())
        .unwrap_or_else(|| "handler".to_string())
}

pub fn string_slice(literals: &[LitStr]) -> TokenStream {
    if literals.is_empty() {
        quote!(&[])
    } else {
        quote!(&[#(#literals),*])
    }
}

pub fn parse_u16(lit: &LitInt) -> syn::Result<u16> {
    lit.base10_parse::<u16>()
}

/// Extract the single generic type argument from `Path<T>`-like types.
pub fn single_generic(ty: &syn::Type) -> Option<syn::Type> {
    let syn::Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    let syn::PathArguments::AngleBracketed(args) = &segment.arguments else {
        return None;
    };
    args.args.iter().find_map(|arg| match arg {
        syn::GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    })
}

pub fn outer_ident(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(type_path) = ty else {
        return None;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

pub fn is_option(ty: &syn::Type) -> Option<syn::Type> {
    (outer_ident(ty).as_deref() == Some("Option"))
        .then(|| single_generic(ty))
        .flatten()
}

pub fn is_vec(ty: &syn::Type) -> Option<syn::Type> {
    (outer_ident(ty).as_deref() == Some("Vec"))
        .then(|| single_generic(ty))
        .flatten()
}
