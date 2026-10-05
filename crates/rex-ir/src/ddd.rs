//! The DDD design artifact: application, modules, application services,
//! per-class design decisions (Sculptor-style), and search definitions,
//! produced by the `.ddd` design DSL and ingested by downstream consumers
//! exactly like [`crate::ifml::IfmlModel`].
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`DDD_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent (wire
//! contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the ActorModel (not the IFML) empty-model
//! discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`,
//!   `returnType`, `optimisticLocking`, ...). Unit-only enums serialize as
//!   bare strings: [`Stereotype`] lowercase (`"entity" | "value" | "dto"`),
//!   [`BuiltinRepositoryOp`] camelCase (`"findById"`, `"findAll"`,
//!   `"save"`, `"delete"`). The one payload-bearing enum,
//!   [`RankingStrategy`], is adjacently tagged instead (`{"type":
//!   "tfIdf"}`, `{"type": "custom", "value": "..."}`).
//! - **Version gate.** [`DddModel::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every optional field is `#[serde(default)]` and
//!   omitted when absent/empty (`skip_serializing_if`), so an empty artifact
//!   serializes as exactly `{"formatVersion":1}` — byte-identical no matter
//!   what is added later.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must follow the same
//!   default + skip discipline.
//! - **Shared IR types.** Signatures reuse the domain IR's [`crate::TypeRef`]
//!   (adjacent tagging: `{"type": "class", "value": {...}}`) and
//!   [`crate::OperationParam`] (`{"name": ..., "type": ...}`); there are no
//!   parallel wire-level signature types.
//!
//! [`Design::class`] names the referenced `.mox` class (unqualified or
//! package-qualified at this layer); resolution against the domain
//! [`crate::Model`] is a driver/consumer concern, never a wire concern.
//! Likewise [`ServiceOperation::capabilities`] loosely references actor
//! capability names (the [`crate::ActorModel`] dimension); it is not
//! validated at the wire layer.
//!
//! A [`RepositoryOperation`] carries **exactly one** of a built-in op
//! ([`RepositoryOperation::builtin`] — the consumer knows its signature) or
//! a declared signature ([`RepositoryOperation::declared`] — a return type
//! and/or parameters). Consistent with the other artifacts this invariant is
//! not validated on deserialize; the constructors make the invalid states
//! hard to author.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::{IrError, Multiplicity, OperationParam, TypeRef};

/// The artifact format version this crate writes and accepts for DDD design
/// artifacts ([`DddModel`]).
///
/// Versioned independently of [`crate::FORMAT_VERSION`] (the domain-model
/// marker), [`crate::ACTOR_MODEL_FORMAT_VERSION`] (the policy marker), and
/// [`crate::ifml::IFML_MODEL_FORMAT_VERSION`] (the interaction marker); bump
/// whenever the DDD wire format changes incompatibly.
pub const DDD_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of a DDD design artifact: one application with its modules of
/// application services, class designs, and search definitions.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
///
/// `Eq` is deliberately not derived: [`Module`]'s search fields carry an
/// `f32` boost, so only [`PartialEq`] equality is available (value equality
/// is unaffected).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DddModel {
    /// Artifact format version. Always [`DDD_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`DddModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// The designed application, when the artifact declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<Application>,
    /// Modules in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modules: Vec<Module>,
}

impl DddModel {
    /// Creates an empty design artifact with [`DDD_MODEL_FORMAT_VERSION`]
    /// stamped.
    pub fn new() -> Self {
        Self {
            format_version: DDD_MODEL_FORMAT_VERSION,
            application: None,
            modules: Vec::new(),
        }
    }

    /// Chainable setter installing the application.
    pub fn application(mut self, application: Application) -> Self {
        self.application = Some(application);
        self
    }

