//! The vendored IANA HTTP status-code catalog.
//!
//! `tools/status-codes` owns the refresh process and writes `data/status-codes.json`. This module
//! embeds that snapshot so the status proc macro and the build-time analyzer agree without
//! touching the network.

use std::sync::OnceLock;

use serde::Deserialize;

const SNAPSHOT: &str = include_str!("../data/status-codes.json");

/// One generated HTTP status wrapper.
#[derive(Clone, Debug, Deserialize)]
pub struct StatusSpec {
    /// The numeric status code.
    pub code: u16,
    /// The Rust identifier (`Created`, `NotFound`, …).
    pub ident: String,
    /// The registry phrase, used in documentation.
    pub phrase: String,
    /// Whether the wrapper carries a JSON body.
    pub kind: StatusKind,
    /// The registry reference, when one exists.
    pub reference: Option<String>,
}

/// The shape of a generated wrapper.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusKind {
    /// `pub struct Name<T>(pub T)` with a JSON body.
    Body,
    /// `pub struct Name` with no body.
    Empty,
}

#[derive(Deserialize)]
struct Snapshot {
    schema_version: u32,
    codes: Vec<StatusSpec>,
}

fn catalog() -> &'static [StatusSpec] {
    static CATALOG: OnceLock<Vec<StatusSpec>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let snapshot: Snapshot =
            serde_json::from_str(SNAPSHOT).expect("the vendored status-code snapshot is valid");
        assert_eq!(
            snapshot.schema_version, 1,
            "the vendored status-code snapshot has an unsupported schema version"
        );
        snapshot.codes
    })
}

/// Every generated status wrapper, in ascending code order.
pub fn all() -> &'static [StatusSpec] {
    catalog()
}

/// Look up a generated wrapper by its Rust identifier.
pub fn by_ident(ident: &str) -> Option<&'static StatusSpec> {
    catalog().iter().find(|spec| spec.ident == ident)
}

/// Look up a generated wrapper by its numeric code.
pub fn by_code(code: u16) -> Option<&'static StatusSpec> {
    catalog().iter().find(|spec| spec.code == code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_snapshot_contains_the_compatibility_wrappers() {
        assert_eq!(by_ident("Created").map(|spec| spec.code), Some(201));
        assert_eq!(by_ident("NoContent").map(|spec| spec.code), Some(204));
        assert_eq!(by_ident("NotFound").map(|spec| spec.code), Some(404));
        assert_eq!(by_ident("Conflict").map(|spec| spec.code), Some(409));
    }

    #[test]
    fn identifiers_and_codes_are_unique() {
        for (index, spec) in all().iter().enumerate() {
            assert!(
                all()[..index].iter().all(|other| other.code != spec.code),
                "duplicate code {}",
                spec.code
            );
            assert!(
                all()[..index].iter().all(|other| other.ident != spec.ident),
                "duplicate identifier {}",
                spec.ident
            );
        }
    }

    #[test]
    fn only_204_205_and_304_are_empty() {
        for spec in all() {
            match spec.kind {
                StatusKind::Empty => {
                    assert!(matches!(spec.code, 204 | 205 | 304), "{}", spec.code);
                }
                StatusKind::Body => {}
            }
        }
    }
}
