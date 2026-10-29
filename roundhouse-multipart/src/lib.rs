//! Typed multipart extraction.
//!
//! `Multipart<F>` reads a `multipart/form-data` body into a derived struct. A derive emits a field
//! table (`name -> kind`, limits, accepted media types), so the extractor routes each part as it
//! arrives instead of collecting every part first. The rejection body is the canonical
//! [`Problem`], so one client parser handles multipart and every other roundhouse extractor.
//!
//! # Features
//!
//! `FilePart` and the extractor are always available. The `multipart-tempfile` feature adds
//! `TempFile`: a part is streamed into an OS temporary file while the request is read, with the
//! file already unlinked so it disappears when the handle closes. Per-field `max_bytes` and the
//! request `max_body` bound how many bytes one request can spool, but they do not bound aggregate
//! concurrent disk use; if the spool filesystem fills, the extractor returns a `500 Problem`
//! (`could not write a temporary file`). Prefer `FilePart` with `max_bytes` for ordinary uploads,
//! and reserve `TempFile` for files too large to buffer in memory.

#![allow(clippy::result_large_err)]

use std::collections::HashMap;
#[cfg(feature = "multipart-tempfile")]
use std::io::{Read, Seek, SeekFrom, Write};

use axum::http::{StatusCode, header};
use bytes::{Bytes, BytesMut};
pub use roundhouse_problem::Problem;
use roundhouse_schema::{ApiSchema, ValidationError, ValidationErrors, pointer};
use serde::de::DeserializeOwned;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Components, ObjectBuilder, Schema, Type};

/// The global body limit used when the app does not set one.
pub const DEFAULT_MAX_BODY: usize = 32 * 1024 * 1024;
/// The part-count bound used when the app does not set one.
pub const DEFAULT_MAX_PARTS: usize = 1024;

/// Limits the app installs on the request, consumed by the multipart extractor.
#[derive(Debug, Clone, Copy, Default)]
pub struct MultipartLimits {
    pub max_body: Option<usize>,
    pub max_parts: Option<usize>,
}

/// What kind of value a field carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldKind {
    Text,
    TextList,
    File,
    FileList,
    #[cfg(feature = "multipart-tempfile")]
    TempFile,
    #[cfg(feature = "multipart-tempfile")]
    TempFileList,
}

impl FieldKind {
    pub fn is_single(self) -> bool {
        match self {
            FieldKind::Text | FieldKind::File => true,
            #[cfg(feature = "multipart-tempfile")]
            FieldKind::TempFile => true,
            FieldKind::TextList | FieldKind::FileList => false,
            #[cfg(feature = "multipart-tempfile")]
            FieldKind::TempFileList => false,
        }
    }

    pub fn is_file(self) -> bool {
        match self {
            FieldKind::File | FieldKind::FileList => true,
            #[cfg(feature = "multipart-tempfile")]
            FieldKind::TempFile | FieldKind::TempFileList => true,
            FieldKind::Text | FieldKind::TextList => false,
        }
    }
}

/// One row in a derived field table.
#[derive(Debug, Clone, Copy)]
pub struct FieldSpec {
    pub name: &'static str,
    pub kind: FieldKind,
    pub required: bool,
    pub max_bytes: Option<usize>,
    pub accept: &'static [&'static str],
}

impl FieldSpec {
    pub const fn new(name: &'static str, kind: FieldKind) -> Self {
        Self {
            name,
            kind,
            required: false,
            max_bytes: None,
            accept: &[],
        }
    }

    pub const fn required(mut self, required: bool) -> Self {
        self.required = required;
        self
    }

    pub const fn max_bytes(mut self, max_bytes: Option<usize>) -> Self {
        self.max_bytes = max_bytes;
        self
    }

    pub const fn accept(mut self, accept: &'static [&'static str]) -> Self {
        self.accept = accept;
        self
    }
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

/// A file streamed to an already-unlinked temporary file while the request is read.
#[cfg(feature = "multipart-tempfile")]
pub struct TempFile {
    file: std::fs::File,
    len: u64,
}

#[cfg(feature = "multipart-tempfile")]
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

#[cfg(feature = "multipart-tempfile")]
impl Read for TempFile {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.file.read(buf)
    }
}

#[cfg(feature = "multipart-tempfile")]
impl Seek for TempFile {
    fn seek(&mut self, position: SeekFrom) -> std::io::Result<u64> {
        self.file.seek(position)
    }
}

#[cfg(feature = "multipart-tempfile")]
impl std::fmt::Debug for TempFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TempFile").field("len", &self.len).finish()
    }
}

/// Values collected while the request is still on the wire.
#[derive(Default)]
pub struct FieldValues {
    values: HashMap<&'static str, Vec<RawField>>,
}