    /// Chainable setter appending a module.
    pub fn module(mut self, module: Module) -> Self {
        self.modules.push(module);
        self
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserializes a design artifact from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`DDD_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != DDD_MODEL_FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: DDD_MODEL_FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for DddModel {
    fn default() -> Self {
        Self::new()
    }
}

/// The designed application.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Application {
    /// Application name.
    pub name: String,
    /// The default domain package name for unqualified references (e.g.
    /// `"nz.example.library"`), when the design declares one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

impl Application {
    /// Creates an application without a base package.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            base: None,
        }
    }
}

/// A module of the application: a cohesive slice of services, designed
/// classes, and search definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Module {
    /// Module name.
    pub name: String,
    /// Application services declared in this module, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<Service>,
    /// Class designs declared in this module, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub designs: Vec<Design>,
    /// Search definitions declared in this module, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub searches: Vec<SearchDef>,
}

impl Module {
    /// Creates an empty module.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            services: Vec::new(),
            designs: Vec::new(),
            searches: Vec::new(),
        }
    }
}

/// An application service: stateless use-case orchestration over
/// repositories and other services.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Service {
    /// Service name.
    pub name: String,
    /// Human-readable description of the service's responsibility.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Service operations, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<ServiceOperation>,
    /// Declared `inject` dependencies: names of repositories and other
    /// services this service delegates to.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<String>,
}

impl Service {
    /// Creates a service with no description, operations, or dependencies.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            operations: Vec::new(),
            dependencies: Vec::new(),
        }
    }
}

/// An operation of a [`Service`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceOperation {
    /// Operation name, unique within its service.
    pub name: String,
    /// The resolved return type. `None` for delegation operations whose
    /// signature is copied from the delegated target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<TypeRef>,
    /// The declared return cardinality; `None` is single-valued. Paired
    /// with [`ServiceOperation::return_type`] — never set without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_multiplicity: Option<Multiplicity>,
    /// Parameters in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<OperationParam>,
    /// Delegation target, when this operation forwards to a dependency's
    /// operation instead of declaring its own body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delegation: Option<Delegation>,
    /// Actor capability names guarding this operation (the
    /// [`crate::ActorModel`] dimension); loosely coupled, not validated at
    /// the wire layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl ServiceOperation {
    /// Creates an operation with no return type, parameters, delegation, or
    /// capabilities.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            return_type: None,
            return_multiplicity: None,
            params: Vec::new(),
            delegation: None,
            capabilities: Vec::new(),
        }
    }

    /// Chainable setter for the declared return cardinality.
    pub fn with_return_multiplicity(mut self, multiplicity: Option<Multiplicity>) -> Self {
        self.return_multiplicity = multiplicity;
        self
    }
}

/// Delegation of a [`ServiceOperation`] to a dependency's operation: the
/// service copies the target's signature rather than declaring its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Delegation {
    /// Name of the injected dependency (repository or service) receiving
    /// the call.
    pub target: String,
    /// Name of the operation invoked on the target.
    pub operation: String,
}

/// The DDD stereotype of a designed class.
///
/// Serializes as a bare lowercase string: `"entity"`, `"value"`, `"dto"`.
/// Abstractness is the separate [`Design::is_abstract`] flag, not a
/// stereotype.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stereotype {
    /// An aggregate-root/characteristic class with identity.
    Entity,
    /// An immutable value object.
    Value,
    /// A data transfer object.
    Dto,
}

/// The design flags of a [`Design`], all defaulting to `false`. A
/// fully-default flags value is omitted entirely on serialize.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignFlags {
    /// Generate scaffolding for the class.
    #[serde(default, skip_serializing_if = "is_false")]
    pub scaffold: bool,
    /// Record an audit trail for the class's mutations.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auditable: bool,
    /// Optimistic locking on the class's persistent state.
    #[serde(default, skip_serializing_if = "is_false")]
    pub optimistic_locking: bool,
    /// The class is never persisted.
    #[serde(default, skip_serializing_if = "is_false")]
    pub non_persistent: bool,
    /// Cache instances of the class.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cache: bool,
}

impl DesignFlags {
    /// `true` when no flag is set — the value that serializes as an omitted
    /// `flags` field.
    pub fn is_empty(&self) -> bool {
        !(self.scaffold
            || self.auditable
            || self.optimistic_locking
            || self.non_persistent
            || self.cache)
    }
}

/// The design decision for one referenced `.mox` class: its stereotype,
/// flags, and optionally its repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Design {
    /// The referenced `.mox` class name (unqualified or package-qualified
    /// at this layer; resolution is a driver/consumer concern).
    pub class: String,
    /// The DDD stereotype of the class.
    pub stereotype: Stereotype,
    /// Whether the class is abstract.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_abstract: bool,
    /// Design flags; omitted entirely when fully default.
    #[serde(default, skip_serializing_if = "DesignFlags::is_empty")]
    pub flags: DesignFlags,
    /// The class's repository, when designed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<Repository>,
}

impl Design {
    /// Creates a concrete design with default flags and no repository.
    pub fn new(class: impl Into<String>, stereotype: Stereotype) -> Self {
        Self {
            class: class.into(),
            stereotype,
            is_abstract: false,
            flags: DesignFlags::default(),
            repository: None,
        }
    }
}

/// The repository designed for a [`Design`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Repository {
    /// Repository name.
    pub name: String,
    /// Repository operations, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<RepositoryOperation>,
}

impl Repository {
    /// Creates a repository with no operations.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            operations: Vec::new(),
        }
    }
}

/// An operation of a [`Repository`]: either a built-in (whose signature the
/// consumer knows) or a declared operation with an explicit signature.
///
/// **Invariant:** exactly one of [`RepositoryOperation::builtin`] or a
/// declared signature ([`RepositoryOperation::return_type`] and/or
/// [`RepositoryOperation::params`]) is present. Not validated on
/// deserialize (consistent with the other artifacts); use the
/// [`RepositoryOperation::builtin`] / [`RepositoryOperation::declared`]
/// constructors.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RepositoryOperation {
    /// Operation name, unique within its repository.
    pub name: String,
    /// The built-in op, when this is one; a built-in has no signature of
    /// its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<BuiltinRepositoryOp>,
    /// The declared return type, when this is a declared operation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_type: Option<TypeRef>,
    /// The declared return cardinality; `None` is single-valued. Paired
    /// with [`RepositoryOperation::return_type`] — never set without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_multiplicity: Option<Multiplicity>,
    /// Declared parameters in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<OperationParam>,
}

impl RepositoryOperation {
    /// Creates a built-in repository operation (no signature of its own).
    pub fn builtin(name: impl Into<String>, builtin: BuiltinRepositoryOp) -> Self {
        Self {
            name: name.into(),
            builtin: Some(builtin),
            return_type: None,
            return_multiplicity: None,
            params: Vec::new(),
        }
    }

    /// Creates a declared repository operation with its explicit signature
    /// (at least one of a return type or a parameter).
    pub fn declared(
        name: impl Into<String>,
        return_type: Option<TypeRef>,
        params: Vec<OperationParam>,
    ) -> Self {
        Self {
            name: name.into(),
            builtin: None,
            return_type,
            return_multiplicity: None,
            params,
        }
    }

    /// Chainable setter for the declared return cardinality.
    pub fn with_return_multiplicity(mut self, multiplicity: Option<Multiplicity>) -> Self {
        self.return_multiplicity = multiplicity;
        self
    }
}

/// The built-in repository operations. Serialized as bare camelCase
/// strings: `"findById"`, `"findAll"`, `"save"`, `"delete"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinRepositoryOp {
    /// Fetch one aggregate by identity.
    FindById,
    /// Fetch all aggregates.
    FindAll,
    /// Insert or update an aggregate.
    Save,
    /// Delete an aggregate.
    Delete,
}

/// A search definition designed over one entity: the indexed full-text
/// fields, the structured filter and sort keys, the computed document
/// projection, and the ranking/analyzer/pagination knobs.
///
/// Like [`Design::class`], [`SearchDef::entity`] loosely references a
/// `.mox` class (unqualified or package-qualified at this layer);
/// resolution against the domain [`crate::Model`] is a driver/consumer
/// concern, never a wire concern.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchDef {
    /// Search name, unique within its module (not validated at the wire
    /// layer, consistent with the other artifacts).
    pub name: String,
    /// Human-readable description of what the search finds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The referenced entity class whose instances are searched.
    pub entity: String,
    /// Full-text fields, in declaration order: the indexed prose a query's
    /// terms match against, ranked by relevance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub text: Vec<SearchField>,
    /// Structured filter keys, in declaration order: exact-match facets a
    /// query may constrain (never free-text searched).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filters: Vec<SearchFilter>,
    /// Structured sort keys, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sort: Vec<SearchSort>,
    /// Computed projection entries of the search document, in declaration
    /// order: named values derived from the matched instance.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub document: Vec<DocumentField>,
    /// The relevance ranking strategy; `None` lets the consumer choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ranking: Option<RankingStrategy>,
    /// The default analyzer applied to the indexed text; per-field
    /// overrides live on [`SearchField::analyzer`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyzer: Option<String>,
    /// Pagination knobs; `None` lets the consumer choose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pagination: Option<Pagination>,
    /// Actor capability names guarding the search (the
    /// [`crate::ActorModel`] dimension); loosely coupled, not validated at
    /// the wire layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl SearchDef {
    /// Creates a search over `entity` with no text fields, filters, sort
    /// keys, document projection, ranking, analyzer, pagination, or
    /// capabilities.
    pub fn new(name: impl Into<String>, entity: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            entity: entity.into(),
            text: Vec::new(),
            filters: Vec::new(),
            sort: Vec::new(),
            document: Vec::new(),
            ranking: None,
            analyzer: None,
            pagination: None,
            capabilities: Vec::new(),
        }
    }

