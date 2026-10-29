use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, RwLock};

use crate::auth::DemoUser;
use crate::domain::{NewPost, Post, PostStatus, UpdatePost};

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    posts: RwLock<HashMap<i64, Post>>,
    next_id: AtomicI64,
    tokens: RwLock<HashMap<String, DemoUser>>,
}

impl Default for AppState {
    fn default() -> Self {
        let state = Self {
            inner: Arc::new(Inner::default()),
        };
        state.inner.tokens.write().unwrap().insert(
            "demo".to_string(),
            DemoUser {
                name: "demo".to_string(),
            },
        );
        state
    }
}

impl AppState {
    pub fn list(&self, tag: Option<&str>, limit: Option<usize>) -> Vec<Post> {
        let mut posts: Vec<_> = self
            .inner
            .posts
            .read()
            .unwrap()
            .values()
            .filter(|post| tag.is_none_or(|tag| post.tags.iter().any(|item| item == tag)))
            .cloned()
            .collect();
        posts.sort_by_key(|post| post.id);
        if let Some(limit) = limit {
            posts.truncate(limit);
        }
        posts
    }

    pub fn get(&self, id: i64) -> Option<Post> {
        self.inner.posts.read().unwrap().get(&id).cloned()
    }

    pub fn get_for_author(&self, author_id: i64, post_id: i64) -> Option<Post> {
        self.get(post_id).filter(|post| post.author_id == author_id)
    }

    pub fn authorize(&self, token: &str) -> Option<DemoUser> {
        self.inner.tokens.read().unwrap().get(token).cloned()
    }

    pub fn create(&self, body: NewPost) -> Post {
        let id = self.inner.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let post = Post {
            id,
            author_id: body.author_id,
            title: body.title,
            body: body.body,
            tags: body.tags,
            status: PostStatus::Draft,
        };
        self.inner.posts.write().unwrap().insert(id, post.clone());
        post
    }

    pub fn update(&self, id: i64, body: UpdatePost) -> Option<Post> {
        let mut posts = self.inner.posts.write().unwrap();
        let post = posts.get_mut(&id)?;
        post.title = body.title;
        post.body = body.body;
        post.tags = body.tags;
        Some(post.clone())
    }

    pub fn delete(&self, id: i64) -> bool {
        self.inner.posts.write().unwrap().remove(&id).is_some()
    }

    pub fn set_status(&self, id: i64, status: PostStatus) -> Option<Post> {
        let mut posts = self.inner.posts.write().unwrap();
        let post = posts.get_mut(&id)?;
        post.status = status;
        Some(post.clone())
    }
}
