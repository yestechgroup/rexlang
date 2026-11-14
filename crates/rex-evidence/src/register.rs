//! The register's data model, parse, validation, and markdown rendering.
//!
//! The register source is `docs/verification/register.json`; everything
//! here is pure (no filesystem) so hosts — the drift gate, future CLI
//! consumers — resolve pointers themselves.

use serde::Deserialize;
use serde::Serialize;

/// The register wire format this crate speaks. A register whose
/// `formatVersion` differs is rejected with both versions named.
pub const FORMAT_VERSION: u32 = 1;

/// The closed obligation-status vocabulary. `"verified"` is deliberately
/// not a status — a register row is never auto-honest; every status is
/// one of these four words, each with a distinct meaning
/// (`docs/VERIFICATION.md`'s legend).
pub const STATUSES: [&str; 4] = ["proved", "tested", "argued", "assumed"];

/// The closed evidence-kind vocabulary: what a pointer is evidence *of*.
pub const EVIDENCE_KINDS: [&str; 5] = ["proof", "test", "mirror", "doc", "code"];

/// The whole register document: the obligations, the pipeline rows, the
/// deliberate holds, and the trusted-item list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Register {
    /// The wire-format gate; must equal [`FORMAT_VERSION`].
    #[serde(default)]
    pub format_version: u32,
    /// The date the register was last reviewed, ISO `YYYY-MM-DD`; every
    /// row's `review` must not precede it.
    #[serde(default)]
    pub reviewed_at: String,
    /// The proof-obligation rows (V1…).
    #[serde(default)]
    pub obligations: Vec<Obligation>,
    /// The pipeline boundary rows.
    #[serde(default)]
    pub pipeline: Vec<PipelineRow>,
    /// The deliberate holds (dependency or strategy), each with a trigger
    /// and a review date.
    #[serde(default)]
    pub hold: Vec<HoldRow>,
    /// Every `#[trusted]` item in the tree, mapped to its register row.
    #[serde(default)]
    pub trusted: Vec<TrustedRow>,
}

/// One proof-obligation row: the claim (issue-#83 wording), its formal
/// statement, its enforcing implementation, a status from the closed
/// vocabulary, and the evidence pointing at what backs it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Obligation {
    /// The stable row id (`V3`); unique, `"V"` + number.
    #[serde(default)]
    pub id: String,
    /// The property in issue-#83 wording.
    #[serde(default)]
    pub claim: String,
    /// The formal specification side.
    #[serde(default)]
    pub statement: String,
    /// The enforcing-implementation side.
    #[serde(default)]
    pub implementation: String,
    /// One of [`STATUSES`]; `"verified"` is rejected by name.
    #[serde(default)]
    pub status: String,
    /// What backs the row — at least one entry.
    #[serde(default)]
    pub evidence: Vec<Evidence>,
    /// Human-summary residue beyond the derived status (open residue,
    /// staging caveats); appended to the rendered Status column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_note: Option<String>,
    /// Required for `argued`/`assumed` rows and for rows carrying a
    /// [`Mirror`]: what would close or retire this row.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// Required alongside [`Obligation::trigger`]; ISO, ≥ `reviewedAt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub review: Option<String>,
    /// Present on spec-mirror rows: the real implementation the proved
    /// mirror stands in for, and what blocks moving the proof in-crate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mirror: Option<Mirror>,
}

/// One evidence pointer: what kind of thing it is, and where it lives
/// (`path` or `path:symbol`, workspace-relative).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Evidence {
    /// One of [`EVIDENCE_KINDS`].
    #[serde(default)]
    pub kind: String,
    /// `path` or `path:symbol`; the gate resolves both against the
    /// workspace root.
    #[serde(default)]
    pub pointer: String,
}

/// A spec-mirror annotation: which real implementation the proved mirror
/// stands in for, and what blocks moving the proof in-crate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Mirror {
    /// The real implementation the mirror mirrors (`rex-ir
    /// `ClassDef::assign_feature_ids``).
    #[serde(default)]
    pub of: String,
    /// The upstream gap keeping the mirror out of the real crate
    /// (`creusot-std 0.14.0 gaps`).
    #[serde(default)]
    pub blocked_by: String,
}

/// One pipeline-boundary row: what the stage wants and what enforces it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PipelineRow {
    /// The boundary name (`Parsing`).
    #[serde(default)]
    pub name: String,
    /// What we want (the issue's wording).
    #[serde(default)]
    pub want: String,
    /// Current enforcement.
    #[serde(default)]
    pub enforcement: String,
    /// The human status summary (free text — pipeline rows summarize, the
    /// closed status vocabulary governs obligations only).
    #[serde(default)]
    pub status: String,
}

