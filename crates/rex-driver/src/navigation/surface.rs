//! The shared spanned-index contract every surface implements (issue
//! rexlang#245).
//!
//! The graph artifact ([`crate::graph`]) is the one node table: it says
//! *which* symbol a name denotes. A [`SurfaceIndex`] is the per-surface span
//! table: it says *where* the declarations and mentions of that surface are.
//! The two meet on the `(kind, name)` key — an [`IndexedDefinition`]'s
//! `(kind, name)` is exactly the graph builder's node key ([`crate::graph`]'s
//! name-derivation helpers, in `graph::ids`), so a language server can resolve
//! a reference through the graph and land on a span through the index.
//!
//! # Resolution split
//!
//! An [`IndexedReference`] carries the name **as written** plus a
//! [`IndexedReference::target_kinds`] hint — the node kinds it may denote.
//! Resolving it to an actual definition (same application, `base`
//! qualification, ambiguity, cross-surface pooling) is the graph index's job,
//! never this module's: the syntax layer sees one file, while resolution is
//! union-wide.
//!
//! # Builders
//!
//! Every builder is a pure function of source text (this crate's
//! `parse → index` discipline): no I/O, no compiler, and a source that does
//! not parse yields an **empty** index rather than a panic. Builders live in
//! the sibling modules for their surface: [`super::actors_index`],
//! [`super::ddd_index`], [`super::evt_index`], [`super::deploy_index`],
//! [`super::uc_index`], [`super::cmmn_index`], and [`super::dmn_index`].
//! `.mox`'s span table is the older [`super::NavigationIndex`] (its adapter
//! to these types lands in 8/8).

use rex_syntax::Span;

/// One declaration site: `(kind, name)` MUST equal the graph builder's node
/// key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedDefinition {
    /// The graph node kind (`"capability"`, `"serviceOperation"`, …).
    pub kind: String,
    /// The surface-qualified name (`Support.RaiseRefund`,
    /// `SupportDesk.desk.TicketService.raiseRefund`).
    pub name: String,
    /// Span of the declared identifier itself.
    pub name_span: Span,
    /// Span of the whole declaration (the declaration keyword and body
    /// included), for hover/peek ranges.
    pub full_span: Span,
}

impl IndexedDefinition {
    /// Creates a definition site. Builders go through this so the
    /// `(kind, name)` contract has one constructor.
    pub(crate) fn new(
        kind: &str,
        name: impl Into<String>,
        name_span: Span,
        full_span: Span,
    ) -> Self {
        Self {
            kind: kind.to_string(),
            name: name.into(),
            name_span,
            full_span,
        }
    }
}

/// One reference site: the name is a hint; the resolver (the graph index)
/// decides the target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedReference {
    /// The acceptable graph node kinds for this mention, in preference order.
    /// One match is a binding; several are the graph's ambiguity concern.
    pub target_kinds: &'static [&'static str],
    /// The name as written (a bare surface-local spelling, usually).
    pub name: String,
    /// Span of the mention as written.
    pub span: Span,
    /// Where the mention appears, for hover text and LSP dispatch.
    pub context: ReferenceContext,
}

