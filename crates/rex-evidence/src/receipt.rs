//! Verification receipts: the versioned, canonical evidence document
//! `rexlang verify` emits and `rexlang verify --check` validates
//! (issues #197/#206, epic #202 phase P1).
//!
//! A receipt is a **report**, never a verdict: it names the source bytes a
//! compile read, the artifact bytes a compile wrote, the generator that
//! wrote them, and the proof-obligation register's rows *as the register
//! states them*. It does not upgrade anything — the register's closed
//! status vocabulary (`proved`/`tested`/`argued`/`assumed`) is the only
//! vocabulary a receipt speaks, and the value `"verified"` is rejected by
//! name. [docs/RECEIPTS.md](https://github.com/yestechgroup/rexlang/blob/main/docs/RECEIPTS.md)
//! is the reader's guide; this module is the wire contract.
//!
//! # The wire contract
//!
//! - **camelCase**, canonical key order (alphabetical, recursively), so two
//!   runs of the same inputs are byte-identical. No timestamps, no absolute
//!   paths, no environment: a receipt holds nothing that changes by itself.
//! - **Closed documents.** Every struct denies unknown fields: a receipt is
//!   a closed claim set, so a fact that is not in this version's schema is a
//!   format change ([`FORMAT_VERSION`]), not a silently ignored key.
//! - **Lenient parse, strict validate.** Missing fields deserialize as empty
//!   defaults and every semantic rule lives in [`check`], exactly like
//!   [`crate::register`] — one home per rule, and each rule has a negative
//!   test.
//! - **Relative paths.** `sources` paths are relative to the repository
//!   root; `artifacts` paths are relative to the receipt's own directory
//!   (so a receipt plus its artifacts is a self-contained bundle an auditor
//!   can copy anywhere). Absolute paths and `..` escapes are rejected.
//!
//! # What [`check`] can and cannot prove
//!
//! It re-hashes every artifact that names a path (the closure supplies the
//! bytes, so the crate stays filesystem-free), validates the closed
//! vocabularies and the sort order that makes the document canonical, and
//! checks the status↔trigger/review discipline. It needs **no** register
//! file, no models, no Creusot, and no network — that independence is the
//! point of issue #206. It cannot prove that the embedded register rows
//! match today's register: that is `rexlang verify --check-fresh`'s job in
//! the workspace (and the CI freshness gate's).

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use crate::register::{is_iso_date, split_pointer, EVIDENCE_KINDS, STATUSES};

/// The receipt wire format this crate speaks. A receipt whose
/// `formatVersion` differs is rejected with both versions named (the
/// `rex-ir` version-gate discipline: `formatVersion` is the only
/// compatibility gate).
pub const FORMAT_VERSION: u32 = 1;

/// The document's self-declared kind (the `kind` field).
pub const KIND: &str = "verificationReceipt";

/// The receipt-local artifact-kind vocabulary. Deliberately **not**
/// `rex-cli`'s `artifact_check` vocabulary: that one is shape-sniffing
/// (an empty `rex_ir::ifml::IfmlModel` collides with a DDD design), while a
/// receipt knows which surface it compiled — so the kind is recorded at
/// emission time and `ifmlManifest` names the control/route manifest
/// (`rex_ir::ifml_manifest`) a flow contributes.
pub const ARTIFACT_KINDS: [&str; 9] = [
    "irModel",
    "actorModel",
    "dddDesign",
    "eventContract",
    "useCaseModel",
    "deployModel",
    "decisionModel",
    "cmmnModel",
    "ifmlManifest",
];

/// The closed `limits` vocabulary: what a receipt explicitly does **not**
/// record. A limit is a promise about the future, not an excuse — each slug
/// is documented in `docs/RECEIPTS.md` with the issue that retires it.
///
/// `mutants-unrecorded` is the interim honesty entry for issue #197's
/// `mutantKilled` deliverable: the register carries no mutation data until
/// the anti-vacuity mutant pipeline (#208) lands, so every row's
/// `mutantKilled` is `null` and the receipt says so out loud instead of
/// leaving the reader to assume a silent `true`.
pub const LIMITS: [&str; 1] = ["mutants-unrecorded"];

