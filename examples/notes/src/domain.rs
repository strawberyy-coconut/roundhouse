use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

/// A note as it is stored and returned.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, ApiSchema)]
pub struct Note {
    pub id: i64,
    #[schema(min_length = 1, max_length = 200)]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// The writable half of a note.
#[derive(Debug, Deserialize, Serialize, ToSchema, ApiSchema)]
pub struct NewNote {
    #[schema(min_length = 1, max_length = 200)]
    pub title: String,
    pub body: Option<String>,
}

/// `/notes/{id}` as a type, so the path parameter is documented.
#[derive(Debug, Deserialize, PathParams)]
pub struct NoteId {
    pub id: i64,
}