/// The grammatical position of an [`IndexedReference`], one variant per place
/// a surface names another symbol.
///
/// The spelling is the LSP contract's prose (`"on-class"`, `"grant-entry"`,
/// `"extends"`, …); see [`ReferenceContext::as_str`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferenceContext {
    // --- `.actor` ---------------------------------------------------------
    /// `capability <C> on <Class>` — the granted class.
    OnClass,
    /// The actor a `grant <Actor> { … }` block is keyed by.
    GrantActor,
    /// A `permit`/`forbid`/delegation entry's capability name.
    GrantEntry,
    /// An actor's `extends <Parent>`.
    Extends,
    /// A delegation's `from <Actor>`.
    DelegationFrom,
    /// A delegation's `to <Actor>`.
    DelegationTo,
    /// A delegation's `purpose <name>`.
    Purpose,
    /// One name of a `never_both { … }` constraint.
    NeverBoth,

    // --- `.ddd` -----------------------------------------------------------
    /// A `capability <C>` clause on an operation or search.
    Capability,
    /// An `invokes <Decision>` clause or an `inject decision <Decision>`
    /// dependency.
    Decision,
    /// An operation's `emits <Event>` name (and, shared, a `.dmn` decision's
    /// `emits { … }` entry — the same grammatical position on both
    /// surfaces).
    Emits,
    /// An `inject <Repository>` service dependency.
    ServiceDependency,
    /// The `=> <Repository>` receiver a service body runs against.
    BodyReceiver,
    /// A design's `entity`/`value`/`dto` class name.
    DesignClass,
    /// A search's `entity <Class>`.
    SearchEntity,
    /// A delegation's `=> <Repository>.<operation>` target.
    DelegationForward,
    /// A machine transition's `via <Trigger>` (or an operation's
    /// `transition <Trigger>` binding).
    Transition,
    /// A machine's `on`/state field (the designed class's state-carrying
    /// feature).
    MachineField,
    /// A machine's state enum type.
    StateEnum,

    // --- `.evt` -----------------------------------------------------------
    /// A subscription's `consumer <Component>`.
    Consumer,
    /// A subscription's `events [ … ]` names.
    Subscribes,
    /// A channel's `publishes <Event>` names.
    Publishes,
    /// An event field's type.
    FieldType,

    // --- `.deploy` --------------------------------------------------------
    /// A component's `design "<path>[#<Module>]"` binding.
    DesignBinding,
    /// A component's `flow "<path>[#<Module>]"` binding.
    FlowBinding,
    /// A component's `requires <capability>` name.
    DeployRequires,
    /// One endpoint of a `connects <from> -> <to>` entry.
    Connection,
    /// A deployment's `for <Application>` clause.
    DeployFor,
    /// A deployment's `use <Profile>` selection.
    ProfileUse,

    // --- `.uc` ------------------------------------------------------------
    /// A participant's `: <type>` reference (the subject's classifier or a
    /// typed actor's domain type).
    ParticipantType,
    /// A bare (untyped) actor participant's name — a policy actor.
    PolicyActor,
    /// A use case's `realizes <alias>.<Service>.<operation>` target.
    Realizes,
    /// A perform step's `: <operation>` binding (the bare form's step name).
    StepOperation,
    /// A perform step's `requires [ … ]` capability names.
    StepRequires,
    /// A message's `of <type>` payload.
    MessagePayload,

    // --- `.cmmn` ----------------------------------------------------------
    /// A role's `as <Actor>` binding.
    RoleActor,
    /// A case-file item's declared type.
    ItemType,
    /// A service task's `execute <alias>.<Service>.<operation>` target.
    Execute,
    /// A decision task's `invokes <Model>.Service` target.
    Invoke,
    /// A criterion's `on <part>` (a plan item, case-file item, or event).
    Criterion,
    /// A task's `performer: <role>` clause.
    Performer,
    /// A stage's nested plan items — the artifact's flat-graph `children`.
    Children,
    /// A decision task's `inputs { … }` names.
    DecisionInput,
    /// A decision task's `outputs { … }` names.
    DecisionOutput,

    // --- `.dmn` -----------------------------------------------------------
    /// An input's declared type.
    InputType,
    /// A business knowledge model parameter's declared type.
    ParameterType,
    /// A declared output's type (a decision output or a BKM's `output`).
    ReturnType,
    /// A decision's `logic invoke <model>` target.
    InvokeModel,
    /// One name of an `information`/`knowledge`/`authority` requirement
    /// line (either end).
    Requirement,
    /// A service's `inputs { … }` names.
    ServiceInput,
    /// A service's `outputs { … }` names.
    ServiceOutput,
    /// A service's `encapsulates { … }` names.
    Encapsulates,
    /// A view's `show { … }` names.
    Show,

    // --- `.ifml` ----------------------------------------------------------
    /// A `use "<Module>"` statement's target module.
    IfmlUse,
    /// A `navigate("<Target>")` interaction target (a view, container, or
    /// component).
    IfmlNavigate,
    /// A `refresh("<Target>")` interaction target (a container or component).
    IfmlRefresh,
    /// An `action("<Name>")` invocation — the `.ifml` action it names (whose
    /// `binds` edge carries the designed operation).
    IfmlAction,
    /// A view's `requires: [ … ]` capability names.
    IfmlRequires,
    /// A view's `roles: [ … ]` role names (`.actor` actors).
    IfmlRoles,
    /// A component's `data: <Class>` binding.
    IfmlData,
}

