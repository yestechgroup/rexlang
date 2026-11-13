//! Executable service/repository operation bodies (issue rexlang#178).
//!
//! A `.ddd` operation may carry a closed procedural statement body:
//! `let`, `let … else error …`, assignment, `guard … else error …`,
//! `if`/`else`, and `return`. The body is typed against:
//!
//! - the operation's declared parameters (lifted per the parameter rules),
//! - the operation's output (a service body's optional return type, a
//!   repository operation's declared return type),
//! - synthetic repository classes registered into a clone of the union's
//!   [`rex_expr::DomainTypes`]: empty features and the repository's IR
//!   operations, with the four body-callable builtins synthesized
//!   (`findById`, `findAll`, `save`, `delete`; `findByExample`/`findByKeys`
//!   are deliberately absent so a call is the named unknown-operation
//!   error).
//!
//! Receiver bindings: a service body's `=> <Repository>` is the primary
//! repository bound as `self`; every *other* repository the service injects
//! binds by its bare dependency name. A repository body binds its own
//! design's repository as `self`. An unresolved service receiver is `E0359`;
//! every typing/flow failure is `E0360` (the closed-subset contract — unlike
//! the sigil Rune bodies, `.ddd` never attaches an unchecked body).
//!
//! The checked body lowers to the operation's `bodies["stmt"]` target as the
//! canonical JSON of [`rex_expr::stmt::StmtBody`], and a service body also
//! records its primary receiver in `bodyReceiver`. Domain-union deferral is
//! the existing discipline: when the union did not lower, the domain
//! diagnostics already block the artifact and body lowering is skipped.

use std::collections::BTreeMap;

use rex_diagnostics::ddd::{E_INVALID_OPERATION_BODY, E_UNKNOWN_BODY_RECEIVER};
use rex_expr::body::BodyOutput;
use rex_expr::stmt::{check_stmt_body, BodyRaise, Stmt, StmtBody, StmtReceiver};
use rex_expr::DomainTypes;
use rex_ir::ddd::{Design, Repository, ServiceOperation};
use rex_ir::{Multiplicity, Operation, OperationParam, PrimitiveType, TypeRef};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::lower::{self, DomainPackage};

use super::{class_features, class_type_ref};

/// The synthetic package key every injected-repository class registers
/// under. A `.mox` package name is a dotted identifier sequence, so this
/// angle-bracketed key cannot collide with a domain package.
const SYNTHETIC_PACKAGE: &str = "<ddd>";

/// One repository design of the module, as the body checker needs it: the
/// declared repository name, the synthetic [`DomainTypes`] class key, and
/// the body-callable operations (the repository's IR operations with the
/// four builtin signatures synthesized).
struct RepositoryInfo {
    name: String,
    key: String,
    operations: Vec<Operation>,
}

impl RepositoryInfo {
    fn receiver<'a>(&'a self, name: &'a str) -> StmtReceiver<'a> {
        StmtReceiver {
            name,
            package: SYNTHETIC_PACKAGE,
            class: &self.key,
        }
    }
}