/// One deliberate hold: something considered and consciously not adopted,
/// with the trigger that reopens it and the date it is re-reviewed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HoldRow {
    /// What is held (`serde_jcs (RFC 8785 canonical JSON)`).
    #[serde(default)]
    pub item: String,
    /// Why it is held.
    #[serde(default)]
    pub reason: String,
    /// What would reopen the decision.
    #[serde(default)]
    pub trigger: String,
    /// When it is re-reviewed (ISO `YYYY-MM-DD`).
    #[serde(default)]
    pub review: String,
}

/// One `#[trusted]` item: the item name, the register row carrying it,
/// and why it is not proved. The drift gate keeps this list
/// bidirectionally consistent with the tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TrustedRow {
    /// The attributed item's name (`push_containment`).
    #[serde(default)]
    pub item: String,
    /// The obligation id carrying it (`V9`); must exist.
    #[serde(default)]
    pub row: String,
    /// Why the item is trusted rather than proved.
    #[serde(default)]
    pub reason: String,
}

/// One validation failure: the rule it broke and a message naming the
/// offending row or field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Violation {
    /// The stable rule name (`status-vocabulary`).
    pub rule: &'static str,
    /// What is wrong, naming the row/field.
    pub message: String,
}

impl Violation {
    fn new(rule: &'static str, message: impl Into<String>) -> Self {
        Violation {
            rule,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Violation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.rule, self.message)
    }
}

/// Parses the register document from its JSON text. Parse is lenient —
/// missing fields deserialize as empty — so every semantic rule has one
/// home in [`validate`].
pub fn parse(text: &str) -> Result<Register, serde_json::Error> {
    serde_json::from_str(text)
}

/// Validates the register's semantics. Returns every violation (not
/// first-fail) so a broken register is fixed in one edit.
#[must_use]
pub fn validate(register: &Register) -> Vec<Violation> {
    let mut out = Vec::new();
    if register.format_version != FORMAT_VERSION {
        out.push(Violation::new(
            "formatVersion",
            format!(
                "unsupported register formatVersion {}; this validator speaks {FORMAT_VERSION} — newer registers need a newer rex-evidence",
                register.format_version
            ),
        ));
    }
    if !is_iso_date(&register.reviewed_at) {
        out.push(Violation::new(
            "reviewedAt",
            format!(
                "reviewedAt {:?} is not an ISO YYYY-MM-DD date",
                register.reviewed_at
            ),
        ));
    }

    let mut seen = std::collections::BTreeSet::new();
    for obligation in &register.obligations {
        validate_obligation(register, obligation, &mut seen, &mut out);
    }
    for row in &register.pipeline {
        for (field, value) in [
            ("name", &row.name),
            ("want", &row.want),
            ("enforcement", &row.enforcement),
            ("status", &row.status),
        ] {
            if value.trim().is_empty() {
                out.push(Violation::new(
                    "pipeline-completeness",
                    format!("pipeline row {:?} has an empty {field}", row.name),
                ));
            }
        }
    }
    for row in &register.hold {
        for (field, value) in [
            ("item", &row.item),
            ("reason", &row.reason),
            ("trigger", &row.trigger),
        ] {
            if value.trim().is_empty() {
                out.push(Violation::new(
                    "hold-completeness",
                    format!("hold row {:?} has an empty {field}", row.item),
                ));
            }
        }
        if !is_iso_date(&row.review) {
            out.push(Violation::new(
                "hold-completeness",
                format!(
                    "hold row {:?} has a non-ISO review date {:?}",
                    row.item, row.review
                ),
            ));
        } else if is_iso_date(&register.reviewed_at)
            && row.review.as_str() < register.reviewed_at.as_str()
        {
            out.push(Violation::new(
                "hold-completeness",
                format!(
                    "hold row {:?} review {} precedes reviewedAt {}",
                    row.item, row.review, register.reviewed_at
                ),
            ));
        }
    }
    for row in &register.trusted {
        if row.item.trim().is_empty() {
            out.push(Violation::new(
                "trusted-row-reference",
                "a trusted row names no item",
            ));
        }
        if row.reason.trim().is_empty() {
            out.push(Violation::new(
                "trusted-row-reference",
                format!("trusted item {:?} names no reason", row.item),
            ));
        }
        if !register.obligations.iter().any(|o| o.id == row.row) {
            out.push(Violation::new(
                "trusted-row-reference",
                format!(
                    "trusted item {:?} references obligation {:?}, which does not exist",
                    row.item, row.row
                ),
            ));
        }
    }
    out
}