    /// Chainable setter for the description.
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Chainable setter for the full-text fields.
    pub fn with_text(mut self, text: Vec<SearchField>) -> Self {
        self.text = text;
        self
    }

    /// Chainable setter for the filter keys.
    pub fn with_filters(mut self, filters: Vec<SearchFilter>) -> Self {
        self.filters = filters;
        self
    }

    /// Chainable setter for the sort keys.
    pub fn with_sort(mut self, sort: Vec<SearchSort>) -> Self {
        self.sort = sort;
        self
    }

    /// Chainable setter for the document projection.
    pub fn with_document(mut self, document: Vec<DocumentField>) -> Self {
        self.document = document;
        self
    }

    /// Chainable setter for the ranking strategy.
    pub fn with_ranking(mut self, ranking: RankingStrategy) -> Self {
        self.ranking = Some(ranking);
        self
    }

    /// Chainable setter for the default analyzer.
    pub fn with_analyzer(mut self, analyzer: impl Into<String>) -> Self {
        self.analyzer = Some(analyzer.into());
        self
    }

    /// Chainable setter for the pagination knobs.
    pub fn with_pagination(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }

    /// Chainable setter for the guarding capabilities.
    pub fn with_capabilities(mut self, capabilities: Vec<String>) -> Self {
        self.capabilities = capabilities;
        self
    }
}