/// A verification receipt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    /// The wire-format gate; must equal [`FORMAT_VERSION`].
    #[serde(default)]
    pub format_version: u32,
    /// [`KIND`].
    #[serde(default)]
    pub kind: String,
    /// What the receipt is about (`support`): the CLI derives it from the
    /// inputs' common file stem, or names it explicitly.
    #[serde(default)]
    pub subject: String,
    /// The generator that produced the receipt. `version` is the `rexlang`
    /// crate version (`rexVersion`'s provenance role for this artifact):
    /// a backend/generator change invalidates the artifact hashes below, so
    /// the receipt records who wrote them.
    #[serde(default)]
    pub generator: Generator,
    /// The top-level inputs the generation command named, sorted,
    /// repo-relative. `sources` is the transitive closure; this is the
    /// subset `--check-fresh` re-runs, so the regenerate command is
    /// derivable from the document itself.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Every file the compile actually read — inputs, imported sources,
    /// pattern-library modules, decision tables — sorted, repo-relative.
    #[serde(default)]
    pub sources: Vec<SourceFile>,
    /// The artifact bytes the compile emitted, sorted by
    /// `(kind, path, sha256)`. A `null` path marks a digest-only entry (a
    /// domain/actor artifact with no committed golden); every other path is
    /// relative to the receipt, and [`check`] re-hashes it.
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
    /// The register the receipt reports: its source path and hash, its
    /// review date, and its rows verbatim-in-shape. Statuses are copied,
    /// never reinterpreted.
    #[serde(default)]
    pub register: RegisterSnapshot,
    /// Every `#[trusted]` item the register declares, as
    /// `(item, row)` — the assumption list issue #197 asks for.
    #[serde(default)]
    pub trusted: Vec<TrustedRef>,
    /// The closed [`LIMITS`] vocabulary, sorted.
    #[serde(default)]
    pub limits: Vec<String>,
}

/// The generator identity: what wrote the receipt, and at which version.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Generator {
    /// The tool name (`rexlang`).
    #[serde(default)]
    pub name: String,
    /// The crate version (`CARGO_PKG_VERSION`), recorded as provenance.
    #[serde(default)]
    pub version: String,
}

/// One source file the compile read: repo-relative path and content hash.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SourceFile {
    /// Repo-relative path, forward slashes, no `.`/`..` components.
    #[serde(default)]
    pub path: String,
    /// Lowercase hex SHA-256 of the file's bytes.
    #[serde(default)]
    pub sha256: String,
}

/// One emitted artifact: what it is, its wire version, and its hash. `path`
/// is relative to the receipt's directory and `null` for digest-only
/// entries.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArtifactRef {
    /// Relative path of the artifact file next to the receipt, or `null`
    /// when the artifact has no committed file (domain and actor artifacts
    /// are digest-only: they ship no example golden).
    #[serde(default)]
    pub path: Option<String>,
    /// One of [`ARTIFACT_KINDS`].
    #[serde(default)]
    pub kind: String,
    /// The artifact's own wire `formatVersion` (the gate its readers use).
    #[serde(default)]
    pub format_version: u32,
    /// Lowercase hex SHA-256 of the exact bytes the CLI emits.
    #[serde(default)]
    pub sha256: String,
}

/// The register snapshot: provenance plus every obligation row.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterSnapshot {
    /// The register's repo-relative source path
    /// (`docs/verification/register.json`).
    #[serde(default)]
    pub source: String,
    /// Lowercase hex SHA-256 of the register file's bytes. A register edit
    /// invalidates the receipt (the freshness gate re-reads it).
    #[serde(default)]
    pub sha256: String,
    /// The register's `reviewedAt` date (ISO `YYYY-MM-DD`).
    #[serde(default)]
    pub reviewed_at: String,
    /// The obligation rows, in register order.
    #[serde(default)]
    pub rows: Vec<RegisterRow>,
}

/// One obligation row: the register's claim, status, and evidence, plus the
/// receipt's `mutantKilled` fact.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RegisterRow {
    /// The stable row id (`V3`).
    #[serde(default)]
    pub id: String,
    /// The property in issue-#83 wording.
    #[serde(default)]
    pub claim: String,
    /// One of [`STATUSES`]; `"verified"` is rejected by name.
    #[serde(default)]
    pub status: String,
    /// What backs the row — at least one entry.
    #[serde(default)]
    pub evidence: Vec<RowEvidence>,
    /// What would close or retire the row (`argued`/`assumed` rows).
    #[serde(default)]
    pub trigger: Option<String>,
    /// When the row is re-reviewed (ISO `YYYY-MM-DD`).
    #[serde(default)]
    pub review: Option<String>,
    /// Whether a recorded mutant was killed against this row's evidence
    /// (#208). `null` for every row until the mutant pipeline lands — see
    /// [`LIMITS`].
    #[serde(default)]
    pub mutant_killed: Option<bool>,
}

/// One evidence pointer, carried from the register: what kind of thing it
/// is and where it lives (workspace-relative `path` or `path:symbol`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RowEvidence {
    /// One of [`EVIDENCE_KINDS`].
    #[serde(default)]
    pub kind: String,
    /// `path` or `path:symbol`.
    #[serde(default)]
    pub pointer: String,
}

/// One `#[trusted]` assumption: the item and the register row carrying it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TrustedRef {
    /// The attributed item's name (`push_containment`).
    #[serde(default)]
    pub item: String,
    /// The obligation id carrying it (`V9`); must exist in
    /// [`RegisterSnapshot::rows`].
    #[serde(default)]
    pub row: String,
}

