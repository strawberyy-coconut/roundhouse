//! Fetch and normalize the IANA HTTP status-code registry.
//!
//! This crate is a development tool. It is not published and no published crate depends on it.
//! The files it writes are consumed by `roundhouse-build`'s analyzer and `roundhouse::status`.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

/// The authoritative IANA CSV registry.
pub const SOURCE_URL: &str =
    "https://www.iana.org/assignments/http-status-codes/http-status-codes-1.csv";

/// The user agent sent to IANA. A descriptive agent is polite and avoids generic-client blocks.
pub const USER_AGENT: &str =
    "roundhouse-status-codes/0.0.0 (+https://github.com/strawberyy-coconut/roundhouse)";

/// The layout version of the vendored JSON.
pub const SCHEMA_VERSION: u32 = 1;

/// One normalized status code.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSpec {
    pub code: u16,
    pub ident: String,
    pub phrase: String,
    pub kind: StatusKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
}

/// How the generated wrapper exposes its body.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatusKind {
    /// A JSON body wrapper: `pub struct Name<T>(pub T)`.
    Body,
    /// A status-only wrapper: `pub struct Name`.
    Empty,
}

/// The committed snapshot.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusSnapshot {
    pub schema_version: u32,
    pub source: String,
    pub generated_by: String,
    pub do_not_edit: String,
    pub codes: Vec<StatusSpec>,
}

/// A raw IANA CSV row.
#[derive(Debug, Deserialize)]
struct Row {
    #[serde(rename = "Value")]
    value: String,
    #[serde(rename = "Description")]
    description: String,
    #[serde(rename = "Reference", default)]
    reference: String,
}

/// Normalize an IANA CSV document into the statuses roundhouse publishes.
///
/// Ranges, unassigned rows, unused/obsoleted/temporary registrations and informational or
/// redirect responses are skipped. `304 Not Modified` is the one redirect with a stable empty
/// representation.
pub fn normalize(csv: &str) -> Result<Vec<StatusSpec>> {
    let mut reader = csv::Reader::from_reader(csv.as_bytes());
    let mut codes = Vec::new();
    for row in reader.deserialize::<Row>() {
        let row = row.context("reading an IANA status-code row")?;
        if let Some(spec) = normalize_row(&row)? {
            codes.push(spec);
        }
    }
    codes.sort_by_key(|spec| spec.code);
    ensure_unique(&codes)?;
    Ok(codes)
}

fn normalize_row(row: &Row) -> Result<Option<StatusSpec>> {
    let Ok(code) = row.value.trim().parse::<u16>() else {
        // A range such as `105-199` or `432-450`.
        return Ok(None);
    };
    if code < 200 {
        // Informational responses are not normal final responses.
        return Ok(None);
    }
    let description = row.description.trim();
    if description == "Unassigned"
        || description.contains("Unused")
        || description.contains("OBSOLETED")
        || description.contains("TEMPORARY")
    {
        return Ok(None);
    }
    let kind = match code {
        204 | 205 | 304 => StatusKind::Empty,
        300..=399 => return Ok(None),
        200..=299 | 400..=599 => StatusKind::Body,
        _ => return Ok(None),
    };
    let ident = ident_for(code, description)
        .with_context(|| format!("deriving an identifier for status {code}"))?;
    Ok(Some(StatusSpec {
        code,
        ident,
        phrase: description.to_string(),
        kind,
        reference: clean_reference(&row.reference),
    }))
}

/// Identifiers that differ from the naive PascalCase conversion.
///
/// `200` becomes `Success` so the generated name cannot shadow `Result::Ok`. `413` and `422`
/// follow the `http` crate's names, which are the ones Rust users already know.
const IDENT_OVERRIDES: &[(u16, &str)] = &[
    (200, "Success"),
    (413, "PayloadTooLarge"),
    (422, "UnprocessableEntity"),
];

fn ident_for(code: u16, description: &str) -> Option<String> {
    if let Some((_, ident)) = IDENT_OVERRIDES
        .iter()
        .find(|(overridden, _)| *overridden == code)
    {
        return Some((*ident).to_string());
    }
    let mut ident = String::new();
    for word in description.split(|character: char| !character.is_ascii_alphanumeric()) {
        if word.is_empty() {
            continue;
        }
        let mut characters = word.chars();
        let first = characters.next()?.to_ascii_uppercase();
        ident.push(first);
        ident.push_str(&characters.as_str().to_ascii_lowercase());
    }
    (!ident.is_empty()).then_some(ident)
}

