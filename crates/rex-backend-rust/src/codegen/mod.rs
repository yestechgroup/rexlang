//! Rust code generation from the Core IR.
//!
//! The generator consumes a fully resolved [`rex_ir::Model`] and emits a
//! single `models.rs` implementing the **arena + typed ids** representation
//! required by the project design: bidirectional references and containment
//! cycles cannot be expressed with Rust references, so every object lives in
//! a per-class [`rex_runtime::slotmap::SlotMap`] inside a generated
//! `Resource`, cross references are generated `XxxId` keys, navigation takes
//! `&Resource`, and every mutator takes `&mut Resource` and maintains
//! `opposite` pairs atomically.
//!
//! Mutator shapes per relation pair (cardinalities of the two ends):
//! - many ↔ many: `add`/`remove` on both ends, pushing/removing on both sides.
//! - single ↔ many: `set` on the single end, `add`/`remove` on the many end;
//!   both keep the pair consistent, including detaching from a previous
//!   holder.
//! - single ↔ single: `set` on both ends with full detach/reattach logic.
//! - one-way containment (no opposite): plain owner-side `add`/`remove`.
//!
//! # Tier 1: operation bodies and datatype create/convert (the contract)
//!
//! Operations with a `rust` body are emitted into the class's impl block with
//! a FIXED signature convention; the body text is embedded verbatim
//! (byte-identical to the IR string — spacing, comments, and everything else
//! the author wrote is the generated method's body):
//!
//! ```text
//! pub fn <snake_name>(&self, res: &Resource, <param>: <type>, ...) -> <Ret> { <verbatim body> }
//! ```
//!
//! Type mapping in signatures: primitive `String` stays `String` (body
//! authors clone/deref as needed), `int` → `i32` and friends as for fields,
//! class-typed params and returns map to the typed id (`BookId`), enums,
//! datatypes, and vocabularies to their Rust type. Note the consequences of
//! the convention: a class-typed return means the BODY must produce a
//! `BookId` (e.g. by `.expect`-ing a lookup), and `String` params are owned.
//! A class-typed parameter or return never carries `Option`.
//!
//! Datatypes with `create`/`convert` rust bodies get a small impl block:
//!
//! ```text
//! impl Date {
//!     pub fn create(it: String) -> Self { <verbatim body> }   // body returns Self
//!     pub fn convert(self) -> String { <verbatim body> }      // body may use self.0
//! }
//! ```
//!
//! When a class has NO `rust` body for an operation, nothing is emitted for
//! it: it remains an abstract hook to implement as a hand-written method in
//! your own modules (as in Tier 0). Instance JSON (see `serialize.rs`) is
//! untouched by all of the above: operations and datatype behavior are never
//! serialized.
//!
//! # Tier 2: neutral `expr` bodies
//!
//! An operation body tagged `expr` holds rexlang's neutral expression
//! language (see `docs/EXPRESSIONS.md`). The body text is parsed and
//! type-checked against the model (with the enclosing class as the implicit
//! `self`: its features are in scope, plus the operation's parameters), then
//! lowered to Rust by [`crate::expr_lower`] — the emitted method has the same
//! fixed signature as Tier 1, with the return type mapped from the
//! expression's checked type (`Option<..>` when the expression can be
//! absent, e.g. `first`). Any parse, type, or lowering failure is a
//! `generate` error naming the operation. A derived feature with an `expr`
//! body lowers the same way into a `pub fn <name>(&self, res: &Resource)`
//! accessor on its class. Declaring both a `rust` and an `expr` body for the
//! same operation is a generation error.
//!
//! # Constraint validation
//!
//! Classes with constrained attributes (`minLength`/`maxLength`/
//! `minimum`/`maximum`) carry a `pub fn validate(&self) ->
//! Vec<rex_runtime::validation::Violation>` method reporting one violation
//! per offended bound — per element for many-valued attributes, with the
//! element index prefixed to the detail. Checks follow the attribute's
//! resolved type: length bounds on `string`, datatypes (the generated
//! newtypes wrap `String`), and `String`-keyed vocabularies (via `key()`),
//! numeric bounds on `int`/`long`, enum literal values (via `value()`), and
//! numeric-keyed vocabularies (via the key facet accessor). Bounds the Rust
//! backend checks descriptively in this milestone — `pattern`, enum literal
//! *name* bounds (compile-time complete), and short/byte/float/double
//! bounds — emit no code. Classes with no checkable constraint emit no
//! `validate` at all, so unconstrained models stay byte-identical.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::bail;
use rex_ir::{
    ClassDef, DatatypeDef, EnumDef, Model, Multiplicity, TypeRef, VocabularyDef, VocabularyEntry,
};

use crate::naming::{rust_ident, slotmap_field, snake_case};
/// Generates the Rust code for `model`.
///
/// Returns a map of relative file path to file contents. Currently emits a
/// single `models.rs`.
pub fn generate(model: &Model) -> anyhow::Result<BTreeMap<String, String>> {
    let code = generate_unit(model)?;
    let mut files = BTreeMap::new();
    files.insert("models.rs".to_string(), code);
    Ok(files)
}

