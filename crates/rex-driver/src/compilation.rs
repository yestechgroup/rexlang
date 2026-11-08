//! The compilation result types the driver's entry points return.

use crate::Diagnostic;

/// The result of compiling a [`SourceFile`](crate::SourceFile).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The lowered Core IR, or `None` when any error-severity diagnostic was
    /// produced. Warnings do not block lowering.
    pub model: Option<rex_ir::Model>,
    /// Parse and semantic diagnostics, in compilation order.
    pub diagnostics: Vec<Diagnostic>,
    /// Diagnostics from the compilation's `import sigil` declarations (sigil
    /// parse/resolution/lowering problems), each tagged with the rosetta
    /// file's key path — these concern rosetta sources, not the `.mox` file.
    /// Errors here block the artifact like any other error.
    pub sigil_diagnostics: Vec<(String, Diagnostic)>,
}
/// The result of compiling a [`SourceFile`](crate::SourceFile) as an `.actor` file against its
/// imported domain models.
///
/// Diagnostics come from every file involved, so each is tagged with its
/// path; group or render them per file (see
/// [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorCompilation {
    /// The standalone actor-policy artifact, or `None` when any
    /// error-severity diagnostic was produced in any file. Warnings do not
    /// block lowering.
    pub model: Option<rex_ir::ActorModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Cedar class lookup consumes this instead of recompiling the domains
    /// standalone (which cannot resolve cross-package references).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the actor file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}
/// The result of compiling a `.ddd` design file against its imported domain
/// models (and, for [`compile_ddd_str_with_actors`](crate::compile_ddd_str_with_actors), an actor-policy
/// artifact). Mirrors [`ActorCompilation`].
///
/// Diagnostics come from every file involved, so each is tagged with its
/// path; group or render them per file (see [`render`](crate::render)).
///
/// Like every artifact type, [`rex_ir::ddd::DddModel`] derives `Eq` (the
/// rex-ir wire-contract equality policy, rule 14) — and so does this
/// compilation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DddCompilation {
    /// The design artifact, or `None` when any error-severity diagnostic
    /// was produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::ddd::DddModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the design file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// The result of compiling an `.evt` event-contract file against its
/// imported domain models. Mirrors [`ActorCompilation`].
///
/// Diagnostics come from every file involved, so each is tagged with its
/// path; group or render them per file (see [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCompilation {
    /// The event-contract artifact, or `None` when any error-severity
    /// diagnostic was produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::events::EventModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Field-type resolution consumes the union (the same model
    /// [`ActorCompilation`] carries for Cedar class lookup).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the `.evt` file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// The result of compiling a `.uc` use case file against its imported
/// domain models and the pre-compiled design, actor, and event artifacts its
/// imports resolve to. Mirrors [`EventCompilation`], with the artifact
/// buckets the `.deploy` pattern carries.
///
/// Diagnostics come from every file involved (the `.uc` file, its imported
/// domains), so each is tagged with its path; group or render them per file
/// (see [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCaseCompilation {
    /// The use case artifact, or `None` when any error-severity diagnostic
    /// was produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::usecases::UseCaseModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Participant and payload type resolution consumes the union (the same
    /// model [`EventCompilation`] carries for field types).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the `.uc` file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// The result of compiling a `.cmmn` case file against its imported domain
/// models and the pre-compiled design, actor, and event artifacts its
/// imports resolve to. Mirrors [`UseCaseCompilation`], with the artifact
/// buckets the `.deploy` pattern carries.
///
/// Diagnostics come from every file involved (the `.cmmn` file, its
/// imported domains), so each is tagged with its path; group or render them
/// per file (see [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmmnCompilation {
    /// The case artifact, or `None` when any error-severity diagnostic was
    /// produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::cmmn::CmmnModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Case-file item type resolution consumes the union (the same model
    /// [`EventCompilation`] carries for field types).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the `.cmmn` file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// The result of compiling several `.mox` files (one package per file) into
/// one multi-package model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MultiCompilation {
    /// The lowered Core IR with one package per input file (input order), or
    /// `None` when any error-severity diagnostic was produced in any file.
    /// Warnings do not block lowering.
    pub model: Option<rex_ir::Model>,
    /// Diagnostics from every file, each tagged with its file's path, in
    /// compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
    /// Diagnostics from the compilation's `import sigil` declarations, each
    /// tagged with the rosetta file's key path. Errors here block the
    /// artifact like any other error.
    pub sigil_diagnostics: Vec<(String, Diagnostic)>,
}
/// A one-shot compilation: the [`Compiled`] result of [`compile`](crate::compile) on a fresh
/// [`Database`](crate::Database), bundled for convenience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compilation {
    /// The lowered Core IR, or `None` when errors occurred.
    pub model: Option<rex_ir::Model>,
    /// Parse and semantic diagnostics, in compilation order.
    pub diagnostics: Vec<Diagnostic>,
    /// Diagnostics from the compilation's `import sigil` declarations, each
    /// tagged with the rosetta file's key path. Errors here block the
    /// artifact like any other error.
    pub sigil_diagnostics: Vec<(String, Diagnostic)>,
}
/// The result of compiling a `.dmn` decision model against its imported
/// domain models and the external decision-table contents its `logic table`
/// paths name. Mirrors [`EventCompilation`].
///
/// Diagnostics come from every file involved, so each is tagged with its
/// path; group or render them per file (see [`render`](crate::render)).
///
/// Like every artifact type, [`rex_ir::dmn::DmnModel`] derives `Eq` (the
/// rex-ir wire-contract equality policy, rule 14) — and so does this
/// compilation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DmnCompilation {
    /// The decision-model artifact, or `None` when any error-severity
    /// diagnostic was produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::dmn::DmnModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Input/output/parameter type resolution consumes the union.
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the `.dmn` file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}
/// The result of compiling a `.deploy` deployment file against its imported
/// design and flow artifacts. Mirrors [`EventCompilation`] (without the
/// domain lowering — a deployment consumes pre-compiled artifacts).
///
/// Diagnostics are all tagged with the deploy file's path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployCompilation {
    /// The deployment artifact, or `None` when any error-severity
    /// diagnostic was produced. Warnings do not block lowering.
    pub model: Option<rex_ir::deploy::DeployModel>,
    /// Diagnostics from the `.deploy` file, each tagged with its path, in
    /// compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}