enum RawField {
    Text(Bytes),
    File(FilePart),
    #[cfg(feature = "multipart-tempfile")]
    Temp(TempFile),
}

impl FieldValues {
    fn push_text(&mut self, name: &'static str, bytes: Bytes) {
        self.values
            .entry(name)
            .or_default()
            .push(RawField::Text(bytes));
    }

    fn push_file(&mut self, name: &'static str, file: FilePart) {
        self.values
            .entry(name)
            .or_default()
            .push(RawField::File(file));
    }

    #[cfg(feature = "multipart-tempfile")]
    fn push_temp(&mut self, name: &'static str, file: TempFile) {
        self.values
            .entry(name)
            .or_default()
            .push(RawField::Temp(file));
    }

    fn has(&self, name: &str) -> bool {
        self.values
            .get(name)
            .is_some_and(|values| !values.is_empty())
    }

    pub fn take_texts(&mut self, name: &str) -> Vec<Bytes> {
        self.take(name)
            .into_iter()
            .filter_map(|field| match field {
                RawField::Text(bytes) => Some(bytes),
                _ => None,
            })
            .collect()
    }

    pub fn take_files(&mut self, name: &str) -> Vec<FilePart> {
        self.take(name)
            .into_iter()
            .filter_map(|field| match field {
                RawField::File(file) => Some(file),
                _ => None,
            })
            .collect()
    }

    #[cfg(feature = "multipart-tempfile")]
    pub fn take_temp_files(&mut self, name: &str) -> Vec<TempFile> {
        self.take(name)
            .into_iter()
            .filter_map(|field| match field {
                RawField::Temp(file) => Some(file),
                _ => None,
            })
            .collect()
    }

    fn take(&mut self, name: &str) -> Vec<RawField> {
        self.values.remove(name).unwrap_or_default()
    }
}

/// Implemented by `#[derive(Multipart)]`.
///
/// The field table is the frozen interface: the derive emits it and the extractor only uses it,
/// so no runtime reflection and no `Vec<Part>` pass is required.
#[allow(clippy::result_large_err)]
pub trait MultipartForm: Sized {
    fn fields() -> &'static [FieldSpec];

    fn deny_unknown() -> bool {
        false
    }

    fn from_fields(fields: &mut FieldValues) -> Result<Self, Problem>;

    fn schema(components: &mut Components) -> RefOr<Schema>;
}

/// Multipart extractor. Parts are routed by the field table and limits are checked as bytes
/// arrive; `TempFile` parts never buffer the whole file in memory.
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
        // The extractor owns the content-type decision: anything that is not a multipart body
        // with a boundary is 415, matching the documented failure set.
        let content_type = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !is_multipart_with_boundary(content_type) {
            return Err(Problem::from_rejection(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "Unsupported Media Type",
                "expected a `multipart/form-data` body with a boundary".to_string(),
            ));
        }

        let limits = request
            .extensions()
            .get::<MultipartLimits>()
            .copied()
            .unwrap_or_default();
        let max_body = limits.max_body.unwrap_or(DEFAULT_MAX_BODY);
        let max_parts = limits.max_parts.unwrap_or(DEFAULT_MAX_PARTS);

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

        let specs = F::fields();
        let mut values = FieldValues::default();
        let mut errors = ValidationErrors::new();
        let mut total = 0usize;
        let mut parts = 0usize;

        while let Some(mut field) = multipart.next_field().await.map_err(multipart_error)? {
            parts += 1;
            if parts > max_parts {
                return Err(too_large(
                    "",
                    "max_parts",
                    format!("request contains more than {max_parts} parts"),
                    max_parts,
                ));
            }

            let name = field.name().unwrap_or_default().to_string();
            let Some(spec) = specs.iter().find(|spec| spec.name == name) else {
                if F::deny_unknown() {
                    errors.push(ValidationError::new(
                        pointer("", &name),
                        "unknown",
                        "unknown multipart field",
                    ));
                }
                drain(&mut field, &mut total, max_body).await?;
                continue;
            };

            if spec.kind.is_single() && values.has(spec.name) {
                return Err(validation(
                    StatusCode::BAD_REQUEST,
                    ValidationError::new(
                        pointer("", spec.name),
                        "duplicate_field",
                        "field may appear only once",
                    ),
                ));
            }

            let file_name = field
                .file_name()
                .map(|name| sanitize_file_name(name.to_string()));
            let content_type = field.content_type().map(str::to_string);

            if spec.kind.is_file()
                && !spec.accept.is_empty()
                && !content_type
                    .as_deref()
                    .is_some_and(|content_type| media_type_matches(spec.accept, content_type))
            {
                return Err(validation(
                    StatusCode::UNSUPPORTED_MEDIA_TYPE,
                    ValidationError::new(
                        pointer("", spec.name),
                        "accept",
                        format!("expected one of: {}", spec.accept.join(", ")),
                    ),
                ));
            }

            let field_limit = spec.max_bytes.unwrap_or(max_body).min(max_body);

            match spec.kind {
                FieldKind::Text | FieldKind::TextList => {
                    let bytes =
                        read_bounded(&mut field, spec.name, field_limit, &mut total, max_body)
                            .await?;
                    values.push_text(spec.name, bytes);
                }
                FieldKind::File | FieldKind::FileList => {
                    let bytes =
                        read_bounded(&mut field, spec.name, field_limit, &mut total, max_body)
                            .await?;
                    values.push_file(
                        spec.name,
                        FilePart {
                            bytes,
                            file_name,
                            content_type,
                        },
                    );
                }
                #[cfg(feature = "multipart-tempfile")]
                FieldKind::TempFile | FieldKind::TempFileList => {
                    let file =
                        stream_to_temp(&mut field, spec.name, field_limit, &mut total, max_body)
                            .await?;
                    values.push_temp(spec.name, file);
                }
            }
        }

        for spec in specs {
            if spec.required && !values.has(spec.name) {
                missing(spec.name, &mut errors);
            }
        }
        if !errors.is_empty() {
            return Err(Problem::from_validation(errors));
        }

        F::from_fields(&mut values).map(Multipart)
    }
}