/// A full-text field of a [`SearchDef`]: the indexed prose of one entity
/// property, optionally weighted and analyzed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchField {
    /// The entity property whose text is indexed.
    pub property: String,
    /// Relevance boost multiplier for matches in this field; `None` uses
    /// the consumer's default weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost: Option<f32>,
    /// Analyzer override for this field; `None` uses the search's default
    /// ([`SearchDef::analyzer`], else the consumer's default).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub analyzer: Option<String>,
}

impl SearchField {
    /// Creates a field with no boost and no analyzer override.
    pub fn new(property: impl Into<String>) -> Self {
        Self {
            property: property.into(),
            boost: None,
            analyzer: None,
        }
    }

    /// Chainable setter for the relevance boost.
    pub fn with_boost(mut self, boost: f32) -> Self {
        self.boost = Some(boost);
        self
    }

    /// Chainable setter for the analyzer override.
    pub fn with_analyzer(mut self, analyzer: impl Into<String>) -> Self {
        self.analyzer = Some(analyzer.into());
        self
    }
}

/// A structured filter key of a [`SearchDef`]: an exact-match facet a query
/// may constrain. A struct, not a bare string, so the wire format can grow
/// per-filter knobs without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilter {
    /// The entity property the filter constrains.
    pub property: String,
}

