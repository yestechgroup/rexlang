//! Diagnostics (errors and warnings) and their rendering with [ariadne].
//!
//! [ariadne]: https://crates.io/crates/ariadne

use std::collections::BTreeSet;

use ariadne::{Config, IndexType, Label, Report, ReportKind, Source};

/// Byte-offset span into the source text.
pub use rex_syntax::Span;

/// Severity of a [`Diagnostic`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// A problem that prevents compilation; no IR artifact is produced.
    Error,
    /// A suspicious construct that does not prevent compilation.
    Warning,
}

/// Machine-readable diagnostic codes, serializable for structured LSP
/// payloads (e.g. quick-fix `data`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticCode {
    /// `feature '<feature>' has class type '<class>'` — the author almost
    /// certainly meant `contains` or `refers`; a quick fix can offer both.
    AttributeWithClassType {
        /// The name of the feature (attribute or derived feature) involved.
        feature: String,
        /// The class-typed name as written in the source.
        class: String,
        /// Byte span of the offending type reference in the source — where
        /// a quick fix must edit.
        type_span: Span,
    },
    /// `unknown capability '<capability>' on …` — the capability name
    /// carries no resolution to a declared actor capability (issue
    /// rexlang#128's J3, resolved at compile time). `suggestion` is the
    /// [`did_you_mean`] closest declared name, when one is close enough to
    /// be a plausible typo — a quick fix can rename the reference to it.
    UnknownCapability {
        /// The capability name as written in the source.
        capability: String,
        /// The suggested declared name, if any.
        suggestion: Option<String>,
        /// Byte span of the offending capability reference — where a
        /// quick fix must edit.
        span: Span,
    },
    /// `capability '<capability>' is granted but no operation or search
    /// carries it` — the granted capability guards a transition that
    /// cannot happen (issue rexlang#128's J4, resolved at compile time).
    /// The fix lives in another file (an operation must carry the
    /// capability), so there is no local quick fix; the payload drives
    /// structured tooling instead.
    GrantedCapabilityUncarried {
        /// The granted capability name as written in the source.
        capability: String,
        /// Byte span of the grant entry's capability reference.
        span: Span,
    },
}

impl DiagnosticCode {
    /// A stable, camelCase name for the code, suitable as the LSP
    /// diagnostic `code` string.
    pub fn name(&self) -> &'static str {
        match self {
            DiagnosticCode::AttributeWithClassType { .. } => "attributeWithClassType",
            DiagnosticCode::UnknownCapability { .. } => "unknownCapability",
            DiagnosticCode::GrantedCapabilityUncarried { .. } => "grantedCapabilityUncarried",
        }
    }
}

/// Suggests the closest declared name for an unknown one, or `None` when
/// nothing is close enough to be a plausible typo — the did-you-mean
/// helper behind the resolver-standard diagnostics (issue rexlang#128).
///
/// Resolution order:
///
/// 1. a case-variant match (ASCII-case-insensitive equality) wins
///    outright — the author spelled the right name in the wrong case;
/// 2. otherwise the candidate with the smallest Levenshtein distance
///    (over `char`s) wins, provided the distance is within budget —
///    `max(2, name_chars / 3)` edits;
/// 3. ties break alphabetically (candidates arrive in `BTreeSet` order
///    and only a strictly smaller distance replaces the best).
///
/// An empty candidate set yields `None` by construction.
pub(crate) fn did_you_mean(name: &str, declared: &BTreeSet<String>) -> Option<String> {
    if let Some(case_variant) = declared
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
    {
        return Some(case_variant.clone());
    }
    let budget = std::cmp::max(2, name.chars().count() / 3);
    let mut best: Option<(usize, &String)> = None;
    for candidate in declared {
        let distance = levenshtein(name, candidate);
        if distance <= budget && best.is_none_or(|(best, _)| distance < best) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate.clone())
}

/// The Levenshtein edit distance between two strings, over `char`s
/// (one insertion, deletion, or substitution per edit; no transposition).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, a_char) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, b_char) in b.iter().enumerate() {
            let cost = usize::from(a_char != b_char);
            current.push(usize::min(
                usize::min(previous[j + 1] + 1, current[j] + 1),
                previous[j] + cost,
            ));
        }
        previous = current;
    }
    previous[b.len()]
}

/// The `attributeWithClassType` payload, serialized with camelCase keys and
/// the span as a `[start, end]` pair.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct AttributeWithClassTypePayload<'a> {
    feature: &'a str,
    class: &'a str,
    type_span: [usize; 2],
}

