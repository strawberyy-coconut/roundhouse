//! The handler index written by `roundhouse_build::prepare`.
//!
//! The analysis runs before the contract harness compiles the application. `#[handler]` reads the
//! index at expansion time and emits typed trait calls for the concrete types the analyzer found
//! at opaque return sites.

use std::sync::OnceLock;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use serde::Deserialize;
use syn::ItemFn;

#[derive(Deserialize)]
struct HandlerIndex {
    version: u32,
    handlers: Vec<HandlerEntry>,
}

#[derive(Clone, Deserialize)]
pub(super) struct HandlerEntry {
    pub key: String,
    pub responses: Vec<ResponseSite>,
    pub unresolved: Vec<String>,
}

#[derive(Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum ResponseSite {
    Type {
        ty: String,
    },
    StatusType {
        ty: String,
    },
    StatusTuple {
        inner: String,
    },
    Call {
        path: String,
        arity: usize,
        is_async: bool,
    },
    StatusCode,
    Redirect,
}

static INDEX: OnceLock<Option<HandlerIndex>> = OnceLock::new();

pub(super) fn lookup(function: &ItemFn) -> Option<HandlerEntry> {
    let index = INDEX.get_or_init(load).as_ref()?;
    let key = handler_key(function);
    index
        .handlers
        .iter()
        .find(|entry| entry.key == key)
        .cloned()
}

fn load() -> Option<HandlerIndex> {
    let path = std::env::var_os("ROUNDHOUSE_HANDLER_INDEX")?;
    let source = std::fs::read_to_string(path).ok()?;
    let index: HandlerIndex = serde_json::from_str(&source).ok()?;
    (index.version == 1).then_some(index)
}

/// The same stable key the analyzer computes: function name + a deterministic token hash.
fn handler_key(function: &ItemFn) -> String {
    let mut hash = Fnv::new();
    hash.write(function.sig.ident.to_string().as_bytes());
    hash_tokens(function.sig.to_token_stream(), &mut hash);
    hash_tokens(function.block.to_token_stream(), &mut hash);
    format!("{}:{:016x}", function.sig.ident, hash.finish())
}

fn hash_tokens(stream: TokenStream, hash: &mut Fnv) {
    for tree in stream {
        match tree {
            TokenTree::Group(group) => {
                hash.write(b"group");
                hash.write(&[match group.delimiter() {
                    Delimiter::Parenthesis => b'(',
                    Delimiter::Brace => b'{',
                    Delimiter::Bracket => b'[',
                    Delimiter::None => b'_',
                }]);
                hash_tokens(group.stream(), hash);
            }
            TokenTree::Ident(ident) => {
                hash.write(b"ident");
                hash.write(ident.to_string().as_bytes());
            }
            TokenTree::Punct(punct) => {
                hash.write(b"punct");
                hash.write(&[punct.as_char() as u8]);
            }
            TokenTree::Literal(literal) => {
                hash.write(b"literal");
                hash.write(literal.to_string().as_bytes());
            }
        }
    }
}

struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn write(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    fn finish(&self) -> u64 {
        self.0
    }
}