fn is_multipart_with_boundary(content_type: &str) -> bool {
    let mut pieces = content_type.split(';');
    let media_type = pieces.next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case("multipart/form-data") {
        return false;
    }
    pieces.any(|piece| {
        let piece = piece.trim();
        piece.len() > "boundary=".len()
            && piece[.."boundary=".len()].eq_ignore_ascii_case("boundary=")
    })
}

fn multipart_error(error: axum::extract::multipart::MultipartError) -> Problem {
    Problem::from_rejection(
        error.status(),
        error.status().canonical_reason().unwrap_or("Bad Request"),
        error.body_text(),
    )
}

async fn read_bounded(
    field: &mut axum::extract::multipart::Field<'_>,
    name: &str,
    limit: usize,
    total: &mut usize,
    max_body: usize,
) -> Result<Bytes, Problem> {
    let mut bytes = BytesMut::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        *total += chunk.len();
        if *total > max_body {
            return Err(too_large(
                name,
                "max_bytes",
                format!("request exceeds the {max_body} byte limit"),
                max_body,
            ));
        }
        if bytes.len() + chunk.len() > limit {
            return Err(too_large(
                name,
                "max_bytes",
                format!("part exceeds the {limit} byte limit"),
                limit,
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes.freeze())
}

#[cfg(feature = "multipart-tempfile")]
async fn stream_to_temp(
    field: &mut axum::extract::multipart::Field<'_>,
    name: &str,
    limit: usize,
    total: &mut usize,
    max_body: usize,
) -> Result<TempFile, Problem> {
    let mut file = tempfile::tempfile().map_err(|error| {
        Problem::from_rejection(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            format!("could not create a temporary file: {error}"),
        )
    })?;
    let mut len = 0u64;
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        *total += chunk.len();
        if *total > max_body {
            return Err(too_large(
                name,
                "max_bytes",
                format!("request exceeds the {max_body} byte limit"),
                max_body,
            ));
        }
        if len as usize + chunk.len() > limit {
            return Err(too_large(
                name,
                "max_bytes",
                format!("part exceeds the {limit} byte limit"),
                limit,
            ));
        }
        file.write_all(&chunk).map_err(|error| {
            Problem::from_rejection(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
                format!("could not write a temporary file: {error}"),
            )
        })?;
        len += chunk.len() as u64;
    }
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        Problem::from_rejection(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
            format!("could not rewind a temporary file: {error}"),
        )
    })?;
    Ok(TempFile { file, len })
}

async fn drain(
    field: &mut axum::extract::multipart::Field<'_>,
    total: &mut usize,
    max_body: usize,
) -> Result<(), Problem> {
    while let Some(chunk) = field.chunk().await.map_err(multipart_error)? {
        *total += chunk.len();
        if *total > max_body {
            return Err(too_large(
                "",
                "max_bytes",
                format!("request exceeds the {max_body} byte limit"),
                max_body,
            ));
        }
    }
    Ok(())
}

fn too_large(name: &str, code: &str, message: String, limit: usize) -> Problem {
    validation(
        StatusCode::PAYLOAD_TOO_LARGE,
        ValidationError::new(pointer("", name), code, message).with_limit(limit),
    )
}

fn validation(status: StatusCode, error: ValidationError) -> Problem {
    let mut errors = ValidationErrors::new();
    errors.push(error);
    Problem::from_validation_with_status(errors, status)
}