/// One validation failure: the exact field path it concerns and what is
/// wrong. Field-addressed (not rule-addressed like
/// [`crate::register::Violation`]) so CI and the future MCP server can point
/// a human at the edit — e.g. `register.rows[2].status`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The field path (`artifacts[1].sha256`).
    pub field: String,
    /// What is wrong, naming the values involved.
    pub message: String,
}

impl Violation {
    fn new(field: impl Into<String>, message: impl Into<String>) -> Self {
        Violation {
            field: field.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.field, self.message)
    }
}

/// The SHA-256 digest of `bytes` as lowercase hex — the receipt's only hash
/// spelling.
#[must_use]
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(bytes);
    let mut out = String::with_capacity(hash.len() * 2);
    for byte in hash {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

impl Receipt {
    /// Serializes the receipt to its canonical JSON: alphabetical key order
    /// at every level (see the module docs), two-space indentation,
    /// no trailing newline — the bytes the freshness gate compares.
    ///
    /// # Errors
    ///
    /// Serialization cannot fail for this plain data model, but the
    /// `serde_json` error is surfaced rather than unwrapped.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        let value = canonical_key_order(serde_json::to_value(self)?);
        serde_json::to_string_pretty(&value)
    }

    /// Parses a receipt document. Parse is lenient (missing fields default
    /// to empty) so every semantic rule has one home in [`check`]; unknown
    /// fields are rejected (the closed-document rule).
    ///
    /// # Errors
    ///
    /// The `serde_json` error for malformed JSON, a wrong type, or an
    /// unknown field.
    pub fn from_json(text: &str) -> Result<Receipt, serde_json::Error> {
        serde_json::from_str(text)
    }
}

/// Recursively re-inserts object keys in alphabetical order, so the emitted
/// bytes are pinned no matter which key order `serde_json::Map` uses.
///
/// This is a deliberate local copy of `rex-backend-common`'s canonical key
/// ordering: `rex-evidence` stays a dependency-light data crate
/// (serde/serde_json/sha2), and the ordering contract is four lines —
/// importing the whole backend utility crate for it would couple the
/// evidence format to the code generators.
fn canonical_key_order(value: Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut entries: Vec<(String, Value)> = map.into_iter().collect();
            entries.sort_by(|left, right| left.0.cmp(&right.0));
            entries
                .into_iter()
                .map(|(key, value)| (key, canonical_key_order(value)))
                .collect::<serde_json::Map<String, Value>>()
                .into()
        }
        Value::Array(items) => items.into_iter().map(canonical_key_order).collect(),
        other => other,
    }
}

/// Validates one receipt document **and** re-hashes its artifacts.
///
/// `text` is the receipt's exact bytes (canonical form is part of the
/// contract, so the bytes are re-serialized and compared). `read` resolves
/// one receipt-relative artifact path to its bytes — the CLI passes a
/// filesystem closure rooted at the receipt's directory; a test passes a
/// map. No register file, model, toolchain, or network is consulted.
///
/// Returns the parsed receipt on success, or every violation (first field
/// first) so a broken document is fixed in one edit.
///
/// # Errors
///
/// A receipt that fails to parse, violates a closed vocabulary, names an
/// artifact that does not exist or does not match its recorded hash, or is
/// not in canonical form.
pub fn check(
    text: &str,
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
) -> Result<Receipt, Vec<Violation>> {
    let receipt = match Receipt::from_json(text) {
        Ok(receipt) => receipt,
        Err(error) => {
            return Err(vec![Violation::new(
                "receipt",
                format!("not a valid receipt document: {error}"),
            )])
        }
    };
    let mut violations = Vec::new();
    validate(&receipt, read, &mut violations);
    if violations.is_empty() {
        match receipt.to_json_pretty() {
            Ok(canonical) if canonical != text => violations.push(Violation::new(
                "receipt",
                "not in canonical form — receipts are byte-pinned; regenerate it with `rexlang verify`",
            )),
            Ok(_) => {}
            Err(error) => violations.push(Violation::new(
                "receipt",
                format!("cannot re-serialize the receipt: {error}"),
            )),
        }
    }
    if violations.is_empty() {
        Ok(receipt)
    } else {
        Err(violations)
    }
}

/// The semantic rules of [`check`], one home each.
fn validate(
    receipt: &Receipt,
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    out: &mut Vec<Violation>,
) {
    if receipt.format_version != FORMAT_VERSION {
        let message = if receipt.format_version > FORMAT_VERSION {
            format!(
                "unsupported formatVersion {}; this validator speaks {FORMAT_VERSION} — a newer receipt needs a newer rex-evidence",
                receipt.format_version
            )
        } else {
            format!(
                "unsupported formatVersion {}; this validator speaks {FORMAT_VERSION}",
                receipt.format_version
            )
        };
        out.push(Violation::new("formatVersion", message));
    }
    if receipt.kind != KIND {
        out.push(Violation::new(
            "kind",
            format!(
                "kind {:?} is not {KIND:?} — this is not a verification receipt",
                receipt.kind
            ),
        ));
    }
    if receipt.subject.trim().is_empty() {
        out.push(Violation::new("subject", "subject is empty"));
    }
    for (field, value) in [
        ("generator.name", &receipt.generator.name),
        ("generator.version", &receipt.generator.version),
    ] {
        if value.trim().is_empty() {
            out.push(Violation::new(field, format!("{field} is empty")));
        }
    }

    validate_inputs(receipt, out);
    validate_sources(receipt, out);
    validate_artifacts(receipt, read, out);
    validate_register(receipt, out);
    validate_limits(receipt, out);
    validate_trusted(receipt, out);
}