impl SearchFilter {
    /// Creates a filter key.
    pub fn new(property: impl Into<String>) -> Self {
        Self {
            property: property.into(),
        }
    }
}

/// A structured sort key of a [`SearchDef`]. A struct, not a bare string,
/// so the wire format can grow per-key knobs without a breaking change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchSort {
    /// The entity property the sort orders by.
    pub property: String,
}

impl SearchSort {
    /// Creates a sort key.
    pub fn new(property: impl Into<String>) -> Self {
        Self {
            property: property.into(),
        }
    }
}

/// A computed projection entry of a [`SearchDef`]'s document: one named
/// value of the search result, derived from the matched instance.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DocumentField {
    /// The entry's name in the search document.
    pub name: String,
    /// The verbatim expression source computing the value (the same
    /// convention as a domain feature's `expr` body); typed against the
    /// entity by the consumer, never at the wire layer.
    pub expr: String,
}

impl DocumentField {
    /// Creates a document field from its name and expression source.
    pub fn new(name: impl Into<String>, expr: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            expr: expr.into(),
        }
    }
}

/// The relevance ranking strategy of a [`SearchDef`].
///
/// Adjacently tagged (the crate's `{"type": ..., "value": ...}` rule):
/// unit variants serialize without content — exactly `{"type": "bm25"}`,
/// `{"type": "tfIdf"}`, `{"type": "exact"}` — and the newtype variant as
/// `{"type": "custom", "value": "..."}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum RankingStrategy {
    /// BM25 relevance scoring.
    Bm25,
    /// TF-IDF relevance scoring.
    TfIdf,
    /// Exact-match filtering only; no relevance ranking.
    Exact,
    /// A consumer-provided strategy, referenced by name — loosely coupled,
    /// like every cross-artifact reference in this module.
    Custom(String),
}

/// The pagination knobs of a [`SearchDef`]. A fully-default value
/// serializes as `{}` (the field itself is optional on the search).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pagination {
    /// The default page size.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    /// The maximum page size a query may request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_limit: Option<u32>,
    /// The search pages by cursor (keyset) rather than by offset.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cursor: bool,
}