/// The `unknownCapability` payload, serialized with camelCase keys and the
/// span as a `[start, end]` pair; `suggestion` is omitted when absent.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct UnknownCapabilityPayload<'a> {
    capability: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    suggestion: Option<&'a str>,
    span: [usize; 2],
}

/// The `grantedCapabilityUncarried` payload, serialized with camelCase keys
/// and the span as a `[start, end]` pair.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct GrantedCapabilityUncarriedPayload<'a> {
    capability: &'a str,
    span: [usize; 2],
}

impl serde::Serialize for DiagnosticCode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeMap;
        match self {
            DiagnosticCode::AttributeWithClassType {
                feature,
                class,
                type_span,
            } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "attributeWithClassType",
                    &AttributeWithClassTypePayload {
                        feature,
                        class,
                        type_span: [type_span.start, type_span.end],
                    },
                )?;
                map.end()
            }
            DiagnosticCode::UnknownCapability {
                capability,
                suggestion,
                span,
            } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "unknownCapability",
                    &UnknownCapabilityPayload {
                        capability,
                        suggestion: suggestion.as_deref(),
                        span: [span.start, span.end],
                    },
                )?;
                map.end()
            }
            DiagnosticCode::GrantedCapabilityUncarried { capability, span } => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(
                    "grantedCapabilityUncarried",
                    &GrantedCapabilityUncarriedPayload {
                        capability,
                        span: [span.start, span.end],
                    },
                )?;
                map.end()
            }
        }
    }
}

/// A single compiler diagnostic: a severity, a message, an optional span,
/// an optional help hint, and an optional machine-readable code.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    /// How severe the diagnostic is.
    pub severity: Severity,
    /// Human-readable description of the problem.
    pub message: String,
    /// Byte span the diagnostic refers to, if any.
    pub span: Option<Span>,
    /// Optional actionable hint shown below the report.
    pub help: Option<String>,
    /// Optional machine-readable code for structured tooling (LSP quick
    /// fixes and the like). Absent unless explicitly attached.
    pub code: Option<DiagnosticCode>,
}

impl Diagnostic {
    /// Creates an error diagnostic.
    pub fn error(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            severity: Severity::Error,
            message: message.into(),
            span,
            help: None,
            code: None,
        }
    }

    /// Creates a warning diagnostic.
    pub fn warning(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            severity: Severity::Warning,
            message: message.into(),
            span,
            help: None,
            code: None,
        }
    }

    /// Attaches a help hint to this diagnostic.
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    /// Attaches a machine-readable code to this diagnostic.
    pub fn with_code(mut self, code: DiagnosticCode) -> Self {
        self.code = Some(code);
        self
    }

    /// `true` when the severity is [`Severity::Error`].
    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

