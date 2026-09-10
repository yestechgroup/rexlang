//! Diagnostics (errors and warnings) and their rendering with [ariadne].
//!
//! [ariadne]: https://crates.io/crates/ariadne

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
}

impl DiagnosticCode {
    /// A stable, camelCase name for the code, suitable as the LSP
    /// diagnostic `code` string.
    pub fn name(&self) -> &'static str {
        match self {
            DiagnosticCode::AttributeWithClassType { .. } => "attributeWithClassType",
        }
    }
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
            .with_config(Config::default().with_color(false).with_index_type(IndexType::Byte));
        builder.add_label(Label::new(location).with_message(""));
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