impl ReferenceContext {
    /// The context's prose spelling — the LSP contract's vocabulary.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::OnClass => "on-class",
            Self::GrantActor => "grant-actor",
            Self::GrantEntry => "grant-entry",
            Self::Extends => "extends",
            Self::DelegationFrom => "delegation-from",
            Self::DelegationTo => "delegation-to",
            Self::Purpose => "purpose",
            Self::NeverBoth => "never-both",
            Self::Capability => "capability",
            Self::Decision => "decision",
            Self::Emits => "emits",
            Self::ServiceDependency => "service-dependency",
            Self::BodyReceiver => "body-receiver",
            Self::DesignClass => "design-class",
            Self::SearchEntity => "search-entity",
            Self::DelegationForward => "delegation-forward",
            Self::Transition => "transition",
            Self::MachineField => "machine-field",
            Self::StateEnum => "state-enum",
            Self::Consumer => "consumer",
            Self::Subscribes => "subscribes",
            Self::Publishes => "publishes",
            Self::FieldType => "field-type",
            Self::DesignBinding => "design-binding",
            Self::FlowBinding => "flow-binding",
            Self::DeployRequires => "deploy-requires",
            Self::Connection => "connection",
            Self::DeployFor => "deploy-for",
            Self::ProfileUse => "profile-use",
            Self::ParticipantType => "participant-type",
            Self::PolicyActor => "policy-actor",
            Self::Realizes => "realizes",
            Self::StepOperation => "step-operation",
            Self::StepRequires => "step-requires",
            Self::MessagePayload => "message-payload",
            Self::RoleActor => "role-actor",
            Self::ItemType => "item-type",
            Self::Execute => "execute",
            Self::Invoke => "invoke",
            Self::Criterion => "criterion",
            Self::Performer => "performer",
            Self::Children => "children",
            Self::DecisionInput => "decision-input",
            Self::DecisionOutput => "decision-output",
            Self::InputType => "input-type",
            Self::ParameterType => "parameter-type",
            Self::ReturnType => "return-type",
            Self::InvokeModel => "invoke-model",
            Self::Requirement => "requirement",
            Self::ServiceInput => "service-input",
            Self::ServiceOutput => "service-output",
            Self::Encapsulates => "encapsulates",
            Self::Show => "show",
            Self::IfmlUse => "ifml-use",
            Self::IfmlNavigate => "ifml-navigate",
            Self::IfmlRefresh => "ifml-refresh",
            Self::IfmlAction => "ifml-action",
            Self::IfmlRequires => "ifml-requires",
            Self::IfmlRoles => "ifml-roles",
            Self::IfmlData => "ifml-data",
        }
    }
}