/// `inputs`: non-empty, relative, sorted, unique, and each one a recorded
/// source.
fn validate_inputs(receipt: &Receipt, out: &mut Vec<Violation>) {
    if receipt.inputs.is_empty() {
        out.push(Violation::new(
            "inputs",
            "inputs is empty — the receipt names no generation command",
        ));
    }
    for (index, input) in receipt.inputs.iter().enumerate() {
        let field = format!("inputs[{index}]");
        check_relative(&field, input, out);
        if let Some(previous) = index.checked_sub(1).map(|i| &receipt.inputs[i]) {
            if previous == input {
                out.push(Violation::new(
                    &field,
                    format!("duplicates inputs[{index_prev}]", index_prev = index - 1),
                ));
            } else if previous > input {
                out.push(Violation::new(
                    &field,
                    format!("breaks the required sort order ({previous:?} precedes {input:?})"),
                ));
            }
        }
        if !receipt.sources.iter().any(|source| &source.path == input) {
            out.push(Violation::new(
                &field,
                format!("{input:?} is not recorded in sources"),
            ));
        }
    }
}

/// `sources`: non-empty, relative, strictly sorted, well-formed hashes.
fn validate_sources(receipt: &Receipt, out: &mut Vec<Violation>) {
    if receipt.sources.is_empty() {
        out.push(Violation::new(
            "sources",
            "sources is empty — a receipt without source hashes proves nothing",
        ));
    }
    for (index, source) in receipt.sources.iter().enumerate() {
        let field = format!("sources[{index}].path");
        check_relative(&field, &source.path, out);
        check_sha256(&format!("sources[{index}].sha256"), &source.sha256, out);
        if let Some(previous) = index.checked_sub(1).map(|i| &receipt.sources[i]) {
            if previous.path == source.path {
                out.push(Violation::new(
                    &field,
                    format!(
                        "duplicates sources[{index_prev}].path",
                        index_prev = index - 1
                    ),
                ));
            } else if previous.path > source.path {
                out.push(Violation::new(
                    &field,
                    format!(
                        "breaks the required sort order ({:?} precedes {:?})",
                        previous.path, source.path
                    ),
                ));
            }
        }
    }
}

/// `artifacts`: the closed kind vocabulary, the sort order, relative paths,
/// and re-hashing every path-bearing entry against the supplied bytes.
fn validate_artifacts(
    receipt: &Receipt,
    read: &mut dyn FnMut(&str) -> Option<Vec<u8>>,
    out: &mut Vec<Violation>,
) {
    if receipt.artifacts.is_empty() {
        out.push(Violation::new(
            "artifacts",
            "artifacts is empty — a receipt that names no artifact is vacuous",
        ));
    }
    let mut previous_key: Option<(String, String, String)> = None;
    let mut seen_paths: Vec<&str> = Vec::new();
    for (index, artifact) in receipt.artifacts.iter().enumerate() {
        if !ARTIFACT_KINDS.contains(&artifact.kind.as_str()) {
            out.push(Violation::new(
                format!("artifacts[{index}].kind"),
                format!(
                    "artifact kind {:?} is outside the closed vocabulary {ARTIFACT_KINDS:?}",
                    artifact.kind
                ),
            ));
        }
        if artifact.format_version == 0 {
            out.push(Violation::new(
                format!("artifacts[{index}].formatVersion"),
                "artifact formatVersion must be at least 1",
            ));
        }
        check_sha256(&format!("artifacts[{index}].sha256"), &artifact.sha256, out);
        if let Some(path) = &artifact.path {
            let field = format!("artifacts[{index}].path");
            check_relative(&field, path, out);
            if seen_paths.contains(&path.as_str()) {
                out.push(Violation::new(
                    &field,
                    format!("{path:?} is recorded twice"),
                ));
            }
            seen_paths.push(path);
            match read(path) {
                None => out.push(Violation::new(
                    &field,
                    format!("artifact file {path:?} does not exist next to the receipt"),
                )),
                Some(bytes) => {
                    let actual = sha256_hex(&bytes);
                    if actual != artifact.sha256 {
                        out.push(Violation::new(
                            format!("artifacts[{index}].sha256"),
                            format!(
                                "artifact {path:?} hashes to {actual}, but the receipt records {}",
                                artifact.sha256
                            ),
                        ));
                    }
                }
            }
        } else {
            // A `null` path is a digest-only entry (no committed golden yet):
            // the hash still pins the emitted bytes, but `--check` has no
            // sibling file to re-hash. `--check-fresh` re-derives which
            // artifacts should have names, so dropping one is caught there.
        }
        let key = (
            artifact.kind.clone(),
            artifact.path.clone().unwrap_or_default(),
            artifact.sha256.clone(),
        );
        if let Some(previous) = &previous_key {
            if previous > &key {
                out.push(Violation::new(
                    format!("artifacts[{index}]"),
                    "breaks the required sort order (by kind, path, sha256)",
                ));
            }
        }
        previous_key = Some(key);
    }
}

