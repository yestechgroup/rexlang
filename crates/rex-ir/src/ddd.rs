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
//!   `"findByExample"`, `"findByKeys"`, `"save"`, `"delete"`). The one
//!   payload-bearing enum,
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
//!   parallel wire-level signature types. A [`StateMachine`]'s enum is the
//!   same [`crate::TypeRef::Enum`].
//! - **State machines.** A [`Design`] may carry a [`StateMachine`] (`machine`
//!   block). The state set is derived — `initial` + `terminals` + transition
//!   endpoints — never serialized as a separate list; terminalness is
//!   membership in `terminals`. A [`Transition`]'s `trigger` is the `via`
//!   workflow action (`None` = bare arrow, action name derived downstream),
//!   and its `guard`/an operation's `condition` are the `when`-condition
//!   rule: verbatim source text between the parentheses, typed by the
//!   driver.
//! - **Child-scoped operations.** A [`Design`] may carry child operations
//!   ([`Design::child_operations`]) declared by module-level `extend
//!   <Class>` blocks (issue rexlang#179): operations over the aggregate's
//!   contained children, each naming the child feature, its resolved
//!   class, and the validated action through
//!   [`ServiceOperation::child_scope`] ([`ChildRef`]).
//! - **Executable bodies.** An operation may carry a checked statement body
//!   (issue rexlang#178): the [`ServiceOperation::bodies`] /
//!   [`RepositoryOperation::bodies`] map's `"stmt"` target holds the
//!   canonical JSON of the `rex_expr::stmt::StmtBody` (opaque here — the
//!   crate does not depend on rex-expr, the [`crate::Operation::bodies`]
//!   precedent), and a service body written `name(...) => Repository { ... }`
//!   also records that repository as [`ServiceOperation::body_receiver`]
//!   (the binding `self` names in the body).
//! - **Equality policy.** Like every artifact root, [`DddModel`] derives
//!   [`Eq`] (wire contract rule 14): the one floating-point field in any
//!   artifact — [`SearchField::boost`] — is wrapped in the ordered
//!   [`Boost`] newtype, transparent on the wire and total in equality,
//!   hashing, and ordering.
//!
//! [`Design::class`] names the referenced `.mox` class (unqualified or
//! package-qualified at this layer); resolution against the domain
//! [`crate::Model`] is a driver/consumer concern, never a wire concern.
//! Likewise [`ServiceOperation::capabilities`] loosely references actor
//! capability names (the [`crate::ActorModel`] dimension),
//! [`ServiceOperation::emits`] loosely references event names (the
//! [`crate::events::EventModel`] dimension), and
//! [`ServiceOperation::invokes`] / [`Service::decision_dependencies`]
//! loosely reference decision-service names (the [`crate::dmn::DmnModel`]
//! dimension); none are validated at the wire layer — the `.ddd`
//! decisions entry point joins the design against the provided decision
//! models at compile time. A reference may carry a **version pin**
//! (rexlang#154, [`crate::dmn::VersionPin`]): pinned references lower
//! into [`ServiceOperation::decision_pins`] /
//! [`Service::decision_dependency_pins`], each a [`DecisionPin`] pairing
//! the name with its compatibility contract.
//!
//! A [`RepositoryOperation`] carries **exactly one** of a built-in op
//! ([`RepositoryOperation::builtin`] — the consumer knows its signature) or
//! a declared signature ([`RepositoryOperation::declared`] — a return type
//! and/or parameters). Consistent with the other artifacts this invariant is
//! not validated on deserialize; the constructors make the invalid states
//! hard to author.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{IrError, Multiplicity, OperationParam, TypeRef};

/// The artifact format version this crate writes and accepts for DDD design
/// artifacts ([`DddModel`]).
///
/// Bump whenever the DDD wire format changes incompatibly; this marker is
/// versioned independently of every other artifact marker — see the
/// [version-marker inventory](crate#version-markers).
pub const DDD_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of a DDD design artifact: one application with its modules of
/// application services, class designs, and search definitions.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
///
/// Every artifact type derives [`Eq`] (wire contract rule 14): this one's
/// only floating-point field — [`SearchField::boost`] — is wrapped in the
/// ordered [`Boost`] newtype, so value equality, hashing, and ordering are
/// total and deterministic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
        crate::wire::VersionedArtifact::to_json_pretty(self)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        crate::wire::VersionedArtifact::to_json(self)
    }

    /// Deserializes a design artifact from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`DDD_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        crate::wire::VersionedArtifact::from_json(json)
    }
}

impl crate::wire::VersionedArtifact for DddModel {
    const FORMAT_VERSION: u32 = DDD_MODEL_FORMAT_VERSION;
}

crate::wire::default_format_version!(DddModel);

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Injected decision-service references (`inject decision <name>;`;
    /// the [`crate::dmn::DmnModel`] dimension); loosely coupled, not
    /// validated at the wire layer — the `.ddd` decisions entry point
    /// joins them against the provided decision models.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_dependencies: Vec<String>,
    /// The **version pins** of [`Self::decision_dependencies`] entries
    /// that carry one (rexlang#154): each names the service (the same
    /// last-segment spelling `decision_dependencies` lowers) and the
    /// [`crate::dmn::VersionPin`] the `inject decision` line wrote.
    /// Pin-less lines lower no entry — the additive-field discipline
    /// keeps every pin-less artifact byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_dependency_pins: Vec<DecisionPin>,
}

