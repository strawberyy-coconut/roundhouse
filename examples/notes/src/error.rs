//! Failures for the notes example.
//!
//! `MissingNote` and `CreateNoteError` are ordinary errors: they implement `IntoResponse` to
//! serve. The `DocumentedError` impls are documentation-only, so `#[roundhouse::build_only]`
//! keeps them out of a production `contract` build.

use roundhouse::doc::{DocumentedError, ResponseDoc, SchemaContext};
use roundhouse::prelude::*;

/// `GET /notes/{id}` could not find the note.
#[derive(Debug)]
pub struct MissingNote;

impl IntoResponse for MissingNote {
    fn into_response(self) -> Response {
        Problem::new(StatusCode::NOT_FOUND, "Note not found").into_response()
    }
}

#[roundhouse::build_only]
impl DocumentedError for MissingNote {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![ResponseDoc::problem(404, "Note not found")]
    }
}

/// The title was blank after trimming.
#[derive(Debug)]
pub struct InvalidTitle;

/// A note with the same title already exists.
#[derive(Debug)]
pub struct DuplicateNote;

/// The two failures `POST /notes` can produce.
#[derive(Debug)]
pub enum CreateNoteError {
    InvalidTitle(InvalidTitle),
    Duplicate(DuplicateNote),
}

impl From<InvalidTitle> for CreateNoteError {
    fn from(error: InvalidTitle) -> Self {
        Self::InvalidTitle(error)
    }
}

impl From<DuplicateNote> for CreateNoteError {
    fn from(error: DuplicateNote) -> Self {
        Self::Duplicate(error)
    }
}

impl IntoResponse for CreateNoteError {
    fn into_response(self) -> Response {
        match self {
            Self::InvalidTitle(_) => {
                Problem::new(StatusCode::UNPROCESSABLE_ENTITY, "Invalid title").into_response()
            }
            Self::Duplicate(_) => {
                Problem::new(StatusCode::CONFLICT, "Note already exists").into_response()
            }
        }
    }
}

#[roundhouse::build_only]
impl DocumentedError for CreateNoteError {
    fn responses(_context: &mut dyn SchemaContext) -> Vec<ResponseDoc> {
        vec![
            ResponseDoc::problem(422, "Invalid title"),
            ResponseDoc::problem(409, "Note already exists"),
        ]
    }
}
