//! Build-time analysis of opaque handler return sites.
//!
//! The contract harness compiles the application, so the types exist; what it cannot see is the
//! hidden type of an `impl Trait` return. This pre-pass parses the library source, classifies the
//! tail and `return` expressions of opaque handlers, and writes an index that `#[handler]` reads
//! at expansion time. The macro then emits typed trait calls, and the compiler verifies them.
//!
//! The analyzer does not synthesize responses. It only names the type at each site; the
//! `DocumentedResponse` / `DocumentedError` impls provide the status, content type, schema and
//! headers.

mod semantic;
mod sites;

use std::error::Error;
use std::fs;
use std::path::Path;

use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens;
use serde::Serialize;
use syn::ItemFn;

/// The layout version of the handler index.
pub const INDEX_VERSION: u32 = 1;

/// The file name the index is written to inside `target/roundhouse`.
pub const INDEX_FILE: &str = "handlers.json";

/// One type-carrying site in an opaque handler body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResponseSite {
    /// A type that names a `DocumentedResponse` implementation.
    Type { ty: String },
    /// A type that lives in `roundhouse::status`; the macro qualifies it against the real crate.
    StatusType { ty: String },
    /// `(StatusCode, T)`, where only `T` contributes a shape; the tuple documents `default`.
    StatusTuple { inner: String },
    /// A free or associated function call. The macro infers its return type with a witness.
    Call {
        path: String,
        arity: usize,
        is_async: bool,
    },
    /// A `StatusCode` value; it documents the OpenAPI `default` response.
    StatusCode,
    /// A redirect constructor; it documents the OpenAPI `default` response.
    Redirect,
}

/// The documentation sites for one handler.
#[derive(Clone, Debug, Serialize)]
pub struct HandlerEntry {
    /// Stable key shared with `#[handler]` (function name + token hash).
    pub key: String,
    /// Response producers resolved from the body.
    pub responses: Vec<ResponseSite>,
    /// Sites the analyzer could not resolve. The macro turns these into the existing unverified
    /// marker; strict builds still reject an operation with no success.
    pub unresolved: Vec<String>,
}

/// The generated handler index.
#[derive(Clone, Debug, Serialize)]
pub struct HandlerIndex {
    pub version: u32,
    pub handlers: Vec<HandlerEntry>,
}

/// Analyze `package_root` and return the handler index.
///
/// This is deliberately a separate step from the harness run so the index can be written before
/// the application library is compiled.
pub fn analyze(package_root: &Path) -> Result<HandlerIndex, Box<dyn Error>> {
    let source_root = package_root.join("src");
    if !source_root.is_dir() {
        return Err(format!(
            "{} has no `src` directory; contract generation needs a Rust source tree",
            package_root.display()
        )
        .into());
    }
    let index = semantic::Index::load(&source_root);
    let mut handlers = Vec::new();
    for function in index.handlers() {
        if !sites::has_opaque_return(&function.sig.output) {
            continue;
        }
        let analysis = sites::analyze_handler(function, &index);
        handlers.push(HandlerEntry {
            key: handler_key(function),
            responses: analysis.sites,
            unresolved: analysis.unresolved,
        });
    }
    handlers.sort_by(|left, right| left.key.cmp(&right.key));
    Ok(HandlerIndex {
        version: INDEX_VERSION,
        handlers,
    })
}

/// Write the index to `target/roundhouse/handlers.json` and return its path.
pub fn write_index(
    package_root: &Path,
    index: &HandlerIndex,
) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let directory = package_root.join("target").join("roundhouse");
    fs::create_dir_all(&directory)?;
    let path = directory.join(INDEX_FILE);
    let mut json = serde_json::to_string_pretty(index)?;
    json.push('\n');
    fs::write(&path, json)?;
    Ok(path)
}

/// A stable key for one handler.
///
/// Stable proc-macro spans do not expose reliable line numbers, so the key is the function name
/// plus a deterministic hash of its signature and body. `#[handler]` computes the same key from
/// the tokens it receives.
pub fn handler_key(function: &ItemFn) -> String {
    let mut hash = Fnv::new();
    hash.write(function.sig.ident.to_string().as_bytes());
    hash_tokens(function.sig.to_token_stream(), &mut hash);
    hash_tokens(function.block.to_token_stream(), &mut hash);
    format!("{}:{:016x}", function.sig.ident, hash.finish())
}

/// Hash the token *structure*, not its rendered spacing.
///
/// `TokenStream::to_string()` depends on token spacing, which differs between a stream parsed
/// from source by the analyzer and one handed to a proc macro by rustc. Walking the tree keeps
/// the key stable.
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_is_stable_across_formatting() {
        let first: ItemFn =
            syn::parse_str("async fn show(id: u64) -> impl IntoResponse { Json(id) }").unwrap();
        let second: ItemFn =
            syn::parse_str("async fn show( id : u64 ) -> impl IntoResponse {\n    Json( id )\n}")
                .unwrap();
        assert_eq!(handler_key(&first), handler_key(&second));
    }
}