impl Service {
    /// Creates a service with no description, operations, or dependencies.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            operations: Vec::new(),
            dependencies: Vec::new(),
            decision_dependencies: Vec::new(),
            decision_dependency_pins: Vec::new(),
        }
    }
}

/// One **version pin** a decision-link reference carries (rexlang#154):
/// the invoked service's name (the same last-segment spelling the
/// `invokes`/`decision_dependencies` lists lower) paired with the
/// [`crate::dmn::VersionPin`] the reference wrote. Only pinned
/// references serialize — the pin-less name lists carry them already.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionPin {
    /// The pinned service's name, as the resolution universe spells it.
    pub name: String,
    /// The declared compatibility contract.
    pub pin: crate::dmn::VersionPin,
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
    /// `true` for a `protected` operation: it stays off the public service
    /// interface (Sculptor's visibility). Additive; omitted when `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_protected: bool,
    /// Actor capability names guarding this operation (the
    /// [`crate::ActorModel`] dimension); loosely coupled, not validated at
    /// the wire layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
    /// Event names this operation emits (the `emits` clause; the
    /// [`crate::events::EventModel`] dimension); loosely coupled, not
    /// validated at the wire layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub emits: Vec<String>,
    /// Decision-service names this operation invokes (the `invokes`
    /// clause; the [`crate::dmn::DmnModel`] dimension); loosely coupled,
    /// not validated at the wire layer — the `.ddd` decisions entry point
    /// joins them against the provided decision models.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub invokes: Vec<String>,
    /// The **version pins** of [`Self::invokes`] entries that carry one
    /// (rexlang#154): each names the invoked service (the same
    /// last-segment spelling `invokes` lowers) and the
    /// [`crate::dmn::VersionPin`] the entry wrote. Pin-less entries lower
    /// no entry — the additive-field discipline keeps every pin-less
    /// artifact byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decision_pins: Vec<DecisionPin>,
    /// The `when (...)` guard condition's source text, verbatim between the
    /// parentheses (the `when`-condition rule); typing is a driver concern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// The resolved field paths of the `when` condition (issue rexlang#180):
    /// every path whose resolution a consumer cannot re-derive from the
    /// artifact alone — injected-scope paths always (the repository→class
    /// binding is design knowledge), own-record paths only in a module with
    /// several machine-bearing designs (where the owning machine is not a
    /// priori derivable). A single-machine module's own-record guard lowers
    /// none. Resolved form, so consumers never re-parse [`Self::condition`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub condition_paths: Vec<ConditionPath>,
    /// The declared transition binding of a delegated operation (issue
    /// rexlang#176): `name => Repository.save transition <Trigger>` — the
    /// machine-bound transition the delegation fires, validated by the
    /// driver against the target repository's designed machine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transition: Option<TransitionRef>,
    /// The child-scoped target of a module-level `extend <Class>` operation
    /// (issue rexlang#179): `boolean addLine(PayRunLineDraft line) =>
    /// lines.add` lowers with the root's child feature (`lines`) and the
    /// resolved child class. `None` for every ordinary service operation
    /// (the additive-field discipline: absent from body-less artifacts, so
    /// existing goldens stay byte-identical).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_scope: Option<ChildRef>,
    /// Checked executable bodies, keyed by target name (the
    /// [`crate::Operation::bodies`] discipline; issue rexlang#178). The
    /// `"stmt"` target holds the canonical JSON of the operation's
    /// `rex_expr::stmt::StmtBody` — rex-ir deliberately does not depend on
    /// rex-expr, so the value is opaque here. Additive; a body-less
    /// operation omits the key.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bodies: BTreeMap<String, String>,
    /// For a statement-body operation written `name(...) => Repository
    /// { ... }`: the `=>` repository bound as `self` in the body. Additive;
    /// absent from every operation without a body receiver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_receiver: Option<String>,
}

