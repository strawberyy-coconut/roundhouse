use roundhouse::prelude::*;
use serde::{Deserialize, Serialize};

/// A note as it is stored and returned.
#[roundhouse::schema]
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub id: i64,
    #[roundhouse(min_length = 1, max_length = 200)]
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
}

/// The writable half of a note.
#[roundhouse::schema]
#[derive(Debug, Deserialize, Serialize)]
pub struct NewNote {
    #[roundhouse(min_length = 1, max_length = 200)]
    pub title: String,
    pub body: Option<String>,
}

/// `/notes/{id}` as a type, so the path parameter is documented.
#[derive(Debug, Deserialize, PathParams)]
pub struct NoteId {
    pub id: i64,
}
