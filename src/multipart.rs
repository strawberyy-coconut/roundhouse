//! One-pass multipart extraction into a generated struct.

use bytes::{Bytes, BytesMut};
use serde::de::DeserializeOwned;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, ObjectBuilder, Schema, Type};

use crate::doc::ApiSchema;
use crate::problem::Problem;
use crate::validate::{ValidationError, ValidationErrors};

const MAX_PART_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 32 * 1024 * 1024;

/// One part read off the connection.
#[derive(Clone)]
pub struct Part {
    pub name: String,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub bytes: Bytes,
}

/// A buffered file field.
#[derive(Clone)]
pub struct FilePart {
    pub bytes: Bytes,
    pub file_name: Option<String>,
    pub content_type: Option<String>,
}

impl FilePart {
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl std::fmt::Debug for FilePart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilePart")
            .field("len", &self.bytes.len())
            .field("file_name", &self.file_name)
            .field("content_type", &self.content_type)
            .finish()
    }
}

/// A file streamed to an already-unlinked temporary file.
pub struct TempFile {
    file: std::fs::File,
    len: u64,
}

impl TempFile {
    pub fn len(&self) -> u64 {
        self.len
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn into_file(self) -> std::fs::File {
        self.file
    }
}

impl std::io::Read for TempFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::{Seek, SeekFrom};
        self.file.seek(SeekFrom::Start(0))?;
        std::io::Read::read(&mut self.file, buf)
    }
}

impl std::fmt::Debug for TempFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TempFile").field("len", &self.len).finish()
    }
}

/// Implemented by `#[derive(Multipart)]`.
#[allow(clippy::result_large_err)]
pub trait MultipartForm: Sized {
    fn from_parts(parts: Vec<Part>) -> Result<Self, Problem>;
    fn schema(components: &mut Components) -> RefOr<Schema>;
    fn deny_unknown() -> bool {
        false
    }
}

/// Multipart extractor. One pass; limits are checked while reading.
pub struct Multipart<F>(pub F);

impl<F> Multipart<F> {
    pub fn into_inner(self) -> F {
        self.0
    }
}

impl<F> std::ops::Deref for Multipart<F> {
    type Target = F;
    fn deref(&self) -> &F {
        &self.0
    }
}

#[allow(clippy::result_large_err)]
impl<F, S> axum::extract::FromRequest<S> for Multipart<F>
where
    F: MultipartForm + Send,
    S: Send + Sync,
{
    type Rejection = Problem;

    async fn from_request(
        request: axum::extract::Request,
        state: &S,
    ) -> Result<Self, Self::Rejection> {
        let mut multipart = axum::extract::Multipart::from_request(request, state)
            .await
            .map_err(|rejection| {
                Problem::from_rejection(
                    rejection.status(),
                    rejection
                        .status()
                        .canonical_reason()
                        .unwrap_or("Bad Request"),
                    rejection.body_text(),
                )
            })?;

        let mut parts = Vec::new();
        let mut total = 0usize;

        while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
            let name = field.name().unwrap_or_default().to_string();
            let file_name = field
                .file_name()
                .map(|name| sanitize_file_name(name.to_string()));
            let content_type = field.content_type().map(str::to_string);
            let mut bytes = BytesMut::new();

            while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
                if bytes.len() + chunk.len() > MAX_PART_BYTES {
                    return Err(limit_problem(&name, MAX_PART_BYTES));
                }
                total += chunk.len();
                if total > MAX_TOTAL_BYTES {
                    return Err(limit_problem(&name, MAX_TOTAL_BYTES));
                }
                bytes.extend_from_slice(&chunk);
            }

            parts.push(Part {
                name,
                file_name,
                content_type,
                bytes: bytes.freeze(),
            });
        }

        F::from_parts(parts).map(Multipart)
    }
}

fn multipart_error(error: axum::extract::multipart::MultipartError) -> Problem {
    Problem::from_rejection(
        error.status(),
        error.status().canonical_reason().unwrap_or("Bad Request"),
        error.body_text(),
    )
}

fn limit_problem(name: &str, limit: usize) -> Problem {
    let mut errors = ValidationErrors::new();
    errors.push(
        ValidationError::new(
            crate::validate::pointer("", name),
            "max_bytes",
            format!("part exceeds the {limit} byte limit"),
        )
        .with_limit(limit),
    );
    Problem::from_validation(errors)
}

fn sanitize_file_name(name: String) -> String {
    let name = name.replace('\0', "");
    let name = name.rsplit(['/', '\\']).next().unwrap_or(&name);
    if name == ".." || name.is_empty() {
        "file".to_string()
    } else {
        name.to_string()
    }
}

