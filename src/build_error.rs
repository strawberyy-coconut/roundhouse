//! Build-time diagnostics.
//!
//! Resolution is fallible. `App::try_build()` returns these as values; `App::build()` panics with
//! their `Display` form so a server can log and exit instead of unwinding inside a handler.

use std::fmt;

use crate::registry::Method;
use roundhouse_schema::ParamIn;

/// Where a declaration lives in the user's source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    pub module_path: &'static str,
    pub file: &'static str,
    pub line: u32,
}

impl Source {
    pub fn new(module_path: &'static str, file: &'static str, line: u32) -> Self {
        Self {
            module_path,
            file,
            line,
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}   {}:{}", self.module_path, self.file, self.line)
    }
}

/// A reason `App::try_build()` refused to build.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BuildError {
    /// Two table entries resolve to the same method and normalised template.
    DuplicateRoute {
        method: Method,
        path: String,
        first: Source,
        second: Source,
    },
    /// Two operations would share one `operationId`.
    DuplicateOperationId {
        operation_id: String,
        first: Source,
        second: Source,
    },
    /// An operation derives no 2xx/3xx response and declares none.
    UndeclaredSuccess {
        operation_id: String,
        source: Source,
    },
    /// A body extractor the document cannot verify (`axum::Json`, hand-mounted bodies).
    UnverifiedBody {
        operation_id: String,
        source: Source,
    },
    /// The same security scheme name is registered with two different kinds.
    SchemeKindConflict {
        name: String,
        first: Source,
        second: Source,
    },
    /// A security requirement with no extractor or documented layer enforcing it.
    UnverifiedSecurity {
        name: String,
        source: Source,
    },
    /// A declared response contradicts the inferred one without `override`.
    ConflictingResponse {
        operation_id: String,
        status: u16,
        source: Source,
    },
    /// Two extractors declare the same parameter with different schemas.
    ConflictingParameter {
        name: String,
        location: ParamIn,
        source: Source,
    },
    /// A path template cannot be served.
    InvalidTemplate { reason: String, source: Source },
    /// The emitted document could not be compiled into a validator.
    SchemaCompilation { message: String },
    /// `App::strict()` promoted a warning into an error.
    Diagnostic { message: String, source: Source },
}

impl fmt::Display for BuildError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuildError::DuplicateRoute {
                method,
                path,
                first,
                second,
            } => write!(
                f,
                "error: duplicate route\n  {method} {path}   {first}\n  {method} {path}   {second}"
            ),
            BuildError::DuplicateOperationId {
                operation_id,
                first,
                second,
            } => write!(
                f,
                "error: duplicate operationId `{operation_id}`\n  {first}\n  {second}\n\
                 hint: set `operation_id = \"...\"` on one table entry"
            ),
            BuildError::UndeclaredSuccess {
                operation_id,
                source,
            } => write!(
                f,
                "error: operation `{operation_id}` declares no success response\n  {source}\n\
                 hint: return a concrete response type or declare `responses(200)` in the table"
            ),
            BuildError::UnverifiedBody {
                operation_id,
                source,
            } => write!(
                f,
                "error: operation `{operation_id}` takes a body extractor the document cannot verify\n  {source}\n\
                 hint: use `roundhouse::Json`, `Form` or `Multipart`, or drop `App::strict()`"
            ),
            BuildError::SchemeKindConflict {
                name,
                first,
                second,
            } => write!(
                f,
                "error: security scheme `{name}` is declared with two different kinds\n  {first}\n  {second}"
            ),
            BuildError::UnverifiedSecurity { name, source } => write!(
                f,
                "error: security requirement `{name}` is documented but nothing enforces it\n  {source}\n\
                 hint: add an `Auth<A>` argument or `layer_documented(auth::<A>())` for that scheme, \
                 or remove it from `security`"
            ),
            BuildError::ConflictingResponse {
                operation_id,
                status,
                source,
            } => write!(
                f,
                "error: operation `{operation_id}` declares a different response for status {status} than \
                 its handler derives\n  {source}\n\
                 hint: make the declared type match, or add `override` to the entry"
            ),
            BuildError::ConflictingParameter {
                name,
                location,
                source,
            } => write!(
                f,
                "error: conflicting parameter `{name}` ({})\n  {source}\n\
                 hint: two extractors document the same parameter with different schemas",
                location.as_str()
            ),
            BuildError::InvalidTemplate { reason, source } => {
                write!(f, "error: invalid route template\n  {reason}\n  {source}")
            }
            BuildError::SchemaCompilation { message } => {
                write!(
                    f,
                    "error: could not compile the emitted schema\n  {message}"
                )
            }
            BuildError::Diagnostic { message, source } => {
                write!(f, "error: {message}\n  {source}")
            }
        }
    }
}

impl std::error::Error for BuildError {}

/// A non-fatal build observation: something the document or router had to work around.
///
/// `App::build()` prints these to stderr; `App::strict()` turns them into [`BuildError`]s.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub message: String,
    pub source: Source,
}

impl Diagnostic {
    pub fn new(source: Source, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source,
        }
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "warning: {}\n  {}", self.message, self.source)
    }
}

/// Collect a diagnostic, or fail the build when `strict` is set.
pub fn report(
    diagnostics: &mut Vec<Diagnostic>,
    strict: bool,
    diagnostic: Diagnostic,
) -> Result<(), BuildError> {
    if strict {
        return Err(BuildError::Diagnostic {
            message: diagnostic.message,
            source: diagnostic.source,
        });
    }
    diagnostics.push(diagnostic);
    Ok(())
}
