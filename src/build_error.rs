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
        }
    }
}

impl std::error::Error for BuildError {}