/// Stage 5c — one module's executable operation bodies, attached after its
/// services and designs lowered (the bodies read the lowered signatures,
/// the designed classes, and the repositories) and before the deferred
/// passes. Returns the stage's diagnostics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_module_bodies(
    module: &dsl::DddModule,
    source: &str,
    packages: &[DomainPackage],
    base: Option<&str>,
    union: Option<&rex_ir::Model>,
    expr_context: Option<&rex_expr::TypeContext>,
    services: &mut [rex_ir::ddd::Service],
    designs: &mut [Design],
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    let (Some(union), Some(expr_context)) = (union, expr_context) else {
        // The union failed to lower: its domain diagnostics already block
        // the artifact, and the typed body checks defer (the existing
        // discipline).
        return local;
    };

    let repositories = collect_repositories(module, packages, base, designs, union);
    if repositories.is_empty() && !body_operations(module) {
        return local;
    }

    // The synthetic repository classes register into a clone of the shared
    // union context; the union itself is never mutated.
    let mut context = expr_context.clone();
    for repository in &repositories {
        context.insert_class(
            SYNTHETIC_PACKAGE,
            &repository.key,
            Vec::new(),
            Vec::new(),
            repository.operations.clone(),
        );
    }

    // Stage 5c-1 — service bodies. The IR services and AST services are in
    // declaration order (one lowered service per declaration), so the zip
    // is the name-independent pairing the other stages rely on.
    for (service, service_out) in module.services.iter().zip(services.iter_mut()) {
        for operation in &service.operations {
            let Some(body) = &operation.body else {
                continue;
            };
            let Some(operation_out) = service_out
                .operations
                .iter_mut()
                .find(|candidate| candidate.name == operation.name.text)
            else {
                continue;
            };
            let Some(receiver) = &operation.body_receiver else {
                // The parser always sets a receiver on a service body form;
                // without one there is nothing to bind `self` to.
                continue;
            };
            let context_label = format!(
                "operation '{}' of service '{}'",
                operation.name.text, service.name.text
            );
            let Some(primary) = repositories
                .iter()
                .find(|repository| repository.name == receiver.text)
            else {
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_UNKNOWN_BODY_RECEIVER} unknown body receiver '{}' on \
                             {context_label}: module '{}' designs no repository '{}'",
                            receiver.text, module.name.text, receiver.text
                        ),
                        Some(receiver.span),
                    )
                    .with_help(
                        "a service body's `=>` receiver must be a repository designed in \
                         the same module",
                    ),
                );
                continue;
            };
            if !service
                .dependencies
                .iter()
                .any(|dependency| dependency.text == receiver.text)
            {
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_UNKNOWN_BODY_RECEIVER} unknown body receiver '{}' on \
                             {context_label}: the receiver repository is not injected by \
                             the service",
                            receiver.text
                        ),
                        Some(receiver.span),
                    )
                    .with_help(format!("add `inject {};` to the service", receiver.text)),
                );
                continue;
            }

            // `self` is the primary repository; every other injected
            // repository of this module binds by its bare name.
            let mut receivers = vec![primary.receiver("self")];
            for dependency in &service.dependencies {
                if dependency.text == receiver.text {
                    continue;
                }
                if let Some(other) = repositories
                    .iter()
                    .find(|repository| repository.name == dependency.text)
                {
                    if !receivers.iter().any(|bound| bound.name == dependency.text) {
                        receivers.push(other.receiver(&dependency.text));
                    }
                }
            }

            attach_body(
                &context,
                &receivers,
                operation_out,
                body,
                operation.span,
                source,
                "service",
                Some(&receiver.text),
                &mut local,
            );
        }
    }

    // Stage 5c-2 — repository bodies. A repository body binds its own
    // design's repository as `self` and records no `bodyReceiver`.
    for (design, design_out) in module.designs.iter().zip(designs.iter_mut()) {
        let (Some(repository), Some(repository_out)) =
            (&design.repository, &mut design_out.repository)
        else {
            continue;
        };
        let Some(info) = repositories
            .iter()
            .find(|candidate| candidate.name == repository.name.text)
        else {
            continue;
        };
        let receivers = vec![info.receiver("self")];
        for operation in &repository.operations {
            let Some(body) = &operation.body else {
                continue;
            };
            let Some(operation_out) = repository_out
                .operations
                .iter_mut()
                .find(|candidate| candidate.name == operation.name.text)
            else {
                continue;
            };
            attach_body(
                &context,
                &receivers,
                operation_out,
                body,
                operation.span,
                source,
                "repository",
                None,
                &mut local,
            );
        }
    }

    local
}

/// Whether the module carries any body form at all (the cheap pre-check
/// before building the synthetic class index).
fn body_operations(module: &dsl::DddModule) -> bool {
    module
        .services
        .iter()
        .flat_map(|service| service.operations.iter())
        .any(|operation| operation.body.is_some())
        || module
            .designs
            .iter()
            .filter_map(|design| design.repository.as_ref())
            .flat_map(|repository| repository.operations.iter())
            .any(|operation| operation.body.is_some())
}