impl std::fmt::Display for ReferenceContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The result of looking up a byte offset in a [`SurfaceIndex`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SurfaceLookup<'a> {
    /// The offset falls inside a definition's name span.
    Definition(&'a IndexedDefinition),
    /// The offset falls inside a reference's span.
    Reference(&'a IndexedReference),
    /// The offset hits nothing the index knows about.
    None,
}

/// The spanned index of one surface file: its declaration sites and its
/// reference sites.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SurfaceIndex {
    /// Every declaration the graph emits a node for, in source order.
    pub definitions: Vec<IndexedDefinition>,
    /// Every name mention of another symbol, in source order.
    pub references: Vec<IndexedReference>,
}

impl SurfaceIndex {
    /// Returns the definition or reference whose span contains `offset`.
    ///
    /// Definitions win over references, the [`super::NavigationIndex`]
    /// convention.
    pub fn at(&self, offset: usize) -> SurfaceLookup<'_> {
        for definition in &self.definitions {
            if definition.name_span.start <= offset && offset < definition.name_span.end {
                return SurfaceLookup::Definition(definition);
            }
        }
        for reference in &self.references {
            if reference.span.start <= offset && offset < reference.span.end {
                return SurfaceLookup::Reference(reference);
            }
        }
        SurfaceLookup::None
    }

    /// The definitions with exactly this `(kind, name)` — the graph node key.
    pub fn definitions_of(&self, kind: &str, name: &str) -> Vec<&IndexedDefinition> {
        self.definitions
            .iter()
            .filter(|definition| definition.kind == kind && definition.name == name)
            .collect()
    }

    /// The references that may denote `(kind, name)`.
    ///
    /// A reference is written the surface's way (usually bare — `RaiseRefund`
    /// for the graph name `Support.RaiseRefund`), so an exact match or a
    /// last-segment match binds, and the reference's
    /// [`IndexedReference::target_kinds`] hint must accept `kind`. This is a
    /// name-based prefilter for the LSP; the graph index remains the
    /// resolution authority.
    pub fn references_to(&self, kind: &str, name: &str) -> Vec<&IndexedReference> {
        let local = name.rsplit('.').next().unwrap_or(name);
        self.references
            .iter()
            .filter(|reference| reference.targets_kind(kind))
            .filter(|reference| reference.name == name || reference.name == local)
            .collect()
    }

    /// True when the index knows nothing at all (an unparsable source).
    pub fn is_empty(&self) -> bool {
        self.definitions.is_empty() && self.references.is_empty()
    }

    /// Records one definition site.
    pub(crate) fn push_definition(
        &mut self,
        kind: &str,
        name: impl Into<String>,
        name_span: Span,
        full_span: Span,
    ) {
        self.definitions
            .push(IndexedDefinition::new(kind, name, name_span, full_span));
    }

    /// Records one reference site.
    pub(crate) fn push_reference(
        &mut self,
        target_kinds: &'static [&'static str],
        name: impl Into<String>,
        span: Span,
        context: ReferenceContext,
    ) {
        let name = name.into();
        if name.is_empty() {
            // A recovered-but-missing name (the parser's empty `Name`) is not
            // a mention; it would be a nameless reference at a zero span.
            return;
        }
        self.references.push(IndexedReference {
            target_kinds,
            name,
            span,
            context,
        });
    }

    /// Sorts both lists into source order, so lookups and document symbols
    /// are stable regardless of the AST's per-kind collection order.
    pub(crate) fn sort(&mut self) {
        self.definitions.sort_by(|left, right| {
            (
                left.name_span.start,
                left.name_span.end,
                &left.kind,
                &left.name,
            )
                .cmp(&(
                    right.name_span.start,
                    right.name_span.end,
                    &right.kind,
                    &right.name,
                ))
        });
        self.references.sort_by(|left, right| {
            (
                left.span.start,
                left.span.end,
                &left.name,
                left.context.as_str(),
            )
                .cmp(&(
                    right.span.start,
                    right.span.end,
                    &right.name,
                    right.context.as_str(),
                ))
        });
    }
}

impl IndexedReference {
    /// `true` when this mention may denote a node of `kind`.
    pub fn targets_kind(&self, kind: &str) -> bool {
        self.target_kinds.contains(&kind)
    }
}
