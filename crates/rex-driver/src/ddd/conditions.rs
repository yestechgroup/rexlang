//! Condition-path resolution for `.ddd` service-operation `when` guards
//! (issue rexlang#180): the driver-side walker over the parsed rex-expr AST
//! that resolves every field path a guard references, lowers the resolved
//! form into the artifact (`conditionPaths`), and diagnoses the unresolvable
//! ones (`E0353`, `E0354`).
//!
//! The split of responsibilities with the type checker is deliberate: a
//! path that **resolves** needs nothing here — the checker types it through
//! the injected-repository scope bindings, and a mistyped comparison
//! surfaces as the ordinary same-type error with the condition span. Only
//! an **unresolvable** path is diagnosed here, and the checker's
//! corresponding unknown-name/unknown-feature error is suppressed by span
//! so one root cause yields one diagnostic.

use std::collections::BTreeSet;

use rex_diagnostics::ddd::{E_AMBIGUOUS_MACHINE_SCOPE, E_UNKNOWN_CONDITION_PATH};
use rex_expr::ast::{Expr, ExprKind, Spanned};
use rex_ir::ddd::{ConditionPath, ConditionScope};

use crate::diagnostic::Diagnostic;

use super::machines::MachineScopes;

/// Everything the walker needs to resolve a guard's field paths: the
/// service's declared injections (the path roots), the module's machine
/// scopes (the own-record spellings), and the domain union (the class
/// feature tables and the enum-literal namespace).
pub(crate) struct PathScan<'a> {
    /// The human context the E0353 message embeds
    /// ("operation 'x' of service 'Y'").
    pub(crate) label: String,
    /// The module name the E0354 message embeds.
    pub(crate) module: &'a str,
    /// The module's machine scopes.
    pub(crate) scopes: &'a MachineScopes,
    /// The op's implicit-self class (the module's unique machine-bearing
    /// class, or the class a resolved transition pin names). A bare
    /// ambiguous machine-field name that is a feature of the self class
    /// resolves there and is not diagnosed.
    pub(crate) self_class: Option<&'a (String, String)>,
    /// The service's injected repositories: the dependency name as declared
    /// → the designed class, package-qualified. First segment of a path
    /// must match one of these names exactly (case-sensitive — the
    /// did-you-mean covers the typo).
    pub(crate) injections: &'a [(String, (String, String))],
    /// Every dependency the service declares (repositories, services, and
    /// searches): a path root matching one of these but not an injected
    /// repository is left to the checker — navigating a service or search
    /// injection has no class to resolve, and the ordinary unknown-name
    /// error says so.
    pub(crate) dependencies: &'a [String],
    /// The fully lowered domain union (feature tables, enum namespace).
    pub(crate) union: &'a rex_ir::Model,
}

/// The walker's output: the resolved rows for the artifact, the
/// expression-relative spans whose checker errors this walker replaced
/// (never both diagnostics for one root cause), and the walker's own
/// diagnostics.
pub(crate) struct ScanOutcome {
    pub(crate) paths: Vec<ConditionPath>,
    pub(crate) suppressed: Vec<rex_expr::ast::Span>,
    pub(crate) diagnostics: Vec<Diagnostic>,
}

/// Resolves every field path of a parsed condition. Rows dedupe; walk order
/// is source order, so the artifact carries the paths in the order the
/// guard mentions them.
pub(crate) fn resolve_condition_paths(ast: &Expr, scan: &PathScan<'_>) -> ScanOutcome {
    let mut out = ScanOutcome {
        paths: Vec::new(),
        suppressed: Vec::new(),
        diagnostics: Vec::new(),
    };
    let mut local: Vec<String> = Vec::new();
    walk(ast, &mut local, scan, &mut out);
    out
}

/// The dotted chain of a left-nested feature-access spine
/// (`a.b.c` → root `a`, members `[b, c]`); `None` when the receiver is not
/// a plain name chain (calls, `?.` on computed receivers, literals).
fn dotted_chain(expr: &Expr) -> Option<(String, Vec<&Spanned<String>>)> {
    let ExprKind::FeatureAccess { receiver, name, .. } = &expr.kind else {
        return None;
    };
    match &receiver.kind {
        ExprKind::Name(root) => Some((root.clone(), vec![name])),
        _ => dotted_chain(receiver).map(|(root, mut rest)| {
            rest.push(name);
            (root, rest)
        }),
    }
}

