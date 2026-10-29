use roundhouse::prelude::*;

pub mod create;
pub mod list;
pub mod show;

// The only mount in this subtree. Every descendant module inherits this prefix, its tags, and
// any layers/security declared here; the leaf modules below do not need their own `mount!`.
mount!("/notes", tags = ["notes"]);