/// Take all parts with one name, preserving arrival order.
pub fn take_parts(parts: &mut Vec<Part>, name: &str) -> Vec<Part> {
    let mut taken = Vec::new();
    let mut index = 0;
    while index < parts.len() {
        if parts[index].name == name {
            taken.push(parts.remove(index));
        } else {
            index += 1;
        }
    }
    taken
}

/// Pick the most specific status for a collected multipart failure set.
pub fn problem(errors: ValidationErrors) -> Problem {
    let status = if errors.errors.iter().any(|error| error.code == "max_bytes") {
        axum::http::StatusCode::PAYLOAD_TOO_LARGE
    } else if errors.errors.iter().any(|error| error.code == "accept") {
        axum::http::StatusCode::UNSUPPORTED_MEDIA_TYPE
    } else {
        axum::http::StatusCode::BAD_REQUEST
    };
    Problem::from_validation_with_status(errors, status)
}

pub fn missing(name: &str, errors: &mut ValidationErrors) {
    errors.push(ValidationError::new(
        crate::validate::pointer("", name),
        "required",
        "field is required",
    ));
}

pub fn text<T: DeserializeOwned + ApiSchema>(
    part: Part,
    name: &str,
    errors: &mut ValidationErrors,
) -> Option<T> {
    match parse_text::<T>(part) {
        Ok(value) => Some(value),
        Err(message) => {
            errors.push(ValidationError::new(
                crate::validate::pointer("", name),
                "invalid",
                message,
            ));
            None
        }
    }
}

fn parse_text<T: DeserializeOwned>(part: Part) -> Result<T, String> {
    let text = std::str::from_utf8(&part.bytes).map_err(|_| "part is not UTF-8".to_string())?;
    serde_json::from_str::<T>(text)
        .or_else(|_| serde_json::from_value::<T>(serde_json::Value::String(text.to_string())))
        .map_err(|error| error.to_string())
}

pub fn file(
    part: Part,
    max_bytes: Option<usize>,
    accept: &[&str],
    name: &str,
    errors: &mut ValidationErrors,
) -> Option<FilePart> {
    if let Some(max) = max_bytes
        && part.bytes.len() > max
    {
        errors.push(
            ValidationError::new(
                crate::validate::pointer("", name),
                "max_bytes",
                format!("file exceeds the {max} byte limit"),
            )
            .with_limit(max),
        );
        return None;
    }
    if !accept.is_empty() {
        let accepted = part
            .content_type
            .as_deref()
            .is_some_and(|content_type| accept.contains(&content_type));
        if !accepted {
            errors.push(ValidationError::new(
                crate::validate::pointer("", name),
                "accept",
                format!("expected one of: {}", accept.join(", ")),
            ));
            return None;
        }
    }
    Some(FilePart {
        bytes: part.bytes,
        file_name: part.file_name,
        content_type: part.content_type,
    })
}

pub fn temp_file(
    part: Part,
    max_bytes: Option<usize>,
    accept: &[&str],
    name: &str,
    errors: &mut ValidationErrors,
) -> Option<TempFile> {
    use std::io::Write;

    let part = file(part, max_bytes, accept, name, errors)?;
    let mut file = tempfile::tempfile()
        .map_err(|error| {
            errors.push(ValidationError::new(
                crate::validate::pointer("", name),
                "temp_file",
                error.to_string(),
            ));
        })
        .ok()?;
    if let Err(error) = file.write_all(&part.bytes) {
        errors.push(ValidationError::new(
            crate::validate::pointer("", name),
            "temp_file",
            error.to_string(),
        ));
        return None;
    }
    Some(TempFile {
        len: part.bytes.len() as u64,
        file,
    })
}

pub fn schema_object() -> ObjectBuilder {
    ObjectBuilder::new().schema_type(Type::Object)
}

pub fn binary_schema() -> RefOr<Schema> {
    crate::doc::schema::string_with_format(utoipa::openapi::schema::SchemaFormat::KnownFormat(
        utoipa::openapi::schema::KnownFormat::Binary,
    ))
}

pub fn file_schema() -> RefOr<Schema> {
    binary_schema()
}

pub fn files_schema() -> RefOr<Schema> {
    crate::doc::schema::array(binary_schema()).into()
}

pub fn optional_file_schema() -> RefOr<Schema> {
    crate::doc::schema::nullable(binary_schema())
}

pub fn optional_files_schema() -> RefOr<Schema> {
    crate::doc::schema::nullable(files_schema())
}