impl ServiceOperation {
    /// Creates an operation with no return type, parameters, delegation,
    /// capabilities, invoked decision services, emitted events, condition
    /// paths, transition binding, child scope, or body.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            return_type: None,
            return_multiplicity: None,
            params: Vec::new(),
            delegation: None,
            is_protected: false,
            capabilities: Vec::new(),
            emits: Vec::new(),
            invokes: Vec::new(),
            decision_pins: Vec::new(),
            condition: None,
            condition_paths: Vec::new(),
            transition: None,
            child_scope: None,
            bodies: BTreeMap::new(),
            body_receiver: None,
        }
    }

    /// Chainable setter for the declared return cardinality.
    pub fn with_return_multiplicity(mut self, multiplicity: Option<Multiplicity>) -> Self {
        self.return_multiplicity = multiplicity;
        self
    }

    /// Chainable setter marking the operation `protected`.
    pub fn with_protected(mut self, is_protected: bool) -> Self {
        self.is_protected = is_protected;
        self
    }

    /// Chainable setter for the emitted event names.
    pub fn with_emits(mut self, emits: Vec<String>) -> Self {
        self.emits = emits;
        self
    }

    /// Chainable setter for the invoked decision-service names.
    pub fn with_invokes(mut self, invokes: Vec<String>) -> Self {
        self.invokes = invokes;
        self
    }

    /// Chainable setter for the resolved condition paths (issue rexlang#180).
    pub fn with_condition_paths(mut self, paths: Vec<ConditionPath>) -> Self {
        self.condition_paths = paths;
        self
    }

    /// Chainable setter for the declared transition binding (issue
    /// rexlang#176).
    pub fn with_transition(mut self, transition: Option<TransitionRef>) -> Self {
        self.transition = transition;
        self
    }

    /// Chainable setter for the child-scoped target (issue rexlang#179).
    pub fn with_child_scope(mut self, child_scope: Option<ChildRef>) -> Self {
        self.child_scope = child_scope;
        self
    }

    /// Chainable setter for the checked executable bodies (issue
    /// rexlang#178).
    pub fn with_bodies(mut self, bodies: BTreeMap<String, String>) -> Self {
        self.bodies = bodies;
        self
    }

    /// Chainable setter for the `=>` body receiver bound as `self` (issue
    /// rexlang#178).
    pub fn with_body_receiver(mut self, body_receiver: Option<String>) -> Self {
        self.body_receiver = body_receiver;
        self
    }
}

/// The explicit transition binding of a delegated [`ServiceOperation`]
/// (issue rexlang#176): `submit => PayRunRepository.save transition Submit`
/// — or, in a multi-machine module, `transition PayRunStatus.Reverse`,
/// qualified by the machine's **enum type name** (rexlang machine
/// declarations are field-named — `machine status: PayRunStatus` — so a
/// machine-name qualifier does not exist as authored). The driver validates
/// the trigger against the machine bound to the delegation target's
/// repository design; the artifact keeps the names exactly as authored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransitionRef {
    /// The trigger name as written — an explicit `via` trigger or a bare
    /// arrow's derived name (the lowerCamel of the target state).
    pub trigger: String,
    /// The machine qualifier — the enum type name when the source wrote the
    /// qualified spelling (`PayRunStatus.Reverse`), `None` when unqualified
    /// (`Submit`). Required in a multi-machine module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<String>,
}

impl TransitionRef {
    /// Creates an unqualified transition reference.
    pub fn new(trigger: impl Into<String>) -> Self {
        Self {
            trigger: trigger.into(),
            machine: None,
        }
    }

    /// Chainable setter for the machine qualifier.
    pub fn with_machine(mut self, machine: Option<String>) -> Self {
        self.machine = machine;
        self
    }
}

/// The child-scoped target of a module-level `extend <Class>` operation
/// (issue rexlang#179).
///
/// An `extend PayRun { boolean addLine(PayRunLineDraft line) => lines.add; }`
/// block declares root-side operations over the aggregate's contained
/// children; the lowered operation carries the root's child feature
/// ([`ChildRef::feature`], as authored), the child class that feature
/// resolves to ([`ChildRef::class`], package-qualified — the
/// [`Delegation::target`] convention), and the validated action
/// ([`ChildRef::action`] — `add`, `remove`, or `save`) so consumers can
/// distinguish the commands without re-reading the source.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChildRef {
    /// The root class's child feature name, exactly as declared in the
    /// domain model (`lines`).
    pub feature: String,
    /// The child class the feature resolves to, package-qualified
    /// (`nz.example.payroll.PayRunLine`).
    pub class: String,
    /// The validated child action: `"add"`, `"remove"`, or `"save"`.
    pub action: String,
}

impl ChildRef {
    /// Creates a child reference from the feature name, its resolved
    /// class's full name, and the validated action.
    pub fn new(
        feature: impl Into<String>,
        class: impl Into<String>,
        action: impl Into<String>,
    ) -> Self {
        Self {
            feature: feature.into(),
            class: class.into(),
            action: action.into(),
        }
    }
}

/// The resolved scope of one [`ConditionPath`] (issue rexlang#180).
///
/// Adjacently tagged: `{"type":"ownRecord"}` for a field of the op's bound
/// entity, `{"type":"injected","value":{"repository":...,"class":...}}` for
/// a field reached through an injected repository (the path's first segment
/// is the injection name as declared; `class` is the repository design's
/// class).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ConditionScope {
    /// A field of the op's own record (the bound entity).
    OwnRecord,
    /// A field reached through an injected repository.
    #[serde(rename_all = "camelCase")]
    Injected {
        /// The injection name exactly as declared (`inject TimecardRepository;`).
        repository: String,
        /// The class the repository design binds.
        class: String,
    },
}

/// One resolved field path of a [`ServiceOperation`]'s `when` condition
/// (issue rexlang#180). The scope records where the path resolves, the
/// field the dotted remainder (single segments in the common case), and the
/// field's declared type — the lowering contract consumers build on, so
/// they never re-parse the condition source string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConditionPath {
    /// Where the path resolves.
    pub scope: ConditionScope,
    /// The field path — one segment (`status`) or a dotted chain
    /// (`holder.name`) under the scope's root.
    pub field: String,
    /// The field's declared type.
    pub field_type: TypeRef,
}

