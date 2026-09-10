//! rex-vocab — vocabulary snapshot plumbing for [rexlang](https://github.com/anton-makes/rexlang).
//!
//! A *vocabulary* is a fixed, versioned set of enumerated keys vendored from
//! an external authority (e.g. `iso:4217` currencies). This crate owns
//! everything around the `.mox` declaration itself:
//!
//! * [`VocabularyProvider`] implementations that fetch snapshots —
//!   [`FileProvider`] (the vendored `vocab/` directory) and [`HttpProvider`]
//!   (one-off downloads, never used at compile time);
//! * [`parse_snapshot`] / [`validate_entries`], which turn snapshot bytes
//!   into typed [`rex_ir::VocabularyEntry`] lists;
//! * [`Lockfile`] (`model.lock`), which pins each vocabulary's version and
//!   content digest so builds stay hermetic and reproducible.
//!
//! # Snapshot wire format
//!
//! ```json
//! {
//!   "vocabulary": "iso:4217",
//!   "version": "2024-01-01",
//!   "entries": [ { "alpha3": "USD", "minorUnits": 2, "symbol": "$" } ]
//! }
//! ```
//!
//! `entries` is a JSON array of objects; each object's fields are facet
//! values, one of which must hold the key named by the vocabulary's `key`
//! declaration. Field order is irrelevant; entry order is preserved into the
//! IR.
//!
//! # Lockfile wire format (`model.lock`)
//!
//! ```json
//! {
//!   "vocabularies": [
//!     {
//!       "source": "iso:4217",
//!       "version": "2024-01-01",
//!       "digest": "sha256:<hex of the snapshot bytes>",
//!       "fetchedAt": "2026-09-10T12:00:00Z"
//!     }
//!   ]
//! }
//! ```

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub use rex_ir::{DefaultValue, PrimitiveType, VocabularyFacet};

/// Errors produced while parsing or validating vocabulary snapshots.
#[derive(Debug, thiserror::Error)]
pub enum VocabularyError {
    /// The snapshot is not valid JSON, or valid JSON that does not match the
    /// snapshot schema.
    #[error("invalid vocabulary snapshot JSON: {0}")]
    Json(#[from] serde_json::Error),
    /// The snapshot contains no entries at all.
    #[error("vocabulary snapshot has no entries")]
    NoEntries,
    /// The snapshot's `vocabulary` field does not match the declared source.
    #[error(
        "vocabulary snapshot declares source '{found}', but the model declares '{expected}'"
    )]
    SourceMismatch {
        /// Source the model declares.
        expected: String,
        /// Source the snapshot carries.
        found: String,
    },
    /// An entry lacks the field named by the vocabulary's key.
    #[error("snapshot entry #{index} is missing its key field '{field}'")]
    MissingKey {
        /// 0-based snapshot entry index.
        index: usize,
        /// Name of the key field.
        field: String,
    },
    /// The entry's key field is present but not a JSON string.
    #[error("snapshot entry #{index} key field '{field}' must be a string, found {found}")]
    KeyNotAString {
        /// 0-based snapshot entry index.
        index: usize,
        /// Name of the key field.
        field: String,
        /// Kind of the JSON value found.
        found: String,
    },
    /// Two entries carry the same key.
    #[error("duplicate vocabulary entry key '{key}'")]
    DuplicateKey {
        /// The repeated key.
        key: String,
    },
    /// An entry lacks a declared facet.
    #[error("snapshot entry '{key}' is missing facet '{facet}' (declared `{type_}`)")]
    MissingFacet {
        /// The entry's key.
        key: String,
        /// Name of the missing facet.
        facet: String,
        /// Declared primitive type of the facet.
        type_: String,
    },
    /// A facet value has the wrong JSON type for its declared primitive.
    #[error("snapshot entry '{key}' facet '{facet}' expects {expected}, found {found}")]
    FacetTypeMismatch {
        /// The entry's key.
        key: String,
        /// Name of the offending facet.
        facet: String,
        /// Expected JSON value kind.
        expected: &'static str,
        /// Kind of the JSON value found.
        found: String,
    },
}

/// The vocabulary a model declares, minus its vendored entries: the input
/// providers and validation work with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VocabularyDeclaration {
    /// External source identifier, e.g. `"iso:4217"`.
    pub source: String,
    /// Pinned snapshot version, when the model declares one.
    pub version: Option<String>,
    /// Name of the facet acting as the unique entry key (e.g. `"alpha3"`).
    pub key: String,
    /// Declared facets (the key field itself need not be among them).
    pub facets: Vec<VocabularyFacet>,
}

