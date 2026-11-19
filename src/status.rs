//! Reusable HTTP status response wrappers.
//!
//! [`Status<CODE, T>`](Status) and [`Empty<CODE>`](Empty) are the universal primitives: they work
//! for any valid HTTP status code without a per-code definition. The named wrappers
//! ([`Created`], [`NoContent`], [`NotFound`], …) come from the shared IANA catalog; a proc macro
//! reads the JSON and expands the internal `macro_rules!` below.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

#[cfg(feature = "build")]
use crate::doc::{DocumentedError, DocumentedResponse, ResponseDoc, SchemaContext, json_response};

/// A response with a type-level status code and a JSON body.
///
/// Unlike `(StatusCode, T)`, the status is part of the type, so the document records the exact
/// code instead of OpenAPI's `default`.
///
/// ```ignore
/// async fn create() -> Status<201, Note> {
///     Status::new(note)
/// }
/// ```
///
/// `T` is serialized as JSON. Use [`Empty`] for statuses that must not carry a body.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Status<const CODE: u16, T>(pub T);

impl<const CODE: u16, T> Status<CODE, T> {
    const STATUS: StatusCode = match StatusCode::from_u16(CODE) {
        Ok(status) => status,
        Err(_) => panic!("`Status<CODE, T>` requires a valid HTTP status code"),
    };

    pub fn new(body: T) -> Self {
        // Force the associated const's validity check when `new` is monomorphized.
        let _ = Self::STATUS;
        Self(body)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<const CODE: u16, T: serde::Serialize> IntoResponse for Status<CODE, T> {
    fn into_response(self) -> Response {
        (Self::STATUS, axum::Json(self.0)).into_response()
    }
}

#[cfg(feature = "build")]
impl<const CODE: u16, T: crate::doc::ApiSchema> DocumentedResponse for Status<CODE, T> {
    fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<T>(context, CODE)
    }
}

#[cfg(feature = "build")]
impl<const CODE: u16, T: crate::doc::ApiSchema> DocumentedError for Status<CODE, T> {
    fn responses(context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        json_response::<T>(context, CODE)
    }
}

/// A status-only response with a type-level status code and no body.
///
/// Use this for `204`, `205`, `304`, or any other response whose status forbids content.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Empty<const CODE: u16>;

impl<const CODE: u16> Empty<CODE> {
    const STATUS: StatusCode = match StatusCode::from_u16(CODE) {
        Ok(status) => status,
        Err(_) => panic!("`Empty<CODE>` requires a valid HTTP status code"),
    };

    pub const fn new() -> Self {
        // Force the associated const's validity check when `new` is monomorphized.
        let _ = Self::STATUS;
        Self
    }
}

impl<const CODE: u16> IntoResponse for Empty<CODE> {
    fn into_response(self) -> Response {
        Self::STATUS.into_response()
    }
}

#[cfg(feature = "build")]
impl<const CODE: u16> DocumentedResponse for Empty<CODE> {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::new(CODE, crate::doc::canonical_reason(CODE))]
    }
}

#[cfg(feature = "build")]
impl<const CODE: u16> DocumentedError for Empty<CODE> {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::new(CODE, crate::doc::canonical_reason(CODE))]
    }
}

macro_rules! __roundhouse_status_body {
    ($name:ident, $code:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name<T>(pub T);

        impl<T> $name<T> {
            pub fn new(body: T) -> Self {
                Self(body)
            }

            pub fn into_inner(self) -> T {
                self.0
            }
        }

        impl<T: ::serde::Serialize> ::axum::response::IntoResponse for $name<T> {
            fn into_response(self) -> ::axum::response::Response {
                let status = ::axum::http::StatusCode::from_u16($code)
                    .expect("the generated status code is valid");
                ::axum::response::IntoResponse::into_response((status, ::axum::Json(self.0)))
            }
        }

        #[cfg(feature = "build")]
        impl<T: crate::doc::ApiSchema> crate::doc::DocumentedResponse for $name<T> {
            fn responses(
                context: &mut dyn crate::doc::SchemaContext,
            ) -> Vec<crate::doc::ResponseDoc> {
                crate::doc::json_response::<T>(context, $code)
            }
        }

        #[cfg(feature = "build")]
        impl<T: crate::doc::ApiSchema> crate::doc::DocumentedError for $name<T> {
            fn responses(
                context: &mut dyn crate::doc::SchemaContext,
            ) -> Vec<crate::doc::ResponseDoc> {
                crate::doc::json_response::<T>(context, $code)
            }
        }
    };
}

macro_rules! __roundhouse_status_empty {
    ($name:ident, $code:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name;

        impl $name {
            pub const fn new() -> Self {
                Self
            }
        }

        impl ::axum::response::IntoResponse for $name {
            fn into_response(self) -> ::axum::response::Response {
                let status = ::axum::http::StatusCode::from_u16($code)
                    .expect("the generated status code is valid");
                ::axum::response::IntoResponse::into_response(status)
            }
        }

        #[cfg(feature = "build")]
        impl crate::doc::DocumentedResponse for $name {
            fn responses(
                _context: &mut dyn crate::doc::SchemaContext,
            ) -> Vec<crate::doc::ResponseDoc> {
                vec![crate::doc::ResponseDoc::new(
                    $code,
                    crate::doc::canonical_reason($code),
                )]
            }
        }

        #[cfg(feature = "build")]
        impl crate::doc::DocumentedError for $name {
            fn responses(
                _context: &mut dyn crate::doc::SchemaContext,
            ) -> Vec<crate::doc::ResponseDoc> {
                vec![crate::doc::ResponseDoc::new(
                    $code,
                    crate::doc::canonical_reason($code),
                )]
            }
        }
    };
}

// The proc macro reads the shared JSON catalog and emits these helper invocations.
roundhouse_macros::http_status_wrappers!();