fn walk(expr: &Expr, local: &mut Vec<String>, scan: &PathScan<'_>, out: &mut ScanOutcome) {
    match &expr.kind {
        ExprKind::FeatureAccess { receiver, .. } => match dotted_chain(expr) {
            // A plain dotted chain: the root decides the scope; the chain
            // is fully handled here (the receiver is a name or a chain).
            Some((root, members)) => resolve_path(&root, &members, expr.span, local, scan, out),
            // A computed receiver (`a.b().c`): walk it; the member access
            // itself resolves through ordinary typing.
            None => walk(receiver, local, scan, out),
        },
        ExprKind::Name(name) => bare_name(name, expr.span, local, scan, out),
        ExprKind::Coalesce { value, default } => {
            walk(value, local, scan, out);
            walk(default, local, scan, out);
        }
        ExprKind::Call { receiver, args, .. } => {
            walk(receiver, local, scan, out);
            for arg in args {
                walk(arg, local, scan, out);
            }
        }
        // A bare function call's name is a package-function reference, not
        // a field path or a scope binding.
        ExprKind::FunctionCall { args, .. } => {
            for arg in args {
                walk(arg, local, scan, out);
            }
        }
        ExprKind::Algebra {
            receiver, lambda, ..
        } => {
            walk(receiver, local, scan, out);
            if let Some((param, body)) = lambda {
                local.push(param.clone());
                walk(body, local, scan, out);
                local.pop();
            }
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            walk(lhs, local, scan, out);
            walk(rhs, local, scan, out);
        }
        ExprKind::Unary { expr: operand, .. } => walk(operand, local, scan, out),
        ExprKind::If { cond, then, else_ } => {
            walk(cond, local, scan, out);
            walk(then, local, scan, out);
            walk(else_, local, scan, out);
        }
        ExprKind::Let { name, init, body } => {
            walk(init, local, scan, out);
            local.push(name.value.clone());
            walk(body, local, scan, out);
            local.pop();
        }
        ExprKind::Lambda { param, body } => {
            local.push(param.clone());
            walk(body, local, scan, out);
            local.pop();
        }
        ExprKind::ListLiteral(items) => {
            for item in items {
                walk(item, local, scan, out);
            }
        }
        // Literals and the date constant carry no names.
        _ => {}
    }
}

/// One bare name reference in a multi-machine module: a uniquely-named
/// machine field resolves (and lowers an own-record path), the
/// enum-derived spelling resolves to its machine's field, an ambiguous
/// field or state name is the `E0354` error. Single-machine modules bind
/// own-record names through the implicit self — nothing to resolve here.
fn bare_name(
    name: &str,
    span: rex_expr::ast::Span,
    local: &[String],
    scan: &PathScan<'_>,
    out: &mut ScanOutcome,
) {
    if !scan.scopes.is_multi_machine() || local.iter().any(|bound| bound == name) {
        return;
    }
    let fields = scan
        .scopes
        .fields
        .iter()
        .filter(|field| field.field == name)
        .collect::<Vec<_>>();
    if let Some(field) = fields.first().copied() {
        if field.unique {
            push_own_record(out, field);
        } else if scan.self_class.is_none() {
            // Without a self scope the ambiguous spelling resolves to
            // nothing — diagnosed, never guessed. With a pin (or the
            // module's unique machine), the name is an ordinary feature of
            // the self class and the checker types it.
            ambiguous_field(name, span, scan, out);
        }
        return;
    }
    if let Some(field) = scan.scopes.fields.iter().find(|field| field.alias == name) {
        push_own_record(out, field);
        return;
    }
    if scan
        .scopes
        .ambiguous_states
        .iter()
        .any(|state| state == name)
    {
        out.suppressed.push(span);
        out.diagnostics.push(
            Diagnostic::error(
                format!(
                    "{E_AMBIGUOUS_MACHINE_SCOPE} ambiguous machine scope: '{name}' is \
                     declared by more than one machine in module '{}'",
                    scan.module
                ),
                None,
            )
            .with_help(
                "qualify as `<EnumType>.<State>` — the enum type of the machine \
                 that declares it; a state several machines declare never \
                 binds unqualified",
            ),
        );
    }
}

/// Lowers an own-record path for the machine field `row` names.
fn push_own_record(out: &mut ScanOutcome, field: &super::machines::MachineField) {
    let candidate = ConditionPath::new(
        ConditionScope::OwnRecord,
        field.field.clone(),
        rex_ir::TypeRef::Enum {
            package: field.enum_type.0.clone(),
            name: field.enum_type.1.clone(),
        },
    );
    if !out.paths.contains(&candidate) {
        out.paths.push(candidate);
    }
}

/// The `E0354` for an ambiguous machine-field reference, help naming the
/// enum-derived spellings that resolve it.
fn ambiguous_field(
    name: &str,
    span: rex_expr::ast::Span,
    scan: &PathScan<'_>,
    out: &mut ScanOutcome,
) {
    let spellings: Vec<String> = scan
        .scopes
        .fields
        .iter()
        .filter(|field| field.field == name)
        .map(|field| format!("`{}`", field.alias))
        .collect();
    out.suppressed.push(span);
    out.diagnostics.push(
        Diagnostic::error(
            format!(
                "{E_AMBIGUOUS_MACHINE_SCOPE} ambiguous machine scope: '{name}' is \
                 declared by more than one machine in module '{}'",
                scan.module
            ),
            None,
        )
        .with_help(format!(
            "the state-typed field is carried by more than one design; address \
             it by its machine's enum type in lowerCamel: {}",
            spellings.join(" or ")
        )),
    );
}