impl From<&rex_ir::VocabularyDef> for VocabularyDeclaration {
    fn from(def: &rex_ir::VocabularyDef) -> Self {
        Self {
            source: def.source.clone(),
            version: def.version.clone(),
            key: def.key.clone(),
            facets: def.facets.clone(),
        }
    }
}

/// A snapshot fetched from a [`VocabularyProvider`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedSnapshot {
    /// The version that was fetched.
    pub version: String,
    /// The raw snapshot bytes (exactly what the digest pins).
    pub bytes: Vec<u8>,
}

/// A source of vocabulary snapshots.
pub trait VocabularyProvider: fmt::Debug {
    /// Fetches the snapshot for `source` at `version`. Providers that cannot
    /// pick a version (all of them, currently) error when `version` is
    /// `None`.
    fn fetch(&self, source: &str, version: Option<&str>) -> anyhow::Result<FetchedSnapshot>;
}

/// Fetches snapshots from a vendored directory of JSON files:
/// `<root>/<sanitized-source>@<version>.json`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileProvider {
    /// The directory holding the `<source>@<version>.json` files.
    pub root: PathBuf,
}

impl VocabularyProvider for FileProvider {
    fn fetch(&self, source: &str, version: Option<&str>) -> anyhow::Result<FetchedSnapshot> {
        let Some(version) = version else {
            return Err(anyhow!(
                "vocabulary '{source}' must be pinned to a version to fetch from disk"
            ));
        };
        let path = self.root.join(format!("{}@{version}.json", sanitize_source(source)));
        let bytes = fs::read(&path).with_context(|| {
            format!(
                "missing vocabulary snapshot {} (expected at {})",
                sanitize_source(source),
                path.display()
            )
        })?;
        Ok(FetchedSnapshot {
            version: version.to_string(),
            bytes,
        })
    }
}

/// Fetches snapshots over HTTP from a URL template with `{source}` and
/// `{version}` placeholders, e.g.
/// `"https://example.com/vocab/{source}@{version}.json"`.
///
/// rexlang builds never fetch over HTTP (they read vendored snapshots);
/// this provider exists for the `rexlang vocab fetch` workflow.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpProvider {
    /// The URL template. `{source}` and `{version}` are substituted.
    pub url_template: String,
}

impl HttpProvider {
    /// Builds the concrete URL for a fetch, sanitizing the source for the
    /// path segment.
    pub fn url_for(&self, source: &str, version: Option<&str>) -> anyhow::Result<String> {
        let Some(version) = version else {
            return Err(anyhow!(
                "vocabulary must be pinned to a version to fetch over http"
            ));
        };
        Ok(self
            .url_template
            .replace("{source}", &sanitize_source(source))
            .replace("{version}", version))
    }
}

impl VocabularyProvider for HttpProvider {
    fn fetch(&self, source: &str, version: Option<&str>) -> anyhow::Result<FetchedSnapshot> {
        let url = self.url_for(source, version)?;
        let mut response = ureq::get(&url)
            .call()
            .map_err(|error| anyhow!("failed to fetch vocabulary from {url}: {error}"))?;
        let bytes = response
            .body_mut()
            .read_to_vec()
            .map_err(|error| anyhow!("failed to read vocabulary from {url}: {error}"))?;
        Ok(FetchedSnapshot {
            // `url_for` guarantees the version is present.
            version: version.expect("version checked by url_for").to_string(),
            bytes,
        })
    }
}

/// Turns every character outside `[A-Za-z0-9._-]` into a filesystem-safe
/// form: `:` becomes `-` (the common case, e.g. `iso:4217` → `iso-4217`) and
/// everything else is dropped.
pub fn sanitize_source(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for c in source.chars() {
        match c {
            ':' => out.push('-'),
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '_' | '-' => out.push(c),
            _ => {}
        }
    }
    out
}

/// The SHA-256 digest of `bytes`, formatted `"sha256:<hex>"` as recorded in
/// [`LockEntry::digest`].
pub fn digest(bytes: &[u8]) -> String {
    let hash = Sha256::digest(bytes);
    let mut out = String::with_capacity(7 + hash.len() * 2);
    out.push_str("sha256:");
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// A parsed snapshot: the wire format described in the [crate
/// docs](crate#snapshot-wire-format).
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedSnapshot {
    /// The `vocabulary` field: the source identifier the snapshot claims.
    pub vocabulary: String,
    /// The `version` field.
    pub version: String,
    /// The entries, in snapshot order. Each entry maps facet names to raw
    /// JSON values.
    pub entries: Vec<BTreeMap<String, serde_json::Value>>,
}

/// Parses snapshot bytes into a [`ParsedSnapshot`]. This checks only the
/// wire shape; use [`validate_entries`] to check it against a declaration.
pub fn parse_snapshot(bytes: &[u8]) -> Result<ParsedSnapshot, VocabularyError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct RawSnapshot {
        vocabulary: String,
        version: String,
        entries: Vec<BTreeMap<String, serde_json::Value>>,
    }
    let raw: RawSnapshot = serde_json::from_slice(bytes)?;
    Ok(ParsedSnapshot {
        vocabulary: raw.vocabulary,
        version: raw.version,
        entries: raw.entries,
    })
}

