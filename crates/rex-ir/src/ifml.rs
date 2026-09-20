//! The IFML interaction-model artifact: views, components, events, and
//! navigation flows, produced by parsing `.ifml` sources.
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`IFML_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent (wire
//! contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged: struct fields serialize as `camelCase`
//! (`formatVersion`, `eventType`, ...), enums with payloads are *adjacently*
//! tagged `{"type": "<variant>", "value": <payload>}` (tags camelCase:
//! `"navigate"`, `"stringLit"`, `"fieldExpr"`, ...), unit-only enums
//! serialize as bare camelCase strings (`"bar"`, `"regexMatch"`), unknown
//! fields are ignored on deserialize, and
//! [`IfmlModel::from_json`] rejects any other `formatVersion` with
//! [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//!
//! Transitional architecture: the `rex-ifml` parser crate lowers `.ifml`
//! sources **directly into these types** (there is no separate syntax tree
//! yet); a future chumsky/spanned-AST re-platform will introduce a proper
//! syntax → IR lowering. `PropertyAssignment::span` is parse-only and never
//! serialized (`#[serde(skip, default)]`).
//!
//! Serde note: enums here all use adjacent tagging (never `transparent`),
//! which round-trips `Option<...>` payloads, boxed subtrees, unit variants
//! (serialized as bare `{"type": ...}`), and tuple payloads alike — no
//! untagged interaction exists in this surface.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::IrError;