/// `register`: provenance, row completeness, the closed status/evidence
/// vocabularies, the trigger/review discipline, and the interim
/// `mutantKilled` rule.
fn validate_register(receipt: &Receipt, out: &mut Vec<Violation>) {
    let register = &receipt.register;
    check_relative("register.source", &register.source, out);
    check_sha256("register.sha256", &register.sha256, out);
    if !is_iso_date(&register.reviewed_at) {
        out.push(Violation::new(
            "register.reviewedAt",
            format!(
                "reviewedAt {:?} is not an ISO YYYY-MM-DD date",
                register.reviewed_at
            ),
        ));
    }
    if register.rows.is_empty() {
        out.push(Violation::new(
            "register.rows",
            "register.rows is empty — the receipt reports no obligation",
        ));
    }
    let mutants_recorded = !receipt
        .limits
        .iter()
        .any(|limit| limit == "mutants-unrecorded");
    let mut seen: Vec<&str> = Vec::new();
    for (index, row) in register.rows.iter().enumerate() {
        let id = &row.id;
        let id_ok =
            id.starts_with('V') && id.len() > 1 && id[1..].chars().all(|c| c.is_ascii_digit());
        if !id_ok {
            out.push(Violation::new(
                format!("register.rows[{index}].id"),
                format!("row id {id:?} is not \"V\" + number"),
            ));
        }
        if seen.contains(&id.as_str()) {
            out.push(Violation::new(
                format!("register.rows[{index}].id"),
                format!("row id {id:?} appears more than once"),
            ));
        }
        seen.push(id);
        if row.claim.trim().is_empty() {
            out.push(Violation::new(
                format!("register.rows[{index}].claim"),
                format!("row {id} has an empty claim"),
            ));
        }
        if row.status == "verified" {
            out.push(Violation::new(
                format!("register.rows[{index}].status"),
                format!(
                    "row {id} has status \"verified\" — never a legal status; a receipt reports the \
                     register's proved/tested/argued/assumed and nothing more"
                ),
            ));
        } else if !STATUSES.contains(&row.status.as_str()) {
            out.push(Violation::new(
                format!("register.rows[{index}].status"),
                format!(
                    "row {id} has status {:?}, outside the closed vocabulary {STATUSES:?}",
                    row.status
                ),
            ));
        }
        if row.evidence.is_empty() {
            out.push(Violation::new(
                format!("register.rows[{index}].evidence"),
                format!("row {id} carries no evidence"),
            ));
        }
        // The register's status↔evidence joins, carried into the receipt so a
        // flipped status is caught by the receipt alone (the register hash
        // covers an edit only for `--check-fresh`).
        let kinds: Vec<&str> = row.evidence.iter().map(|e| e.kind.as_str()).collect();
        if row.status == "proved" && !kinds.contains(&"proof") && !kinds.contains(&"mirror") {
            out.push(Violation::new(
                format!("register.rows[{index}].status"),
                format!(
                    "row {id} claims proved but carries no proof or mirror evidence — the receipt \
                     cannot certify a status the row's own evidence does not back"
                ),
            ));
        }
        if row.status == "tested" && !kinds.contains(&"test") {
            out.push(Violation::new(
                format!("register.rows[{index}].status"),
                format!("row {id} claims tested but carries no test evidence"),
            ));
        }
        for (evidence_index, evidence) in row.evidence.iter().enumerate() {
            if !EVIDENCE_KINDS.contains(&evidence.kind.as_str()) {
                out.push(Violation::new(
                    format!("register.rows[{index}].evidence[{evidence_index}].kind"),
                    format!(
                        "row {id} has evidence kind {:?}, outside the closed vocabulary {EVIDENCE_KINDS:?}",
                        evidence.kind
                    ),
                ));
            }
            if split_pointer(&evidence.pointer).is_none() {
                out.push(Violation::new(
                    format!("register.rows[{index}].evidence[{evidence_index}].pointer"),
                    format!(
                        "row {id} has a malformed evidence pointer {:?} — expected path or path:symbol",
                        evidence.pointer
                    ),
                ));
            }
        }
        let open = matches!(row.status.as_str(), "argued" | "assumed");
        if open {
            match &row.trigger {
                Some(trigger) if !trigger.trim().is_empty() => {}
                _ => out.push(Violation::new(
                    format!("register.rows[{index}].trigger"),
                    format!(
                        "row {id} is {} with no trigger — an open row must name what would close it",
                        row.status
                    ),
                )),
            }
            match &row.review {
                Some(review) if !is_iso_date(review) => out.push(Violation::new(
                    format!("register.rows[{index}].review"),
                    format!("row {id} has a non-ISO review date {review:?}"),
                )),
                Some(review)
                    if is_iso_date(&register.reviewed_at)
                        && review.as_str() < register.reviewed_at.as_str() =>
                {
                    out.push(Violation::new(
                        format!("register.rows[{index}].review"),
                        format!(
                            "row {id} review {review} precedes reviewedAt {}",
                            register.reviewed_at
                        ),
                    ));
                }
                Some(_) => {}
                None => out.push(Violation::new(
                    format!("register.rows[{index}].review"),
                    format!("row {id} has a trigger but no review date"),
                )),
            }
        }
        if row.mutant_killed.is_some() && !mutants_recorded {
            out.push(Violation::new(
                format!("register.rows[{index}].mutantKilled"),
                format!(
                    "row {id} records a mutant kill while limits declares \"mutants-unrecorded\" — \
                     drop the limit (issue #208) before recording mutant facts"
                ),
            ));
        }
    }
}