/// Validates a parsed snapshot against a declaration and lowers it into IR
/// entries.
///
/// Checks, in order: the snapshot is for the declared source; it has at
/// least one entry; every entry carries a unique string key under
/// `decl.key`; every declared facet is present with a value matching its
/// primitive type (integer types → JSON integer, `String`/`char` → JSON
/// string, `boolean` → JSON bool, `float`/`double` → JSON number).
///
/// Facet values become [`DefaultValue`]s. Fractional `float`/`double`
/// values (which [`DefaultValue`] cannot represent losslessly) are stored as
/// their exact JSON text via [`DefaultValue::String`]; consumers reinterpret
/// them using the facet declaration.
pub fn validate_entries(
    parsed: &ParsedSnapshot,
    decl: &VocabularyDeclaration,
) -> Result<Vec<rex_ir::VocabularyEntry>, VocabularyError> {
    if parsed.vocabulary != decl.source {
        return Err(VocabularyError::SourceMismatch {
            expected: decl.source.clone(),
            found: parsed.vocabulary.clone(),
        });
    }
    if parsed.entries.is_empty() {
        return Err(VocabularyError::NoEntries);
    }

    let mut seen = std::collections::HashSet::new();
    let mut entries = Vec::with_capacity(parsed.entries.len());
    for (index, raw) in parsed.entries.iter().enumerate() {
        let key = match raw.get(&decl.key) {
            Some(serde_json::Value::String(key)) => key.clone(),
            Some(other) => {
                return Err(VocabularyError::KeyNotAString {
                    index,
                    field: decl.key.clone(),
                    found: json_kind(other).to_string(),
                });
            }
            None => {
                return Err(VocabularyError::MissingKey {
                    index,
                    field: decl.key.clone(),
                });
            }
        };
        if !seen.insert(key.clone()) {
            return Err(VocabularyError::DuplicateKey { key });
        }

        let mut facets = BTreeMap::new();
        for facet in &decl.facets {
            let Some(value) = raw.get(&facet.name) else {
                return Err(VocabularyError::MissingFacet {
                    key: key.clone(),
                    facet: facet.name.clone(),
                    type_: facet.type_.to_string(),
                });
            };
            let value = facet_value(&key, &facet.name, facet.type_, value)?;
            facets.insert(facet.name.clone(), value);
        }
        entries.push(rex_ir::VocabularyEntry { key, facets });
    }
    Ok(entries)
}

/// Converts one facet value, checking the JSON kind against the declared
/// primitive.
fn facet_value(
    key: &str,
    facet: &str,
    type_: PrimitiveType,
    value: &serde_json::Value,
) -> Result<DefaultValue, VocabularyError> {
    let mismatch = |expected: &'static str| {
        Err(VocabularyError::FacetTypeMismatch {
            key: key.to_string(),
            facet: facet.to_string(),
            expected,
            found: json_kind(value).to_string(),
        })
    };
    match (type_, value) {
        (PrimitiveType::String, serde_json::Value::String(text)) => {
            Ok(DefaultValue::String(text.clone()))
        }
        (PrimitiveType::Char, serde_json::Value::String(text)) => {
            Ok(DefaultValue::String(text.clone()))
        }
        (
            PrimitiveType::Int | PrimitiveType::Long | PrimitiveType::Short | PrimitiveType::Byte,
            serde_json::Value::Number(number),
        ) => match number.as_i64() {
            Some(int) => Ok(DefaultValue::Int(int)),
            None => mismatch("an integer"),
        },
        (PrimitiveType::Float | PrimitiveType::Double, serde_json::Value::Number(number)) => {
            // DefaultValue has no fractional variant; integral values keep
            // their numeric form, fractional ones are stored as their exact
            // JSON text (see `validate_entries`).
            match number.as_i64() {
                Some(int) => Ok(DefaultValue::Int(int)),
                None => Ok(DefaultValue::String(number.to_string())),
            }
        }
        (PrimitiveType::Boolean, serde_json::Value::Bool(bool)) => Ok(DefaultValue::Bool(*bool)),
        (type_, _) => {
            let expected = match type_ {
                PrimitiveType::String | PrimitiveType::Char => "a string",
                PrimitiveType::Int | PrimitiveType::Long | PrimitiveType::Short
                | PrimitiveType::Byte => "an integer",
                PrimitiveType::Float | PrimitiveType::Double => "a number",
                PrimitiveType::Boolean => "a boolean",
            };
            mismatch(expected)
        }
    }
}

