//! Utilities shared by the rexlang code-generation backends.
//!
//! The backends ([`rex-backend-rust`], [`rex-backend-jsonschema`],
//! [`rex-backend-cedar`], [`rex-backend-cmmn`]) are independent peers
//! that must agree on two cross-cutting behaviors: the canonical JSON
//! key order their emitted bytes are pinned against, and the
//! `snake_case` instance-identifier convention the Rust serializer and
//! the JSON Schema `id` patterns define together. Both live here so
//! there is exactly one implementation to keep in sync, plus the shared
//! generated-file writer used by every backend's `generate_to_dir`.
//!
//! [`rex-backend-rust`]: https://docs.rs/rex-backend-rust
//! [`rex-backend-jsonschema`]: https://docs.rs/rex-backend-jsonschema
//! [`rex-backend-cedar`]: https://docs.rs/rex-backend-cedar
//! [`rex-backend-cmmn`]: https://docs.rs/rex-backend-cmmn

use std::collections::BTreeMap;
use std::path::Path;

/// Recursively re-inserts object keys in alphabetical order, so the emitted
/// bytes are pinned no matter which key order `serde_json::Map` uses (it is
/// a `BTreeMap` unless some crate in the build graph enables the
/// `preserve_order` feature, which flips it to insertion order).
pub fn canonical_key_order(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut entries: Vec<(String, serde_json::Value)> = map.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            entries
                .into_iter()
                .map(|(key, value)| (key, canonical_key_order(value)))
                .collect::<serde_json::Map<String, serde_json::Value>>()
                .into()
        }
        serde_json::Value::Array(items) => items
            .into_iter()
            .map(canonical_key_order)
            .collect::<Vec<_>>()
            .into(),
        other => other,
    }
}

/// Converts a `.mox` identifier (usually camelCase) to snake_case.
///
/// `getBook` -> `get_book`, `URLCount` -> `url_count`, `books` -> `books`.
///
/// The Rust instance serializer names instances `<snake_case singular>/<n>`
/// and the JSON Schema wire `id` patterns validate exactly that spelling,
/// so the two backends must share this one conversion.
pub fn snake_case(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    let mut out = String::with_capacity(name.len() + 4);
    for (index, ch) in chars.iter().copied().enumerate() {
        if ch.is_ascii_uppercase() {
            let prev_lower_or_digit = index
                .checked_sub(1)
                .map(|p| chars[p].is_ascii_lowercase() || chars[p].is_ascii_digit())
                .unwrap_or(false);
            let next_lower = chars
                .get(index + 1)
                .map(|n| n.is_ascii_lowercase())
                .unwrap_or(false);
            let starts_run_after_word = prev_lower_or_digit;
            let starts_short_run = index > 0 && next_lower;
            if starts_run_after_word || starts_short_run {
                out.push('_');
            }
            out.extend(ch.to_lowercase());
        } else {
            out.push(ch);
        }
    }
    out
}

/// Writes a backend's generated files (relative path -> contents) into
/// `out_dir`, creating parent directories as needed. Iteration follows
/// the map's key order, so the write order is deterministic.
pub fn write_files(files: &BTreeMap<String, String>, out_dir: &Path) -> anyhow::Result<()> {
    for (relative, contents) in files {
        let path = out_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_key_order_sorts_nested_objects() {
        let value = serde_json::json!({
            "zeta": 1,
            "alpha": { "y": [2, {"m": 1, "a": 0}], "b": true },
            "mid": "keep"
        });
        let ordered = canonical_key_order(value);
        assert_eq!(
            serde_json::to_string(&ordered).unwrap(),
            r#"{"alpha":{"b":true,"y":[2,{"a":0,"m":1}]},"mid":"keep","zeta":1}"#
        );
    }

    #[test]
    fn snake_case_conversion() {
        assert_eq!(snake_case("getBook"), "get_book");
        assert_eq!(snake_case("URLCount"), "url_count");
        assert_eq!(snake_case("books"), "books");
        assert_eq!(snake_case("Library"), "library");
        assert_eq!(snake_case("HTTPServer"), "http_server");
    }

    #[test]
    fn write_files_creates_parents_and_writes_contents() {
        static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let unique = COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "rex-backend-common-write-files-{}-{unique}",
            std::process::id(),
        ));
        std::fs::remove_dir_all(&root).ok();
        let files = BTreeMap::from([
            ("flat.txt".to_string(), "flat".to_string()),
            ("nested/deep/file.txt".to_string(), "deep".to_string()),
        ]);
        write_files(&files, &root).expect("write files");
        assert_eq!(
            std::fs::read_to_string(root.join("flat.txt")).unwrap(),
            "flat"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("nested/deep/file.txt")).unwrap(),
            "deep"
        );
        std::fs::remove_dir_all(&root).ok();
    }
}