/// `limits`: the closed vocabulary, sorted and unique.
fn validate_limits(receipt: &Receipt, out: &mut Vec<Violation>) {
    for (index, limit) in receipt.limits.iter().enumerate() {
        if !LIMITS.contains(&limit.as_str()) {
            out.push(Violation::new(
                format!("limits[{index}]"),
                format!("limit {limit:?} is outside the closed vocabulary {LIMITS:?}"),
            ));
        }
        if index > 0 && receipt.limits[index - 1] >= *limit {
            out.push(Violation::new(
                format!("limits[{index}]"),
                "limits must be sorted and unique",
            ));
        }
    }
}

/// `trusted`: every row reference resolves into the embedded rows.
fn validate_trusted(receipt: &Receipt, out: &mut Vec<Violation>) {
    for (index, trusted) in receipt.trusted.iter().enumerate() {
        if trusted.item.trim().is_empty() {
            out.push(Violation::new(
                format!("trusted[{index}].item"),
                "a trusted row names no item",
            ));
        }
        if !receipt
            .register
            .rows
            .iter()
            .any(|row| row.id == trusted.row)
        {
            out.push(Violation::new(
                format!("trusted[{index}].row"),
                format!(
                    "trusted item {:?} references obligation {:?}, which the receipt does not carry",
                    trusted.item, trusted.row
                ),
            ));
        }
    }
}

/// A path a receipt may name: relative, no `.`/`..` components, no
/// backslashes (the wire spelling is forward slashes).
fn check_relative(field: &str, path: &str, out: &mut Vec<Violation>) {
    if path.trim().is_empty() {
        out.push(Violation::new(field, "path is empty"));
        return;
    }
    if path.starts_with('/') || path.starts_with('\\') || path.contains(':') {
        out.push(Violation::new(
            field,
            format!("path {path:?} is absolute; receipts name relative paths only"),
        ));
        return;
    }
    if path.contains('\\') {
        out.push(Violation::new(
            field,
            format!("path {path:?} uses backslashes; the wire spelling is forward slashes"),
        ));
        return;
    }
    for component in path.split('/') {
        if component == ".." {
            out.push(Violation::new(
                field,
                format!("path {path:?} escapes its base with `..`"),
            ));
            return;
        }
        if component == "." || component.is_empty() {
            out.push(Violation::new(
                field,
                format!("path {path:?} is not normalized (empty or `.` component)"),
            ));
            return;
        }
    }
}