fn clean_reference(reference: &str) -> Option<String> {
    let reference = reference.trim();
    let reference = reference
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or(reference);
    (!reference.is_empty()).then(|| reference.to_string())
}

fn ensure_unique(codes: &[StatusSpec]) -> Result<()> {
    for (index, spec) in codes.iter().enumerate() {
        if let Some(previous) = codes[..index].iter().find(|other| other.code == spec.code) {
            anyhow::bail!(
                "duplicate status code {}: {} and {}",
                spec.code,
                previous.ident,
                spec.ident
            );
        }
        if let Some(previous) = codes[..index]
            .iter()
            .find(|other| other.ident == spec.ident)
        {
            anyhow::bail!(
                "duplicate identifier {}: statuses {} and {}",
                spec.ident,
                previous.code,
                spec.code
            );
        }
    }
    Ok(())
}

/// Build the committed snapshot from normalized codes.
pub fn snapshot(codes: Vec<StatusSpec>) -> StatusSnapshot {
    StatusSnapshot {
        schema_version: SCHEMA_VERSION,
        source: SOURCE_URL.to_string(),
        generated_by: "tools/status-codes".to_string(),
        do_not_edit: "Run `cargo run -p roundhouse-status-codes --bin update-status-codes` to \
                      regenerate."
            .to_string(),
        codes,
    }
}

/// Render the snapshot as the committed pretty JSON.
pub fn render(snapshot: &StatusSnapshot) -> Result<String> {
    let mut json = serde_json::to_string_pretty(snapshot)?;
    json.push('\n');
    Ok(json)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"Value,Description,Reference
100,Continue,"[RFC 9110, Section 15.2.1]"
105-199,Unassigned,
200,OK,"[RFC 9110, Section 15.3.1]"
201,Created,"[RFC 9110, Section 15.3.2]"
204,No Content,"[RFC 9110, Section 15.3.5]"
205,Reset Content,"[RFC 9110, Section 15.3.6]"
304,Not Modified,"[RFC 9110, Section 15.4.5]"
306,(Unused),"[RFC 9110, Section 15.4.7]"
413,Content Too Large,"[RFC 9110, Section 15.5.14]"
418,(Unused),"[RFC 9110, Section 15.5.19]"
422,Unprocessable Content,"[RFC 9110, Section 15.5.21]"
510,Not Extended (OBSOLETED),[RFC 2774]
"#;

    #[test]
    fn skips_ranges_and_special_rows() {
        let codes = normalize(SAMPLE).expect("sample normalizes");
        let numbers: Vec<u16> = codes.iter().map(|spec| spec.code).collect();
        assert_eq!(numbers, vec![200, 201, 204, 205, 304, 413, 422]);
    }

    #[test]
    fn applies_overrides_and_body_kinds() {
        let codes = normalize(SAMPLE).expect("sample normalizes");
        assert_eq!(codes[0].ident, "Success");
        assert_eq!(codes[0].kind, StatusKind::Body);
        assert_eq!(codes[2].ident, "NoContent");
        assert_eq!(codes[2].kind, StatusKind::Empty);
        assert_eq!(codes[4].ident, "NotModified");
        assert_eq!(codes[4].kind, StatusKind::Empty);
        assert_eq!(codes[5].ident, "PayloadTooLarge");
        assert_eq!(codes[6].ident, "UnprocessableEntity");
    }

    #[test]
    fn strips_reference_brackets() {
        let codes = normalize(SAMPLE).expect("sample normalizes");
        assert_eq!(
            codes[1].reference.as_deref(),
            Some("RFC 9110, Section 15.3.2")
        );
    }

    #[test]
    fn quoted_descriptions_keep_their_commas() {
        let csv = "Value,Description,Reference\n\
                   418,\"I'm a teapot\",[RFC 2324]\n";
        let codes = normalize(csv).expect("quoted row normalizes");
        assert_eq!(codes[0].phrase, "I'm a teapot");
        assert_eq!(codes[0].ident, "IMATeapot");
    }

    #[test]
    fn duplicate_identifiers_are_rejected() {
        let csv = "Value,Description,Reference\n\
                   201,Created,\n\
                   418,Created,\n";
        let error = normalize(csv).expect_err("duplicate identifiers are rejected");
        assert!(error.to_string().contains("duplicate identifier Created"));
    }
}
