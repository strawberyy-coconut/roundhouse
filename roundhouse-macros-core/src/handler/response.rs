use proc_macro2::TokenStream;
use quote::quote;
use syn::{GenericArgument, ItemFn, PathArguments, ReturnType, Type};

use super::index::{self, ResponseSite};

/// Expand the handler's documented responses.
///
/// Named return types go through the existing autoref probe. Opaque return types (`impl Trait`)
/// cannot be named by the macro, so the build-time analyzer records the concrete types at the
/// body's tail and `return` sites. The macro re-emits them as typed trait calls, and the compiler
/// verifies the analysis.
pub(super) fn response_statements(item: &ItemFn) -> Vec<TokenStream> {
    let krate = crate::util::krate();
    let ReturnType::Type(_, ty) = &item.sig.output else {
        return named_response_statements(&quote!(()));
    };
    if !contains_impl_trait(ty) {
        return named_response_statements(&quote!(#ty));
    }
    opaque_response_statements(item, &krate)
}

fn named_response_statements(ty: &TokenStream) -> Vec<TokenStream> {
    let krate = crate::util::krate();
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

fn opaque_response_statements(item: &ItemFn, krate: &TokenStream) -> Vec<TokenStream> {
    let mut statements = Vec::new();
    let mut needs_async = false;
    let mut needs_sync = false;
    let mut needs_never = false;

    let sites: Vec<ResponseSite> = match index::lookup(item) {
        Some(entry) => {
            if !entry.unresolved.is_empty() {
                statements.push(quote! {
                    docs.verification.mark_response();
                });
            }
            entry.responses
        }
        None => {
            // No index entry: the analyzer did not see this handler (for example, a
            // macro-generated body) or the contract build did not run. Keep the operation honest.
            statements.push(quote! {
                docs.verification.mark_response();
            });
            Vec::new()
        }
    };

    for site in &sites {
        if let ResponseSite::Call { is_async, .. } = site {
            needs_never = true;
            if *is_async {
                needs_async = true;
            } else {
                needs_sync = true;
            }
        }
    }

    if needs_never {
        statements.push(quote! {
            fn __rh_never<T>() -> T {
                panic!("roundhouse: documentation witness was executed")
            }
        });
    }
    if needs_async {
        statements.push(quote! {
            fn __rh_infer_async<Fut, Out>(
                _: impl ::core::ops::FnOnce() -> Fut,
                context: &mut dyn #krate::doc::SchemaContext,
            ) -> ::std::vec::Vec<#krate::doc::ResponseDoc>
            where
                Fut: ::core::future::Future<Output = Out>,
                Out: #krate::doc::DocumentedResponse,
            {
                <Out as #krate::doc::DocumentedResponse>::responses(context)
            }
        });
    }
    if needs_sync {
        statements.push(quote! {
            fn __rh_infer_sync<Out>(
                _: impl ::core::ops::FnOnce() -> Out,
                context: &mut dyn #krate::doc::SchemaContext,
            ) -> ::std::vec::Vec<#krate::doc::ResponseDoc>
            where
                Out: #krate::doc::DocumentedResponse,
            {
                <Out as #krate::doc::DocumentedResponse>::responses(context)
            }
        });
    }

    for site in &sites {
        statements.push(emit_site(site, krate));
    }

    let (ok, error) = return_shape(&item.sig.output);

    if let Some(error) = &error {
        match error {
            ShapeType::Opaque => {
                // An opaque error type cannot be named by the macro.
                statements.push(quote! {
                    docs.verification.mark_response();
                });
            }
            ShapeType::Named(ty) => {
                let ty = ty.as_ref();
                statements.push(quote! {
                    docs.extend_responses(
                        <#ty as #krate::doc::DocumentedError>::responses(context),
                    );
                });
            }
        }
    }

    if let (Shape::ImplTrait, None) = (&ok, &error)
        && is_documented_error_only(&item.sig.output)
    {
        // The handler only returns an error-shaped opaque type; there is no success to document.
        statements.push(quote! {
            docs.verification.mark_response();
        });
    }

    if ok == Shape::Wrapped {
        // An opaque response hidden inside `Option` or another wrapper has paths the analyzer
        // cannot see (for example `None`). Keep the marker honest.
        statements.push(quote! {
            docs.verification.mark_response();
        });
    }

    if sites.is_empty() {
        statements.push(quote! {
            docs.verification.mark_response();
        });
    }

    statements
}

fn emit_site(site: &ResponseSite, krate: &TokenStream) -> TokenStream {
    match site {
        ResponseSite::Type { ty } => match syn::parse_str::<Type>(ty) {
            Ok(mut ty) => {
                rewrite_known_types(&mut ty, krate);
                quote! {
                    docs.extend_responses(
                        <#ty as #krate::doc::DocumentedResponse>::responses(context),
                    );
                }
            }
            Err(_) => quote! {
                docs.verification.mark_response();
            },
        },
        ResponseSite::StatusTuple { inner } => match syn::parse_str::<Type>(inner) {
            Ok(mut inner) => {
                rewrite_known_types(&mut inner, krate);
                quote! {
                    docs.extend_responses(
                        <(#krate::axum::http::StatusCode, #inner) as #krate::doc::DocumentedResponse>::responses(context),
                    );
                }
            }
            Err(_) => quote! {
                docs.verification.mark_response();
            },
        },
        ResponseSite::Call {
            path,
            arity,
            is_async,
        } => match syn::parse_str::<syn::Path>(path) {
            Ok(path) => {
                let arguments = (0..*arity).map(|_| quote!(__rh_never()));
                let helper = if *is_async {
                    quote!(__rh_infer_async)
                } else {
                    quote!(__rh_infer_sync)
                };
                quote! {
                    docs.extend_responses(
                        #helper(|| #path(#(#arguments),*), context),
                    );
                }
            }
            Err(_) => quote! {
                docs.verification.mark_response();
            },
        },
        ResponseSite::StatusCode => quote! {
            docs.extend_responses(
                <#krate::axum::http::StatusCode as #krate::doc::DocumentedResponse>::responses(context),
            );
        },
        ResponseSite::Redirect => quote! {
            docs.extend_responses(
                <#krate::axum::response::Redirect as #krate::doc::DocumentedResponse>::responses(context),
            );
        },
    }
}

/// Rewrite response wrapper names that may not be imported in the handler's module.
///
/// The analyzer records types as they are spelled where it resolved them; a helper's return type
/// can name `Html` or `Bytes` in a module the handler does not import from. The trait impls live
/// in known axum/std locations, so qualifying these names makes the generated call robust.
fn rewrite_known_types(ty: &mut Type, krate: &TokenStream) {
    match ty {
        Type::Path(path) if path.qself.is_none() && path.path.segments.len() == 1 => {
            let segment = path.path.segments.first().expect("one segment");
            let ident = segment.ident.to_string();
            let arguments = segment.arguments.clone();
            let base = match ident.as_str() {
                "Bytes" => Some(quote!(#krate::axum::body::Bytes)),
                "Body" => Some(quote!(#krate::axum::body::Body)),
                "Html" => Some(quote!(#krate::axum::response::Html)),
                "Sse" => Some(quote!(#krate::axum::response::sse::Sse)),
                "Json" => Some(quote!(#krate::axum::Json)),
                "Created" => Some(quote!(#krate::Created)),
                "NoContent" => Some(quote!(#krate::NoContent)),
                "Problem" => Some(quote!(#krate::Problem)),
                "Attachment" => Some(quote!(#krate::Attachment)),
                "File" => Some(quote!(#krate::File)),
                "String" => Some(quote!(::std::string::String)),
                _ => None,
            };
            if let Some(base) = base {
                *ty = syn::parse_quote!(#base #arguments);
            }
        }
        Type::Reference(reference) => rewrite_known_types(&mut reference.elem, krate),
        Type::Paren(paren) => rewrite_known_types(&mut paren.elem, krate),
        Type::Group(group) => rewrite_known_types(&mut group.elem, krate),
        Type::Tuple(tuple) => {
            for element in &mut tuple.elems {
                rewrite_known_types(element, krate);
            }
        }
        Type::Path(path) => {
            for segment in &mut path.path.segments {
                if let PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
                    for argument in &mut arguments.args {
                        if let GenericArgument::Type(ty) = argument {
                            rewrite_known_types(ty, krate);
                        }
                    }
                }
            }
        }
        _ => {}
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Named,
    ImplTrait,
    Wrapped,
}

enum ShapeType {
    Named(Box<Type>),
    Opaque,
}

fn return_shape(output: &ReturnType) -> (Shape, Option<ShapeType>) {
    let ReturnType::Type(_, ty) = output else {
        return (Shape::Named, None);
    };
    if let Some((ok, error)) = result_parts(ty) {
        let ok_shape = if contains_impl_trait(&ok) {
            Shape::ImplTrait
        } else {
            Shape::Named
        };
        let error_shape = if contains_impl_trait(&error) {
            ShapeType::Opaque
        } else {
            ShapeType::Named(Box::new(error))
        };
        return (ok_shape, Some(error_shape));
    }
    match ty.as_ref() {
        Type::ImplTrait(_) if contains_impl_trait(ty) => (Shape::ImplTrait, None),
        ty if contains_impl_trait(ty) => (Shape::Wrapped, None),
        _ => (Shape::Named, None),
    }
}

fn result_parts(ty: &Type) -> Option<(Type, Type)> {
    let Type::Path(path) = ty else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if segment.ident != "Result" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let mut types = arguments.args.iter().filter_map(|argument| match argument {
        GenericArgument::Type(ty) => Some(ty.clone()),
        _ => None,
    });
    Some((types.next()?, types.next()?))
}

fn is_documented_error_only(output: &ReturnType) -> bool {
    let ReturnType::Type(_, ty) = output else {
        return false;
    };
    let Type::ImplTrait(impl_trait) = ty.as_ref() else {
        return false;
    };
    let mut has_error = false;
    let mut has_response = false;
    for bound in &impl_trait.bounds {
        let syn::TypeParamBound::Trait(bound) = bound else {
            continue;
        };
        let Some(segment) = bound.path.segments.last() else {
            continue;
        };
        match segment.ident.to_string().as_str() {
            "DocumentedError" => has_error = true,
            "DocumentedResponse" | "IntoResponse" => has_response = true,
            _ => {}
        }
    }
    has_error && !has_response
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
            if let PathArguments::AngleBracketed(arguments) = &segment.arguments {
                arguments.args.iter().any(|argument| match argument {
                    GenericArgument::Type(ty) => contains_impl_trait(ty),
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
    fn impl_trait_is_opaque() {
        for source in [
            "impl IntoResponse",
            "Json<impl ApiSchema>",
            "Result<impl IntoResponse, Problem>",
        ] {
            let ty: Type = syn::parse_str(source).expect("valid type");
            assert!(contains_impl_trait(&ty), "{source}");
        }
    }

    #[test]
    fn result_parts_reads_the_error_type() {
        let ty: Type = syn::parse_str("Result<impl IntoResponse, DbError>").unwrap();
        let (ok, error) = result_parts(&ty).expect("result");
        assert!(contains_impl_trait(&ok));
        assert!(!contains_impl_trait(&error));
    }
}