fn missing(name: &str, errors: &mut ValidationErrors) {
    errors.push(ValidationError::new(
        pointer("", name),
        "required",
        "field is required",
    ));
}

/// Deserialize one text part through serde, so enums and scalars behave exactly as they do in
/// JSON.
pub fn parse_text<T: DeserializeOwned>(
    bytes: Bytes,
    name: &str,
    errors: &mut ValidationErrors,
) -> Option<T> {
    let text = match std::str::from_utf8(&bytes) {
        Ok(text) => text,
        Err(_) => {
            errors.push(ValidationError::new(
                pointer("", name),
                "invalid",
                "part is not UTF-8",
            ));
            return None;
        }
    };
    match serde_json::from_str::<T>(text)
        .or_else(|_| serde_json::from_value::<T>(serde_json::Value::String(text.to_string())))
    {
        Ok(value) => Some(value),
        Err(error) => {
            errors.push(ValidationError::new(
                pointer("", name),
                "invalid",
                error.to_string(),
            ));
            None
        }
    }
}

/// Collect panics on a required field into one 400, preserving the pointer.
pub fn require<T>(value: Option<T>, name: &str, errors: &mut ValidationErrors) -> Option<T> {
    if value.is_none() {
        missing(name, errors);
    }
    value
}

/// The final validation pass for a derived struct.
pub fn finish<T>(value: Result<T, Problem>, errors: ValidationErrors) -> Result<T, Problem> {
    if errors.is_empty() {
        value
    } else {
        Err(Problem::from_validation(errors))
    }
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

/// `accept` matches the media type only: case-insensitive, parameters stripped.
fn media_type_matches(accept: &[&str], content_type: &str) -> bool {
    let media_type = content_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    accept
        .iter()
        .any(|candidate| candidate.trim().to_ascii_lowercase() == media_type)
}

pub fn schema_object() -> ObjectBuilder {
    ObjectBuilder::new().schema_type(Type::Object)
}

pub fn binary_schema() -> RefOr<Schema> {
    roundhouse_schema::schema::string_with_format(
        utoipa::openapi::schema::SchemaFormat::KnownFormat(
            utoipa::openapi::schema::KnownFormat::Binary,
        ),
    )
}

pub fn file_schema() -> RefOr<Schema> {
    binary_schema()
}

pub fn files_schema() -> RefOr<Schema> {
    roundhouse_schema::schema::array(binary_schema()).into()
}

pub fn optional_file_schema() -> RefOr<Schema> {
    roundhouse_schema::schema::nullable(binary_schema())
}

pub fn optional_files_schema() -> RefOr<Schema> {
    roundhouse_schema::schema::nullable(files_schema())
}

/// Convenience for generated code: the schema of a text field.
pub fn text_schema<T: ApiSchema>(components: &mut Components) -> RefOr<Schema> {
    T::register(components);
    T::schema()
}

/// Convenience for generated code: a nullable text schema.
pub fn optional_text_schema<T: ApiSchema>(components: &mut Components) -> RefOr<Schema> {
    T::register(components);
    roundhouse_schema::schema::nullable(T::schema())
}

/// Convenience for generated code: a repeated text schema.
pub fn text_list_schema<T: ApiSchema>(components: &mut Components) -> RefOr<Schema> {
    T::register(components);
    roundhouse_schema::schema::array(T::schema()).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "multipart-tempfile")]
    #[test]
    fn temp_file_reads_from_the_current_position_and_seeks() {
        let mut file = tempfile::tempfile().unwrap();
        file.write_all(b"hello").unwrap();
        file.seek(SeekFrom::Start(0)).unwrap();
        let mut temp = TempFile { file, len: 5 };

        let mut buf = [0u8; 3];
        assert_eq!(temp.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"hel");
        assert_eq!(temp.read(&mut buf).unwrap(), 2);
        assert_eq!(&buf[..2], b"lo");
        assert_eq!(temp.read(&mut buf).unwrap(), 0);

        temp.seek(SeekFrom::Start(1)).unwrap();
        let mut rest = String::new();
        temp.read_to_string(&mut rest).unwrap();
        assert_eq!(rest, "ello");
    }

    #[test]
    fn accept_matches_the_media_type_only() {
        assert!(media_type_matches(
            &["text/plain"],
            "text/plain; charset=utf-8"
        ));
        assert!(media_type_matches(&["Text/Plain"], "text/plain"));
        assert!(!media_type_matches(&["text/plain"], "application/json"));
    }

    #[test]
    fn file_names_are_reduced_to_a_basename() {
        assert_eq!(sanitize_file_name("../../etc/passwd".to_string()), "passwd");
        assert_eq!(sanitize_file_name("..".to_string()), "file");
        assert_eq!(sanitize_file_name("a\0b.txt".to_string()), "ab.txt");
    }
}