/// Generates the Rust code for `model` and writes it into `out_dir`.
pub fn generate_to_dir(model: &Model, out_dir: &Path) -> anyhow::Result<()> {
    let files = generate(model)?;
    rex_backend_common::write_files(&files, out_dir)
}

/// A generated class with everything the emitter needs precomputed.
pub(crate) struct ClassCtx<'a> {
    pub(crate) class: &'a ClassDef,
    /// Owning package name (expressions resolve features within it).
    pub(crate) package: String,
    pub(crate) id_type: String,
    pub(crate) slot_field: String,
    pub(crate) single: String,
}

/// All resolved inputs for one generation unit.
pub(crate) struct Unit<'a> {
    pub(crate) model: &'a Model,
    pub(crate) type_name: String,
    pub(crate) classes: Vec<ClassCtx<'a>>,
    pub(crate) enums: Vec<&'a EnumDef>,
    pub(crate) datatypes: Vec<&'a DatatypeDef>,
    pub(crate) vocabularies: Vec<&'a VocabularyDef>,
}

impl<'a> Unit<'a> {
    pub(crate) fn class(&self, name: &str) -> anyhow::Result<&ClassCtx<'a>> {
        self.classes
            .iter()
            .find(|c| c.class.name == name)
            .ok_or_else(|| anyhow::anyhow!("class '{name}' not found in model"))
    }

    fn first_enum_literal(&self, enum_name: &str) -> anyhow::Result<&'a rex_ir::EnumLiteral> {
        let enum_def = self
            .enums
            .iter()
            .find(|e| e.name == enum_name)
            .ok_or_else(|| anyhow::anyhow!("enum '{enum_name}' not found in model"))?;
        enum_def
            .literals
            .first()
            .ok_or_else(|| anyhow::anyhow!("enum '{enum_name}' has no literals"))
    }

    /// The first vendored entry of a vocabulary, used as its default value.
    fn first_vocabulary_entry(&self, vocabulary_name: &str) -> anyhow::Result<&'a VocabularyEntry> {
        let vocabulary = self
            .vocabularies
            .iter()
            .find(|v| v.name == vocabulary_name)
            .ok_or_else(|| anyhow::anyhow!("vocabulary '{vocabulary_name}' not found in model"))?;
        vocabulary
            .entries
            .first()
            .ok_or_else(|| anyhow::anyhow!("vocabulary '{vocabulary_name}' has no entries"))
    }
}

pub(crate) fn collect_unit(model: &Model) -> anyhow::Result<Unit<'_>> {
    let mut classes = Vec::new();
    let mut seen = BTreeSet::new();
    let mut enums = Vec::new();
    let mut datatypes = Vec::new();
    let mut vocabularies = Vec::new();
    let mut seen_vocabularies = BTreeSet::new();
    for package in &model.packages {
        enums.extend(package.enums.iter());
        datatypes.extend(package.datatypes.iter());
        for vocabulary in &package.vocabularies {
            if !seen_vocabularies.insert(vocabulary.name.clone()) {
                bail!(
                    "duplicate vocabulary name '{}' across packages; the Rust backend flattens \
                     packages into one module and requires unique vocabulary names",
                    vocabulary.name
                );
            }
            vocabularies.push(vocabulary);
        }
        for class in &package.classes {
            if !seen.insert(class.name.clone()) {
                bail!(
                    "duplicate class name '{}' across packages; the Rust backend flattens \
                     packages into one module and requires unique class names",
                    class.name
                );
            }
            classes.push(ClassCtx {
                class,
                package: package.name.clone(),
                id_type: format!("{}Id", rust_ident(&class.name)),
                slot_field: slotmap_field(&class.name),
                single: snake_case(&class.name),
            });
        }
    }
    let type_name = model
        .packages
        .first()
        .map(|p| p.name.clone())
        .unwrap_or_else(|| "model".to_string());
    Ok(Unit {
        model,
        type_name,
        classes,
        enums,
        datatypes,
        vocabularies,
    })
}

fn generate_unit(model: &Model) -> anyhow::Result<String> {
    let unit = collect_unit(model)?;
    let mut e = String::new();
    emit_header(&mut e, model, &unit);
    emit_new_key_types(&mut e, &unit);
    e.push('\n');
    emit_datatypes(&mut e, &unit);
    e.push('\n');
    emit_enums(&mut e, &unit)?;
    e.push('\n');
    emit_vocabularies(&mut e, &unit)?;
    for class in &unit.classes {
        e.push('\n');
        emit_struct(&mut e, &unit, class)?;
    }
    emit_functions(&mut e, &unit)?;
    e.push('\n');
    emit_resource(&mut e, &unit)?;
    e.push('\n');
    crate::serialize::emit(&mut e, &unit)?;
    Ok(e)
}

