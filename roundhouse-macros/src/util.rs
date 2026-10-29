use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::parse::ParseStream;
use syn::{LitInt, LitStr, Path, Type};

/// The path to the `roundhouse` crate as the *expanding* crate names it.
///
/// Expansions must not hardcode `::roundhouse`: a renamed dependency
/// (`roundhouse = { package = "roundhouse", ... }` under another key) would fail to resolve
/// every generated item.
pub fn krate() -> TokenStream {
    use proc_macro_crate::{FoundCrate, crate_name};

    // Rustdoc compiles doctests as a temporary crate but reads this package's manifest, so
    // `proc_macro_crate` reports `Itself`. The doctest still links the real crate by name, and
    // `crate` would point at the temporary crate. Rustdoc marks those compilations with
    // `UNSTABLE_RUSTDOC_TEST_PATH`.
    if std::env::var_os("UNSTABLE_RUSTDOC_TEST_PATH").is_some() {
        return quote!(::roundhouse);
    }

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

/// Parse a `security = [..]` list. Entries are scheme *types*: the guard is monomorphized over
/// `A`, so the declaration and the check cannot drift apart.
pub fn parse_security_types(input: ParseStream<'_>) -> syn::Result<Vec<Type>> {
    let content;
    syn::bracketed!(content in input);
    let mut values: Vec<Type> = Vec::new();
    while !content.is_empty() {
        if content.peek(LitStr) {
            let literal: LitStr = content.parse()?;
            return Err(syn::Error::new(
                literal.span(),
                format!(
                    "security takes scheme types, not the string `{}`; name the type that \
                     implements `AuthScheme` (for example `Bearer`)",
                    literal.value()
                ),
            ));
        }
        values.push(content.parse::<Type>()?);
        if content.peek(syn::Token![,]) {
            content.parse::<syn::Token![,]>()?;
        } else {
            break;
        }
    }
    for ty in &values {
        if matches!(outer_ident(ty).as_deref(), Some("Auth") | Some("Option")) {
            return Err(syn::Error::new_spanned(
                ty,
                "security names a scheme type, not `Auth<..>` or `Option<..>`; write `Bearer` \
                 and put scopes or optionality on the handler argument",
            ));
        }
    }
    Ok(values)
}

/// Expand scheme types into the const slice a `RouteRecord`/`MountRecord` stores.
pub fn auth_slice(types: &[Type]) -> TokenStream {
    if types.is_empty() {
        quote!(&[])
    } else {
        let krate = krate();
        quote!(&[#(#krate::auth::AuthGuard::__new::<#types>()),*])
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