/// The artifact format version this crate writes and accepts for IFML
/// interaction-model artifacts ([`IfmlModel`]).
///
/// Versioned independently of [`crate::FORMAT_VERSION`] (the domain-model
/// marker) and [`crate::ACTOR_MODEL_FORMAT_VERSION`] (the policy marker);
/// bump whenever the IFML wire format changes incompatibly.
pub const IFML_MODEL_FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum IfmlDefinition {
    Domain(DomainDeclaration),
    View(ViewDeclaration),
    Action(ActionDeclaration),
    Module(ModuleDeclaration),
    Actor(ActorDeclaration),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DomainDeclaration {
    pub name: String,
    pub schema_name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ViewDeclaration {
    pub name: String,
    pub label: Option<String>,
    pub is_landmark: bool,
    pub is_xor: bool,
    pub is_modal: bool,
    pub params: Vec<ParameterDecl>,
    pub properties: Vec<PropertyAssignment>,
    pub containers: Vec<ContainerDeclaration>,
    pub components: Vec<ComponentDeclaration>,
    pub events: Vec<EventHandler>,
    pub module_uses: Vec<ModuleUse>,
    #[serde(default)]
    pub roles: Vec<String>,
    /// Capability requirements declared as `requires: [CapA, CapB];` on a
    /// view. Extracted from the property bag; empty when absent.
    #[serde(default)]
    pub requires: Vec<String>,
    pub condition: Option<Expression>,
    pub position: Option<Position>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Position {
    pub x: f64,
    pub y: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerDeclaration {
    pub name: String,
    /// `label "…";` inside the container body; must precede property
    /// assignments (PEG ordering).
    #[serde(default)]
    pub label: Option<String>,
    pub is_default: bool,
    /// `xor: true;` extracted from the property bag; siblings carrying it
    /// form one exclusive group.
    #[serde(default)]
    pub is_xor: bool,
    pub params: Vec<ParameterDecl>,
    pub properties: Vec<PropertyAssignment>,
    /// Containers declared inside this container (the grammar allows
    /// nesting to any depth).
    #[serde(default)]
    pub containers: Vec<ContainerDeclaration>,
    pub components: Vec<ComponentDeclaration>,
    pub events: Vec<EventHandler>,
    pub module_uses: Vec<ModuleUse>,
    pub condition: Option<Expression>,
    pub position: Option<Position>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ComponentType {
    List,
    Form,
    Details,
    Search,
    Tree,
    Chart,
    Table,
    Button,
    Link,
    Menu,
    Image,
    Embedded,
    Custom(String),
}

impl ComponentType {
    pub fn as_str(&self) -> &str {
        match self {
            ComponentType::List => "list",
            ComponentType::Form => "form",
            ComponentType::Details => "details",
            ComponentType::Search => "search",
            ComponentType::Tree => "tree",
            ComponentType::Chart => "chart",
            ComponentType::Table => "table",
            ComponentType::Button => "button",
            ComponentType::Link => "link",
            ComponentType::Menu => "menu",
            ComponentType::Image => "image",
            ComponentType::Embedded => "embedded",
            ComponentType::Custom(s) => s.as_str(),
        }
    }
}

impl From<&str> for ComponentType {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "list" => ComponentType::List,
            "form" => ComponentType::Form,
            "details" => ComponentType::Details,
            "search" => ComponentType::Search,
            "tree" => ComponentType::Tree,
            "chart" => ComponentType::Chart,
            "table" => ComponentType::Table,
            "button" => ComponentType::Button,
            "link" => ComponentType::Link,
            "menu" => ComponentType::Menu,
            "image" => ComponentType::Image,
            "embedded" => ComponentType::Embedded,
            _ => ComponentType::Custom(s.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ComponentDeclaration {
    pub name: String,
    pub component_type: Option<ComponentType>,
    pub spec: Option<ComponentSpec>,
    pub properties: Vec<PropertyAssignment>,
    pub events: Vec<EventHandler>,
    pub condition: Option<Expression>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ComponentSpec {
    Table(TableSpec),
    Form(FormSpec),
    Chart(ChartSpec),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TableSpec {
    pub columns: Vec<ColumnDef>,
    pub pagination: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ColumnDef {
    Field {
        label: String,
        field: PropertyRef,
    },
    Lookup {
        label: String,
        field: PropertyRef,
        lookup: String,
    },
    Expression {
        label: String,
        expr: Expression,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyRef {
    pub entity: String,
    pub property: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSpec {
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDef {
    pub name: String,
    pub input: InputFieldType,
    pub required: bool,
    pub validations: Vec<Expression>,
    pub values: Vec<String>,
    #[serde(default)]
    pub messages: Vec<String>,
}

/// A `use "Module" as alias { ... };` statement instantiating a declared
/// module inside a view or container body.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleUse {
    pub module: String,
    pub alias: Option<String>,
    #[serde(default)]
    pub properties: Vec<PropertyAssignment>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum InputFieldType {
    Text,
    TextArea,
    Password,
    Email,
    Number,
    Date,
    Time,
    DateTime,
    Dropdown,
    RadioGroup,
    Checkbox,
    Toggle,
    File,
    Hidden,
    Custom(String),
}

impl From<&str> for InputFieldType {
    fn from(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "text" => InputFieldType::Text,
            "textarea" => InputFieldType::TextArea,
            "password" => InputFieldType::Password,
            "email" => InputFieldType::Email,
            "number" => InputFieldType::Number,
            "date" => InputFieldType::Date,
            "time" => InputFieldType::Time,
            "datetime" => InputFieldType::DateTime,
            "dropdown" => InputFieldType::Dropdown,
            "radio" => InputFieldType::RadioGroup,
            "checkbox" => InputFieldType::Checkbox,
            "toggle" => InputFieldType::Toggle,
            "file" => InputFieldType::File,
            "hidden" => InputFieldType::Hidden,
            _ => InputFieldType::Custom(s.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChartSpec {
    pub kind: ChartKind,
    pub label_field: Option<String>,
    pub value_fields: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChartKind {
    Bar,
    Line,
    Pie,
    Radar,
    Metric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PropertyAssignment {
    pub key: String,
    pub value: ValueExpression,
    #[serde(skip, default)]
    pub span: Option<(usize, usize)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ObjectMember {
    pub key: String,
    pub value: ValueExpression,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ValueExpression {
    Identifier(String),
    String(String),
    Number(f64),
    Bool(bool),
    Array(Vec<ValueExpression>),
    Object(Vec<ObjectMember>),
    Call(String, Vec<ValueExpression>),
    FieldAccess {
        object: Box<ValueExpression>,
        field: String,
    },
    BinOp {
        left: Box<ValueExpression>,
        op: BinOp,
        right: Box<ValueExpression>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<ValueExpression>,
    },
    Group(Box<ValueExpression>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventHandler {
    pub event_type: EventType,
    pub params: Vec<String>,
    /// Capability requirements declared as `requires: [CapA, CapB];` between
    /// the event param and the if-condition; empty when unguarded.
    #[serde(default)]
    pub requires: Vec<String>,
    pub condition: Option<Expression>,
    pub action: EventAction,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum EventType {
    Select,
    Submit,
    Click,
    Change,
    Load,
    Save,
    Cancel,
    Delete,
    Confirm,
    Back,
    Custom(String),
}

#[allow(clippy::inherent_to_string)]
impl EventType {
    pub fn as_str(&self) -> &str {
        match self {
            EventType::Select => "select",
            EventType::Submit => "submit",
            EventType::Click => "click",
            EventType::Change => "change",
            EventType::Load => "load",
            EventType::Save => "save",
            EventType::Cancel => "cancel",
            EventType::Delete => "delete",
            EventType::Confirm => "confirm",
            EventType::Back => "back",
            EventType::Custom(s) => s.as_str(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum EventAction {
    Navigate {
        target: String,
        binding: Option<ParameterBinding>,
    },
    Refresh {
        target: String,
        binding: Option<ParameterBinding>,
    },
    ActionInvocation {
        name: String,
        body: Option<ActionBody>,
    },
    Stay,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionBody {
    pub properties: Vec<PropertyAssignment>,
    pub handlers: Vec<EventHandler>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterDecl {
    pub name: String,
    pub type_ref: String,
    #[serde(default)]
    pub default: Option<ValueExpression>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ParameterBinding {
    pub pairs: Vec<(String, Expression)>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum Expression {
    Ident(String),
    StringLit(String),
    NumLit(f64),
    BoolLit(bool),
    FieldExpr {
        object: Box<Expression>,
        field: String,
    },
    BinOp {
        left: Box<Expression>,
        op: BinOp,
        right: Box<Expression>,
    },
    UnaryOp {
        op: UnaryOp,
        operand: Box<Expression>,
    },
    Group(Box<Expression>),
    Call {
        name: String,
        args: Vec<Expression>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    RegexMatch,
    NegRegex,
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    And,
    Or,
}

impl BinOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            BinOp::Eq => "==",
            BinOp::Ne => "!=",
            BinOp::Lt => "<",
            BinOp::Le => "<=",
            BinOp::Gt => ">",
            BinOp::Ge => ">=",
            BinOp::RegexMatch => "~=",
            BinOp::NegRegex => "!~",
            BinOp::Add => "+",
            BinOp::Sub => "-",
            BinOp::Mul => "*",
            BinOp::Div => "/",
            BinOp::Mod => "%",
            BinOp::And => "&&",
            BinOp::Or => "||",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UnaryOp {
    Not,
    Neg,
}

impl UnaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            UnaryOp::Not => "!",
            UnaryOp::Neg => "-",
        }
    }
}

/// Render an expression back to its DSL source form, deterministically:
/// binary operators are surrounded by single spaces, unary prefixes bind
/// directly, and explicit groups keep their parentheses.
pub fn render_expression(expr: &Expression) -> String {
    match expr {
        Expression::Ident(s) => s.clone(),
        Expression::StringLit(s) => format!("\"{s}\""),
        Expression::NumLit(n) => n.to_string(),
        Expression::BoolLit(b) => b.to_string(),
        Expression::FieldExpr { object, field } => {
            format!("{}.{}", render_expression(object), field)
        }
        Expression::BinOp { left, op, right } => format!(
            "{} {} {}",
            render_expression(left),
            op.as_str(),
            render_expression(right)
        ),
        Expression::UnaryOp { op, operand } => {
            format!("{}{}", op.as_str(), render_expression(operand))
        }
        Expression::Group(inner) => format!("({})", render_expression(inner)),
        Expression::Call { name, args } => {
            let args_str: Vec<String> = args.iter().map(render_expression).collect();
            format!("{}({})", name, args_str.join(", "))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionDeclaration {
    pub name: String,
    pub properties: Vec<PropertyAssignment>,
    pub events: Vec<EventHandler>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleDeclaration {
    pub name: String,
    pub input_params: Vec<ParameterDecl>,
    pub output_params: Vec<ParameterDecl>,
    pub properties: Vec<PropertyAssignment>,
    pub containers: Vec<ContainerDeclaration>,
    pub components: Vec<ComponentDeclaration>,
    pub events: Vec<EventHandler>,
}

/// A top-level `actor "Name" { ... }` declaration. Actors carry
/// label-style properties only; event handlers and node persistence
/// are deferred until the roles/permissions slice lands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorDeclaration {
    pub name: String,
    pub properties: Vec<PropertyAssignment>,
}

/// The root of an IFML interaction model: a standalone, versioned wire
/// artifact (see the [wire format contract](crate#wire-format-contract) and
/// the [module docs](self)).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IfmlModel {
    /// Artifact format version. Always [`IFML_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`IfmlModel::from_json`] rejects
    /// anything else.
    #[serde(default)]
    pub format_version: u32,
    #[serde(default)]
    pub domains: Vec<DomainDeclaration>,
    #[serde(default)]
    pub views: Vec<ViewDeclaration>,
    #[serde(default)]
    pub actions: Vec<ActionDeclaration>,
    #[serde(default)]
    pub modules: Vec<ModuleDeclaration>,
    #[serde(default)]
    pub actors: Vec<ActorDeclaration>,
    /// Raw string values of top-level `import "..."` statements, in source
    /// order. Duplicates are preserved; resolution/dedup is a resolver
    /// concern, not a parser concern.
    #[serde(default)]
    pub imports: Vec<String>,
}

impl IfmlModel {
    /// Creates an IFML model artifact with [`IFML_MODEL_FORMAT_VERSION`]
    /// stamped.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        domains: Vec<DomainDeclaration>,
        views: Vec<ViewDeclaration>,
        actions: Vec<ActionDeclaration>,
        modules: Vec<ModuleDeclaration>,
        actors: Vec<ActorDeclaration>,
        imports: Vec<String>,
    ) -> Self {
        Self {
            format_version: IFML_MODEL_FORMAT_VERSION,
            domains,
            views,
            actions,
            modules,
            actors,
            imports,
        }
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserializes an IFML artifact from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`IFML_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != IFML_MODEL_FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: IFML_MODEL_FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for IfmlModel {
    fn default() -> Self {
        Self::new(
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
            Vec::new(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(object: &str, name: &str) -> Expression {
        Expression::FieldExpr {
            object: Box::new(Expression::Ident(object.to_string())),
            field: name.to_string(),
        }
    }

    fn bin_op(left: Expression, op: BinOp, right: Expression) -> Expression {
        Expression::BinOp {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    #[test]
    fn test_render_literals() {
        assert_eq!(render_expression(&Expression::Ident("x".to_string())), "x");
        assert_eq!(
            render_expression(&Expression::StringLit("admin".to_string())),
            "\"admin\""
        );
        assert_eq!(render_expression(&Expression::NumLit(42.0)), "42");
        assert_eq!(render_expression(&Expression::BoolLit(true)), "true");
    }

    #[test]
    fn test_render_field_call_and_unary() {
        assert_eq!(render_expression(&field("row", "active")), "row.active");
        assert_eq!(
            render_expression(&Expression::Call {
                name: "today".to_string(),
                args: vec![field("a", "b")],
            }),
            "today(a.b)"
        );
        assert_eq!(
            render_expression(&Expression::UnaryOp {
                op: UnaryOp::Not,
                operand: Box::new(field("user", "locked")),
            }),
            "!user.locked"
        );
        assert_eq!(
            render_expression(&Expression::UnaryOp {
                op: UnaryOp::Neg,
                operand: Box::new(Expression::NumLit(1.5)),
            }),
            "-1.5"
        );
    }

    #[test]
    fn test_render_bin_ops_and_grouping() {
        let expr = bin_op(
            bin_op(
                field("user", "role"),
                BinOp::Eq,
                Expression::StringLit("admin".to_string()),
            ),
            BinOp::And,
            bin_op(
                field("account", "balance"),
                BinOp::Ge,
                Expression::NumLit(100.0),
            ),
        );
        assert_eq!(
            render_expression(&expr),
            "user.role == \"admin\" && account.balance >= 100"
        );

        let grouped = Expression::Group(Box::new(bin_op(
            Expression::Ident("a".to_string()),
            BinOp::Or,
            Expression::Ident("b".to_string()),
        )));
        assert_eq!(render_expression(&grouped), "(a || b)");

        let regex = bin_op(
            field("row", "name"),
            BinOp::NegRegex,
            Expression::StringLit("^A".to_string()),
        );
        assert_eq!(render_expression(&regex), "row.name !~ \"^A\"");
    }

    #[test]
    fn test_bin_op_and_unary_op_as_str() {
        assert_eq!(BinOp::Eq.as_str(), "==");
        assert_eq!(BinOp::RegexMatch.as_str(), "~=");
        assert_eq!(BinOp::Mod.as_str(), "%");
        assert_eq!(UnaryOp::Not.as_str(), "!");
        assert_eq!(UnaryOp::Neg.as_str(), "-");
    }

    /// A small model exercising views, components, events, expressions, and
    /// actors for the JSON tests below.
    fn sample_model() -> IfmlModel {
        IfmlModel::new(
            vec![DomainDeclaration {
                name: "sales".to_string(),
                schema_name: "sales".to_string(),
            }],
            vec![ViewDeclaration {
                name: "CustomerList".to_string(),
                label: Some("Customers".to_string()),
                is_landmark: true,
                is_xor: false,
                is_modal: false,
                params: vec![ParameterDecl {
                    name: "slug".to_string(),
                    type_ref: "String".to_string(),
                    default: Some(ValueExpression::String("home".to_string())),
                }],
                properties: vec![PropertyAssignment {
                    key: "filter".to_string(),
                    value: ValueExpression::BinOp {
                        left: Box::new(ValueExpression::Identifier("status".to_string())),
                        op: BinOp::Eq,
                        right: Box::new(ValueExpression::String("active".to_string())),
                    },
                    // Parse-only: `span` is skipped on serialize, so serde
                    // round trips always carry None (see
                    // `parse_span_is_never_serialized`).
                    span: None,
                }],
                containers: Vec::new(),
                components: vec![ComponentDeclaration {
                    name: "grid".to_string(),
                    component_type: Some(ComponentType::List),
                    spec: None,
                    properties: Vec::new(),
                    events: vec![EventHandler {
                        event_type: EventType::Select,
                        params: vec!["row".to_string()],
                        requires: vec!["ViewCustomer".to_string()],
                        condition: Some(field("row", "active")),
                        action: EventAction::Navigate {
                            target: "CustomerDetail".to_string(),
                            binding: Some(ParameterBinding {
                                pairs: vec![("customerId".to_string(), field("row", "id"))],
                            }),
                        },
                    }],
                    condition: None,
                }],
                events: Vec::new(),
                module_uses: Vec::new(),
                roles: vec!["admin".to_string()],
                requires: Vec::new(),
                condition: None,
                position: None,
            }],
            Vec::new(),
            Vec::new(),
            vec![ActorDeclaration {
                name: "Admin".to_string(),
                properties: Vec::new(),
            }],
            vec!["rexlang/auth.actor".to_string()],
        )
    }

    #[test]
    fn new_and_default_stamp_format_version() {
        assert_eq!(sample_model().format_version, IFML_MODEL_FORMAT_VERSION);
        assert_eq!(
            IfmlModel::default().format_version,
            IFML_MODEL_FORMAT_VERSION
        );
        assert_eq!(IFML_MODEL_FORMAT_VERSION, 1);
    }

    #[test]
    fn to_json_pretty_round_trips_through_from_json() {
        let model = sample_model();
        let json = model.to_json_pretty().expect("serialize");
        let parsed = IfmlModel::from_json(&json).expect("deserialize");
        assert_eq!(parsed, model);
        assert_eq!(parsed.to_json_pretty().expect("re-serialize"), json);
    }

    #[test]
    fn from_json_rejects_wrong_format_version() {
        let json = r#"{"formatVersion": 2, "domains": [], "views": []}"#;
        match IfmlModel::from_json(json) {
            Err(IrError::UnsupportedFormatVersion { found, expected }) => {
                assert_eq!(found, 2);
                assert_eq!(expected, IFML_MODEL_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
        }
    }

    #[test]
    fn from_json_rejects_missing_format_version() {
        let json = r#"{"domains": [], "views": []}"#;
        match IfmlModel::from_json(json) {
            Err(IrError::UnsupportedFormatVersion { found, expected }) => {
                assert_eq!(found, 0, "an absent version reports as 0");
                assert_eq!(expected, IFML_MODEL_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
        }
    }

    #[test]
    fn from_json_tolerates_unknown_fields() {
        let json = r#"{
            "formatVersion": 1,
            "domains": [],
            "views": [],
            "nobodyKnowsMe": {"nested": [1, 2, 3]}
        }"#;
        let model = IfmlModel::from_json(json).expect("unknown fields are ignored");
        assert!(model.domains.is_empty());
        assert!(model.views.is_empty());
    }

    #[test]
    fn wire_format_is_camel_case_with_type_value_tagged_enums() {
        let json = sample_model().to_json().expect("serialize");
        assert!(json.contains("\"formatVersion\":1"), "{json}");
        assert!(json.contains("\"schemaName\":\"sales\""), "{json}");
        assert!(json.contains("\"isLandmark\":true"), "{json}");
        assert!(
            json.contains("\"componentType\":{\"type\":\"list\"}"),
            "{json}"
        );
        assert!(
            json.contains("\"eventType\":{\"type\":\"select\"}"),
            "{json}"
        );
        assert!(
            json.contains(
                "\"action\":{\"type\":\"navigate\",\"value\":{\"target\":\"CustomerDetail\""
            ),
            "{json}"
        );
        assert!(
            json.contains("\"default\":{\"type\":\"string\",\"value\":\"home\"}"),
            "ValueExpression payloads are adjacent-tagged too: {json}"
        );
    }

    #[test]
    fn unit_only_enums_serialize_as_bare_camel_case_strings() {
        let spec = ChartSpec {
            kind: ChartKind::Bar,
            label_field: Some("month".to_string()),
            value_fields: vec!["sales".to_string()],
        };
        let json = serde_json::to_string(&spec).expect("serialize chart spec");
        assert_eq!(
            json,
            r#"{"kind":"bar","labelField":"month","valueFields":["sales"]}"#
        );
        let json = serde_json::to_string(&BinOp::RegexMatch).expect("serialize bin op");
        assert_eq!(json, r#""regexMatch""#);
    }

    #[test]
    fn parse_span_is_never_serialized() {
        let property = PropertyAssignment {
            key: "mode".to_string(),
            value: ValueExpression::Identifier("edit".to_string()),
            span: Some((12, 34)),
        };
        let json = serde_json::to_string(&property).expect("serialize property");
        assert_eq!(
            json,
            r#"{"key":"mode","value":{"type":"identifier","value":"edit"}}"#
        );
        // `skip` also means deserialize resets the field: spans are
        // parse-only and never survive an artifact round trip.
        let parsed: PropertyAssignment = serde_json::from_str(&json).expect("deserialize property");
        assert_eq!(parsed.span, None);
    }
}