impl ConditionPath {
    /// Creates a condition path.
    pub fn new(scope: ConditionScope, field: impl Into<String>, field_type: TypeRef) -> Self {
        Self {
            scope,
            field: field.into(),
            field_type,
        }
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
///
/// The bools carry the **effective** decision, not the raw spelling: the
/// design DSL defaults `auditable` and `optimisticLocking` on for entity
/// designs (Sculptor's polarity, opted out with `!auditable`), so a plain
/// `entity Book` design serializes both as `true` while a `value`/`dto`
/// design leaves them absent.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignFlags {
    /// Generate scaffolding for the class.
    #[serde(default, skip_serializing_if = "is_false")]
    pub scaffold: bool,
    /// Record an audit trail for the class's mutations. Defaults to `true`
    /// for entity designs; `!auditable` opts out.
    #[serde(default, skip_serializing_if = "is_false")]
    pub auditable: bool,
    /// Optimistic locking on the class's persistent state. Defaults to
    /// `true` for entity designs; `!optimisticLocking` opts out.
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
    /// The class's lifecycle state machine, when designed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub machine: Option<StateMachine>,
    /// Child-scoped operations declared by the module's `extend <Class>`
    /// blocks (issue rexlang#179), in declaration order: root-side
    /// operations over the aggregate's contained children. Each carries a
    /// [`ChildRef`] in [`ServiceOperation::child_scope`]. Additive; an
    /// extend-less design omits the key (the byte-identical discipline).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_operations: Vec<ServiceOperation>,
}

impl Design {
    /// Creates a concrete design with default flags, no repository, no
    /// machine, and no child operations.
    pub fn new(class: impl Into<String>, stereotype: Stereotype) -> Self {
        Self {
            class: class.into(),
            stereotype,
            is_abstract: false,
            flags: DesignFlags::default(),
            repository: None,
            machine: None,
            child_operations: Vec::new(),
        }
    }

    /// Chainable setter for the child-scoped operations (issue rexlang#179).
    pub fn with_child_operations(mut self, child_operations: Vec<ServiceOperation>) -> Self {
        self.child_operations = child_operations;
        self
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

/// The lifecycle state machine of a [`Design`]: a `machine <field>: <Enum>`
/// block over one enum-typed feature of the designed class.
///
/// The state set is the union of [`StateMachine::initial`], the
/// [`StateMachine::terminals`], and every transition endpoint; consumers
/// derive it (the artifact never repeats a bare list). Whether a state is
/// terminal is carried by the [`StateMachine::terminals`] list, not by a
/// per-state flag.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StateMachine {
    /// The designed class's feature carrying the state. The driver binds the
    /// machine to it (the feature must exist and be enum-typed); the wire
    /// layer carries the name loosely.
    pub field: String,
    /// The enum the states are drawn from.
    #[serde(rename = "enumType")]
    pub enum_type: TypeRef,
    /// The initial state.
    pub initial: String,
    /// Terminal states, in declaration order. A terminal state takes no
    /// outgoing transitions; the driver validates the machine before
    /// lowering, so consumers may rely on it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub terminals: Vec<String>,
    /// Transitions in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<Transition>,
}

impl StateMachine {
    /// Creates a state machine with no terminals or transitions.
    pub fn new(field: impl Into<String>, enum_type: TypeRef, initial: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            enum_type,
            initial: initial.into(),
            terminals: Vec::new(),
            transitions: Vec::new(),
        }
    }

    /// Every state of the machine, in canonical order: the initial state
    /// first, then the terminals in declaration order, then every remaining
    /// transition endpoint in first-appearance order (duplicates removed).
    pub fn states(&self) -> Vec<&str> {
        let mut states = vec![self.initial.as_str()];
        for terminal in &self.terminals {
            if !states.contains(&terminal.as_str()) {
                states.push(terminal);
            }
        }
        for transition in &self.transitions {
            for endpoint in [&transition.from, &transition.to] {
                if !states.contains(&endpoint.as_str()) {
                    states.push(endpoint);
                }
            }
        }
        states
    }
}

/// One transition of a [`StateMachine`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Transition {
    /// The source state name.
    pub from: String,
    /// The target state name.
    pub to: String,
    /// The `via` trigger — the workflow action performed through this
    /// transition. `None` for a bare arrow, whose action name is derived
    /// (the lowerCamel of the target state; documented in `docs/DDD.md`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    /// The guard condition's source text, verbatim between the parentheses
    /// (the `when`-condition rule). Parsing, typing, and evaluation are
    /// driver concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guard: Option<String>,
}