/// Validates one obligation row against the whole register.
fn validate_obligation(
    register: &Register,
    obligation: &Obligation,
    seen: &mut std::collections::BTreeSet<String>,
    out: &mut Vec<Violation>,
) {
    let id = &obligation.id;
    if !obligation.id.starts_with('V')
        || obligation.id.len() <= 1
        || !obligation.id[1..].chars().all(|c| c.is_ascii_digit())
    {
        out.push(Violation::new(
            "id-grammar",
            format!("obligation id {id:?} is not \"V\" + number"),
        ));
    }
    if !seen.insert(obligation.id.clone()) {
        out.push(Violation::new(
            "duplicate-id",
            format!("obligation id {id:?} appears more than once"),
        ));
    }
    for (field, value) in [
        ("claim", &obligation.claim),
        ("statement", &obligation.statement),
        ("implementation", &obligation.implementation),
    ] {
        if value.trim().is_empty() {
            out.push(Violation::new(
                "row-completeness",
                format!("obligation {id} has an empty {field}"),
            ));
        }
    }

    if obligation.status == "verified" {
        out.push(Violation::new(
            "status-vocabulary",
            format!(
                "obligation {id} has status \"verified\" — never a legal status; a row is proved, tested, argued, or assumed, each by its own evidence",
            ),
        ));
    } else if !STATUSES.contains(&obligation.status.as_str()) {
        out.push(Violation::new(
            "status-vocabulary",
            format!(
                "obligation {id} has status {:?}, outside the closed vocabulary {STATUSES:?}",
                obligation.status
            ),
        ));
    }

    if obligation.evidence.is_empty() {
        out.push(Violation::new(
            "evidence-required",
            format!("obligation {id} carries no evidence"),
        ));
    }
    for evidence in &obligation.evidence {
        if !EVIDENCE_KINDS.contains(&evidence.kind.as_str()) {
            out.push(Violation::new(
                "evidence-kind",
                format!(
                    "obligation {id} has evidence kind {:?}, outside the closed vocabulary {EVIDENCE_KINDS:?}",
                    evidence.kind
                ),
            ));
        }
        if split_pointer(&evidence.pointer).is_none() {
            out.push(Violation::new(
                "pointer-grammar",
                format!(
                    "obligation {id} has a malformed evidence pointer {:?} — expected path or path:symbol",
                    evidence.pointer
                ),
            ));
        }
    }

    let kinds: Vec<&str> = obligation
        .evidence
        .iter()
        .map(|e| e.kind.as_str())
        .collect();
    if obligation.status == "proved" && !kinds.contains(&"proof") && obligation.mirror.is_none() {
        out.push(Violation::new(
            "proved-needs-proof",
            format!("obligation {id} claims proved but carries no proof (or mirror) evidence"),
        ));
    }
    if obligation.status == "tested" && !kinds.contains(&"test") {
        out.push(Violation::new(
            "tested-needs-test",
            format!("obligation {id} claims tested but carries no test evidence"),
        ));
    }

    let needs_schedule =
        matches!(obligation.status.as_str(), "argued" | "assumed") || obligation.mirror.is_some();
    if needs_schedule {
        match &obligation.trigger {
            Some(trigger) if trigger.trim().is_empty() => out.push(Violation::new(
                "trigger-required",
                format!("obligation {id} has an empty trigger"),
            )),
            None => out.push(Violation::new(
                "trigger-required",
                format!(
                    "obligation {id} is {} with no trigger — an open row must name what would close it",
                    obligation.status
                ),
            )),
            _ => {}
        }
        match &obligation.review {
            Some(review) if !is_iso_date(review) => out.push(Violation::new(
                "review-date",
                format!("obligation {id} has a non-ISO review date {review:?}"),
            )),
            Some(review)
                if is_iso_date(&register.reviewed_at)
                    && review.as_str() < register.reviewed_at.as_str() =>
            {
                out.push(Violation::new(
                    "review-after-reviewedAt",
                    format!(
                        "obligation {id} review {review} precedes reviewedAt {}",
                        register.reviewed_at
                    ),
                ));
            }
            None => out.push(Violation::new(
                "review-required",
                format!("obligation {id} has a trigger but no review date"),
            )),
            _ => {}
        }
        if let Some(mirror) = &obligation.mirror {
            for (field, value) in [("of", &mirror.of), ("blockedBy", &mirror.blocked_by)] {
                if value.trim().is_empty() {
                    out.push(Violation::new(
                        "row-completeness",
                        format!("obligation {id} mirror has an empty {field}"),
                    ));
                }
            }
        }
    }
}