/// A well-formed hash: exactly 64 lowercase hex digits.
fn check_sha256(field: &str, digest: &str, out: &mut Vec<Violation>) {
    let well_formed = digest.len() == 64
        && digest
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !well_formed {
        out.push(Violation::new(
            field,
            format!("{digest:?} is not a lowercase hex SHA-256 (64 characters)"),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(fill: char) -> String {
        std::iter::repeat_n(fill, 64).collect()
    }

    fn source(path: &str, digest: char) -> SourceFile {
        SourceFile {
            path: path.to_string(),
            sha256: hex(digest),
        }
    }

    fn row(id: &str, status: &str) -> RegisterRow {
        RegisterRow {
            id: id.to_string(),
            claim: format!("property {id}"),
            status: status.to_string(),
            evidence: vec![RowEvidence {
                kind: "test".to_string(),
                pointer: "crates/rex-verify/tests/invariants.rs:p1_x".to_string(),
            }],
            trigger: Some("the toolchain bump lands".to_string()),
            review: Some("2027-02-13".to_string()),
            mutant_killed: None,
        }
    }

    /// A minimal valid receipt, byte-canonical once serialized.
    fn sample() -> Receipt {
        Receipt {
            format_version: FORMAT_VERSION,
            kind: KIND.to_string(),
            subject: "support".to_string(),
            generator: Generator {
                name: "rexlang".to_string(),
                version: "0.2.0".to_string(),
            },
            inputs: vec!["examples/support.mox".to_string()],
            sources: vec![
                source("examples/support.mox", 'a'),
                source("patterns/ifml/workflow.ifml", 'b'),
            ],
            artifacts: vec![ArtifactRef {
                path: None,
                kind: "irModel".to_string(),
                format_version: 1,
                sha256: hex('c'),
            }],
            register: RegisterSnapshot {
                source: "docs/verification/register.json".to_string(),
                sha256: hex('d'),
                reviewed_at: "2026-11-14".to_string(),
                rows: vec![row("V1", "tested"), row("V3", "argued")],
            },
            trusted: vec![TrustedRef {
                item: "push_containment".to_string(),
                row: "V1".to_string(),
            }],
            limits: vec!["mutants-unrecorded".to_string()],
        }
    }

    fn validate_text(text: &str) -> Vec<Violation> {
        check(text, &mut |_| None).expect_err("invalid receipt")
    }

    fn fields(violations: &[Violation]) -> Vec<&str> {
        violations.iter().map(|v| v.field.as_str()).collect()
    }

    #[test]
    fn serialization_is_deterministic_and_canonical() {
        let receipt = sample();
        let first = receipt.to_json_pretty().expect("serialize");
        let second = receipt.to_json_pretty().expect("serialize");
        assert_eq!(first, second, "two serializations must be byte-identical");

        // Alphabetical key order at the root and inside every struct.
        let value: Value = serde_json::from_str(&first).expect("valid JSON");
        let root: Vec<&String> = value.as_object().expect("object").keys().collect();
        let mut sorted = root.clone();
        sorted.sort();
        assert_eq!(root, sorted, "root keys are alphabetical");
        let generator: Vec<&String> = value["generator"].as_object().unwrap().keys().collect();
        assert_eq!(generator, vec!["name", "version"]);
        let row: Vec<&String> = value["register"]["rows"][0]
            .as_object()
            .unwrap()
            .keys()
            .collect();
        assert_eq!(
            row,
            vec![
                "claim",
                "evidence",
                "id",
                "mutantKilled",
                "review",
                "status",
                "trigger"
            ]
        );
    }

    #[test]
    fn round_trip_is_lossless() {
        let receipt = sample();
        let text = receipt.to_json_pretty().expect("serialize");
        let reloaded = Receipt::from_json(&text).expect("parse");
        assert_eq!(reloaded, receipt);
        assert_eq!(reloaded.to_json_pretty().expect("re-serialize"), text);
    }

    #[test]
    fn a_valid_receipt_passes_with_its_artifact_bytes() {
        let bytes = b"{\"formatVersion\":1}".to_vec();
        let mut receipt = sample();
        receipt.artifacts = vec![ArtifactRef {
            path: Some("support.ddd.json".to_string()),
            kind: "dddDesign".to_string(),
            format_version: 1,
            sha256: sha256_hex(&bytes),
        }];
        let text = receipt.to_json_pretty().expect("serialize");
        let mut read = |path: &str| (path == "support.ddd.json").then(|| bytes.clone());
        let parsed = check(&text, &mut read).expect("receipt is valid");
        assert_eq!(parsed, receipt);
    }

    #[test]
    fn a_higher_format_version_is_rejected_by_name() {
        let mut receipt = sample();
        receipt.format_version = FORMAT_VERSION + 1;
        let text = receipt.to_json_pretty().expect("serialize");
        let violations = validate_text(&text);
        assert_eq!(fields(&violations), vec!["formatVersion"]);
        assert!(
            violations[0].message.contains("newer rex-evidence"),
            "{violations:?}"
        );
    }

    #[test]
    fn a_verified_status_is_rejected_with_the_reason() {
        let mut receipt = sample();
        receipt.register.rows[1].status = "verified".to_string();
        let text = receipt.to_json_pretty().expect("serialize");
        let violations = validate_text(&text);
        assert!(
            fields(&violations).contains(&"register.rows[1].status"),
            "{violations:?}"
        );
        assert!(
            violations[0].message.contains("never a legal status"),
            "{violations:?}"
        );
    }

    #[test]
    fn an_unknown_status_names_its_field() {
        let mut receipt = sample();
        receipt.register.rows[0].status = "looks-good".to_string();
        let text = receipt.to_json_pretty().expect("serialize");
        let violations = validate_text(&text);
        assert_eq!(fields(&violations), vec!["register.rows[0].status"]);
    }

    #[test]
    fn an_edited_artifact_hash_names_the_hash_field() {
        let bytes = b"artifact".to_vec();
        let mut receipt = sample();
        receipt.artifacts = vec![ArtifactRef {
            path: Some("support.ddd.json".to_string()),
            kind: "dddDesign".to_string(),
            format_version: 1,
            sha256: sha256_hex(&bytes),
        }];
        let mut tampered = receipt.clone();
        tampered.artifacts[0].sha256 = hex('e');
        let text = tampered.to_json_pretty().expect("serialize");
        let violations = check(&text, &mut |_| Some(bytes.clone())).expect_err("hash mismatch");
        assert_eq!(fields(&violations), vec!["artifacts[0].sha256"]);
        assert!(
            violations[0].message.contains(&sha256_hex(&bytes)),
            "the message names the actual hash: {violations:?}"
        );
    }

    #[test]
    fn a_missing_artifact_names_the_path_field() {
        let mut receipt = sample();
        receipt.artifacts = vec![ArtifactRef {
            path: Some("support.ddd.json".to_string()),
            kind: "dddDesign".to_string(),
            format_version: 1,
            sha256: hex('d'),
        }];
        let text = receipt.to_json_pretty().expect("serialize");
        let violations = validate_text(&text);
        assert_eq!(fields(&violations), vec!["artifacts[0].path"]);
        assert!(
            violations[0].message.contains("does not exist"),
            "{violations:?}"
        );
    }

    #[test]
    fn an_absolute_path_is_rejected_on_every_path_field() {
        // sources[i].path
        let mut receipt = sample();
        receipt.sources[0].path = "/etc/passwd".to_string();
        let violations = validate_text(&receipt.to_json_pretty().expect("serialize"));
        assert!(
            violations
                .iter()
                .any(|v| v.field == "sources[0].path" && v.message.contains("absolute")),
            "{violations:?}"
        );

        // inputs[i] (and the matching source, so the two agree)
        let mut receipt = sample();
        receipt.inputs[0] = "/tmp/support.mox".to_string();
        receipt.sources[0].path = "/tmp/support.mox".to_string();
        let violations = validate_text(&receipt.to_json_pretty().expect("serialize"));
        assert!(
            violations
                .iter()
                .any(|v| v.field == "inputs[0]" && v.message.contains("absolute")),
            "{violations:?}"
        );

        // register.source
        let mut receipt = sample();
        receipt.register.source = "/tmp/register.json".to_string();
        let violations = validate_text(&receipt.to_json_pretty().expect("serialize"));
        assert!(
            violations
                .iter()
                .any(|v| v.field == "register.source" && v.message.contains("absolute")),
            "{violations:?}"
        );
    }

    #[test]
    fn a_parent_escape_is_rejected() {
        let mut receipt = sample();
        receipt.sources[0].path = "../secrets/support.mox".to_string();
        let text = receipt.to_json_pretty().expect("serialize");
        let violations = validate_text(&text);
        assert!(
            violations
                .iter()
                .any(|v| v.field == "sources[0].path" && v.message.contains("escapes")),
            "{violations:?}"
        );
    }

    #[test]
    fn a_non_canonical_byte_form_is_rejected() {
        let receipt = sample();
        let text = receipt.to_json_pretty().expect("serialize");
        // The canonical form has no trailing newline; anything else means
        // the file was hand-edited or produced by a different emitter.
        let reformatted = format!("{text}\n");
        let violations = validate_text(&reformatted);
        assert_eq!(fields(&violations), vec!["receipt"]);
        assert!(
            violations[0].message.contains("canonical form"),
            "{violations:?}"
        );
    }

    #[test]
    fn an_unknown_root_key_is_rejected_by_name() {
        let receipt = sample();
        let mut value: Value =
            serde_json::from_str(&receipt.to_json_pretty().expect("serialize")).expect("JSON");
        value["verified"] = Value::Bool(true);
        let text = serde_json::to_string_pretty(&value).expect("serialize");
        let violations = validate_text(&text);
        assert_eq!(fields(&violations), vec!["receipt"]);
        assert!(violations[0].message.contains("verified"), "{violations:?}");
    }

    #[test]
    fn mutant_facts_require_dropping_the_limit() {
        let mut receipt = sample();
        receipt.register.rows[0].mutant_killed = Some(true);
        let text = receipt.to_json_pretty().expect("serialize");
        let violations = validate_text(&text);
        assert!(
            fields(&violations).contains(&"register.rows[0].mutantKilled"),
            "{violations:?}"
        );

        receipt.limits.clear();
        let text = receipt.to_json_pretty().expect("serialize");
        check(&text, &mut |_| None).expect("a recorded mutant is legal once the limit is dropped");
    }

    #[test]
    fn unsorted_sources_are_rejected() {
        let mut receipt = sample();
        receipt.sources.reverse();
        let text = receipt.to_json_pretty().expect("serialize");
        let violations = validate_text(&text);
        assert!(
            fields(&violations).contains(&"sources[1].path"),
            "{violations:?}"
        );
    }
}