impl Transition {
    /// Creates an unguarded transition.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            trigger: None,
            guard: None,
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
    /// `true` for a `protected` operation: it stays off the public
    /// repository interface (Sculptor's visibility). Additive; omitted when
    /// `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub is_protected: bool,
    /// The `when (...)` guard condition's source text, verbatim between the
    /// parentheses (the `when`-condition rule); typing is a driver concern.
    /// A built-in never carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    /// Checked executable bodies, keyed by target name (the
    /// [`crate::Operation::bodies`] discipline; issue rexlang#178). The
    /// `"stmt"` target holds the canonical JSON of the operation's
    /// `rex_expr::stmt::StmtBody` — opaque here. Additive; a body-less
    /// operation omits the key. A repository body binds `self` to the
    /// operation's own repository design, so no receiver is recorded.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bodies: BTreeMap<String, String>,
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
            is_protected: false,
            condition: None,
            bodies: BTreeMap::new(),
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
            is_protected: false,
            condition: None,
            bodies: BTreeMap::new(),
        }
    }

    /// Chainable setter for the declared return cardinality.
    pub fn with_return_multiplicity(mut self, multiplicity: Option<Multiplicity>) -> Self {
        self.return_multiplicity = multiplicity;
        self
    }

    /// Chainable setter marking the operation `protected`.
    pub fn with_protected(mut self, is_protected: bool) -> Self {
        self.is_protected = is_protected;
        self
    }

    /// Chainable setter installing the `when (...)` condition's source text.
    pub fn with_condition(mut self, condition: Option<String>) -> Self {
        self.condition = condition;
        self
    }

    /// Chainable setter for the checked executable bodies (issue
    /// rexlang#178).
    pub fn with_bodies(mut self, bodies: BTreeMap<String, String>) -> Self {
        self.bodies = bodies;
        self
    }
}

/// The built-in repository operations. Serialized as bare camelCase
/// strings: `"findById"`, `"findAll"`, `"findByExample"`, `"findByKeys"`,
/// `"save"`, `"delete"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BuiltinRepositoryOp {
    /// Fetch one aggregate by identity.
    FindById,
    /// Fetch all aggregates.
    FindAll,
    /// Fetch the aggregates matching an example instance.
    FindByExample,
    /// Fetch the aggregates by their natural keys.
    FindByKeys,
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchField {
    /// The entity property whose text is indexed.
    pub property: String,
    /// Relevance boost multiplier for matches in this field; `None` uses
    /// the consumer's default weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub boost: Option<Boost>,
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
    pub fn with_boost(mut self, boost: impl Into<Boost>) -> Self {
        self.boost = Some(boost.into());
        self
    }

    /// Chainable setter for the analyzer override.
    pub fn with_analyzer(mut self, analyzer: impl Into<String>) -> Self {
        self.analyzer = Some(analyzer.into());
        self
    }
}

