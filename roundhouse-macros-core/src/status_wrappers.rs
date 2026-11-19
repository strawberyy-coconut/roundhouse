use proc_macro2::{Ident, Span, TokenStream};
use quote::quote;
use roundhouse_schema::status_codes::{self, StatusKind};

/// Emit one helper-macro invocation per vendored HTTP status code.
///
/// The helper macros live in `roundhouse::status` and own the actual type definitions. The proc
/// macro only reads the shared JSON catalog and turns it into those invocations.
pub fn expand(input: TokenStream) -> TokenStream {
    if !input.is_empty() {
        return syn::Error::new(
            Span::call_site(),
            "`http_status_wrappers!` takes no arguments",
        )
        .to_compile_error();
    }

    let mut output = TokenStream::new();
    for spec in status_codes::all() {
        let ident = Ident::new(&spec.ident, Span::call_site());
        let code = spec.code;
        let doc = match spec.reference.as_deref() {
            Some(reference) if !reference.is_empty() => {
                format!("`{} {}` — {}.", spec.code, spec.phrase, reference)
            }
            _ => format!("`{} {}`.", spec.code, spec.phrase),
        };
        let doc = syn::LitStr::new(&doc, Span::call_site());
        let invocation = match spec.kind {
            StatusKind::Body => quote!(__roundhouse_status_body!(#ident, #code, #doc);),
            StatusKind::Empty => quote!(__roundhouse_status_empty!(#ident, #code, #doc);),
        };
        output.extend(invocation);
    }
    output
}