/// The sigil-lowered package functions ([`rex_ir::Package::functions`],
/// wire contract rule 15) as free functions in the flattened module.
///
/// Body policy: a `rust` body emits verbatim (the class-operation
/// convention); a `rosetta`-only or body-less function — today every
/// lowered Rune function — emits a `todo!()` stub named after the
/// function, because the verbatim Rune text is never emitted as code (wire
/// contract rule 15). Bare `expr`-body calls (issue #63) lower to exactly
/// these stubs. An `expr` body *on a function itself* is a generate-time
/// error: function bodies are the Rune statement IR's job
/// (docs/RUNE-BODIES.md), not the neutral expression language's. Function
/// names must stay unique after snake_case (two packages declaring the
/// same name, or `fooBar`/`foo_bar`, collide in the flattened module) —
/// generation bails naming both declarations. Emits nothing for
/// function-less models, keeping their generated code byte-identical.
fn emit_functions(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for package in &unit.model.packages {
        for function in &package.functions {
            let snake = rust_ident(&snake_case(&function.name));
            if let Some(first) = seen.get(&snake) {
                bail!(
                    "duplicate function name '{snake}' ('{first}' and '{}') across packages; \
                     the Rust backend flattens packages into one module and requires unique \
                     snake_case function names",
                    function.name
                );
            }
            seen.insert(snake, function.name.clone());
        }
    }
    for package in &unit.model.packages {
        for function in &package.functions {
            let expr_body = function.bodies.get("expr");
            let rust_body = function.bodies.get("rust");
            if let (Some(_), Some(_)) = (expr_body, rust_body) {
                bail!(
                    "function '{}' has conflicting 'rust' and 'expr' bodies; \
                     declare exactly one body per function",
                    function.name
                );
            }
            if expr_body.is_some() {
                bail!(
                    "function '{}' has an 'expr' body; function bodies are not lowered yet \
                     (docs/RUNE-BODIES.md)",
                    function.name
                );
            }
            let params = function
                .params
                .iter()
                .map(|param| {
                    Ok(format!(
                        "{}: {}",
                        rust_ident(&snake_case(&param.name)),
                        fn_param_type(&param.type_, param.multiplicity.as_ref())?
                    ))
                })
                .collect::<anyhow::Result<Vec<_>>>()?
                .join(", ");
            let ret = fn_return_type(&function.return_type, function.return_multiplicity.as_ref())?;
            let snake = rust_ident(&snake_case(&function.name));
            e.push('\n');
            match rust_body {
                Some(body) => e.push_str(&format!(
                    "/// Package function `{name}` (rust body embedded verbatim).\n\
                     pub fn {snake}({params}) -> {ret} {{\n{body}\n}}\n",
                    name = function.name,
                )),
                None => {
                    let mut doc = format!("/// Package function `{}`", function.name);
                    if let Some(description) = &function.description {
                        doc.push_str(&format!(": {description}"));
                    }
                    e.push_str(&format!(
                        "{doc}\n\
                         /// Stub: the verbatim `rosetta` body stays in the IR (wire\n\
                         /// contract rule 15); nothing executes Rune bodies yet.\n\
                         pub fn {snake}({params}) -> {ret} {{\n\
                         \x20   todo!()\n\
                         }}\n",
                    ));
                }
            }
        }
    }
    Ok(())
}

/// The Rust type of one package-function parameter: the `op_type` mapping,
/// wrapped in `Vec<..>` when the parameter carries a many multiplicity (the
/// feature-field convention; sigil `is_array` parameters carry `0..*`).
fn fn_param_type(type_: &TypeRef, multiplicity: Option<&Multiplicity>) -> anyhow::Result<String> {
    let inner = op_type(type_)?;
    Ok(match multiplicity {
        Some(multiplicity) if multiplicity.is_many() => format!("Vec<{inner}>"),
        _ => inner,
    })
}

/// The Rust return type of one package function: the `fn_param_type`
/// convention for the return position, plus the `Option<..>` lift a `0..1`
/// cardinality takes in field position (lower 0) — the same mapping rex-expr
/// applies when typing a call to the function, so a lowered Rune output
/// cardinality (`return_multiplicity`) shapes the stub the call sites type
/// against.
fn fn_return_type(type_: &TypeRef, multiplicity: Option<&Multiplicity>) -> anyhow::Result<String> {
    let inner = op_type(type_)?;
    Ok(match multiplicity {
        Some(multiplicity) if multiplicity.is_many() => format!("Vec<{inner}>"),
        Some(multiplicity) if multiplicity.lower == 0 => format!("Option<{inner}>"),
        _ => inner,
    })
}

mod classes;
mod ops;
mod resource;
#[cfg(test)]
mod tests;
mod types;
mod validate;
mod values;

use classes::emit_struct;
use ops::{lower_derived_expr_body, lower_op_expr_body};
use resource::emit_resource;
use types::default_expr;
use types::op_type;
use values::{emit_datatypes, emit_enums, emit_header, emit_new_key_types, emit_vocabularies};