crate::wire::total_order_float! {
    /// The relevance-boost weight of a [`SearchField`] (`boost <n>` in the
    /// source): an ordered, equality-preserving wrapper around `f32`, so the
    /// artifact chain derives [`Eq`] end to end (wire contract rule 14).
    ///
    /// # Wire layout
    ///
    /// `#[serde(transparent)]`: a `Boost` serializes and deserializes exactly
    /// as the bare JSON number it wraps — artifacts are byte-identical to the
    /// raw-`f32` encoding.
    ///
    /// # Equality and hashing
    ///
    /// [`PartialEq`]/[`Eq`] follow `f32::total_cmp` semantics: equality is
    /// bit-pattern equality, which makes it reflexive even for `NaN` (a `NaN`
    /// equals only another `NaN` with the same bit pattern) and distinguishes
    /// `-0.0` from `0.0` (both deliberate). [`Hash`] hashes the raw bit
    /// pattern, consistent with that equality.
    ///
    /// # Ordering
    ///
    /// [`Ord`]/[`PartialOrd`] are `f32::total_cmp`: a genuine total order
    /// (`-0.0 < 0.0`, every `NaN` above [`f32::INFINITY`]), so boosts can be
    /// sorted and compared like any other key.
    ///
    /// # Where `NaN`/`-0.0` can come from
    ///
    /// The `.ddd` grammar accepts only integer boosts (`boost 3`) and JSON
    /// cannot encode `NaN`, so neither source nor wire artifacts produce one
    /// today; the `NaN`/`-0.0` behavior above is the type's deliberate
    /// contract for programmatic construction and any future float-tolerant
    /// syntax, not dead-letter handling.
    Boost(f32)
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

// The `skip_serializing_if` paths above resolve `is_false` in this
// module's scope; the shared predicate lives in `wire`.
use crate::wire::is_false;

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
    /// declared repository operations, delegation, capabilities, invoked
    /// decision services, injected decision dependencies, dependencies,
    /// and a qualified base package.
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
                            is_protected: false,
                            capabilities: vec!["BorrowBooks".to_string()],
                            emits: vec!["LoanStateChanged".to_string()],
                            invokes: vec!["RiskScore".to_string()],
                            decision_pins: vec![DecisionPin {
                                name: "RiskScore".to_string(),
                                pin: crate::dmn::VersionPin::Compatible,
                            }],
                            condition: None,
                            condition_paths: Vec::new(),
                            transition: None,
                            child_scope: None,
                            bodies: BTreeMap::new(),
                            body_receiver: None,
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
                            is_protected: false,
                            capabilities: Vec::new(),
                            emits: Vec::new(),
                            invokes: Vec::new(),
                            decision_pins: Vec::new(),
                            condition: None,
                            condition_paths: Vec::new(),
                            transition: None,
                            child_scope: None,
                            bodies: BTreeMap::new(),
                            body_receiver: None,
                        },
                    ],
                    dependencies: vec![
                        "LoanRepository".to_string(),
                        "NotificationService".to_string(),
                    ],
                    decision_dependencies: vec!["RiskScore".to_string()],
                    decision_dependency_pins: vec![DecisionPin {
                        name: "RiskScore".to_string(),
                        pin: crate::dmn::VersionPin::Exact("1.4.2".to_string()),
                    }],
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
                        machine: None,
                        child_operations: Vec::new(),
                    },
                    Design {
                        class: "Money".to_string(),
                        stereotype: Stereotype::Value,
                        is_abstract: false,
                        flags: DesignFlags::default(),
                        repository: None,
                        machine: None,
                        child_operations: Vec::new(),
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
                        machine: None,
                        child_operations: Vec::new(),
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

    /// The shared wire-contract characterization (see
    /// `wire::tests::assert_wire_contract`): an empty artifact serializes
    /// as exactly the version marker (as both `new()` and `Default` — the
    /// helper pins `Default`), the gate rejects wrong/absent versions,
    /// unknown fields are tolerated, and the populated design round-trips
    /// byte-stably through both serializers.
    #[test]
    fn empty_model_serializes_as_exactly_format_version() {
        crate::wire::tests::assert_wire_contract(r#"{"formatVersion":1}"#, &sample_model());
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

    /// The transition binding and the resolved condition paths serialize in
    /// the wire shapes of issues rexlang#176/#180: camelCase keys, the
    /// unqualified `machine` omitted, and the adjacent tagging of
    /// `ConditionScope` (`{"type":"ownRecord"}` /
    /// `{"type":"injected","value":{...}}`).
    #[test]
    fn transition_refs_and_condition_paths_serialize_as_advertised() {
        let unqualified = TransitionRef::new("Submit");
        assert_eq!(
            serde_json::to_string(&unqualified).expect("serialize"),
            r#"{"trigger":"Submit"}"#,
            "an unqualified binding omits `machine`"
        );
        let qualified =
            TransitionRef::new("Reverse").with_machine(Some("PayRunStatus".to_string()));
        assert_eq!(
            serde_json::to_string(&qualified).expect("serialize"),
            r#"{"trigger":"Reverse","machine":"PayRunStatus"}"#
        );

        let own = ConditionPath::new(
            ConditionScope::OwnRecord,
            "status",
            TypeRef::Enum {
                package: "nz.example.payroll".to_string(),
                name: "PayRunStatus".to_string(),
            },
        );
        assert_eq!(
            serde_json::to_string(&own).expect("serialize"),
            r#"{"scope":{"type":"ownRecord"},"field":"status","fieldType":{"type":"enum","value":{"package":"nz.example.payroll","name":"PayRunStatus"}}}"#
        );
        let injected = ConditionPath::new(
            ConditionScope::Injected {
                repository: "TimecardRepository".to_string(),
                class: "Timecard".to_string(),
            },
            "status",
            TypeRef::Enum {
                package: "nz.example.payroll".to_string(),
                name: "TimecardStatus".to_string(),
            },
        );
        assert_eq!(
            serde_json::to_string(&injected).expect("serialize"),
            r#"{"scope":{"type":"injected","value":{"repository":"TimecardRepository","class":"Timecard"}},"field":"status","fieldType":{"type":"enum","value":{"package":"nz.example.payroll","name":"TimecardStatus"}}}"#
        );

        // Both round-trip: the wire forms reload as the same values.
        let round: ConditionPath =
            serde_json::from_str(&serde_json::to_string(&injected).unwrap()).expect("reload");
        assert_eq!(round, injected);

        // Absence is byte-silent: an operation with no paths and no binding
        // serializes without the new keys (the golden discipline — existing
        // artifacts stay byte-identical).
        let plain = ServiceOperation::new("approve");
        let json = serde_json::to_string(&plain).expect("serialize");
        assert!(!json.contains("conditionPaths"), "{json}");
        assert!(!json.contains("transition"), "{json}");
    }

    /// Issue rexlang#179's additive fields are byte-silent when absent: a
    /// body-less design and an operation with no child scope serialize
    /// exactly as before the fields existed, and the populated forms use
    /// the documented camelCase wire shapes.
    #[test]
    fn child_scope_and_child_operations_are_absent_unless_declared() {
        let design = Design::new("Money", Stereotype::Value);
        assert_eq!(
            serde_json::to_string(&design).expect("serialize"),
            r#"{"class":"Money","stereotype":"value"}"#,
            "a child-operation-less design omits the key"
        );
        let op = ServiceOperation::new("approve");
        assert_eq!(
            serde_json::to_string(&op).expect("serialize"),
            r#"{"name":"approve"}"#,
            "an unscoped operation omits `childScope`"
        );

        let scoped = ServiceOperation::new("addLine").with_child_scope(Some(ChildRef::new(
            "lines",
            "nz.example.payroll.PayRunLine",
            "add",
        )));
        let json = serde_json::to_string(&scoped).expect("serialize");
        assert_eq!(
            json,
            r#"{"name":"addLine","childScope":{"feature":"lines","class":"nz.example.payroll.PayRunLine","action":"add"}}"#
        );
        let parsed: ServiceOperation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, scoped);

        let design = Design::new("PayRun", Stereotype::Entity).with_child_operations(vec![scoped]);
        let json = serde_json::to_string(&design).expect("serialize");
        assert_eq!(
            json,
            r#"{"class":"PayRun","stereotype":"entity","childOperations":[{"name":"addLine","childScope":{"feature":"lines","class":"nz.example.payroll.PayRunLine","action":"add"}}]}"#
        );
        let parsed: Design = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, design);
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
            machine: None,
            child_operations: Vec::new(),
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
            machine: None,
            child_operations: Vec::new(),
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
    fn loose_reference_lists_are_omitted_when_empty_and_round_trip_when_set() {
        // Additive rule 9: an operation with no `emits`/`invokes` clause —
        // and a service with no `inject decision` line — serializes without
        // those keys (artifacts authored before the clauses stay
        // byte-identical), and populated lists round-trip.
        let mut op = ServiceOperation::new("approve");
        let json = serde_json::to_string(&op).expect("serialize");
        assert!(!json.contains("emits"), "empty emits are omitted: {json}");
        assert!(
            !json.contains("invokes"),
            "empty invokes are omitted: {json}"
        );
        let parsed: ServiceOperation =
            serde_json::from_str(&json).expect("deserialize without emits");
        assert_eq!(parsed, op, "the default is the empty list");
        let service = Service::new("LoanService");
        let json = serde_json::to_string(&service).expect("serialize");
        assert!(
            !json.contains("decisionDependencies"),
            "empty decision dependencies are omitted: {json}"
        );
        let parsed: Service = serde_json::from_str(&json).expect("deserialize without injects");
        assert_eq!(parsed, service, "the default is the empty list");

        op.emits = vec!["PayRunStateChanged".to_string()];
        let json = serde_json::to_string(&op).expect("serialize");
        assert_eq!(
            json, r#"{"name":"approve","emits":["PayRunStateChanged"]}"#,
            "camelCase key, in declaration order: {json}"
        );
        let parsed: ServiceOperation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, op);
        assert_eq!(
            serde_json::to_string(&parsed).expect("re-serialize"),
            json,
            "re-serialization is byte-identical"
        );

        op.invokes = vec!["RiskScore".to_string()];
        let json = serde_json::to_string(&op).expect("serialize");
        assert_eq!(
            json, r#"{"name":"approve","emits":["PayRunStateChanged"],"invokes":["RiskScore"]}"#,
            "the invokes key follows emits, in declaration order: {json}"
        );
        let parsed: ServiceOperation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, op);

        // The version pins (rexlang#154) serialize additively after the
        // name lists: camelCase keys, only pinned references appear, and
        // the shared `dmn::VersionPin` adjacent tagging holds.
        let mut service = Service::new("LoanService");
        service.decision_dependencies = vec!["RiskScore".to_string()];
        let json = serde_json::to_string(&service).expect("serialize");
        assert_eq!(
            json, r#"{"name":"LoanService","decisionDependencies":["RiskScore"]}"#,
            "a pin-less inject lowers no pin entry: {json}"
        );
        let parsed: Service = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, service);

        service.decision_dependency_pins = vec![DecisionPin {
            name: "RiskScore".to_string(),
            pin: crate::dmn::VersionPin::Compatible,
        }];
        let json = serde_json::to_string(&service).expect("serialize");
        assert_eq!(
            json,
            r#"{"name":"LoanService","decisionDependencies":["RiskScore"],"decisionDependencyPins":[{"name":"RiskScore","pin":{"type":"compatible"}}]}"#,
            "the compatible pin serializes as the bare tag: {json}"
        );
        let parsed: Service = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, service);
        assert_eq!(
            serde_json::to_string(&parsed).expect("re-serialize"),
            json,
            "re-serialization is byte-identical"
        );

        op.decision_pins = vec![DecisionPin {
            name: "RiskScore".to_string(),
            pin: crate::dmn::VersionPin::Exact("1.4.2".to_string()),
        }];
        let json = serde_json::to_string(&op).expect("serialize");
        assert!(
            json.contains(
                r#""decisionPins":[{"name":"RiskScore","pin":{"type":"exact","value":"1.4.2"}}]"#
            ),
            "the exact pin carries its value under the adjacent tag: {json}"
        );
        let parsed: ServiceOperation = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed, op);

        // The chainable setters.
        let op = ServiceOperation::new("submit")
            .with_emits(vec!["PayRunSubmitted".to_string()])
            .with_invokes(vec!["CreditCheck".to_string()]);
        assert_eq!(op.emits, vec!["PayRunSubmitted".to_string()]);
        assert_eq!(op.invokes, vec!["CreditCheck".to_string()]);
        assert!(op.decision_pins.is_empty());
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

    // -----------------------------------------------------------------------
    // Boost: the ordered float newtype (wire contract rule 14)
    // -----------------------------------------------------------------------

    #[test]
    fn boost_serializes_as_the_bare_f32_and_round_trips() {
        for value in [2.5f32, 1.0, 0.1, 3.402_823_5e38, -0.0] {
            let json = serde_json::to_string(&Boost::new(value)).expect("serialize");
            assert_eq!(
                json,
                serde_json::to_string(&value).expect("plain f32"),
                "transparent layout: {json}"
            );
            let back: Boost = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back, Boost::new(value));
            assert_eq!(back.value(), value);
        }
    }

    #[test]
    fn boost_equality_is_bit_pattern_equality() {
        // Reflexive even for NaN; a NaN equals only its own bit pattern.
        assert_eq!(Boost::new(f32::NAN), Boost::new(f32::NAN));
        assert_ne!(
            Boost::new(f32::NAN),
            Boost::new(f32::from_bits(f32::NAN.to_bits() + 1))
        );
        assert_eq!(Boost::new(2.5), Boost::new(2.5));
        assert_ne!(Boost::new(2.5), Boost::new(2.75));
        // Intended: -0.0 is its own value, distinct from 0.0.
        assert_ne!(Boost::new(-0.0), Boost::new(0.0));
    }

    #[test]
    fn boost_hash_is_consistent_with_equality() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        fn hash_of(value: f32) -> u64 {
            let mut hasher = DefaultHasher::new();
            Boost::new(value).hash(&mut hasher);
            hasher.finish()
        }
        assert_eq!(hash_of(2.5), hash_of(2.5));
        assert_ne!(hash_of(-0.0), hash_of(0.0));
        assert_ne!(
            hash_of(f32::NAN),
            hash_of(f32::from_bits(f32::NAN.to_bits() + 1))
        );
    }

    /// Issue #178's body fields are additive: absent from every body-less
    /// operation (the byte-identical discipline) and present in the
    /// documented camelCase shapes when populated. The `"stmt"` value is an
    /// opaque canonical-JSON string — rex-ir never depends on rex-expr.
    #[test]
    fn operation_bodies_are_additive_and_round_trip() {
        const BODY: &str = r#"{"stmts":[{"kind":"return","expr":"true"}]}"#;

        let plain = ServiceOperation::new("approve");
        assert_eq!(
            serde_json::to_string(&plain).expect("serialize"),
            r#"{"name":"approve"}"#,
            "a body-less operation omits `bodies` and `bodyReceiver`"
        );
        let plain_repository = RepositoryOperation::declared(
            "total",
            Some(TypeRef::Primitive(PrimitiveType::Int)),
            Vec::new(),
        );
        assert_eq!(
            serde_json::to_string(&plain_repository).expect("serialize"),
            r#"{"name":"total","returnType":{"type":"primitive","value":"int"}}"#,
            "a body-less repository operation omits `bodies`"
        );

        let mut bodies = std::collections::BTreeMap::new();
        bodies.insert("stmt".to_string(), BODY.to_string());
        let operation = ServiceOperation::new("approve")
            .with_bodies(bodies.clone())
            .with_body_receiver(Some("PayRunRepository".to_string()));
        assert_eq!(
            serde_json::to_string(&operation).expect("serialize"),
            format!(
                r#"{{"name":"approve","bodies":{{"stmt":{}}},"bodyReceiver":"PayRunRepository"}}"#,
                serde_json::to_string(BODY).expect("quote the body")
            )
        );
        let round: ServiceOperation =
            serde_json::from_str(&serde_json::to_string(&operation).unwrap()).expect("reload");
        assert_eq!(round, operation);

        let repository_operation = RepositoryOperation::declared(
            "total",
            Some(TypeRef::Primitive(PrimitiveType::Int)),
            Vec::new(),
        )
        .with_bodies(bodies);
        let json = serde_json::to_string(&repository_operation).expect("serialize");
        assert!(json.contains(r#""bodies":{"stmt":"#), "{json}");
        assert!(
            !json.contains("bodyReceiver"),
            "repositories carry no receiver"
        );
        let round: RepositoryOperation = serde_json::from_str(&json).expect("reload");
        assert_eq!(round, repository_operation);

        let empty = DddModel::new();
        assert_eq!(
            empty.to_json().expect("serialize"),
            r#"{"formatVersion":1}"#
        );
    }

    #[test]
    fn boost_orders_like_total_cmp() {
        let mut values = [
            Boost::new(2.5),
            Boost::new(f32::NAN),
            Boost::new(0.0),
            Boost::new(f32::NEG_INFINITY),
            Boost::new(-0.0),
            Boost::new(f32::INFINITY),
            Boost::new(-1.0),
        ];
        values.sort();
        // Bit patterns, because -0.0 == 0.0 under plain f32 equality.
        let bits: Vec<u32> = values.iter().map(|boost| boost.value().to_bits()).collect();
        assert_eq!(
            bits,
            vec![
                f32::NEG_INFINITY.to_bits(),
                (-1.0f32).to_bits(),
                (-0.0f32).to_bits(),
                0.0f32.to_bits(),
                2.5f32.to_bits(),
                f32::INFINITY.to_bits(),
                f32::NAN.to_bits(),
            ]
        );
    }
}
