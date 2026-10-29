use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

use crate::domain::Note;

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

    pub fn get(&self, id: i64) -> Option<Note> {
        self.inner.notes.read().unwrap().get(&id).cloned()
    }

    pub fn create(&self, title: String, body: Option<String>) -> Note {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let note = Note { id, title, body };
        self.inner.notes.write().unwrap().insert(id, note.clone());
        note
    }
}