/// One dotted path: the root decides the scope, the members resolve as
/// fields of the injected repository's designed class.
#[allow(clippy::too_many_arguments)]
fn resolve_path(
    root: &str,
    members: &[&Spanned<String>],
    root_span: rex_expr::ast::Span,
    local: &[String],
    scan: &PathScan<'_>,
    out: &mut ScanOutcome,
) {
    // First segment must match an injected repository exactly as declared
    // (case-sensitive; the did-you-mean covers the typo).
    let Some((_, class)) = scan.injections.iter().find(|(name, _)| name == root) else {
        // A path through a declared injection that is not a repository
        // (a service or a search) has no class to resolve: the ordinary
        // unknown-name error says so. Anything else unbound is the E0353
        // error — the checker's duplicate is suppressed by span.
        let declared = scan.dependencies.iter().any(|dep| dep == root);
        if !declared && !bound_elsewhere(root, local, scan) {
            let candidates: BTreeSet<String> = scan
                .dependencies
                .iter()
                .cloned()
                .collect::<BTreeSet<String>>();
            let suggestion = rex_diagnostics::did_you_mean_adaptive(root, &candidates);
            let help = match &suggestion {
                Some(suggestion) => format!("did you mean '{suggestion}'?"),
                None if candidates.is_empty() => "the service declares no injections".to_string(),
                None => format!(
                    "the service injects: {}",
                    candidates.iter().cloned().collect::<Vec<_>>().join(", ")
                ),
            };
            out.suppressed.push(root_span);
            out.diagnostics.push(
                Diagnostic::error(
                    format!(
                        "{E_UNKNOWN_CONDITION_PATH} unknown condition path '{root}' in {}",
                        scan.label
                    ),
                    None,
                )
                .with_help(help),
            );
        }
        return;
    };
    resolve_field_chain(root, members, class, scan, out);
}

/// Whether the path root binds through any other scope mechanism: a unique
/// machine field or alias, a machine state, an enum of the union (the
/// rule-L3 literal namespace), a local `let`/lambda binding (tracked by
/// the walk), or a feature of the op's self class. The checker types those
/// paths ordinarily.
fn bound_elsewhere(root: &str, local: &[String], scan: &PathScan<'_>) -> bool {
    local.iter().any(|bound| bound == root)
        || scan
            .scopes
            .fields
            .iter()
            .any(|field| (field.unique && field.field == root) || field.alias == root)
        || scan
            .scopes
            .machines
            .iter()
            .any(|machine| machine.states.iter().any(|state| state == root))
        || scan
            .union
            .packages
            .iter()
            .any(|package| package.enums.iter().any(|enum_| enum_.name == root))
        || scan.self_class.is_some_and(|(package, class)| {
            super::class_features(scan.union, package, class)
                .is_some_and(|features| features.iter().any(|feature| feature.name == root))
        })
}

/// Resolves the dotted remainder of an injected path against the designed
/// class: every segment must name a feature of the class reached so far,
/// and the row carries the final segment's declared type. A deeper chain
/// flattens into the dotted field path (`holder.name`).
fn resolve_field_chain(
    root: &str,
    members: &[&Spanned<String>],
    class: &(String, String),
    scan: &PathScan<'_>,
    out: &mut ScanOutcome,
) {
    let mut current = class.clone();
    let mut field_path: Vec<String> = Vec::new();
    let mut field_type: Option<rex_ir::TypeRef> = None;
    for member in members {
        let feature =
            super::class_features(scan.union, &current.0, &current.1).and_then(|features| {
                features
                    .iter()
                    .find(|feature| feature.name == member.value)
                    .cloned()
            });
        let Some(feature) = feature else {
            let class_features = super::class_features(scan.union, &current.0, &current.1)
                .map(|features| {
                    features
                        .iter()
                        .map(|feature| feature.name.clone())
                        .collect::<BTreeSet<String>>()
                })
                .unwrap_or_default();
            let suggestion = rex_diagnostics::did_you_mean_adaptive(&member.value, &class_features);
            let help = match &suggestion {
                Some(suggestion) => format!("did you mean '{suggestion}'?"),
                None if class_features.is_empty() => "the class carries no features".to_string(),
                None => format!(
                    "the class declares: {}",
                    class_features
                        .iter()
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            out.suppressed.push(member.span);
            out.diagnostics.push(
                Diagnostic::error(
                    format!(
                        "{E_UNKNOWN_CONDITION_PATH} unknown field '{}' on '{}' in {}",
                        member.value, current.1, scan.label
                    ),
                    None,
                )
                .with_help(help),
            );
            return;
        };
        field_path.push(member.value.clone());
        field_type = Some(feature.type_.clone());
        if let rex_ir::TypeRef::Class { package, name } = &feature.type_ {
            current = (package.clone(), name.clone());
        } else {
            // Navigating into a non-class: the remaining segments are the
            // checker's business (it reports the mistyped access); the
            // resolved prefix still lowers as the row.
            break;
        }
    }
    let Some(field_type) = field_type else {
        return;
    };
    let candidate = ConditionPath::new(
        ConditionScope::Injected {
            repository: root.to_string(),
            class: class.1.clone(),
        },
        field_path.join("."),
        field_type,
    );
    if !out.paths.contains(&candidate) {
        out.paths.push(candidate);
    }
}
