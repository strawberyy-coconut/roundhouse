use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

use crate::domain::Note;
use crate::error::{DuplicateNote, MissingNote};

#[derive(Clone, Default)]
pub struct AppState {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    notes: RwLock<HashMap<i64, Note>>,
    next_id: AtomicI64,
}

impl AppState {
    pub fn list(&self) -> Vec<Note> {
        let mut notes: Vec<_> = self.inner.notes.read().unwrap().values().cloned().collect();
        notes.sort_by_key(|note| note.id);
        notes
    }

    pub fn find(&self, id: i64) -> Result<Note, MissingNote> {
        self.inner
            .notes
            .read()
            .unwrap()
            .get(&id)
            .cloned()
            .ok_or(MissingNote)
    }

    pub fn create(&self, title: String, body: Option<String>) -> Result<Note, DuplicateNote> {
        let mut notes = self.inner.notes.write().unwrap();
        if notes.values().any(|note| note.title == title) {
            return Err(DuplicateNote);
        }
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let note = Note { id, title, body };
        notes.insert(id, note.clone());
        Ok(note)
    }
}