fn is_false(flag: &bool) -> bool {
    !*flag
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PrimitiveType;

    fn class_ref(name: &str) -> TypeRef {
        TypeRef::Class {
            package: "nz.example.library".to_string(),
            name: name.to_string(),
        }
    }

    fn param(name: &str, type_: TypeRef) -> OperationParam {
        OperationParam {
            name: name.to_string(),
            type_,
            multiplicity: None,
        }
    }

    /// A full-featured model: every stereotype, every flag, builtin and
    /// declared repository operations, delegation, capabilities,
    /// dependencies, and a qualified base package.
    fn sample_model() -> DddModel {
        DddModel::new()
            .application(Application {
                name: "Library".to_string(),
                base: Some("nz.example.library".to_string()),
            })
            .module(Module {
                name: "catalogue".to_string(),
                services: vec![Service {
                    name: "LoanService".to_string(),
                    description: Some("Manages loans".to_string()),
                    operations: vec![
                        ServiceOperation {
                            name: "borrow".to_string(),
                            return_type: Some(TypeRef::Primitive(PrimitiveType::Boolean)),
                            return_multiplicity: None,
                            params: vec![param("book", class_ref("Book"))],
                            delegation: None,
                            capabilities: vec!["BorrowBooks".to_string()],
                        },
                        ServiceOperation {
                            name: "renew".to_string(),
                            return_type: None,
                            return_multiplicity: None,
                            params: Vec::new(),
                            delegation: Some(Delegation {
                                target: "LoanRepository".to_string(),
                                operation: "save".to_string(),
                            }),
                            capabilities: Vec::new(),
                        },
                    ],
                    dependencies: vec![
                        "LoanRepository".to_string(),
                        "NotificationService".to_string(),
                    ],
                }],
                designs: vec![
                    Design {
                        class: "nz.example.library::Book".to_string(),
                        stereotype: Stereotype::Entity,
                        is_abstract: false,
                        flags: DesignFlags {
                            scaffold: true,
                            cache: true,
                            ..Default::default()
                        },
                        repository: Some(Repository {
                            name: "BookRepository".to_string(),
                            operations: vec![
                                RepositoryOperation::builtin(
                                    "findById",
                                    BuiltinRepositoryOp::FindById,
                                ),
                                RepositoryOperation::builtin(
                                    "findAll",
                                    BuiltinRepositoryOp::FindAll,
                                ),
                                RepositoryOperation::builtin("save", BuiltinRepositoryOp::Save),
                                RepositoryOperation::builtin("delete", BuiltinRepositoryOp::Delete),
                                RepositoryOperation::declared(
                                    "findByTitle",
                                    Some(TypeRef::Primitive(PrimitiveType::String)),
                                    vec![param("title", TypeRef::Primitive(PrimitiveType::String))],
                                ),
                            ],
                        }),
                    },
                    Design {
                        class: "Money".to_string(),
                        stereotype: Stereotype::Value,
                        is_abstract: false,
                        flags: DesignFlags::default(),
                        repository: None,
                    },
                    Design {
                        class: "LoanSummary".to_string(),
                        stereotype: Stereotype::Dto,
                        is_abstract: true,
                        flags: DesignFlags {
                            auditable: true,
                            optimistic_locking: true,
                            non_persistent: true,
                            ..Default::default()
                        },
                        repository: None,
                    },
                ],
                searches: Vec::new(),
            })
    }

    #[test]
    fn new_and_default_stamp_format_version() {
        assert_eq!(sample_model().format_version, DDD_MODEL_FORMAT_VERSION);
        assert_eq!(DddModel::default().format_version, DDD_MODEL_FORMAT_VERSION);
        assert_eq!(DDD_MODEL_FORMAT_VERSION, 1);
    }

    #[test]
    fn empty_model_serializes_as_exactly_format_version() {
        assert_eq!(
            DddModel::new().to_json().expect("serialize"),
            r#"{"formatVersion":1}"#
        );
        assert_eq!(
            DddModel::default().to_json().expect("serialize"),
            r#"{"formatVersion":1}"#
        );
        let parsed = DddModel::from_json(r#"{"formatVersion":1}"#).expect("deserialize");
        assert_eq!(parsed, DddModel::new());
    }

    #[test]
    fn full_model_round_trips_through_json() {
        let model = sample_model();
        let json = model.to_json().expect("serialize");
        let parsed = DddModel::from_json(&json).expect("deserialize");
        assert_eq!(parsed, model);
        assert_eq!(parsed.to_json().expect("re-serialize"), json);
    }

    #[test]
    fn full_model_round_trips_through_json_pretty() {
        let model = sample_model();
        let json = model.to_json_pretty().expect("serialize");
        let parsed = DddModel::from_json(&json).expect("deserialize");
        assert_eq!(parsed, model);
        assert_eq!(parsed.to_json_pretty().expect("re-serialize"), json);
    }

    #[test]
    fn wire_format_is_camel_case_with_no_snake_case_keys() {
        let json = sample_model().to_json().expect("serialize");
        assert!(json.contains("\"formatVersion\":1"), "{json}");
        assert!(json.contains("\"returnType\""), "{json}");
        assert!(json.contains("\"isAbstract\":true"), "{json}");
        assert!(json.contains("\"optimisticLocking\":true"), "{json}");
        assert!(json.contains("\"nonPersistent\":true"), "{json}");
        assert!(json.contains("\"base\":\"nz.example.library\""), "{json}");
        assert!(
            json.contains(
                r#"{"type":"class","value":{"package":"nz.example.library","name":"Book"}}"#
            ),
            "signatures reuse the IR TypeRef adjacent tagging: {json}"
        );
        assert!(!json.contains('_'), "no snake_case keys: {json}");
    }

    #[test]
    fn builtin_ops_serialize_as_camel_case_strings() {
        assert_eq!(
            serde_json::to_string(&BuiltinRepositoryOp::FindById).expect("serialize"),
            r#""findById""#
        );
        assert_eq!(
            serde_json::to_string(&BuiltinRepositoryOp::FindAll).expect("serialize"),
            r#""findAll""#
        );
        assert_eq!(
            serde_json::to_string(&BuiltinRepositoryOp::Save).expect("serialize"),
            r#""save""#
        );
        assert_eq!(
            serde_json::to_string(&BuiltinRepositoryOp::Delete).expect("serialize"),
            r#""delete""#
        );
        let op = RepositoryOperation::builtin("findById", BuiltinRepositoryOp::FindById);
        assert_eq!(
            serde_json::to_string(&op).expect("serialize"),
            r#"{"name":"findById","builtin":"findById"}"#,
            "a builtin op has no signature of its own"
        );
    }

    #[test]
    fn stereotypes_serialize_as_lowercase_strings() {
        assert_eq!(
            serde_json::to_string(&Stereotype::Entity).expect("serialize"),
            r#""entity""#
        );
        assert_eq!(
            serde_json::to_string(&Stereotype::Value).expect("serialize"),
            r#""value""#
        );
        assert_eq!(
            serde_json::to_string(&Stereotype::Dto).expect("serialize"),
            r#""dto""#
        );
    }

    #[test]
    fn default_flags_and_abstract_false_are_omitted() {
        let design = Design {
            class: "Money".to_string(),
            stereotype: Stereotype::Value,
            is_abstract: false,
            flags: DesignFlags::default(),
            repository: None,
        };
        assert_eq!(
            serde_json::to_string(&design).expect("serialize"),
            r#"{"class":"Money","stereotype":"value"}"#
        );
    }

    #[test]
    fn all_flags_serialize_with_camel_case_keys() {
        let design = Design {
            class: "Loan".to_string(),
            stereotype: Stereotype::Entity,
            is_abstract: true,
            flags: DesignFlags {
                scaffold: true,
                auditable: true,
                optimistic_locking: true,
                non_persistent: true,
                cache: true,
            },
            repository: None,
        };
        assert_eq!(
            serde_json::to_string(&design).expect("serialize"),
            r#"{"class":"Loan","stereotype":"entity","isAbstract":true,"flags":{"scaffold":true,"auditable":true,"optimisticLocking":true,"nonPersistent":true,"cache":true}}"#
        );
        assert!(DesignFlags::default().is_empty());
        assert!(!design.flags.is_empty());
    }

    #[test]
    fn from_json_rejects_wrong_format_version() {
        let json = r#"{"formatVersion": 2, "modules": []}"#;
        match DddModel::from_json(json) {
            Err(IrError::UnsupportedFormatVersion { found, expected }) => {
                assert_eq!(found, 2);
                assert_eq!(expected, DDD_MODEL_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
        }
    }

    #[test]
    fn from_json_rejects_zero_and_missing_format_version() {
        for json in [r#"{"formatVersion":0}"#, r#"{"modules":[]}"#] {
            match DddModel::from_json(json) {
                Err(IrError::UnsupportedFormatVersion { found, expected }) => {
                    assert_eq!(found, 0, "an absent version reports as 0");
                    assert_eq!(expected, DDD_MODEL_FORMAT_VERSION);
                }
                other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
            }
        }
    }

    #[test]
    fn from_json_tolerates_unknown_fields() {
        let json = r#"{
            "formatVersion": 1,
            "nobodyKnowsMe": {"nested": [1, 2, 3]},
            "application": {"name": "Library", "base": "nz.example.library"}
        }"#;
        let model = DddModel::from_json(json).expect("unknown fields are ignored");
        assert_eq!(
            model
                .application
                .map(|a| a.base.unwrap_or_default())
                .as_deref(),
            Some("nz.example.library")
        );
    }

    // -----------------------------------------------------------------------
    // Search definitions (the `searches` feature of the artifact)
    // -----------------------------------------------------------------------

    /// A full-featured search definition: every collection populated, a
    /// per-field boost and analyzer, `Custom` ranking, pagination, and
    /// capabilities.
    fn sample_search() -> SearchDef {
        SearchDef::new("BookSearch", "nz.example.library::Book")
            .with_description("Full-text catalogue search")
            .with_text(vec![
                SearchField::new("title")
                    .with_boost(2.0)
                    .with_analyzer("standard"),
                SearchField::new("synopsis"),
            ])
            .with_filters(vec![SearchFilter::new("category")])
            .with_sort(vec![SearchSort::new("title")])
            .with_document(vec![DocumentField::new(
                "label",
                r#"title + " - " + synopsis"#,
            )])
            .with_ranking(RankingStrategy::Custom("recency".to_string()))
            .with_analyzer("english")
            .with_pagination(Pagination {
                limit: Some(20),
                max_limit: Some(100),
                cursor: true,
            })
            .with_capabilities(vec!["SearchBooks".to_string()])
    }

    /// The sample model with the search definition installed.
    fn search_model() -> DddModel {
        let mut module = Module::new("catalogue");
        module.searches.push(sample_search());
        DddModel::new().module(module)
    }

    #[test]
    fn full_search_round_trips_through_json() {
        let model = search_model();
        let json = model.to_json().expect("serialize");
        let parsed = DddModel::from_json(&json).expect("deserialize");
        assert_eq!(parsed, model);
        assert_eq!(parsed.to_json().expect("re-serialize"), json);
    }

    #[test]
    fn search_wire_format_is_camel_case_with_adjacent_ranking_tags() {
        let json = search_model().to_json().expect("serialize");
        assert!(json.contains("\"searches\":[{"), "{json}");
        assert!(json.contains("\"maxLimit\":100"), "{json}");
        assert!(json.contains("\"boost\":2.0"), "{json}");
        assert!(
            json.contains(r#"{"type":"custom","value":"recency"}"#),
            "ranking is adjacently tagged: {json}"
        );
        assert!(
            json.contains(r#"title + \" - \" + synopsis"#),
            "the expr is verbatim source: {json}"
        );
        assert!(!json.contains('_'), "no snake_case keys: {json}");
    }

    #[test]
    fn ranking_strategies_serialize_as_adjacent_tags() {
        for (strategy, json) in [
            (RankingStrategy::Bm25, r#"{"type":"bm25"}"#),
            (RankingStrategy::TfIdf, r#"{"type":"tfIdf"}"#),
            (RankingStrategy::Exact, r#"{"type":"exact"}"#),
        ] {
            assert_eq!(serde_json::to_string(&strategy).expect("serialize"), json);
            assert_eq!(
                serde_json::from_str::<RankingStrategy>(json).expect("deserialize"),
                strategy
            );
        }
        assert_eq!(
            serde_json::to_string(&RankingStrategy::Custom("x".to_string())).expect("serialize"),
            r#"{"type":"custom","value":"x"}"#
        );
    }

    #[test]
    fn a_search_less_module_omits_the_searches_key() {
        let module = Module {
            name: "catalogue".to_string(),
            services: vec![Service::new("LoanService")],
            designs: vec![Design::new("Money", Stereotype::Value)],
            searches: Vec::new(),
        };
        assert_eq!(
            serde_json::to_string(&module).expect("serialize"),
            r#"{"name":"catalogue","services":[{"name":"LoanService"}],"designs":[{"class":"Money","stereotype":"value"}]}"#
        );
    }
}