/// Builds the module's repository-design table: each declared repository,
/// its designed class (resolved silently — the design stage owns the rule-3
/// error for an unresolvable class), the synthetic class key, and the
/// body-callable operations.
fn collect_repositories(
    module: &dsl::DddModule,
    packages: &[DomainPackage],
    base: Option<&str>,
    designs: &[Design],
    union: &rex_ir::Model,
) -> Vec<RepositoryInfo> {
    let mut repositories = Vec::new();
    for (design, design_out) in module.designs.iter().zip(designs.iter()) {
        let (Some(repository), Some(repository_out)) = (&design.repository, &design_out.repository)
        else {
            continue;
        };
        if repositories
            .iter()
            .any(|info: &RepositoryInfo| info.name == repository.name.text)
        {
            continue;
        }
        let mut discarded: Vec<Diagnostic> = Vec::new();
        let help = format!(
            "the design of '{}' in module '{}' must target a class declared in the \
             imported domains",
            design.class.text, module.name.text
        );
        let Some(class) = lower::resolve_ddd_class(
            &class_type_ref(&design.class),
            packages,
            base,
            &help,
            &mut discarded,
        ) else {
            continue;
        };
        let key = format!("{}.{}", module.name.text, repository.name.text);
        let operations = synthetic_operations(&class, repository_out, union);
        repositories.push(RepositoryInfo {
            name: repository.name.text.clone(),
            key,
            operations,
        });
    }
    repositories
}

/// The receiver-callable operations of one repository's synthetic class:
/// every declared operation with a return type (a void operation cannot be
/// called in an expression), plus the four body-callable builtins with
/// synthesized signatures. `findByExample`/`findByKeys` are deliberately
/// omitted — the closed subset refuses them as unknown operations.
fn synthetic_operations(
    class: &(String, String),
    repository: &Repository,
    union: &rex_ir::Model,
) -> Vec<Operation> {
    let entity = TypeRef::Class {
        package: class.0.clone(),
        name: class.1.clone(),
    };
    let id = class_features(union, &class.0, &class.1)
        .and_then(|features| {
            features
                .iter()
                .find(|feature| feature.name == "id" || feature.is_id)
                .map(|feature| feature.type_.clone())
        })
        .unwrap_or(TypeRef::Primitive(PrimitiveType::String));
    let mut operations = Vec::new();
    for operation in &repository.operations {
        if let Some(return_type) = &operation.return_type {
            let mut lowered = Operation::new(
                operation.name.clone(),
                return_type.clone(),
                operation.params.clone(),
            );
            lowered.return_multiplicity = operation.return_multiplicity;
            operations.push(lowered);
            continue;
        }
        let builtin = operation
            .builtin
            .expect("a repository operation carries a builtin or a signature");
        let synthesized = match builtin {
            rex_ir::ddd::BuiltinRepositoryOp::FindById => synthetic_operation(
                "findById",
                entity.clone(),
                vec![param("key", id.clone())],
                Some(Multiplicity::OPTIONAL),
            ),
            rex_ir::ddd::BuiltinRepositoryOp::FindAll => synthetic_operation(
                "findAll",
                entity.clone(),
                Vec::new(),
                Some(Multiplicity::MANY),
            ),
            rex_ir::ddd::BuiltinRepositoryOp::Save => synthetic_operation(
                "save",
                entity.clone(),
                vec![param("entity", entity.clone())],
                None,
            ),
            rex_ir::ddd::BuiltinRepositoryOp::Delete => synthetic_operation(
                "delete",
                TypeRef::Primitive(PrimitiveType::Boolean),
                vec![param("key", id.clone())],
                None,
            ),
            rex_ir::ddd::BuiltinRepositoryOp::FindByExample
            | rex_ir::ddd::BuiltinRepositoryOp::FindByKeys => continue,
        };
        operations.push(synthesized);
    }
    operations
}

fn param(name: &str, type_: TypeRef) -> OperationParam {
    OperationParam {
        name: name.to_string(),
        type_,
        multiplicity: None,
    }
}

fn synthetic_operation(
    name: &str,
    return_type: TypeRef,
    params: Vec<OperationParam>,
    multiplicity: Option<Multiplicity>,
) -> Operation {
    let mut operation = Operation::new(name, return_type, params);
    operation.return_multiplicity = multiplicity;
    operation
}