/// Renders diagnostics into a plain-text (colorless) ariadne report string,
/// sorted by source span. Parse errors and semantic diagnostics can be mixed;
/// they are merged and ordered here.
pub fn render(path: &str, source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut ordered: Vec<&Diagnostic> = diagnostics.iter().collect();
    ordered.sort_by_key(|diagnostic| diagnostic.span.map(|span| (span.start, span.end)));

    let mut out: Vec<u8> = Vec::new();
    for diagnostic in ordered {
        let kind = match diagnostic.severity {
            Severity::Error => ReportKind::Error,
            Severity::Warning => ReportKind::Warning,
        };
        let (start, end) = diagnostic
            .span
            .map(|span| (span.start, span.end))
            .unwrap_or((0, 0));
        let location = (path, start..end.max(start));
        let mut builder = Report::build(kind, location.clone())
            .with_message(&diagnostic.message)
            .with_config(
                Config::default()
                    .with_color(false)
                    .with_index_type(IndexType::Byte),
            );
        builder.add_label(Label::new(location).with_message(&diagnostic.message));
        if let Some(help) = &diagnostic.help {
            builder = builder.with_help(help);
        }
        builder
            .finish()
            .write((path, Source::from(source)), &mut out)
            .expect("writing a diagnostic into a buffer cannot fail");
    }
    String::from_utf8(out).expect("diagnostics are valid UTF-8")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn declared(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|name| name.to_string()).collect()
    }

    // --- did_you_mean ---

    #[test]
    fn case_variant_match_wins() {
        assert_eq!(
            did_you_mean(
                "approvepayrun",
                &declared(&["TerminateWorker", "ApprovePayRun"])
            ),
            Some("ApprovePayRun".to_string())
        );
        assert_eq!(
            did_you_mean("APPROVEPAYRUN", &declared(&["ApprovePayRun"])),
            Some("ApprovePayRun".to_string())
        );
    }

    #[test]
    fn close_typo_suggests_the_nearest_declared_name() {
        assert_eq!(
            did_you_mean(
                "ApprovePayRunn",
                &declared(&["TerminateWorker", "ApprovePayRun"])
            ),
            Some("ApprovePayRun".to_string())
        );
    }

    #[test]
    fn distant_name_suggests_nothing() {
        assert_eq!(
            did_you_mean("GrantReward", &declared(&["TerminateWorker"])),
            None,
            "distance 9 over a 4-edit budget: no plausible typo"
        );
    }

    #[test]
    fn empty_declared_set_suggests_nothing() {
        assert_eq!(did_you_mean("Anything", &declared(&[])), None);
    }

    #[test]
    fn ties_break_alphabetically() {
        assert_eq!(
            did_you_mean("ab", &declared(&["ad", "ac"])),
            Some("ac".to_string()),
            "both one edit away; the alphabetically-first candidate wins"
        );
    }

    #[test]
    fn short_names_get_a_two_edit_budget() {
        assert_eq!(
            did_you_mean("helo", &declared(&["hello"])),
            Some("hello".to_string()),
            "four chars: budget max(2, 1) = 2; one deletion is within it"
        );
        assert_eq!(did_you_mean("wxyz", &declared(&["hello"])), None);
    }

    #[test]
    fn long_names_scale_their_budget() {
        let declared = declared(&["ApprovePayRunStatusTransition"]);
        // 11 substitutions — beyond every budget.
        assert_eq!(did_you_mean("TerminateWorkerActions", &declared), None);
        // 3 edits on a 27-char name: budget max(2, 9) = 9 — within it.
        assert_eq!(
            did_you_mean("ApprovePayRunStatusTransitioX", &declared),
            Some("ApprovePayRunStatusTransition".to_string())
        );
    }

    // --- DiagnosticCode serialization ---

    #[test]
    fn code_names_are_stable_camel_case() {
        let unknown = DiagnosticCode::UnknownCapability {
            capability: "ApprovePayRu".to_string(),
            suggestion: None,
            span: (7..19).into(),
        };
        let carried = DiagnosticCode::GrantedCapabilityUncarried {
            capability: "TerminateWorker".to_string(),
            span: (3..18).into(),
        };
        assert_eq!(unknown.name(), "unknownCapability");
        assert_eq!(carried.name(), "grantedCapabilityUncarried");
    }

    #[test]
    fn unknown_capability_serializes_with_a_suggestion() {
        let code = DiagnosticCode::UnknownCapability {
            capability: "ApprovePayRu".to_string(),
            suggestion: Some("ApprovePayRun".to_string()),
            span: (7..19).into(),
        };
        let json = serde_json::to_value(&code).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "unknownCapability": {
                    "capability": "ApprovePayRu",
                    "suggestion": "ApprovePayRun",
                    "span": [7, 19]
                }
            })
        );
    }

    #[test]
    fn unknown_capability_omits_an_absent_suggestion() {
        let code = DiagnosticCode::UnknownCapability {
            capability: "WhollyUnrelated".to_string(),
            suggestion: None,
            span: (0..15).into(),
        };
        let json = serde_json::to_value(&code).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "unknownCapability": {
                    "capability": "WhollyUnrelated",
                    "span": [0, 15]
                }
            })
        );
    }

    #[test]
    fn granted_capability_uncarried_serializes_camel_case() {
        let code = DiagnosticCode::GrantedCapabilityUncarried {
            capability: "TerminateWorker".to_string(),
            span: (3..18).into(),
        };
        let json = serde_json::to_value(&code).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "grantedCapabilityUncarried": {
                    "capability": "TerminateWorker",
                    "span": [3, 18]
                }
            })
        );
    }

    #[test]
    fn attribute_with_class_type_payload_is_unchanged() {
        let code = DiagnosticCode::AttributeWithClassType {
            feature: "author".to_string(),
            class: "Book".to_string(),
            type_span: (31..35).into(),
        };
        let json = serde_json::to_value(&code).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "attributeWithClassType": {
                    "feature": "author",
                    "class": "Book",
                    "typeSpan": [31, 35]
                }
            })
        );
    }
}
