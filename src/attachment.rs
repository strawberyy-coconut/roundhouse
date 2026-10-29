//! Attachment, File and SSE response helpers.

use axum::body::Body;
use axum::http::{HeaderValue, header};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use utoipa::openapi::schema::{KnownFormat, SchemaFormat};

use crate::doc::{self, HeaderDoc, ResponseDoc};

/// Content type implied by an attachment body.
pub trait AttachmentType {
    const CONTENT_TYPE: &'static str;
}

impl AttachmentType for Vec<u8> {
    const CONTENT_TYPE: &'static str = "application/octet-stream";
}
impl AttachmentType for Bytes {
    const CONTENT_TYPE: &'static str = "application/octet-stream";
}
impl AttachmentType for String {
    const CONTENT_TYPE: &'static str = "text/plain; charset=utf-8";
}
impl AttachmentType for &'static str {
    const CONTENT_TYPE: &'static str = "text/plain; charset=utf-8";
}
impl AttachmentType for &'static [u8] {
    const CONTENT_TYPE: &'static str = "application/octet-stream";
}

/// A download body with a filename and an explicit media type.
pub struct Attachment<T> {
    pub body: T,
    pub filename: Option<String>,
    pub content_type: &'static str,
}

impl<T: AttachmentType> Attachment<T> {
    pub fn new(body: T) -> Self {
        Self {
            body,
            filename: None,
            content_type: T::CONTENT_TYPE,
        }
    }

    pub fn filename(mut self, filename: impl Into<String>) -> Self {
        self.filename = Some(filename.into());
        self
    }

    pub fn content_type(mut self, content_type: &'static str) -> Self {
        self.content_type = content_type;
        self
    }
}

impl<T: Into<Body>> IntoResponse for Attachment<T> {
    fn into_response(self) -> Response {
        let mut builder = Response::builder()
            .status(axum::http::StatusCode::OK)
            .header(header::CONTENT_TYPE, self.content_type);
        if let Some(filename) = &self.filename {
            let safe = filename.replace(['"', '\\', '\r', '\n'], "");
            if let Ok(value) = HeaderValue::try_from(format!("attachment; filename=\"{safe}\"")) {
                builder = builder.header(header::CONTENT_DISPOSITION, value);
            }
        }
        builder
            .body(self.body.into())
            .unwrap_or_else(|_| axum::http::StatusCode::INTERNAL_SERVER_ERROR.into_response())
    }
}

/// A raw file response, defaulting to `application/octet-stream`.
pub struct File {
    pub bytes: Bytes,
    pub content_type: &'static str,
}

impl File {
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self {
            bytes: bytes.into(),
            content_type: "application/octet-stream",
        }
    }

    pub fn content_type(mut self, content_type: &'static str) -> Self {
        self.content_type = content_type;
        self
    }
}

impl IntoResponse for File {
    fn into_response(self) -> Response {
        (
            [
                (header::CONTENT_TYPE, self.content_type),
                (header::CONTENT_DISPOSITION, "attachment"),
            ],
            self.bytes,
        )
            .into_response()
    }
}

pub fn attachment_response<T: AttachmentType>(status: u16) -> ResponseDoc {
    ResponseDoc {
        status,
        description: doc::canonical_reason(status).to_string(),
        content_type: Some(T::CONTENT_TYPE.to_string()),
        schema: Some(doc::schema::string_with_format(SchemaFormat::KnownFormat(
            KnownFormat::Binary,
        ))),
        headers: vec![HeaderDoc {
            name: "Content-Disposition".to_string(),
            schema: doc::schema::string(),
            required: true,
            append: false,
        }],
    }
}

pub fn file_response(status: u16) -> ResponseDoc {
    ResponseDoc {
        status,
        description: doc::canonical_reason(status).to_string(),
        content_type: Some("application/octet-stream".to_string()),
        schema: Some(doc::schema::string_with_format(SchemaFormat::KnownFormat(
            KnownFormat::Binary,
        ))),
        headers: vec![HeaderDoc {
            name: "Content-Disposition".to_string(),
            schema: doc::schema::string(),
            required: false,
            append: false,
        }],
    }
}

pub fn sse_response(status: u16) -> ResponseDoc {
    ResponseDoc {
        status,
        description: doc::canonical_reason(status).to_string(),
        content_type: Some("text/event-stream".to_string()),
        schema: Some(doc::schema::any_value()),
        headers: Vec::new(),
    }
}