/// Lowers one AST body to the statement IR, parses every expression slot on
/// its raw span, and type-checks the result. On success the canonical JSON
/// lands in the operation's `bodies["stmt"]` target (and a service body
/// records its `bodyReceiver`).
#[allow(clippy::too_many_arguments)]
fn attach_body(
    context: &DomainTypes,
    receivers: &[StmtReceiver<'_>],
    operation: &mut impl BodyOperation,
    body: &dsl::DddBody,
    operation_span: Span,
    source: &str,
    kind: &str,
    body_receiver: Option<&str>,
    local: &mut Vec<Diagnostic>,
) {
    let mut lowered = lower_body(source, body, operation_span);
    let label = format!("invalid {kind} body in operation '{}'", operation.name());
    if !lowered.parse_errors.is_empty() {
        // A slot that does not parse is already a hard error; running the
        // checker would re-report the same parse failure through `type_in`.
        for (span, message) in lowered.parse_errors {
            local.push(Diagnostic::error(
                format!("{E_INVALID_OPERATION_BODY} {label}: {message}"),
                Some(span),
            ));
        }
        return;
    }

    let output = operation.return_type().map(|type_| BodyOutput {
        name: "return",
        type_,
        multiplicity: operation.return_multiplicity(),
    });
    if let Err(errors) = check_stmt_body(
        context,
        operation.params(),
        output,
        receivers,
        &lowered.body,
    ) {
        for error in errors {
            let span = lowered
                .slots
                .span_for(&error.slot, &error.message)
                .unwrap_or(operation_span);
            local.push(Diagnostic::error(
                format!(
                    "{E_INVALID_OPERATION_BODY} {label}: {}: {}",
                    error.slot, error.message
                ),
                Some(span),
            ));
        }
        return;
    }

    operation.bodies_mut().insert(
        "stmt".to_string(),
        serde_json::to_string(&lowered.body).expect("serialize body"),
    );
    operation.set_body_receiver(body_receiver.map(str::to_string));
}

/// The slice of an operation the body stage reads and writes: service and
/// repository operations share the body target, the output slot, the
/// parameters, and the body receiver (only services set one).
trait BodyOperation {
    fn name(&self) -> &str;
    fn return_type(&self) -> Option<&TypeRef>;
    fn return_multiplicity(&self) -> Option<&Multiplicity>;
    fn params(&self) -> &[OperationParam];
    fn bodies_mut(&mut self) -> &mut BTreeMap<String, String>;
    fn set_body_receiver(&mut self, receiver: Option<String>);
}

impl BodyOperation for ServiceOperation {
    fn name(&self) -> &str {
        &self.name
    }
    fn return_type(&self) -> Option<&TypeRef> {
        self.return_type.as_ref()
    }
    fn return_multiplicity(&self) -> Option<&Multiplicity> {
        self.return_multiplicity.as_ref()
    }
    fn params(&self) -> &[OperationParam] {
        &self.params
    }
    fn bodies_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.bodies
    }
    fn set_body_receiver(&mut self, receiver: Option<String>) {
        self.body_receiver = receiver;
    }
}

impl BodyOperation for rex_ir::ddd::RepositoryOperation {
    fn name(&self) -> &str {
        &self.name
    }
    fn return_type(&self) -> Option<&TypeRef> {
        self.return_type.as_ref()
    }
    fn return_multiplicity(&self) -> Option<&Multiplicity> {
        self.return_multiplicity.as_ref()
    }
    fn params(&self) -> &[OperationParam] {
        &self.params
    }
    fn bodies_mut(&mut self) -> &mut BTreeMap<String, String> {
        &mut self.bodies
    }
    fn set_body_receiver(&mut self, _receiver: Option<String>) {}
}

/// One lowered body: the statement IR, the slot-label → span map its
/// diagnostics use, and any expression-slot parse failures.
struct LoweredBody {
    body: StmtBody,
    slots: SlotSpans,
    parse_errors: Vec<(Span, String)>,
}

/// The diagnostic spans of the statements a [`BodyError`] can name, keyed by
/// the checker's slot label. Errors arrive in statement order; the first
/// unused matching row wins, so repeated labels (two `let`s of one name)
/// still span their own statements.
#[derive(Default)]
struct SlotSpans {
    rows: Vec<SlotSpan>,
}

struct SlotSpan {
    label: String,
    span: Span,
    raise_span: Option<Span>,
    used: bool,
}

impl SlotSpans {
    fn push(&mut self, label: String, span: Span, raise_span: Option<Span>) {
        self.rows.push(SlotSpan {
            label,
            span,
            raise_span,
            used: false,
        });
    }

    fn span_for(&mut self, label: &str, message: &str) -> Option<Span> {
        let raise = message.starts_with("unknown error variant")
            || message.starts_with("error variant")
            || message.starts_with("error argument");
        self.rows
            .iter_mut()
            .find(|row| !row.used && row.label == label)
            .map(|row| {
                row.used = true;
                if raise {
                    row.raise_span.unwrap_or(row.span)
                } else {
                    row.span
                }
            })
    }
}