/// `true` when `s` is a real calendar date in ISO `YYYY-MM-DD` form.
/// Hand-rolled (this crate is dependency-light): real month lengths, real
/// leap years. Shared with [`crate::receipt`], whose rows carry the same
/// review-schedule dates.
#[must_use]
pub(crate) fn is_iso_date(s: &str) -> bool {
    let bytes = s.as_bytes();
    if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    if !bytes
        .iter()
        .enumerate()
        .all(|(i, b)| (i == 4 || i == 7) || b.is_ascii_digit())
    {
        return false;
    }
    let (Ok(year), Ok(month), Ok(day)) = (
        s[..4].parse::<i32>(),
        s[5..7].parse::<u32>(),
        s[8..10].parse::<u32>(),
    ) else {
        return false;
    };
    if !(1..=12).contains(&month) {
        return false;
    }
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        _ if leap => 29,
        _ => 28,
    };
    (1..=days).contains(&day)
}

/// Splits a pointer into `(path, Option<symbol>)` per the grammar
/// `path(:symbol)?` — path characters `A-Za-z0-9_./\-`, symbol characters
/// `A-Za-z0-9_`. `None` when the pointer is malformed. Shared with
/// [`crate::receipt`], which carries the pointers verbatim.
#[must_use]
pub(crate) fn split_pointer(pointer: &str) -> Option<(&str, Option<&str>)> {
    let valid_path = |p: &str| {
        !p.is_empty()
            && p.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | '\\' | '-'))
    };
    let valid_symbol =
        |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if let Some((path, symbol)) = pointer.rsplit_once(':') {
        if valid_path(path) && valid_symbol(symbol) {
            return Some((path, Some(symbol)));
        }
    }
    if valid_path(pointer) {
        Some((pointer, None))
    } else {
        None
    }
}

/// Derives one obligation's human Status cell from its closed-vocabulary
/// status plus its mirror/note — the human summary the register legend
/// describes.
#[must_use]
fn status_cell(obligation: &Obligation) -> String {
    let mut summary = match obligation.status.as_str() {
        "proved" if obligation.mirror.is_some() => "proved (spec mirror)".to_string(),
        "argued" => "argued + tested".to_string(),
        other => other.to_string(),
    };
    if let Some(note) = &obligation.status_note {
        summary.push_str(" — ");
        summary.push_str(note);
    }
    summary
}

/// Escapes a cell's text for a markdown table row.
fn cell(text: &str) -> String {
    text.replace('|', "\\|")
}

/// Renders the register as the generated markdown block of
/// `docs/VERIFICATION.md`: the obligation table, the pipeline table, the
/// evidence index, and the hold table. The gate byte-compares this
/// against the document between the register markers.
#[must_use]
pub fn render_markdown(register: &Register) -> String {
    let mut out = String::new();

    out.push_str("| # | Property (issue #83 wording) | Formal specification | Enforcing implementation | Status |\n");
    out.push_str("|---|---|---|---|---|\n");
    for obligation in &register.obligations {
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(&obligation.id),
            cell(&obligation.claim),
            cell(&obligation.statement),
            cell(&obligation.implementation),
            cell(&status_cell(obligation)),
        ));
    }

    out.push_str("\n### Pipeline-level obligations (issue's five boundaries)\n\n");
    out.push_str("| Boundary | What we want (issue) | Current enforcement | Status |\n");
    out.push_str("|---|---|---|---|\n");
    for row in &register.pipeline {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            cell(&row.name),
            cell(&row.want),
            cell(&row.enforcement),
            cell(&row.status),
        ));
    }

    out.push_str("\n### Evidence\n\n");
    out.push_str("| ID | Status | Evidence | Trigger | Review |\n");
    out.push_str("|---|---|---|---|---|\n");
    for obligation in &register.obligations {
        let evidence = obligation
            .evidence
            .iter()
            .map(|e| format!("`{}`:`{}`", cell(&e.kind), cell(&e.pointer)))
            .collect::<Vec<String>>()
            .join("<br>");
        let evidence = if evidence.is_empty() {
            "—".to_string()
        } else {
            evidence
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            cell(&obligation.id),
            cell(&obligation.status),
            evidence,
            cell(obligation.trigger.as_deref().unwrap_or("—")),
            cell(obligation.review.as_deref().unwrap_or("—")),
        ));
    }

    out.push_str("\n### Deliberate holds\n\n");
    out.push_str("| Item | Reason | Trigger | Review |\n");
    out.push_str("|---|---|---|---|\n");
    for row in &register.hold {
        out.push_str(&format!(
            "| {} | {} | {} | {} |\n",
            cell(&row.item),
            cell(&row.reason),
            cell(&row.trigger),
            cell(&row.review),
        ));
    }
    out
}
