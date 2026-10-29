//! Declared documentation for tower layers that terminate requests.

use super::{BodyDoc, HeaderDoc, Param, ResponseDoc};
use crate::auth::{AuthScheme, SchemeDoc};

/// What a layer guarantees, merged into every operation beneath its mount.
#[derive(Clone, Default)]
pub struct LayerDoc {
    pub responses: Vec<ResponseDoc>,
    pub parameters: Vec<Param>,
    pub body: Option<BodyDoc>,
    pub required_auth: Vec<crate::auth::AuthDoc>,
    pub optional_auth: Vec<crate::auth::AuthDoc>,
    pub schemes: Vec<SchemeDoc>,
}

impl LayerDoc {
    pub fn none() -> Self {
        Self::default()
    }
}

fn terminating(status: u16, title: &str, headers: Vec<HeaderDoc>) -> LayerDoc {
    let mut response = ResponseDoc::problem(status, title);
    response.headers = headers;
    LayerDoc {
        responses: vec![response],
        ..LayerDoc::default()
    }
}

/// A rate limiter answering `429`, with the `Retry-After` response header.
pub fn rate_limit(status: u16) -> LayerDoc {
    terminating(
        status,
        "Too Many Requests",
        vec![HeaderDoc {
            name: "Retry-After".to_string(),
            schema: super::schema::integer(utoipa::openapi::schema::KnownFormat::Int64),
            required: true,
            append: false,
        }],
    )
}

/// A timeout layer answering `504`.
pub fn timeout(status: u16) -> LayerDoc {
    terminating(status, "Gateway Timeout", Vec::new())
}

/// A body-size limit answering `413`.
pub fn body_limit(status: u16) -> LayerDoc {
    terminating(status, "Payload Too Large", Vec::new())
}

/// An auth-enforcing layer: the same security and rejection docs as `Auth<A>`.
pub fn auth<A: AuthScheme>() -> LayerDoc {
    LayerDoc {
        responses: super::auth_failures::<A>(),
        required_auth: vec![crate::auth::auth_doc::<A, crate::auth::NoScopes>()],
        optional_auth: Vec::new(),
        schemes: vec![crate::auth::auth_scheme::<A>()],
        ..LayerDoc::default()
    }
}
