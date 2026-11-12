//! The stripped-name uniqueness check (issue rexlang#177): two classes or
//! enums in one package whose names strip to the same module identifier, or
//! map to the same effective table name, collide silently downstream —
//! consumers generate the same module path / pg table and one declaration
//! overwrites the other. The check is per-package over class and enum
//! declarations, exempts pairs where either class is a containment target
//! (contained value objects share their owner's module downstream by
//! design), and offers the class-level `table "…"` override as the
//! table-name escape hatch. Diagnosed as E0300 (module identifier) and
//! E0301 (table name) — the `driver.domain` band.
//!
//! The derivation deliberately lives in rex-driver: rex-driver must not
//! depend on backend crates, and rex-backend-common's `snake_case` is that
//! backend's shared spelling. This module matches its semantics so a
//! consumer's derived module/table name equals the driver's check key; the
//! test vectors below pin both sides of that agreement.

use super::{ClassRecord, FKind};
use crate::diagnostic::Diagnostic;
use rex_diagnostics::domain::{E_STRIPPED_NAME_COLLISION, E_TABLE_NAME_COLLISION};
use rex_syntax::ast as mox;
use rex_syntax::Span;
use std::collections::{HashMap, HashSet};

/// Converts a `.mox` identifier (usually PascalCase) to snake_case.
///
/// Matches `rex_backend_common::snake_case` exactly (`getBook` → `get_book`,
/// `HTTPServer` → `http_server`, `Library` → `library`) — the backends key
/// module paths and default table names on this spelling, so the driver's
/// uniqueness check must derive the same key the consumers derive.
fn snake_case(name: &str) -> String {
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

/// The module identifier a declaration's name lowers to downstream: one
/// trailing `Type` suffix removed (suffix only — `TypeWriter` keeps its
/// name; `PayRunTypeType` strips once), then snake_case (`PayRun` and
/// `PayRunType` both strip to `pay_run`).
///
/// A name that is *only* `Type` keeps it: stripping would produce an empty
/// identifier, and `type` is a better module name than none.
pub(crate) fn stripped_module_name(name: &str) -> String {
    let stem = match name.strip_suffix("Type") {
        Some(stem) if !stem.is_empty() => stem,
        _ => name,
    };
    snake_case(stem)
}

/// One class or enum declaration participating in the uniqueness check.
pub(crate) struct NameRecord {
    /// The owning package (the check's scope is one package).
    pub package: String,
    /// The file the declaration came from; diagnostics are tagged with it
    /// (the multi-file pipelines need the tag, the single-file one strips it).
    pub file: String,
    /// The declaration name as written.
    pub name: String,
    /// Span of the name token (the diagnostic site — the later declaration).
    pub span: Span,
    /// Class or enum.
    pub kind: NameKind,
    /// The class's declared `table "…"` override; `None` on enums (enums
    /// have no table).
    pub table: Option<String>,
    /// `true` when some class declares a `contains` feature whose type
    /// resolves to this class (the exemption: at least one member of a
    /// colliding pair being a containment target excuses the pair).
    pub contained: bool,
}

/// The kind of a checked declaration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameKind {
    Class,
    Enum,
}

impl NameKind {
    /// The lowercase noun the diagnostics name the declaration with.
    fn label(self) -> &'static str {
        match self {
            NameKind::Class => "class",
            NameKind::Enum => "enum",
        }
    }
}

/// Builds the check's records from the lowered classes and the AST enum
/// declarations (classes carry their name span and table override on the
/// [`ClassRecord`]; enums only exist on the AST side, so their records are
/// built from the declarations). `contained` is derived here from every
/// included class's `contains` features — callers pass only the classes and
/// enums whose files lowered error-free, so a broken file can neither hide
/// nor fake containment for the healthy ones.
pub(crate) fn name_records<'a>(
    classes: impl IntoIterator<Item = &'a ClassRecord>,
    enums: impl IntoIterator<Item = (&'a str, &'a str, &'a mox::EnumDecl)>,
) -> Vec<NameRecord> {
    let class_records: Vec<&ClassRecord> = classes.into_iter().collect();
    let contained: HashSet<(&str, &str)> = class_records
        .iter()
        .flat_map(|class| class.features.iter())
        .filter(|feature| matches!(feature.kind, FKind::Containment))
        .filter_map(|feature| feature.type_class.as_ref())
        .map(|(package, name)| (package.as_str(), name.as_str()))
        .collect();
    let mut records: Vec<NameRecord> = class_records
        .iter()
        .map(|class| NameRecord {
            contained: contained.contains(&(class.package.as_str(), class.name.as_str())),
            package: class.package.clone(),
            file: class.file.clone(),
            name: class.name.clone(),
            span: class.name_span,
            kind: NameKind::Class,
            table: class.def.table_name.clone(),
        })
        .collect();
    for (package, file, decl) in enums {
        records.push(NameRecord {
            package: package.to_string(),
            file: file.to_string(),
            name: decl.name.text.clone(),
            span: decl.name.span,
            kind: NameKind::Enum,
            // Enums have no table: they join neither the table key nor the
            // exemption (a `contains` never targets an enum).
            table: None,
            contained: false,
        });
    }
    records
}