/// The JSON value kind, for error messages.
fn json_kind(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "a boolean",
        serde_json::Value::Number(number) if number.is_i64() || number.is_u64() => "an integer",
        serde_json::Value::Number(_) => "a number",
        serde_json::Value::String(_) => "a string",
        serde_json::Value::Array(_) => "an array",
        serde_json::Value::Object(_) => "an object",
    }
}

/// One pinned vocabulary in a [`Lockfile`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LockEntry {
    /// External source identifier, e.g. `"iso:4217"`.
    pub source: String,
    /// The pinned snapshot version.
    pub version: String,
    /// Content digest of the snapshot bytes: `"sha256:<hex>"`.
    pub digest: String,
    /// When the snapshot was fetched, RFC 3339 UTC.
    pub fetched_at: String,
}

/// The `model.lock` file: the set of vocabulary snapshots a model is pinned
/// to. See the [crate docs](crate#lockfile-wire-format-modellock) for the
/// wire format.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lockfile {
    /// One entry per vocabulary, in insertion order.
    #[serde(default)]
    pub vocabularies: Vec<LockEntry>,
}

impl Lockfile {
    /// Reads a lockfile from disk.
    pub fn read(path: &Path) -> anyhow::Result<Self> {
        let bytes = fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
        serde_json::from_slice(&bytes)
            .with_context(|| format!("invalid lockfile {}", path.display()))
    }

    /// Writes this lockfile to disk (pretty-printed, trailing newline).
    pub fn write(&self, path: &Path) -> anyhow::Result<()> {
        let mut json = serde_json::to_string_pretty(self)
            .context("lockfile serialization cannot fail")?;
        json.push('\n');
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("cannot create {}", parent.display()))?;
        }
        fs::write(path, json).with_context(|| format!("cannot write {}", path.display()))
    }

    /// The pin for `source`, if any.
    pub fn entry_for(&self, source: &str) -> Option<&LockEntry> {
        self.vocabularies.iter().find(|entry| entry.source == source)
    }

    /// Inserts or replaces the pin for `entry.source`.
    pub fn insert(&mut self, entry: LockEntry) {
        match self.vocabularies.iter_mut().find(|existing| existing.source == entry.source) {
            Some(existing) => *existing = entry,
            None => self.vocabularies.push(entry),
        }
    }
}

/// The current time as an RFC 3339 UTC timestamp, e.g.
/// `"2026-09-10T12:34:56Z"`.
pub fn rfc3339_now() -> String {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    format_rfc3339(seconds)
}

/// Formats UNIX epoch seconds as an RFC 3339 UTC timestamp
/// (`YYYY-MM-DDThh:mm:ssZ`), using Howard Hinnant's civil-from-days
/// algorithm (no external time dependency).
fn format_rfc3339(unix_seconds: u64) -> String {
    let days = (unix_seconds / 86_400) as i64;
    let seconds_of_day = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds_of_day / 3600,
        (seconds_of_day % 3600) / 60,
        seconds_of_day % 60
    )
}

/// Converts a count of days since 1970-01-01 into a (year, month, day)
/// proleptic-Gregorian date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let day_of_era = (z - era * 146_097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365; // [0, 399]
    let year = year_of_era as i64 + era * 400;
    let day_of_year =
        day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100); // [0, 365]
    let mp = (5 * day_of_year + 2) / 153; // [0, 11]
    let day = (day_of_year - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_rfc3339_renders_utc_timestamps() {
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1), "1970-01-01T00:00:01Z");
        assert_eq!(format_rfc3339(1704067200), "2024-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(1735689600), "2025-01-01T00:00:00Z");
        assert_eq!(format_rfc3339(951782400), "2000-02-29T00:00:00Z");
        assert_eq!(format_rfc3339(1767225599), "2025-12-31T23:59:59Z");
    }

    #[test]
    fn fetched_at_now_is_rfc3339_utc() {
        let stamp = rfc3339_now();
        assert!(stamp.ends_with('Z'), "stamp was: {stamp}");
        assert_eq!(stamp.len(), 20, "stamp was: {stamp}");
    }
}
