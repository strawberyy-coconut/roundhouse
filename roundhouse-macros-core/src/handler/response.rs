use proc_macro2::TokenStream;
use quote::quote;
use syn::{ReturnType, Type};

/// Expand the handler's declared return type into response documentation.
///
/// The type itself owns the documented status, content type, schema and headers through
/// [`DocumentedResponse`](roundhouse::doc::DocumentedResponse). A bare `impl Trait` return type
/// cannot be named, so it contributes nothing; the contract build requires an explicit
/// declaration for those handlers.
pub(super) fn response_statements(output: &ReturnType) -> Vec<TokenStream> {
    let krate = crate::util::krate();
    let ty = match output {
        ReturnType::Default => quote!(()),
        ReturnType::Type(_, ty) if contains_impl_trait(ty) => return Vec::new(),
        ReturnType::Type(_, ty) => quote!(#ty),
    };
    vec![quote! {
        match {
            use #krate::__private::ProbeResponses as _;
            (&#krate::__private::ResponseProbe::<#ty>::new()).probe_responses(context)
        } {
            ::core::option::Option::Some(__rh_responses) => {
                docs.extend_responses(__rh_responses);
            }
            // The type does not implement `DocumentedResponse`. Keep the handler compiling; the
            // strict contract build rejects the missing success unless a declaration covers it.
            ::core::option::Option::None => {}
        }
    }]
}

fn contains_impl_trait(ty: &Type) -> bool {
    match ty {
        Type::ImplTrait(_) => true,
        Type::Array(array) => contains_impl_trait(&array.elem),
        Type::Group(group) => contains_impl_trait(&group.elem),
        Type::Paren(paren) => contains_impl_trait(&paren.elem),
        Type::Ptr(ptr) => contains_impl_trait(&ptr.elem),
        Type::Reference(reference) => contains_impl_trait(&reference.elem),
        Type::Slice(slice) => contains_impl_trait(&slice.elem),
        Type::Tuple(tuple) => tuple.elems.iter().any(contains_impl_trait),
        Type::Path(path) => path.path.segments.iter().any(|segment| {
            if let syn::PathArguments::AngleBracketed(arguments) = &segment.arguments {
                arguments.args.iter().any(|argument| match argument {
                    syn::GenericArgument::Type(ty) => contains_impl_trait(ty),
                    _ => false,
                })
            } else {
                false
            }
        }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn named_types_are_documented() {
        for source in ["Json<Note>", "Result<Json<Note>, Problem>", "&'static str"] {
            let ty: Type = syn::parse_str(source).expect("valid type");
            assert!(!contains_impl_trait(&ty), "{source}");
        }
    }

    #[test]
    fn impl_trait_is_never_documented() {
        for source in [
            "impl IntoResponse",
            "Json<impl ApiSchema>",
            "Sse<impl Stream>",
        ] {
            let ty: Type = syn::parse_str(source).expect("valid type");
            assert!(contains_impl_trait(&ty), "{source}");
        }
    }
}