/// Which collision key a pass checks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    /// The stripped module identifier (E0300) — over classes and enums.
    Module,
    /// The effective table name (E0301) — over classes only.
    Table,
}

/// The key value one declaration maps to under `key`: the stripped module
/// identifier, or the effective table name (declared `table` override, else
/// the derived default).
fn key_of(record: &NameRecord, key: Key) -> String {
    match key {
        Key::Module => stripped_module_name(&record.name),
        Key::Table => record
            .table
            .clone()
            .unwrap_or_else(|| stripped_module_name(&record.name)),
    }
}

/// Runs the per-package stripped-name uniqueness check over the records (in
/// declaration order) and returns the diagnostics, each tagged with the
/// later declaration's file. Key 1 (E0300) compares every class and enum
/// against its package-mates by stripped module identifier; key 2 (E0301)
/// compares classes by effective table name. A pair is exempt when either
/// member is a containment target; a declaration colliding with an exempt
/// mate still collides with any non-exempt one, so chains diagnose the
/// first non-exempt occupant.
pub(crate) fn check_name_uniqueness(records: &[NameRecord]) -> Vec<(String, Diagnostic)> {
    let mut tagged: Vec<(String, Diagnostic)> = Vec::new();
    // Packages in first-appearance order; the check never crosses packages.
    let mut package_order: Vec<String> = Vec::new();
    let mut scoped: HashMap<&str, Vec<&NameRecord>> = HashMap::new();
    for record in records {
        if !package_order.iter().any(|name| name == &record.package) {
            package_order.push(record.package.clone());
        }
        scoped
            .entry(record.package.as_str())
            .or_default()
            .push(record);
    }
    for package in &package_order {
        let in_package = &scoped[package.as_str()];
        diagnose_collisions(in_package, Key::Module, &mut tagged);
        diagnose_collisions(in_package, Key::Table, &mut tagged);
    }
    tagged
}

/// One collision-key pass over one package's records (declaration order).
/// Each record is compared against its earlier key-mates; the first
/// non-exempt mate is named in the diagnostic, spanned on the later record.
/// The table pass skips a pair whose members also strip to the same module
/// identifier: that pair's E0300 already names the root cause (one rename
/// resolves both), and the module diagnostic must not double-report as a
/// table collision.
fn diagnose_collisions(records: &[&NameRecord], key: Key, tagged: &mut Vec<(String, Diagnostic)>) {
    for (index, record) in records.iter().enumerate() {
        // The table key is a class concern: enums declare no table and join
        // neither side of an E0301 pair.
        if key == Key::Table && record.kind != NameKind::Class {
            continue;
        }
        let key_value = key_of(record, key);
        let collision = records[..index].iter().find(|earlier| {
            if key_of(earlier, key) != key_value {
                return false;
            }
            if key == Key::Table
                && stripped_module_name(&earlier.name) == stripped_module_name(&record.name)
            {
                // Subsumed by the pair's E0300 (the names strip alike).
                return false;
            }
            !(earlier.contained || record.contained)
        });
        let Some(earlier) = collision else {
            continue;
        };
        let (code, noun, help) = match key {
            Key::Module => (
                E_STRIPPED_NAME_COLLISION,
                "stripped module name",
                "rename one of the declarations, or give one class a distinct `table \"…\"`",
            ),
            Key::Table => (
                E_TABLE_NAME_COLLISION,
                "table name",
                "give one of the classes a distinct `table \"…\"`",
            ),
        };
        tagged.push((
            record.file.clone(),
            Diagnostic::error(
                format!(
                    "{code} {noun} '{key_value}' is shared by {} '{}' and {} '{}'",
                    earlier.kind.label(),
                    earlier.name,
                    record.kind.label(),
                    record.name,
                ),
                Some(record.span),
            )
            .with_help(help),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::stripped_module_name;

    #[test]
    fn stripping_removes_one_trailing_type_suffix_then_snakes() {
        assert_eq!(stripped_module_name("PayRun"), "pay_run");
        assert_eq!(stripped_module_name("PayRunType"), "pay_run");
        assert_eq!(stripped_module_name("Book"), "book");
    }

    #[test]
    fn stripping_is_suffix_only_and_strips_at_most_once() {
        // `Type` mid-name never strips.
        assert_eq!(stripped_module_name("TypeWriter"), "type_writer");
        // One trailing suffix only.
        assert_eq!(stripped_module_name("PayRunTypeType"), "pay_run_type");
        assert_eq!(stripped_module_name("HTTPServer"), "http_server");
    }

    #[test]
    fn snake_case_matches_the_backend_shared_semantics() {
        // rex-backend-common's documented vectors.
        assert_eq!(stripped_module_name("getBook"), "get_book");
        assert_eq!(stripped_module_name("HTTPServer"), "http_server");
        assert_eq!(stripped_module_name("Library"), "library");
    }

    #[test]
    fn a_bare_type_name_keeps_it() {
        assert_eq!(stripped_module_name("Type"), "type");
    }
}
