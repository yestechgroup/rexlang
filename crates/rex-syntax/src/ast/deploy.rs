//! The `.deploy` deployment DSL AST.

use super::*;

// --- .deploy deployment DSL --------------------------------------------------

/// The root node of a parsed `.deploy` source: `import` declarations
/// followed by `application`, `profile`, and `deployment` declarations.
/// Imports must precede the first declaration (a later `import` is a syntax
/// error); the three declaration kinds may be interleaved and are collected
/// into per-kind lists — the same fold the `.evt` file makes. An empty
/// import or declaration list is legal, and duplicate imports are a driver
/// concern, not a syntax error.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployFile {
    /// The `import` declarations in source order.
    pub imports: Vec<ImportDecl>,
    /// Declared applications, in source order.
    pub applications: Vec<DeployApplicationDecl>,
    /// Declared profiles, in source order.
    pub profiles: Vec<DeployProfileDecl>,
    /// Declared deployments, in source order.
    pub deployments: Vec<DeployDeploymentDecl>,
}

/// An `application <name> { ... }` declaration: one logical architecture —
/// named components and the connections between them. Deliberately
/// infrastructure-free: a component records what it *is*, never where it
/// runs (that is the profile's job).
#[derive(Debug, Clone, PartialEq)]
pub struct DeployApplicationDecl {
    /// The application name.
    pub name: Name,
    /// Components in source order.
    pub components: Vec<DeployComponentDecl>,
    /// `connects a -> b` declarations in source order.
    pub connections: Vec<DeployConnectionDecl>,
    /// Span of the whole declaration, `application` keyword included.
    pub span: Span,
}

/// A `component <name>: <kind> { ... }` declaration: one runtime building
/// block of a [`DeployApplicationDecl`], with its required capabilities,
/// baseline settings, and design/flow bindings. The body is optional.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployComponentDecl {
    /// The component name.
    pub name: Name,
    /// The component kind as written (`api`, `worker`, `database`, `queue`,
    /// `objectStore`, or `frontend`); vocabulary validation is a driver
    /// concern.
    pub kind: Name,
    /// `requires <capability>, ...` names in source order.
    pub requires: Vec<Name>,
    /// Baseline `name: value` settings in source order.
    pub settings: Vec<DeploySettingDecl>,
    /// `design "<path>[#<Module>]"` bindings in source order.
    pub designs: Vec<DeployBindingDecl>,
    /// `flow "<path>[#<Module>]"` bindings in source order.
    pub flows: Vec<DeployBindingDecl>,
    /// `env { NAME = "value" ... }` entries in source order.
    pub env: Vec<DeployEnvEntry>,
    /// Span of the whole declaration, `component` keyword included.
    pub span: Span,
}

/// One `NAME = "value"` entry of a [`DeployComponentDecl`]'s `env { ... }`
/// block: an opaque string environment variable. The name is a bare word —
/// keyword tokens included (the qname-segment rule) — and the value is the
/// unescaped string literal, carried verbatim. Env is deliberately *not* a
/// setting: it is never policy-addressable and never validated against the
/// kind's vocabulary (the closed setting-table discipline stays intact).
#[derive(Debug, Clone, PartialEq)]
pub struct DeployEnvEntry {
    /// The env var name as written.
    pub name: Name,
    /// The env var's string value (escapes applied), verbatim.
    pub value: String,
    /// Span of the whole entry.
    pub span: Span,
}

/// One `connects <from> -> <to>` entry of an [`DeployApplicationDecl`]: a
/// directed dependency between two components, referenced by name.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployConnectionDecl {
    /// The depending component's name.
    pub from: Name,
    /// The depended-on component's name.
    pub to: Name,
    /// Span of the whole entry, `connects` keyword included.
    pub span: Span,
}

/// One `design "..."` or `flow "..."` binding of a component: an import
/// path (matched as written against a file import) plus the optional
/// `#<Module>` suffix.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployBindingDecl {
    /// The reference string with standard escapes applied, verbatim
    /// including any `#<Module>` suffix.
    pub reference: String,
    /// `true` for a `design` binding, `false` for a `flow` binding.
    pub is_design: bool,
    /// Span of the whole entry, `design`/`flow` keyword included.
    pub span: Span,
}

/// A `profile <name> { ... }` declaration: a reusable deployment
/// configuration — the selected target plus defaults, policies, and
/// resource mappings. Targets are implementation-provided (the driver's
/// capability tables), never authored here beyond naming one.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployProfileDecl {
    /// The profile name.
    pub name: Name,
    /// The `target: <name>` line; recovered as `None` (a reported syntax
    /// error) when absent.
    pub target: Option<Name>,
    /// Default `component.setting: value` entries in source order.
    pub defaults: Vec<DeploySettingDecl>,
    /// `require (...)`/`prohibit (...)` policies in source order.
    pub policies: Vec<DeployPolicyDecl>,
    /// `kind -> platform.resource` mappings in source order.
    pub mappings: Vec<DeployMappingDecl>,
    /// Span of the whole declaration, `profile` keyword included.
    pub span: Span,
}

/// One `require (...)`/`prohibit (...)` clause of a
/// [`DeployProfileDecl`]: a boolean condition over the effective settings
/// of every deployment selecting the profile. The expression is captured
/// raw (the `when`-condition convention) and parsed/typed/evaluated by the
/// driver.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployPolicyDecl {
    /// `true` for `prohibit`, `false` for `require`.
    pub prohibit: bool,
    /// Span of the parenthesized condition, parentheses included; the
    /// driver slices strictly inside these bounds.
    pub expr: Span,
    /// Span of the whole entry, `require`/`prohibit` keyword included.
    pub span: Span,
}

/// One `<kind> -> <platform.resource>` entry of a [`DeployProfileDecl`]'s
/// `mappings` block: how a component kind translates into a namespaced
/// platform resource (`kubernetes.deployment`, `cloudflare.d1`).
#[derive(Debug, Clone, PartialEq)]
pub struct DeployMappingDecl {
    /// The component kind as written.
    pub kind: Name,
    /// The namespaced resource identifier.
    pub resource: QualifiedName,
    /// Span of the whole entry.
    pub span: Span,
}

/// A `deployment <name> for <application> { use <profile> ... }`
/// declaration: one application deployed through one profile with
/// application-specific `configure` overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployDeploymentDecl {
    /// The deployment name.
    pub name: Name,
    /// The deployed application's name.
    pub application: Name,
    /// The `use <name>` profile selection; recovered as `None` (a reported
    /// syntax error) when absent.
    pub profile: Option<Name>,
    /// `component.setting: value` overrides in source order.
    pub configure: Vec<DeploySettingDecl>,
    /// Span of the whole declaration, `deployment` keyword included.
    pub span: Span,
}

/// One `path: value` setting line, at any of the three levels (component
/// baseline, profile default, deployment override). The path is a
/// qualified name — a single segment inside a component body, a
/// `component.setting` pair inside `defaults`/`configure`.
#[derive(Debug, Clone, PartialEq)]
pub struct DeploySettingDecl {
    /// The dotted setting path as written.
    pub path: QualifiedName,
    /// The setting's value.
    pub value: DeploySettingValue,
    /// Span of the whole line.
    pub span: Span,
}

/// The value half of a [`DeploySettingDecl`]: one token — a string
/// literal, an integer literal, or a bare word.
#[derive(Debug, Clone, PartialEq)]
pub enum DeploySettingValue {
    /// A string literal, escapes applied.
    Str(String),
    /// An integer literal.
    Int(i64),
    /// A bare word (the enum-value spelling).
    Word(Name),
}
