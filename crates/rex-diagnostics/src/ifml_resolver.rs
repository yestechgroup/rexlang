//! The `ifml.resolver` band: `E0001`–`E0099`, the cross-file module
//! resolution diagnostics of the `.ifml` compiler (rex-ifml's `resolve`
//! module re-exports these constants; this crate stays dependency-free).
//! Claim a new code by adding a documented constant plus an `Entry` to
//! `ENTRIES` — the meanings and fixes render into `docs/DIAGNOSTICS.md`.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const RESOLVER_BAND: &str = "ifml.resolver";

/// An `.ifml` import has no content in the bundle.
pub const E_IMPORT_NOT_PROVIDED: &str = "E0001";
/// A file (main or imported) failed to parse.
pub const E_PARSE: &str = "E0002";
/// A circular `.ifml` import.
pub const E_CIRCULAR: &str = "E0003";
/// Two distinct imported files declare the same module name.
pub const E_AMBIGUOUS_MODULE: &str = "E0004";
/// One file declares the same module name twice.
pub const E_DUPLICATE_MODULE: &str = "E0005";
/// A `use` targets no declared module.
pub const E_UNKNOWN_MODULE: &str = "E0006";
/// An override matches neither an input parameter nor a module property.
pub const E_UNKNOWN_INPUT: &str = "E0007";
/// An input without a default receives no override.
pub const E_MISSING_INPUT: &str = "E0008";
/// A literal override's type contradicts the input's declared type.
pub const E_INPUT_TYPE: &str = "E0009";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: E_IMPORT_NOT_PROVIDED,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Import not provided",
        meaning: "An `.ifml` import has no content in the `IfmlImports` bundle (`imported ifml '<path>' was not provided`).",
        fix: "Provide the import's source in the `IfmlImports` bundle, keyed `(importing path, import path as written)`.",
    },
    Entry {
        code: E_PARSE,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Parse error",
        meaning: "A file (main or imported) failed to parse; the message carries the parse error.",
        fix: "Fix the reported syntax error; `rexlang fmt` restores canonical layout.",
    },
    Entry {
        code: E_CIRCULAR,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Circular import",
        meaning: "A circular `.ifml` import (`circular ifml import involving '<path>'`).",
        fix: "Break the cycle — move the shared module into its own file and import it from both sides.",
    },
    Entry {
        code: E_AMBIGUOUS_MODULE,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Ambiguous module",
        meaning: "Two distinct imported files declare the same module name (`ambiguous module '<name>' ...`).",
        fix: "Rename one of the conflicting modules; main-file declarations shadow imports silently.",
    },
    Entry {
        code: E_DUPLICATE_MODULE,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Duplicate module",
        meaning: "One file declares the same module name twice (`duplicate module '<name>'`).",
        fix: "Remove or rename the second declaration — the first one wins.",
    },
    Entry {
        code: E_UNKNOWN_MODULE,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Unknown module",
        meaning: "A `use` targets no declared module (`unknown module '<name>'`).",
        fix: "Declare the module locally, or import the file that declares it.",
    },
    Entry {
        code: E_UNKNOWN_INPUT,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Unknown input",
        meaning: "An override matches neither an input parameter nor a module property (`unknown input '<p>' for module '<m>'`).",
        fix: "Spell the input exactly as the module's `input { }` block declares it.",
    },
    Entry {
        code: E_MISSING_INPUT,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Missing input",
        meaning: "An input without a default receives no override (`module '<m>' requires input '<p>'`).",
        fix: "Pass the required input at the `use` site, or give it a default in the module.",
    },
    Entry {
        code: E_INPUT_TYPE,
        band: RESOLVER_BAND,
        severity: Severity::Error,
        title: "Input type mismatch",
        meaning: "A literal override's type contradicts the input's declared type (`input '<p>' of module '<m>' expects <ty>`); structural check only — string literals suit `String`/`Uuid`/custom-identifier types, numbers suit `Int`/`Float`, booleans suit `Boolean`.",
        fix: "Use a literal of the declared type, or a non-literal expression (typed by the checker).",
    },
];