/// Lowers every statement, recording the expression-slot parse errors and
/// the diagnostic spans. The statement IR is still built on parse errors so
/// the caller can report the first failure per slot.
fn lower_body(source: &str, body: &dsl::DddBody, fallback: Span) -> LoweredBody {
    let mut lowered = LoweredBody {
        body: StmtBody::default(),
        slots: SlotSpans::default(),
        parse_errors: Vec::new(),
    };
    lowered.body.stmts = lower_stmts(
        source,
        &body.stmts,
        fallback,
        &mut lowered.slots,
        &mut lowered.parse_errors,
    );
    lowered
}

fn lower_stmts(
    source: &str,
    stmts: &[dsl::DddStmt],
    fallback: Span,
    slots: &mut SlotSpans,
    errors: &mut Vec<(Span, String)>,
) -> Vec<Stmt> {
    let mut lowered = Vec::new();
    for stmt in stmts {
        lowered.push(match stmt {
            dsl::DddStmt::Let { name, expr, raise } => {
                let slot = match raise {
                    Some(_) => format!("let-else '{}'", name.text),
                    None => format!("let '{}'", name.text),
                };
                let text = slot_text(source, *expr);
                parse_slot(&text, *expr, &slot, errors);
                slots.push(slot.clone(), *expr, raise.as_ref().map(|raise| raise.span));
                match raise {
                    Some(raise) => Stmt::LetElse {
                        name: name.text.clone(),
                        expr: text,
                        error: lower_raise(source, raise, &slot, errors),
                    },
                    None => Stmt::Let {
                        name: name.text.clone(),
                        expr: text,
                    },
                }
            }
            dsl::DddStmt::Assign { path, expr } => {
                let label: Vec<&str> = path.iter().map(|name| name.text.as_str()).collect();
                let slot = format!("assign '{}'", label.join("."));
                let text = slot_text(source, *expr);
                parse_slot(&text, *expr, &slot, errors);
                slots.push(slot, *expr, None);
                Stmt::Assign {
                    path: path.iter().map(|name| name.text.clone()).collect(),
                    expr: text,
                }
            }
            dsl::DddStmt::Guard { expr, raise } => {
                let text = slot_text(source, *expr);
                let slot = format!("guard '{text}'");
                parse_slot(&text, *expr, &slot, errors);
                let lowered_raise = lower_raise(source, raise, &slot, errors);
                slots.push(slot, *expr, Some(raise.span));
                Stmt::Guard {
                    expr: text,
                    error: lowered_raise,
                }
            }
            dsl::DddStmt::If { cond, then, else_ } => {
                let text = slot_text(source, *cond);
                let slot = format!("if '{text}'");
                parse_slot(&text, *cond, &slot, errors);
                slots.push(slot, *cond, None);
                Stmt::If {
                    cond: text,
                    then: lower_stmts(source, then, fallback, slots, errors),
                    else_: lower_stmts(source, else_, fallback, slots, errors),
                }
            }
            dsl::DddStmt::Return { expr } => {
                let text = expr.map(|span| {
                    let text = slot_text(source, span);
                    parse_slot(&text, span, "return", errors);
                    text
                });
                slots.push("return".to_string(), expr.unwrap_or(fallback), None);
                Stmt::Return { expr: text }
            }
        });
    }
    lowered
}

fn lower_raise(
    source: &str,
    raise: &dsl::DddRaise,
    slot: &str,
    errors: &mut Vec<(Span, String)>,
) -> BodyRaise {
    BodyRaise {
        variant: raise.variant.text.clone(),
        args: raise
            .args
            .iter()
            .map(|span| {
                let text = slot_text(source, *span);
                parse_slot(&text, *span, slot, errors);
                text
            })
            .collect(),
    }
}

/// Slices one expression slot's source text (the parser's token spans are
/// exact, so no trimming shifts offsets).
fn slot_text(source: &str, span: Span) -> String {
    source.get(span.start..span.end).unwrap_or("").to_string()
}

/// Parses one expression slot; parse errors are mapped from the
/// expression-relative span to file offsets and labeled with the slot (the
/// message keeps the detail).
fn parse_slot(text: &str, span: Span, slot: &str, errors: &mut Vec<(Span, String)>) {
    let parsed = rex_expr::parse(text);
    for error in parsed.errors {
        let start = span.start.saturating_add(error.span.start).min(span.end);
        let end = span
            .start
            .saturating_add(error.span.end)
            .clamp(start, span.end);
        errors.push(((start..end).into(), format!("{slot}: {}", error.message)));
    }
}
