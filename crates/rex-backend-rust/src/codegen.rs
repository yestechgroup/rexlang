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
    ClassDef, DatatypeDef, DefaultValue, EnumDef, Feature, FeatureKind, Model, Operation,
    PrimitiveType, TypeRef, VocabularyDef, VocabularyEntry, VocabularyFacet,
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
    for (relative, contents) in &files {
        let path = out_dir.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, contents)?;
    }
    Ok(())
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

fn collect_unit(model: &Model) -> anyhow::Result<Unit<'_>> {
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

fn value_type(type_: &TypeRef) -> anyhow::Result<String> {
    Ok(match type_ {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => "String".to_string(),
            PrimitiveType::Int => "i32".to_string(),
            PrimitiveType::Long => "i64".to_string(),
            PrimitiveType::Short => "i16".to_string(),
            PrimitiveType::Float => "f32".to_string(),
            PrimitiveType::Double => "f64".to_string(),
            PrimitiveType::Boolean => "bool".to_string(),
            PrimitiveType::Byte => "i8".to_string(),
            PrimitiveType::Char => "char".to_string(),
            // The calendar date primitive maps to the runtime support type:
            // a newtype over days since the epoch with ISO-8601
            // serialization (issue #9).
            PrimitiveType::Date => "rex_runtime::Date".to_string(),
        },
        TypeRef::Class { name, .. }
        | TypeRef::Enum { name, .. }
        | TypeRef::Datatype { name, .. }
        | TypeRef::Vocabulary { name, .. } => rust_ident(name),
        TypeRef::Interface { name, .. } => {
            bail!("interfaces are not materialized by the Rust backend yet ('{name}')")
        }
    })
}

fn is_reference(kind: FeatureKind) -> bool {
    matches!(
        kind,
        FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container
    )
}

/// The Rust type of an operation signature slot (param or return), per the
/// Tier 1 convention: classes map to their typed id (`BookId`), enums,
/// datatypes, and vocabularies to their Rust type, primitives as for fields.
fn op_type(type_: &TypeRef) -> anyhow::Result<String> {
    match type_ {
        TypeRef::Class { name, .. } => Ok(format!("{}Id", rust_ident(name))),
        TypeRef::Interface { name, .. } => {
            bail!("interfaces are not materialized by the Rust backend yet ('{name}')")
        }
        other => value_type(other),
    }
}

fn feature_doc(feature: &Feature) -> &'static str {
    match feature.kind {
        FeatureKind::Attribute => "attribute",
        FeatureKind::Containment => "contains",
        FeatureKind::CrossReference => "refers",
        FeatureKind::Container => "container",
    }
}

/// The stored field type for a feature.
///
/// Many features store `Vec<T>`; references store ids (`Option<T>` when
/// optional — ids cannot be defaulted); scalar attributes are `T` when
/// required (`lower >= 1`) and `Option<T>` when optional.
fn field_type(_unit: &Unit<'_>, feature: &Feature) -> anyhow::Result<String> {
    if feature.is_derived {
        bail!(
            "derived feature '{}' should not be materialized",
            feature.name
        );
    }
    let inner = match feature.kind {
        FeatureKind::Attribute => value_type(&feature.type_)?,
        FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container => {
            match &feature.type_ {
                TypeRef::Class { name, .. } => format!("{}Id", rust_ident(name)),
                other => bail!(
                    "feature '{}' must have a class type, found {}",
                    feature.name,
                    other.qualified_name().unwrap_or_else(|| "primitive".into())
                ),
            }
        }
    };
    Ok(if feature.multiplicity.is_many() {
        format!("Vec<{inner}>")
    } else if is_reference(feature.kind) || feature.multiplicity.lower == 0 {
        format!("Option<{inner}>")
    } else {
        inner
    })
}

/// The initial value expression for a required scalar attribute field.
fn default_expr(unit: &Unit<'_>, feature: &Feature) -> anyhow::Result<String> {
    // A declared default on a vocabulary-typed attribute names an entry key.
    if let (Some(DefaultValue::String(key)), TypeRef::Vocabulary { name, .. }) =
        (&feature.default, &feature.type_)
    {
        let vocabulary = unit
            .vocabularies
            .iter()
            .find(|v| v.name == *name)
            .ok_or_else(|| anyhow::anyhow!("vocabulary '{name}' not found in model"))?;
        if !vocabulary.entries.iter().any(|entry| entry.key == *key) {
            bail!("default '{key}' is not an entry of vocabulary '{name}'");
        }
        return Ok(format!("{}::{}", rust_ident(name), variant_ident(key)?));
    }
    if let Some(default) = &feature.default {
        return Ok(match default {
            DefaultValue::String(value) => match &feature.type_ {
                // A datatype-typed attribute stores the wrapped String: the
                // default must construct the newtype, not the bare inner
                // value.
                TypeRef::Datatype { name, .. } => {
                    format!("{}({value:?}.to_string())", rust_ident(name))
                }
                _ => format!("{value:?}.to_string()"),
            },
            DefaultValue::Int(value) => value.to_string(),
            DefaultValue::Bool(value) => value.to_string(),
            DefaultValue::EnumLiteral(literal) => {
                let enum_name = match &feature.type_ {
                    TypeRef::Enum { name, .. } => rust_ident(name),
                    other => bail!(
                        "default EnumLiteral on non-enum feature '{}' ({})",
                        feature.name,
                        other.qualified_name().unwrap_or_default()
                    ),
                };
                format!("{enum_name}::{}", rust_ident(literal))
            }
        });
    }
    Ok(match &feature.type_ {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => "String::new()".to_string(),
            PrimitiveType::Int
            | PrimitiveType::Long
            | PrimitiveType::Short
            | PrimitiveType::Byte => "0".to_string(),
            PrimitiveType::Float | PrimitiveType::Double => "0.0".to_string(),
            PrimitiveType::Boolean => "false".to_string(),
            PrimitiveType::Char => "'\\0'".to_string(),
            // Dates take no declared defaults (the .mox surface rejects
            // them); the implicit default of a required date attribute is
            // the epoch 1970-01-01.
            PrimitiveType::Date => "rex_runtime::Date::EPOCH".to_string(),
        },
        TypeRef::Enum { name, .. } => {
            let first = unit.first_enum_literal(name)?;
            format!("{}::{}", rust_ident(name), rust_ident(&first.name))
        }
        TypeRef::Datatype { name, .. } => format!("{}::default()", rust_ident(name)),
        TypeRef::Vocabulary { name, .. } => {
            let first = unit.first_vocabulary_entry(name)?;
            format!("{}::{}", rust_ident(name), variant_ident(&first.key)?)
        }
        TypeRef::Class { name, .. } => bail!(
            "required attribute '{}' has class type '{name}' with no default",
            feature.name
        ),
        TypeRef::Interface { name, .. } => bail!(
            "required attribute '{}' has interface type '{name}'",
            feature.name
        ),
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
    e.push('\n');
    emit_resource(&mut e, &unit)?;
    e.push('\n');
    crate::serialize::emit(&mut e, &unit)?;
    Ok(e)
}

fn emit_header(e: &mut String, model: &Model, unit: &Unit<'_>) {
    e.push_str("//! Generated by rexlang — DO NOT EDIT.\n//!\n");
    e.push_str(&format!(
        "//! Source model: formatVersion {}, rexVersion {}.\n//!\n",
        model.format_version,
        model.rex_version.as_deref().unwrap_or("unknown")
    ));
    e.push_str("//! Notes:\n");
    e.push_str(
        "//! - operations with a `rust` body are emitted as methods on the class\n\
         //!   (the body text is embedded verbatim); body-less operations stay\n\
         //!   abstract — implement them as hand-written methods in your own modules;\n",
    );
    if unit
        .classes
        .iter()
        .any(|c| c.class.features.iter().any(|f| f.is_derived))
    {
        e.push_str(
            "//! - derived features are declared in the IR but not materialized as fields;\n",
        );
    }
    e.push_str(
        "//! - datatype target bindings are recorded as doc comments on the newtypes,\n\
         //!   never as dependencies; interface conformance is not materialized yet.\n",
    );
    e.push_str(
        "//! - vocabulary-typed attributes serialize their entry key string;\n\
         //!   loading resolves the key through the vocabulary's `TryFrom<&str>`.\n",
    );
    if unit.classes.iter().any(|c| !c.class.extends.is_empty()) {
        e.push_str(
            "//! - inheritance declared via `extends` is not materialized in this milestone.\n",
        );
    }
    e.push_str("\nuse rex_runtime::slotmap::SlotMap;\n");
}

fn emit_new_key_types(e: &mut String, unit: &Unit<'_>) {
    if unit.classes.is_empty() {
        return;
    }
    e.push_str("rex_runtime::slotmap::new_key_type! {\n");
    for class in &unit.classes {
        e.push_str(&format!("    pub struct {};\n", class.id_type));
    }
    e.push_str("}\n");
}

fn emit_datatypes(e: &mut String, unit: &Unit<'_>) {
    for datatype in &unit.datatypes {
        e.push_str("/// Opaque datatype newtype.\n");
        if let Some(platform) = &datatype.platform {
            e.push_str(&format!("/// Declared `wraps {platform}`.\n"));
        }
        if datatype.target_bindings.is_empty() {
            e.push_str("/// No target bindings declared.\n");
        } else {
            e.push_str("/// Target bindings:\n");
            for (target, type_name) in &datatype.target_bindings {
                e.push_str(&format!("/// - {target}: `{type_name}`\n"));
            }
        }
        e.push_str("#[derive(Debug, Clone, PartialEq, Eq, Default, Hash)]\n");
        e.push_str(&format!(
            "pub struct {}(pub String);\n",
            rust_ident(&datatype.name)
        ));
        // Tier 1: create/convert bodies become a small impl block. `create`
        // takes the wrapped value and returns `Self`; `convert` consumes
        // `self` and returns the wrapped `String`. Bodies are verbatim.
        let create = datatype.create.get("rust");
        let convert = datatype.convert.get("rust");
        if create.is_some() || convert.is_some() {
            e.push_str(&format!("impl {} {{\n", rust_ident(&datatype.name)));
            if let Some(body) = create {
                e.push_str(&format!(
                    "    /// Tier 1 `create` body, embedded verbatim.\n    pub fn create(it: String) -> Self {{\n{body}\n    }}\n"
                ));
            }
            if let Some(body) = convert {
                e.push_str(&format!(
                    "    /// Tier 1 `convert` body, embedded verbatim.\n    pub fn convert(self) -> String {{\n{body}\n    }}\n"
                ));
            }
            e.push_str("}\n");
        }
    }
}

fn emit_enums(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    for enum_def in &unit.enums {
        if enum_def.literals.is_empty() {
            bail!("enum '{}' has no literals", enum_def.name);
        }
        let name = rust_ident(&enum_def.name);
        e.push_str("/// Generated enum.\n");
        e.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
        e.push_str("#[repr(i64)]\n");
        e.push_str(&format!("pub enum {name} {{\n"));
        for literal in &enum_def.literals {
            if let Some(label) = &literal.label {
                e.push_str(&format!(
                    "    /// Literal value {}, label {:?}.\n",
                    literal.value, label
                ));
            }
            e.push_str(&format!(
                "    {} = {},\n",
                rust_ident(&literal.name),
                literal.value
            ));
        }
        e.push_str("}\n\n");

        e.push_str(&format!("impl {name} {{\n"));
        e.push_str("    /// The declared literal name.\n");
        e.push_str("    pub fn name(self) -> &'static str {\n        match self {\n");
        for literal in &enum_def.literals {
            e.push_str(&format!(
                "            Self::{} => {:?},\n",
                rust_ident(&literal.name),
                literal.name
            ));
        }
        e.push_str("        }\n    }\n\n");
        e.push_str("    /// The human-readable label if declared, else the literal name.\n");
        e.push_str("    pub fn label(self) -> &'static str {\n        match self {\n");
        for literal in &enum_def.literals {
            let label = literal
                .label
                .clone()
                .unwrap_or_else(|| literal.name.clone());
            e.push_str(&format!(
                "            Self::{} => {:?},\n",
                rust_ident(&literal.name),
                label
            ));
        }
        e.push_str("        }\n    }\n\n");
        e.push_str("    /// The declared integer value.\n");
        e.push_str("    pub fn value(self) -> i64 {\n        self as i64\n    }\n}\n\n");

        // TryFrom<i64>: deduplicate repeated values (first literal wins).
        let mut seen_values: BTreeSet<i64> = BTreeSet::new();
        e.push_str(&format!("impl TryFrom<i64> for {name} {{\n"));
        e.push_str("    type Error = rex_runtime::RexError;\n\n");
        e.push_str("    fn try_from(value: i64) -> Result<Self, Self::Error> {\n");
        e.push_str("        match value {\n");
        for literal in &enum_def.literals {
            if seen_values.insert(literal.value) {
                e.push_str(&format!(
                    "            {} => Ok(Self::{}),\n",
                    literal.value,
                    rust_ident(&literal.name)
                ));
            }
        }
        e.push_str(&format!(
            "            other => Err(rex_runtime::RexError::InvalidInstance {{\n                message: format!(\"unknown {name} value {{other}}\"),\n            }}),\n"
        ));
        e.push_str("        }\n    }\n}\n\n");

        e.push_str(&format!("impl Default for {name} {{\n"));
        e.push_str("    fn default() -> Self {\n");
        e.push_str(&format!(
            "        Self::{}\n",
            rust_ident(&enum_def.literals[0].name)
        ));
        e.push_str("    }\n}\n\n");

        e.push_str(&format!("impl std::fmt::Display for {name} {{\n"));
        e.push_str(
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n\
             \x20       f.write_str(self.label())\n    }\n}\n",
        );
    }
    Ok(())
}

/// A vocabulary entry key mapped to its enum variant identifier.
struct Variant {
    ident: String,
    key: String,
}

/// Maps an entry key to its enum variant identifier.
///
/// Keys must already be valid Rust identifiers (bare keywords are raw
/// escaped as `r#kw`); anything else is a generation-time error.
fn variant_ident(key: &str) -> anyhow::Result<String> {
    let mut chars = key.chars();
    let valid = match chars.next() {
        Some(first) => {
            (first.is_alphabetic() || first == '_')
                && chars.all(|rest| rest.is_alphanumeric() || rest == '_')
        }
        None => false,
    };
    if !valid {
        bail!(
            "vocabulary entry key {key:?} is not a valid Rust identifier; vocabulary keys \
             must be identifiers (letters, digits, underscores; not starting with a digit) \
             so they can name enum variants"
        );
    }
    Ok(rust_ident(key))
}

/// Maps the variants of `vocabulary`, rejecting duplicate keys.
fn variants_of(vocabulary: &VocabularyDef) -> anyhow::Result<Vec<Variant>> {
    let mut variants = Vec::with_capacity(vocabulary.entries.len());
    let mut seen = BTreeSet::new();
    for entry in &vocabulary.entries {
        let ident = variant_ident(&entry.key)?;
        if !seen.insert(ident.clone()) {
            bail!(
                "vocabulary '{}' has duplicate entry key {key:?}",
                vocabulary.name,
                key = entry.key
            );
        }
        variants.push(Variant {
            ident,
            key: entry.key.clone(),
        });
    }
    Ok(variants)
}

/// The Rust type of one facet accessor.
fn facet_rust_type(type_: PrimitiveType) -> &'static str {
    match type_ {
        PrimitiveType::String => "&'static str",
        PrimitiveType::Int | PrimitiveType::Long => "i64",
        PrimitiveType::Short => "i16",
        PrimitiveType::Byte => "i8",
        PrimitiveType::Float | PrimitiveType::Double => "f64",
        PrimitiveType::Boolean => "bool",
        PrimitiveType::Char => "char",
        // Date facets cannot reach the backend through the driver (the
        // driver rejects them); the mapping exists for hand-built IRs, and
        // `facet_literal` refuses the stray `DefaultValue` shapes.
        PrimitiveType::Date => "rex_runtime::Date",
    }
}

/// The literal expression of one facet value.
///
/// Fractional `float`/`double` values are stored as their exact JSON number
/// text in [`DefaultValue::String`] (see rex-vocab's `validate_entries`) and
/// are emitted verbatim as the float literal.
fn facet_literal(facet: &VocabularyFacet, value: &DefaultValue) -> anyhow::Result<String> {
    let unexpected = |expected: &str| {
        anyhow::anyhow!("facet '{}' expects {expected}, found {value:?}", facet.name)
    };
    match (facet.type_, value) {
        (PrimitiveType::String, DefaultValue::String(text)) => Ok(format!("{text:?}")),
        (PrimitiveType::Char, DefaultValue::String(text)) => {
            match text.chars().collect::<Vec<_>>()[..] {
                [c] => Ok(format!("{c:?}")),
                _ => Err(unexpected("a single character")),
            }
        }
        (PrimitiveType::Int | PrimitiveType::Long, DefaultValue::Int(value)) => {
            Ok(value.to_string())
        }
        (PrimitiveType::Short, DefaultValue::Int(value)) => i16::try_from(*value)
            .map(|value| value.to_string())
            .map_err(|_| anyhow::anyhow!("facet '{}' value {value} does not fit i16", facet.name)),
        (PrimitiveType::Byte, DefaultValue::Int(value)) => i8::try_from(*value)
            .map(|value| value.to_string())
            .map_err(|_| anyhow::anyhow!("facet '{}' value {value} does not fit i8", facet.name)),
        (PrimitiveType::Boolean, DefaultValue::Bool(value)) => Ok(value.to_string()),
        (PrimitiveType::Float | PrimitiveType::Double, DefaultValue::Int(value)) => {
            Ok(format!("{value}.0"))
        }
        (PrimitiveType::Float | PrimitiveType::Double, DefaultValue::String(text)) => {
            match text.parse::<f64>() {
                Ok(_) => Ok(text.clone()),
                Err(_) => Err(unexpected("a number")),
            }
        }
        _ => Err(unexpected(facet_rust_type(facet.type_))),
    }
}

/// Emits one closed enum per vocabulary: variants are the entry keys, with
/// `key`/facet accessors, key-based lookup, `Display`, and first-entry
/// `Default`.
fn emit_vocabularies(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    for vocabulary in &unit.vocabularies {
        if vocabulary.entries.is_empty() {
            bail!("vocabulary '{}' has no entries", vocabulary.name);
        }
        let variants = variants_of(vocabulary)?;
        let name = rust_ident(&vocabulary.name);

        e.push_str("/// Generated vocabulary enum.\n");
        e.push_str(&format!(
            "/// Vocabulary {name} from `{source}`{version}, {count} entries.\n",
            source = vocabulary.source,
            version = match &vocabulary.version {
                Some(version) => format!(" version `{version}`"),
                None => String::new(),
            },
            count = variants.len(),
        ));
        e.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
        // `repr(transparent)` is rejected for enums with more than one
        // variant (E0731); `u8` keeps the same plain-value intent.
        e.push_str("#[repr(u8)]\n");
        e.push_str(&format!("pub enum {name} {{\n"));
        for variant in &variants {
            e.push_str(&format!("    {},\n", variant.ident));
        }
        e.push_str("}\n\n");

        e.push_str(&format!("impl {name} {{\n"));
        e.push_str("    /// The declared entry key.\n");
        e.push_str("    pub fn key(self) -> &'static str {\n        match self {\n");
        for variant in &variants {
            e.push_str(&format!(
                "            Self::{} => {:?},\n",
                variant.ident, variant.key
            ));
        }
        e.push_str("        }\n    }\n\n");
        for facet in &vocabulary.facets {
            e.push_str(&format!(
                "    /// The `{facet}` facet of this entry.\n",
                facet = facet.name
            ));
            e.push_str(&format!(
                "    pub fn {field}(self) -> {type_} {{\n        match self {{\n",
                field = rust_ident(&snake_case(&facet.name)),
                type_ = facet_rust_type(facet.type_),
            ));
            for entry in &vocabulary.entries {
                let value = entry.facets.get(&facet.name).ok_or_else(|| {
                    anyhow::anyhow!(
                        "entry '{}' of vocabulary '{}' is missing facet '{}'",
                        entry.key,
                        vocabulary.name,
                        facet.name
                    )
                })?;
                e.push_str(&format!(
                    "            Self::{} => {},\n",
                    variant_ident(&entry.key)?,
                    facet_literal(facet, value)?
                ));
            }
            e.push_str("        }\n    }\n\n");
        }
        e.push_str("}\n\n");

        e.push_str(&format!("impl TryFrom<&str> for {name} {{\n"));
        e.push_str("    type Error = rex_runtime::RexError;\n\n");
        e.push_str("    fn try_from(key: &str) -> Result<Self, Self::Error> {\n");
        e.push_str("        match key {\n");
        for variant in &variants {
            e.push_str(&format!(
                "            {key:?} => Ok(Self::{}),\n",
                variant.ident,
                key = variant.key
            ));
        }
        e.push_str(&format!(
            "            other => Err(rex_runtime::RexError::InvalidInstance {{\n                message: format!(\"unknown {name} entry key {{other:?}}\"),\n            }}),\n"
        ));
        e.push_str("        }\n    }\n}\n\n");

        e.push_str(&format!("impl Default for {name} {{\n"));
        e.push_str("    fn default() -> Self {\n");
        e.push_str(&format!("        Self::{}\n", variants[0].ident));
        e.push_str("    }\n}\n\n");

        e.push_str(&format!("impl std::fmt::Display for {name} {{\n"));
        e.push_str(
            "    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {\n\
             \x20       f.write_str(self.key())\n    }\n}\n",
        );
    }
    Ok(())
}

/// Parses, type-checks, and lowers one operation `expr` body, returning the
/// lowered Rust code together with the expression's checked type.
///
/// Generation is the point where the neutral language is *proved* for this
/// model: a body that does not parse, does not type-check, or whose type is
/// not the declared return type (or `Option` of it — an absent result, e.g.
/// from `first`) fails `generate` with an error naming the operation.
fn lower_op_expr_body(
    model: &Model,
    package: &str,
    class: &ClassDef,
    operation: &Operation,
    source: &str,
) -> anyhow::Result<(String, rex_expr::Ty)> {
    use rex_expr::TypeChecker;

    let parsed = rex_expr::parse(source);
    let Some(ast) = parsed.ast else {
        bail!(
            "operation '{}' expr body could not be parsed: {}",
            operation.name,
            join_errors(&parsed.errors)
        );
    };
    if !parsed.errors.is_empty() {
        bail!(
            "operation '{}' expr body has parse errors: {}",
            operation.name,
            join_errors(&parsed.errors)
        );
    }

    let mut checker =
        TypeChecker::new(rex_expr::TypeContext::from_model(model)).with_self(package, &class.name);
    for param in &operation.params {
        checker = checker.with_binding(&param.name, rex_expr::Ty::from_type_ref(&param.type_));
    }
    let expr_ty = checker.type_of(&ast).map_err(|errors| {
        anyhow::anyhow!(
            "operation '{}' expr body failed to type-check: {}",
            operation.name,
            join_errors(&errors)
        )
    })?;

    let declared = rex_expr::Ty::from_type_ref(&operation.return_type);
    let allowed = expr_ty == declared || expr_ty.inner().is_some_and(|inner| *inner == declared);
    if !allowed {
        bail!(
            "operation '{}' expr body has type {}, but the operation returns {}",
            operation.name,
            expr_ty,
            declared
        );
    }

    let ctx = crate::expr_lower::LowerCtx::new(model, package, &class.name, &operation.params);
    let code = crate::expr_lower::lower_expr(&ast, &expr_ty, &ctx).map_err(|error| {
        anyhow::anyhow!(
            "operation '{}' expr body failed to lower: {}",
            operation.name,
            error.message
        )
    })?;
    Ok((code, expr_ty))
}

/// The checked expression type of a derived feature per the spec's feature
/// typing: the declared type, `List`-wrapped when to-many, `Option`-wrapped
/// when the lower bound is 0.
fn derived_feature_ty(feature: &Feature) -> rex_expr::Ty {
    let base = rex_expr::Ty::from_type_ref(&feature.type_);
    if feature.multiplicity.is_many() {
        base.list()
    } else if feature.multiplicity.lower == 0 {
        base.optional()
    } else {
        base
    }
}

/// Parses, type-checks, and lowers one derived feature's `expr` body,
/// returning the lowered Rust code together with the accessor's return type.
///
/// The expression must have the feature's declared type — or the inner type
/// of it when the feature is optional, in which case the (total) expression
/// simply never yields the absent state and is wrapped in `Some`.
fn lower_derived_expr_body(
    model: &Model,
    package: &str,
    class: &ClassDef,
    feature: &Feature,
    source: &str,
) -> anyhow::Result<(String, String)> {
    let parsed = rex_expr::parse(source);
    let Some(ast) = parsed.ast else {
        bail!(
            "derived feature '{}' expr body could not be parsed: {}",
            feature.name,
            join_errors(&parsed.errors)
        );
    };
    if !parsed.errors.is_empty() {
        bail!(
            "derived feature '{}' expr body has parse errors: {}",
            feature.name,
            join_errors(&parsed.errors)
        );
    }

    let checker = rex_expr::TypeChecker::new(rex_expr::TypeContext::from_model(model))
        .with_self(package, &class.name);
    let expr_ty = checker.type_of(&ast).map_err(|errors| {
        anyhow::anyhow!(
            "derived feature '{}' expr body failed to type-check: {}",
            feature.name,
            join_errors(&errors)
        )
    })?;

    let feature_ty = derived_feature_ty(feature);
    let is_inner = feature_ty.inner().is_some_and(|inner| *inner == expr_ty);
    if expr_ty != feature_ty && !is_inner {
        bail!(
            "derived feature '{}' expr body has type {}, but the feature is {}",
            feature.name,
            expr_ty,
            feature_ty
        );
    }

    let ctx = crate::expr_lower::LowerCtx::new(model, package, &class.name, &[]);
    let lowered = crate::expr_lower::lower_expr(&ast, &expr_ty, &ctx).map_err(|error| {
        anyhow::anyhow!(
            "derived feature '{}' expr body failed to lower: {}",
            feature.name,
            error.message
        )
    })?;
    let lowered = if is_inner {
        format!("Some({lowered})")
    } else {
        lowered
    };
    Ok((lowered, expr_return_type(&feature_ty)?))
}

/// Comma-joins expression errors for a generation error message.
fn join_errors(errors: &[rex_expr::ExprError]) -> String {
    errors
        .iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// The Rust return type for a lowered body: the expression's type mapped by
/// the Tier-1 signature rules (`Option` of the mapped inner for an optional
/// result).
fn expr_return_type(expr_ty: &rex_expr::Ty) -> anyhow::Result<String> {
    match expr_ty {
        rex_expr::Ty::Option(inner) => Ok(format!("Option<{}>", expr_return_type(inner)?)),
        rex_expr::Ty::List(_) => {
            anyhow::bail!("list-typed expression results are not a valid body return ({expr_ty})")
        }
        other => {
            let Some(type_ref) = type_ref_of(other) else {
                anyhow::bail!("unsupported expression result type {other}");
            };
            op_type(&type_ref)
        }
    }
}

/// Converts a (non-`List`/`Option`) expression type back to its IR type
/// reference so the Tier-1 signature mapping applies unchanged.
fn type_ref_of(ty: &rex_expr::Ty) -> Option<TypeRef> {
    use rex_expr::NamedKind;
    Some(match ty {
        rex_expr::Ty::Primitive(primitive) => TypeRef::Primitive(*primitive),
        rex_expr::Ty::Named {
            kind,
            package,
            name,
        } => match kind {
            NamedKind::Class => TypeRef::Class {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Enum => TypeRef::Enum {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Datatype => TypeRef::Datatype {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Interface => TypeRef::Interface {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Vocabulary => TypeRef::Vocabulary {
                package: package.clone(),
                name: name.clone(),
            },
        },
        _ => return None,
    })
}

fn emit_struct(e: &mut String, unit: &Unit<'_>, class: &ClassCtx<'_>) -> anyhow::Result<()> {
    let name = rust_ident(&class.class.name);
    if !class.class.extends.is_empty() {
        let extends: Vec<String> = class
            .class
            .extends
            .iter()
            .map(|t| t.qualified_name().unwrap_or_else(|| "primitive".into()))
            .collect();
        e.push_str(&format!("/// Declared `extends {}`.\n", extends.join(", ")));
    }
    e.push_str("/// Generated class struct.\n");
    e.push_str("#[derive(Debug, Clone, PartialEq)]\n");
    e.push_str(&format!("pub struct {name} {{\n"));
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        e.push_str(&format!(
            "    /// feature id {}: {}\n",
            feature.id,
            feature_doc(feature)
        ));
        e.push_str(&format!(
            "    pub {}: {},\n",
            rust_ident(&snake_case(&feature.name)),
            field_type(unit, feature)?
        ));
    }
    e.push_str("}\n\n");

    e.push_str(&format!("impl Default for {name} {{\n"));
    e.push_str("    fn default() -> Self {\n        Self {\n");
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        let field = rust_ident(&snake_case(&feature.name));
        let value = if feature.multiplicity.is_many() {
            "Vec::new()".to_string()
        } else if is_reference(feature.kind) || feature.multiplicity.lower == 0 {
            "None".to_string()
        } else {
            default_expr(unit, feature)?
        };
        e.push_str(&format!("            {field}: {value},\n"));
    }
    e.push_str("        }\n    }\n}\n\n");

    // Struct impl: attribute setters + reference navigation + derived
    // accessors (Tier 2: an `expr` body becomes an accessor) + operation
    // bodies (Tier 1: a `rust` body becomes a method; body-less operations
    // are hand-written hooks and emit nothing).
    let mut methods = String::new();
    for feature in &class.class.features {
        if feature.is_derived {
            continue;
        }
        let field = rust_ident(&snake_case(&feature.name));
        match feature.kind {
            FeatureKind::Attribute => {
                if feature.is_read_only {
                    continue;
                }
                let value_type = if feature.multiplicity.is_many() {
                    format!("Vec<{}>", value_type(&feature.type_)?)
                } else {
                    value_type(&feature.type_)?
                };
                // Optional scalar fields store `Option<T>`; their setter takes
                // the plain value and wraps it.
                let assign = if feature.multiplicity.is_many() || feature.multiplicity.lower >= 1 {
                    format!("self.{field} = {field};")
                } else {
                    format!("self.{field} = Some({field});")
                };
                methods.push_str(&format!(
                    "    /// Sets `{field}` (feature id {}). Safety: plain stored value, no opposites.\n    pub fn set_{field}(&mut self, {field}: {value_type}) {{\n        {assign}\n    }}\n\n",
                    feature.id
                ));
            }
            FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container => {
                let target = match &feature.type_ {
                    TypeRef::Class { name, .. } => name,
                    _ => continue,
                };
                let target_ctx = unit.class(target)?;
                let target_struct = rust_ident(&target_ctx.class.name);
                let (signature, resolve) = if feature.multiplicity.is_many() {
                    (
                        format!("pub fn {field}<'a>(&self, res: &'a Resource) -> Vec<&'a {target_struct}>"),
                        format!(
                            "self.{field}.iter().filter_map(|id| res.{}(*id)).collect()",
                            target_ctx.single
                        ),
                    )
                } else {
                    (
                        format!(
                            "pub fn {field}<'a>(&self, res: &'a Resource) -> Option<&'a {target_struct}>"
                        ),
                        format!(
                            "self.{field}.as_ref().and_then(|id| res.{}(*id))",
                            target_ctx.single
                        ),
                    )
                };
                methods.push_str(&format!(
                    "    /// Resolves `{field}` against the resource (feature id {}).\n    {signature} {{\n        {resolve}\n    }}\n\n",
                    feature.id
                ));
            }
        }
    }
    // Derived-feature accessors (Tier 2): `expr` bodies lower like
    // operations, with no parameters and the feature's declared typing as
    // the return contract.
    for feature in &class.class.features {
        if !feature.is_derived {
            continue;
        }
        let expr_body = feature.bodies.get("expr");
        let rust_body = feature.bodies.get("rust");
        if let (Some(_), Some(_)) = (expr_body, rust_body) {
            bail!(
                "derived feature '{}' has conflicting 'rust' and 'expr' bodies; \
                 derived bodies must use the neutral expression language",
                feature.name
            );
        }
        let Some(expr_body) = expr_body else {
            continue;
        };
        let (lowered, ret) =
            lower_derived_expr_body(unit.model, &class.package, class.class, feature, expr_body)?;
        methods.push_str(&format!(
            "    /// Derived feature `{name}`, lowered from the neutral `expr` body.\n    pub fn {snake}(&self, res: &Resource) -> {ret} {{\n        {lowered}\n    }}\n\n",
            name = feature.name,
            snake = rust_ident(&snake_case(&feature.name)),
        ));
    }
    for operation in &class.class.operations {
        let expr_body = operation.bodies.get("expr");
        let rust_body = operation.bodies.get("rust");
        if let (Some(_), Some(_)) = (expr_body, rust_body) {
            bail!(
                "operation '{}' has conflicting 'rust' and 'expr' bodies; \
                 declare exactly one body per operation",
                operation.name
            );
        }
        let params = operation
            .params
            .iter()
            .map(|param| {
                Ok(format!(
                    "{}: {}",
                    rust_ident(&snake_case(&param.name)),
                    op_type(&param.type_)?
                ))
            })
            .collect::<anyhow::Result<Vec<_>>>()?
            .join(", ");
        let head = if params.is_empty() {
            String::new()
        } else {
            format!(", {params}")
        };
        if let Some(expr_body) = expr_body {
            let (lowered, expr_ty) = lower_op_expr_body(
                unit.model,
                &class.package,
                class.class,
                operation,
                expr_body,
            )?;
            let ret = expr_return_type(&expr_ty)?;
            methods.push_str(&format!(
                "    /// Declared operation `{name}` (lowered from the neutral `expr` body).\n    pub fn {snake}(&self, res: &Resource{head}) -> {ret} {{\n        {lowered}\n    }}\n\n",
                name = operation.name,
                snake = rust_ident(&snake_case(&operation.name)),
            ));
            continue;
        }
        let Some(body) = rust_body else {
            continue;
        };
        methods.push_str(&format!(
            "    /// Declared operation `{name}` (rust body embedded verbatim).\n    pub fn {snake}(&self, res: &Resource{head}) -> {ret} {{\n{body}\n    }}\n\n",
            name = operation.name,
            snake = rust_ident(&snake_case(&operation.name)),
            ret = op_type(&operation.return_type)?,
        ));
    }
    // Constraint validation: only classes with at least one checkable
    // constraint carry a `validate` method, so unconstrained models keep
    // byte-identical generated code.
    emit_validate_method(&mut methods, unit, class)?;
    if !methods.is_empty() {
        e.push_str(&format!("impl {name} {{\n"));
        e.push_str(&methods);
        e.push_str("}\n");
    }
    Ok(())
}

/// The attributes of `class` carrying declared constraints: the candidates
/// for a generated `validate` method. Derived features and non-attribute
/// features never carry constraints.
fn constrained_attributes(class: &ClassDef) -> impl Iterator<Item = &Feature> {
    class.features.iter().filter(|feature| {
        feature.kind == FeatureKind::Attribute
            && !feature.is_derived
            && !feature.constraints.is_empty()
    })
}

/// The runtime-checkable quantity of one constrained attribute: the string
/// family checks a length expression, the numeric family an `i64` value
/// expression (the IR stores bounds as `i64`). The expression reads the
/// element value bound to `value` by the multiplicity shape.
enum Checked {
    Length(String),
    Number(String),
}

/// One `if … { violations.push(…) }` block for a single bound.
fn bound_check(feature_name: &str, kind: &str, comparison: &str, detail: &str) -> String {
    format!(
        "            if {comparison} {{\n                \
         violations.push(rex_runtime::validation::Violation {{\n                    \
         feature: {feature_name:?}.to_string(),\n                    \
         kind: rex_runtime::validation::ConstraintKind::{kind},\n                    \
         detail: format!(\"{detail}\"),\n                \
         }});\n            }}\n"
    )
}

/// Emits the `validate` method for `class` into the accumulated `methods`
/// buffer. Nothing is emitted when no constraint of the class is checkable
/// by the Rust backend.
fn emit_validate_method(
    methods: &mut String,
    unit: &Unit<'_>,
    class: &ClassCtx<'_>,
) -> anyhow::Result<()> {
    let mut body = String::new();
    for feature in constrained_attributes(class.class) {
        emit_attribute_checks(unit, feature, &mut body)?;
    }
    if body.is_empty() {
        return Ok(());
    }
    methods.push_str(
        "    /// Reports one violation per offended attribute constraint.\n    \
         pub fn validate(&self) -> Vec<rex_runtime::validation::Violation> {\n        \
         let mut violations = Vec::new();\n",
    );
    methods.push_str(&body);
    methods.push_str("        violations\n    }\n\n");
    Ok(())
}

/// Emits the check statements for one constrained attribute: one
/// `violations.push` per offended bound, per element for many-valued
/// attributes (the element index is prefixed to the detail). Absent values
/// (optional attributes) are not violations — multiplicity is not a
/// constraint.
fn emit_attribute_checks(
    unit: &Unit<'_>,
    feature: &Feature,
    out: &mut String,
) -> anyhow::Result<()> {
    let field = rust_ident(&snake_case(&feature.name));
    let constraints = &feature.constraints;
    // How the checked quantity is computed from the element value. Bounds
    // the backend checks descriptively in this milestone (enum literal
    // *name* bounds, short/byte/float/double bounds) map to `None` and emit
    // nothing. Class/interface attributes cannot carry constraints (the
    // driver rejects them), so their presence means a corrupt IR.
    let checked = match &feature.type_ {
        TypeRef::Primitive(PrimitiveType::String) => {
            Some(Checked::Length("value.chars().count()".to_string()))
        }
        TypeRef::Primitive(PrimitiveType::Int) => {
            Some(Checked::Number("i64::from(*value)".to_string()))
        }
        TypeRef::Primitive(PrimitiveType::Long) => Some(Checked::Number("*value".to_string())),
        TypeRef::Datatype { .. } => Some(Checked::Length("value.0.chars().count()".to_string())),
        TypeRef::Enum { .. } => Some(Checked::Number("value.value()".to_string())),
        TypeRef::Vocabulary { name, .. } => {
            let vocabulary = unit
                .vocabularies
                .iter()
                .find(|v| v.name == *name)
                .ok_or_else(|| anyhow::anyhow!("vocabulary '{name}' not found in model"))?;
            let key_facet = vocabulary
                .facets
                .iter()
                .find(|facet| facet.name == vocabulary.key)
                .ok_or_else(|| {
                    anyhow::anyhow!("vocabulary '{name}' has no key facet '{}'", vocabulary.key)
                })?;
            match key_facet.type_ {
                PrimitiveType::String => {
                    Some(Checked::Length("value.key().chars().count()".to_string()))
                }
                PrimitiveType::Int | PrimitiveType::Long => Some(Checked::Number(format!(
                    "value.{}()",
                    rust_ident(&snake_case(&key_facet.name))
                ))),
                _ => None,
            }
        }
        TypeRef::Class { name, .. } | TypeRef::Interface { name, .. } => bail!(
            "feature '{}' has constraints, but class and interface types take none ('{name}')",
            feature.name
        ),
        _ => None,
    };
    let Some(checked) = checked else {
        return Ok(());
    };
    let many = feature.multiplicity.is_many();
    let indexed = |detail: String| {
        if many {
            format!("[{{index}}] {detail}")
        } else {
            detail
        }
    };
    let mut checks = String::new();
    match checked {
        Checked::Length(length_expr) => {
            if constraints.min_length.is_some() || constraints.max_length.is_some() {
                checks.push_str(&format!("            let len = {length_expr};\n"));
                if let Some(min) = constraints.min_length {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "MinLength",
                        &format!("len < {min}"),
                        &indexed(format!("length {{len}} is below minLength {min}")),
                    ));
                }
                if let Some(max) = constraints.max_length {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "MaxLength",
                        &format!("len > {max}"),
                        &indexed(format!("length {{len}} exceeds maxLength {max}")),
                    ));
                }
            }
        }
        Checked::Number(number_expr) => {
            if constraints.minimum.is_some() || constraints.maximum.is_some() {
                checks.push_str(&format!("            let number = {number_expr};\n"));
                if let Some(min) = constraints.minimum {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "Minimum",
                        &format!("number < {min}"),
                        &indexed(format!("value {{number}} is below minimum {min}")),
                    ));
                }
                if let Some(max) = constraints.maximum {
                    checks.push_str(&bound_check(
                        &feature.name,
                        "Maximum",
                        &format!("number > {max}"),
                        &indexed(format!("value {{number}} exceeds maximum {max}")),
                    ));
                }
            }
        }
    }
    if checks.is_empty() {
        return Ok(());
    }
    if many {
        out.push_str(&format!(
            "        for (index, value) in self.{field}.iter().enumerate() {{\n{checks}        }}\n"
        ));
    } else if feature.multiplicity.lower == 0 {
        out.push_str(&format!(
            "        if let Some(value) = &self.{field} {{\n{checks}        }}\n"
        ));
    } else {
        out.push_str(&format!(
            "        {{\n            let value = &self.{field};\n{checks}        }}\n"
        ));
    }
    Ok(())
}

fn emit_resource(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    e.push_str("/// Arena root: owns every object of the model.\n");
    e.push_str("#[derive(Debug, Default, Clone)]\n");
    e.push_str("pub struct Resource {\n");
    for class in &unit.classes {
        e.push_str(&format!(
            "    pub {}: SlotMap<{}, {}>,\n",
            class.slot_field,
            class.id_type,
            rust_ident(&class.class.name)
        ));
    }
    e.push_str("}\n\n");

    e.push_str("impl PartialEq for Resource {\n");
    e.push_str("    fn eq(&self, other: &Self) -> bool {\n");
    for class in &unit.classes {
        e.push_str(&format!(
            "        self.{slot}.len() == other.{slot}.len()\n\
             \x20           && self.{slot}.iter().all(|(key, value)| other.{slot}.get(key) == Some(value))\n\
             \x20           &&\n",
            slot = class.slot_field,
        ));
    }
    e.push_str("        true\n");
    e.push_str("    }\n}\n\n");

    e.push_str("impl rex_runtime::Resource for Resource {\n");
    e.push_str("    fn type_name(&self) -> &'static str {\n");
    e.push_str(&format!("        {:?}\n", unit.type_name));
    e.push_str("    }\n}\n\n");

    e.push_str("impl Resource {\n");
    for class in &unit.classes {
        let struct_name = rust_ident(&class.class.name);
        e.push_str(&format!(
            "    /// Looks up a {single} by id.\n    pub fn {single}(&self, id: {id_type}) -> Option<&{struct_name}> {{\n        self.{slot}.get(id)\n    }}\n\n",
            single = class.single,
            id_type = class.id_type,
            slot = class.slot_field,
        ));
        e.push_str(&format!(
            "    /// Mutable lookup for a {single} by id.\n    pub fn {single}_mut(&mut self, id: {id_type}) -> Option<&mut {struct_name}> {{\n        self.{slot}.get_mut(id)\n    }}\n\n",
            single = class.single,
            id_type = class.id_type,
            slot = class.slot_field,
        ));
        e.push_str(&format!(
            "    /// Creates a default-initialized {struct_name}.\n    pub fn new_{single}(&mut self) -> {id_type} {{\n        self.{slot}.insert({struct_name}::default())\n    }}\n\n",
            single = class.single,
            id_type = class.id_type,
            slot = class.slot_field,
        ));
    }
    emit_pair_mutators(e, unit)?;
    e.push_str("}\n");
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Cardinality {
    Many,
    Single,
}

/// One end of a paired relation: a feature plus its owning class context.
struct End<'a> {
    class: &'a ClassCtx<'a>,
    feature: &'a Feature,
}

impl End<'_> {
    fn cardinality(&self) -> Cardinality {
        if self.feature.multiplicity.is_many() {
            Cardinality::Many
        } else {
            Cardinality::Single
        }
    }

    fn field(&self) -> String {
        rust_ident(&snake_case(&self.feature.name))
    }
}

/// Emits all opposite-maintaining mutators.
///
/// Containment/container pairs are emitted once per feature of the pair (each
/// pair has exactly one `contains` end and one `container` end). Cross
/// reference pairs are emitted once, from the canonical
/// lexicographically-first end, to avoid duplicates.
fn emit_pair_mutators(e: &mut String, unit: &Unit<'_>) -> anyhow::Result<()> {
    for class in &unit.classes {
        for feature in &class.class.features {
            if feature.kind != FeatureKind::Containment {
                continue;
            }
            let target = match &feature.type_ {
                TypeRef::Class { name, .. } => name,
                _ => continue,
            };
            let owner = End { class, feature };
            let child_class = unit.class(target)?;
            let child_feature = feature.opposite.as_ref().and_then(|opposite| {
                child_class
                    .class
                    .features
                    .iter()
                    .find(|f| f.name == opposite.feature)
            });
            match child_feature {
                Some(child_feature) => {
                    let child = End {
                        class: child_class,
                        feature: child_feature,
                    };
                    emit_pair(e, &owner, &child)?;
                }
                None => emit_one_sided_containment(e, &owner, child_class)?,
            }
        }
    }

    let mut emitted: BTreeSet<(String, String)> = BTreeSet::new();
    for class in &unit.classes {
        for feature in &class.class.features {
            if feature.kind != FeatureKind::CrossReference {
                continue;
            }
            let target = match &feature.type_ {
                TypeRef::Class { name, .. } => name,
                _ => continue,
            };
            let other_class = unit.class(target)?;
            let Some(other_feature) = feature.opposite.as_ref().and_then(|opposite| {
                other_class
                    .class
                    .features
                    .iter()
                    .find(|f| f.name == opposite.feature)
            }) else {
                continue;
            };
            let key = (
                format!("{}/{}", class.class.name, feature.name),
                format!("{}/{}", other_class.class.name, other_feature.name),
            );
            let canonical = if key.0 <= key.1 {
                key.clone()
            } else {
                (key.1, key.0)
            };
            if !emitted.insert(canonical) {
                continue;
            }
            emit_pair(
                e,
                &End { class, feature },
                &End {
                    class: other_class,
                    feature: other_feature,
                },
            )?;
        }
    }
    Ok(())
}

/// Emits mutators for both ends of a relation pair.
fn emit_pair(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    match (a.cardinality(), b.cardinality()) {
        (Cardinality::Many, Cardinality::Many) => {
            emit_add_remove_many(e, a, b)?;
            emit_add_remove_many(e, b, a)?;
        }
        (Cardinality::Single, Cardinality::Many) => {
            emit_set_single(e, a, b)?;
            emit_add_remove_to_single(e, b, a)?;
        }
        (Cardinality::Many, Cardinality::Single) => {
            emit_add_remove_to_single(e, a, b)?;
            emit_set_single(e, b, a)?;
        }
        (Cardinality::Single, Cardinality::Single) => {
            emit_set_single_single(e, a, b)?;
            emit_set_single_single(e, b, a)?;
        }
    }
    Ok(())
}

/// `add`/`remove` for a many end whose opposite is also many.
fn emit_add_remove_many(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Adds `{b_single}` to `{a_single}.{field}`, maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_add_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) || !self.{b_slot}.contains_key({b_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           if !a.{field}.contains(&{b_single}) {{\n\
         \x20               a.{field}.push({b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           if !b.{opposite}.contains(&{a_single}) {{\n\
         \x20               b.{opposite}.push({a_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n\n\
         \x20   /// Removes `{b_single}` from `{a_single}.{field}`, maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_remove_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field}.retain(|id| *id != {b_single});\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           b.{opposite}.retain(|id| *id != {a_single});\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// `set` for a single end `a` whose opposite end `b` is many.
fn emit_set_single(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Sets `{a_single}.{field}` (or clears it), maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_set_{field}(&mut self, {a_single}: {a_id}, {b_single}: Option<{b_id}>) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(target) = {b_single} {{\n\
         \x20           if !self.{b_slot}.contains_key(target) {{\n\
         \x20               return;\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       let current = self.{a_slot}.get({a_single}).and_then(|s| s.{field});\n\
         \x20       if current == {b_single} {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(old) = current {{\n\
         \x20           if let Some(old_b) = self.{b_slot}.get_mut(old) {{\n\
         \x20               old_b.{opposite}.retain(|id| *id != {a_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(new_b) = {b_single} {{\n\
         \x20           if let Some(b) = self.{b_slot}.get_mut(new_b) {{\n\
         \x20               if !b.{opposite}.contains(&{a_single}) {{\n\
         \x20                   b.{opposite}.push({a_single});\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field} = {b_single};\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// `add`/`remove` for a many end `a` whose opposite end `b` is single.
fn emit_add_remove_to_single(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Adds `{b_single}` to `{a_single}.{field}`, detaching it from any previous `{opposite}` holder.\n\
         \x20   pub fn {a_single}_add_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) || !self.{b_slot}.contains_key({b_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       let current = self.{b_slot}.get({b_single}).and_then(|b| b.{opposite});\n\
         \x20       if current == Some({a_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(old) = current {{\n\
         \x20           if let Some(old_a) = self.{a_slot}.get_mut(old) {{\n\
         \x20               old_a.{field}.retain(|id| *id != {b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           if !a.{field}.contains(&{b_single}) {{\n\
         \x20               a.{field}.push({b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           b.{opposite} = Some({a_single});\n\
         \x20       }}\n\
         \x20   }}\n\n\
         \x20   /// Removes `{b_single}` from `{a_single}.{field}`, clearing `{b_single}.{opposite}` if it pointed here.\n\
         \x20   pub fn {a_single}_remove_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field}.retain(|id| *id != {b_single});\n\
         \x20       }}\n\
         \x20       if let Some(b) = self.{b_slot}.get_mut({b_single}) {{\n\
         \x20           if b.{opposite} == Some({a_single}) {{\n\
         \x20               b.{opposite} = None;\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// `set` for a single end whose opposite is also single.
fn emit_set_single_single(e: &mut String, a: &End<'_>, b: &End<'_>) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        a.class.single.clone(),
        &a.class.id_type,
        &a.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        b.class.single.clone(),
        &b.class.id_type,
        &b.class.slot_field,
    );
    let field = a.field();
    let opposite = b.field();
    e.push_str(&format!(
        "    /// Sets `{a_single}.{field}` (or clears it), maintaining `{b_single}.{opposite}`.\n\
         \x20   pub fn {a_single}_set_{field}(&mut self, {a_single}: {a_id}, {b_single}: Option<{b_id}>) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(target) = {b_single} {{\n\
         \x20           if !self.{b_slot}.contains_key(target) {{\n\
         \x20               return;\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       let current = self.{a_slot}.get({a_single}).and_then(|s| s.{field});\n\
         \x20       if current == {b_single} {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(old) = current {{\n\
         \x20           if let Some(old_b) = self.{b_slot}.get_mut(old) {{\n\
         \x20               if old_b.{opposite} == Some({a_single}) {{\n\
         \x20                   old_b.{opposite} = None;\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(new_b) = {b_single} {{\n\
         \x20           if let Some(prev) = self.{b_slot}.get(new_b).and_then(|b| b.{opposite}) {{\n\
         \x20               if prev != {a_single} {{\n\
         \x20                   if let Some(prev_a) = self.{a_slot}.get_mut(prev) {{\n\
         \x20                       prev_a.{field} = None;\n\
         \x20                   }}\n\
         \x20               }}\n\
         \x20           }}\n\
         \x20           if let Some(b) = self.{b_slot}.get_mut(new_b) {{\n\
         \x20               b.{opposite} = Some({a_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field} = {b_single};\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

/// Owner-side mutators for a `contains` feature without an opposite: plain
/// id-list maintenance with no back-pointer to keep in sync.
fn emit_one_sided_containment(
    e: &mut String,
    owner: &End<'_>,
    child_class: &ClassCtx<'_>,
) -> anyhow::Result<()> {
    let (a_single, a_id, a_slot) = (
        owner.class.single.clone(),
        &owner.class.id_type,
        &owner.class.slot_field,
    );
    let (b_single, b_id, b_slot) = (
        child_class.single.clone(),
        &child_class.id_type,
        &child_class.slot_field,
    );
    let field = owner.field();
    e.push_str(&format!(
        "    /// Adds `{b_single}` to `{a_single}.{field}` (one-way containment, no opposite).\n\
         \x20   pub fn {a_single}_add_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if !self.{a_slot}.contains_key({a_single}) || !self.{b_slot}.contains_key({b_single}) {{\n\
         \x20           return;\n\
         \x20       }}\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           if !a.{field}.contains(&{b_single}) {{\n\
         \x20               a.{field}.push({b_single});\n\
         \x20           }}\n\
         \x20       }}\n\
         \x20   }}\n\n\
         \x20   /// Removes `{b_single}` from `{a_single}.{field}` (one-way containment, no opposite).\n\
         \x20   pub fn {a_single}_remove_{field}(&mut self, {a_single}: {a_id}, {b_single}: {b_id}) {{\n\
         \x20       if let Some(a) = self.{a_slot}.get_mut({a_single}) {{\n\
         \x20           a.{field}.retain(|id| *id != {b_single});\n\
         \x20       }}\n\
         \x20   }}\n"
    ));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_ir::{
        EnumLiteral, Multiplicity, Operation, OperationParam, Package, VocabularyDef,
        VocabularyEntry,
    };

    const PKG: &str = "nz.example.payments";

    fn vocabulary_ref() -> TypeRef {
        TypeRef::Vocabulary {
            package: PKG.to_string(),
            name: "Currency".to_string(),
        }
    }

    fn entry(key: &str, symbol: &str, minor_units: i64) -> VocabularyEntry {
        VocabularyEntry {
            key: key.to_string(),
            facets: BTreeMap::from([
                (
                    "symbol".to_string(),
                    DefaultValue::String(symbol.to_string()),
                ),
                ("minorUnits".to_string(), DefaultValue::Int(minor_units)),
            ]),
        }
    }

    /// The IR equivalent of `tests/conformance/models/currency.mox`.
    fn currency_model() -> Model {
        let mut model = Model::new();
        let mut package = Package::new(PKG);
        package.vocabularies.push(VocabularyDef {
            name: "Currency".to_string(),
            description: None,
            source: "iso:4217".to_string(),
            version: Some("2024-01-01".to_string()),
            key: "alpha3".to_string(),
            facets: vec![
                rex_ir::VocabularyFacet {
                    name: "symbol".to_string(),
                    type_: PrimitiveType::String,
                },
                rex_ir::VocabularyFacet {
                    name: "minorUnits".to_string(),
                    type_: PrimitiveType::Int,
                },
            ],
            entries: vec![
                entry("USD", "$", 2),
                entry("EUR", "€", 2),
                entry("JPY", "¥", 0),
            ],
        });
        package.classes.push(ClassDef::new(
            "Account",
            vec![],
            vec![
                Feature::new(
                    "owner",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "currency",
                    FeatureKind::Attribute,
                    vocabulary_ref(),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "backup",
                    FeatureKind::Attribute,
                    vocabulary_ref(),
                    Multiplicity::OPTIONAL,
                ),
                Feature::new(
                    "watchlist",
                    FeatureKind::Attribute,
                    vocabulary_ref(),
                    Multiplicity::MANY,
                ),
            ],
        ));
        model.packages.push(package);
        model
    }

    fn generate_currency() -> String {
        generate(&currency_model())
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs")
    }

    /// A model with a required, an optional, and a many-valued date
    /// attribute (issue #9): the shape of the `date` primitive's lowering.
    fn date_model() -> Model {
        let mut model = Model::new();
        let mut package = Package::new(PKG);
        package.classes.push(ClassDef::new(
            "Loan",
            vec![],
            vec![
                Feature::new(
                    "effectiveDate",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Date),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "maturityDate",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Date),
                    Multiplicity::OPTIONAL,
                ),
                Feature::new(
                    "renewals",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Date),
                    Multiplicity::MANY,
                ),
            ],
        ));
        model.packages.push(package);
        model
    }

    fn generate_dates() -> String {
        generate(&date_model())
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs")
    }

    #[test]
    fn date_attributes_map_to_the_runtime_date_type() {
        let code = generate_dates();
        // Required date attribute: a `rex_runtime::Date` field defaulting to
        // the epoch (dates take no declared defaults in this milestone).
        assert!(
            code.contains("pub effective_date: rex_runtime::Date,"),
            "{code}"
        );
        assert!(
            code.contains("effective_date: rex_runtime::Date::EPOCH,"),
            "{code}"
        );
        // Optional and many-valued follow the usual shapes.
        assert!(
            code.contains("pub maturity_date: Option<rex_runtime::Date>,"),
            "{code}"
        );
        assert!(
            code.contains("pub renewals: Vec<rex_runtime::Date>,"),
            "{code}"
        );
    }

    #[test]
    fn date_attributes_serialize_iso_strings_and_load_strictly() {
        let code = generate_dates();
        // Save: ISO-8601 strings (required, optional, and many shapes).
        assert!(
            code.contains("serde_json::Value::String(obj.effective_date.to_string())"),
            "required date save:\n{code}"
        );
        assert!(
            code.contains("serde_json::Value::String((*v).to_string())"),
            "optional/many date save:\n{code}"
        );
        // Load: the strict ISO parse; a non-ISO string is InvalidInstance.
        assert!(
            code.contains("expect_date(value, \"Loan.effectiveDate\")?"),
            "required date load:\n{code}"
        );
        assert!(
            code.contains("expect_date(element, \"Loan.renewals\")?"),
            "{code}"
        );
        assert!(
            code.contains("must be an ISO-8601 date (YYYY-MM-DD)"),
            "strict loader helper:\n{code}"
        );
    }

    #[test]
    fn date_setters_take_runtime_dates() {
        let code = generate_dates();
        assert!(
            code.contains(
                "pub fn set_effective_date(&mut self, effective_date: rex_runtime::Date)"
            ),
            "{code}"
        );
        assert!(
            code.contains("pub fn set_maturity_date(&mut self, maturity_date: rex_runtime::Date)"),
            "{code}"
        );
    }

    #[test]
    fn vocabulary_enum_with_key_and_facet_accessors() {
        let code = generate_currency();
        assert!(
            code.contains("#[repr(u8)]\npub enum Currency {\n    USD,\n    EUR,\n    JPY,\n}"),
            "enum shape:\n{code}"
        );
        assert!(code.contains("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]"));
        assert!(
            code.contains("Vocabulary Currency")
                && code.contains("iso:4217")
                && code.contains("2024-01-01")
                && code.contains("3 entries"),
            "doc comment must name vocabulary, source, version, and entry count:\n{code}"
        );
        assert!(code.contains("pub fn key(self) -> &'static str"));
        assert!(code.contains("Self::USD => \"USD\","));
        assert!(code.contains("pub fn symbol(self) -> &'static str"));
        assert!(code.contains("Self::JPY => \"¥\""));
        assert!(code.contains("pub fn minor_units(self) -> i64"));
        assert!(code.contains("Self::JPY => 0,"));
    }

    #[test]
    fn vocabulary_try_from_display_and_default() {
        let code = generate_currency();
        assert!(code.contains("impl TryFrom<&str> for Currency {"));
        assert!(
            code.contains("RexError::InvalidInstance") && code.contains("unknown Currency"),
            "lookup failure must be an InvalidInstance naming the vocabulary:\n{code}"
        );
        assert!(code.contains("impl std::fmt::Display for Currency {"));
        assert!(code.contains("impl Default for Currency {"));
        assert!(
            code.contains("        Self::USD\n"),
            "Default is the first entry:\n{code}"
        );
    }

    #[test]
    fn vocabulary_attributes_follow_enum_field_rules() {
        let code = generate_currency();
        assert!(
            code.contains("pub currency: Currency,"),
            "required → plain:\n{code}"
        );
        assert!(
            code.contains("pub backup: Option<Currency>,"),
            "optional → Option:\n{code}"
        );
        assert!(
            code.contains("pub watchlist: Vec<Currency>,"),
            "many → Vec:\n{code}"
        );
        assert!(code.contains("pub fn set_currency(&mut self, currency: Currency)"));
        assert!(code.contains("pub fn set_backup(&mut self, backup: Currency)"));
        assert!(
            code.contains("self.backup = Some(backup);"),
            "optional scalar setters wrap in Some:\n{code}"
        );
        assert!(code.contains("pub fn set_watchlist(&mut self, watchlist: Vec<Currency>)"));
        assert!(
            code.contains("currency: Currency::USD,"),
            "required vocabulary attribute defaults to the first entry:\n{code}"
        );
        assert!(code.contains("backup: None,"));
        assert!(code.contains("watchlist: Vec::new(),"));
    }

    #[test]
    fn declared_vocabulary_default_resolves_to_a_variant() {
        let mut model = currency_model();
        model.packages[0].classes[0].features[1] = Feature::new(
            "currency",
            FeatureKind::Attribute,
            vocabulary_ref(),
            Multiplicity::REQUIRED,
        )
        .with_default(DefaultValue::String("EUR".to_string()));
        let code = generate(&model)
            .expect("generate")
            .remove("models.rs")
            .unwrap();
        assert!(
            code.contains("currency: Currency::EUR,"),
            "declared default must lower to the matching variant:\n{code}"
        );
    }

    /// A declared default on a datatype-typed attribute must construct the
    /// newtype, not the bare inner String (regression: the generated
    /// `Default` impl assigned a `String` to the datatype's field and the
    /// emitted crate did not compile — caught by the compile-everything
    /// fixture, issue #16).
    #[test]
    fn datatype_attribute_default_wraps_the_newtype() {
        let mut model = currency_model();
        model.packages[0]
            .datatypes
            .push(DatatypeDef::new("Sku", None));
        model.packages[0].classes[0].features.push(
            Feature::new(
                "sku",
                FeatureKind::Attribute,
                TypeRef::Datatype {
                    package: PKG.to_string(),
                    name: "Sku".to_string(),
                },
                Multiplicity::REQUIRED,
            )
            .with_default(DefaultValue::String("FIX-0001".to_string())),
        );
        let code = generate(&model)
            .expect("generate")
            .remove("models.rs")
            .unwrap();
        assert!(
            code.contains("sku: Sku(\"FIX-0001\".to_string()),"),
            "datatype default must wrap the newtype:\n{code}"
        );
    }

    #[test]
    fn vocabulary_default_that_is_not_an_entry_is_an_error() {
        let mut model = currency_model();
        model.packages[0].classes[0].features[1] = Feature::new(
            "currency",
            FeatureKind::Attribute,
            vocabulary_ref(),
            Multiplicity::REQUIRED,
        )
        .with_default(DefaultValue::String("XYZ".to_string()));
        assert!(
            generate(&model).is_err(),
            "unknown default key must fail generation"
        );
    }

    #[test]
    fn non_identifier_entry_keys_are_rejected() {
        let mut model = currency_model();
        model.packages[0].vocabularies[0]
            .entries
            .push(entry("not a key!", "?", 1));
        let error = generate(&model).expect_err("non-identifier key must fail generation");
        assert!(
            error.to_string().contains("not a key!"),
            "error must name the offending key: {error:#}"
        );
    }

    #[test]
    fn keyword_entry_keys_are_raw_escaped() {
        let mut model = currency_model();
        model.packages[0].vocabularies[0]
            .entries
            .push(entry("type", "T", 1));
        let code = generate(&model)
            .expect("generate")
            .remove("models.rs")
            .unwrap();
        assert!(
            code.contains("r#type,\n"),
            "keyword keys escape as r#type:\n{code}"
        );
    }

    #[test]
    fn float_facet_uses_the_exact_json_number_text() {
        let mut model = currency_model();
        let vocabulary = &mut model.packages[0].vocabularies[0];
        vocabulary.facets.push(rex_ir::VocabularyFacet {
            name: "rate".to_string(),
            type_: PrimitiveType::Double,
        });
        // validate_entries guarantees every entry carries every facet.
        for (index, entry) in vocabulary.entries.iter_mut().enumerate() {
            entry.facets.insert(
                "rate".to_string(),
                if index == 0 {
                    // Fractional values keep their exact JSON number text.
                    DefaultValue::String("1.125".to_string())
                } else {
                    DefaultValue::Int(1)
                },
            );
        }
        let code = generate(&model)
            .expect("generate")
            .remove("models.rs")
            .unwrap();
        assert!(
            code.contains("pub fn rate(self) -> f64"),
            "accessor:\n{code}"
        );
        assert!(
            code.contains("Self::USD => 1.125,"),
            "exact JSON number text is emitted as the literal:\n{code}"
        );
        assert!(
            code.contains("Self::EUR => 1.0,"),
            "integral values become x.0 float literals:\n{code}"
        );
    }

    #[test]
    fn vocabulary_json_saves_and_loads_by_key() {
        let code = generate_currency();
        assert!(
            code.contains(".key().to_string()"),
            "save writes the entry key string:\n{code}"
        );
        assert!(
            code.contains("Currency::try_from(expect_string("),
            "load resolves the key via TryFrom<&str>:\n{code}"
        );
        assert!(
            code.contains("vocabulary-typed attributes serialize their entry key string"),
            "module header documents the vocabulary JSON contract:\n{code}"
        );
    }

    #[test]
    fn duplicate_vocabulary_names_across_packages_are_rejected() {
        let mut model = currency_model();
        let mut second = Package::new("nz.example.other");
        second.vocabularies.push(VocabularyDef {
            name: "Currency".to_string(),
            description: None,
            source: "other:source".to_string(),
            version: None,
            key: "code".to_string(),
            facets: vec![],
            entries: vec![entry("USD", "$", 2)],
        });
        model.packages.push(second);
        let error = generate(&model).expect_err("duplicate vocabulary names must fail");
        assert!(
            error
                .to_string()
                .contains("duplicate vocabulary name 'Currency'"),
            "error was: {error:#}"
        );
    }

    #[test]
    fn empty_vocabulary_is_rejected() {
        let mut model = currency_model();
        model.packages[0].vocabularies[0].entries.clear();
        let error = generate(&model).expect_err("empty vocabulary must fail");
        assert!(
            error
                .to_string()
                .contains("vocabulary 'Currency' has no entries"),
            "error was: {error:#}"
        );
    }

    /// A `readonly` feature keeps its stored field but gets **no** setter; an
    /// `id` feature is codegen-neutral (stored and settable like any other).
    /// The driver's lowering stage sets these flags (see the rex-driver
    /// modifier tests); this locks in the backend's contract.
    #[test]
    fn readonly_feature_has_no_setter_and_id_feature_is_unchanged() {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes.push(ClassDef::new(
            "Gadget",
            vec![],
            vec![
                Feature::new(
                    "plain",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "code",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::REQUIRED,
                )
                .read_only(),
                Feature::new(
                    "sku",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::REQUIRED,
                )
                .identifier(),
            ],
        ));
        model.packages.push(package);
        let code = generate(&model)
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");

        // All three features are stored as fields.
        assert!(code.contains("pub plain: String,"), "plain field:\n{code}");
        assert!(
            code.contains("pub code: String,"),
            "readonly field stays stored:\n{code}"
        );
        assert!(
            code.contains("pub sku: String,"),
            "id field stays stored:\n{code}"
        );

        // Plain and `id` features get setters; `readonly` does not.
        assert!(code.contains("pub fn set_plain"), "control setter:\n{code}");
        assert!(
            code.contains("pub fn set_sku"),
            "`id` is codegen-neutral: same setters as any other feature:\n{code}"
        );
        assert!(
            !code.contains("pub fn set_code"),
            "`readonly` features must not get a setter:\n{code}"
        );
    }

    // --- Tier 1: operation bodies and datatype create/convert ----------------

    fn op_model() -> Model {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.datatypes.push(
            DatatypeDef::new("Date", None)
                .bind("rust", "chrono::NaiveDate")
                .with_body("create", "rust", " Date(it) ")
                .with_body("convert", "rust", " self.0.clone() "),
        );
        package.classes.push(ClassDef::new(
            "Library",
            vec![],
            vec![Feature::new(
                "books",
                FeatureKind::Containment,
                TypeRef::Class {
                    package: "demo".to_string(),
                    name: "Book".to_string(),
                },
                Multiplicity::MANY,
            )],
        ));
        package.classes[0].operations.push(
            Operation::new(
                "getBook",
                TypeRef::Class {
                    package: "demo".to_string(),
                    name: "Book".to_string(),
                },
                vec![
                    OperationParam {
                        name: "title".to_string(),
                        type_: TypeRef::Primitive(PrimitiveType::String),
                    },
                    OperationParam {
                        name: "shelf".to_string(),
                        type_: TypeRef::Class {
                            package: "demo".to_string(),
                            name: "Shelf".to_string(),
                        },
                    },
                ],
            )
            .with_body(
                "rust",
                " res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) ",
            ),
        );
        package.classes[0].operations.push(Operation::new(
            "countBooks",
            TypeRef::Primitive(PrimitiveType::Int),
            vec![],
        ));
        package.classes.push(ClassDef::new("Book", vec![], vec![]));
        package.classes.push(ClassDef::new("Shelf", vec![], vec![]));
        model.packages.push(package);
        model
    }

    #[test]
    fn operation_signature_convention_and_verbatim_body() {
        let code = generate(&op_model())
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");
        // Fixed signature: `&self, res: &Resource, params…` with String
        // params as `String`, class params as `ShelfId`, class return as
        // `BookId` — and the body embedded verbatim (byte-identical, on its
        // own line between the braces).
        assert!(
            code.contains("pub fn get_book(&self, res: &Resource, title: String, shelf: ShelfId) -> BookId {\n res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) \n    }"),
            "signature + verbatim body:\n{code}"
        );
        // The operation lives inside the class's impl block.
        let impl_start = code.find("impl Library {").expect("Library impl");
        let fn_at = code.find("pub fn get_book").expect("operation in output");
        assert!(fn_at > impl_start, "operation must be in the impl block");
    }

    #[test]
    fn body_less_operation_emits_nothing() {
        let code = generate(&op_model())
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");
        assert!(
            !code.contains("count_books"),
            "body-less ops are hand-written hooks, not generated:\n{code}"
        );
    }

    #[test]
    fn datatype_create_convert_impl_convention() {
        let code = generate(&op_model())
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");
        // `create(it: String) -> Self` and `convert(self) -> String`, bodies
        // verbatim. The example bodies compile: `create` returns
        // `Self` directly (the body's choice — no `Some(...)` wrapping),
        // `convert` clones through the newtype's `self.0`.
        assert!(
            code.contains("impl Date {\n    /// Tier 1 `create` body, embedded verbatim.\n    pub fn create(it: String) -> Self {\n Date(it) \n    }\n    /// Tier 1 `convert` body, embedded verbatim.\n    pub fn convert(self) -> String {\n self.0.clone() \n    }\n}"),
            "datatype impl:\n{code}"
        );
    }

    #[test]
    fn module_header_documents_the_body_contract() {
        let code = generate(&op_model())
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");
        assert!(
            code.contains("operations with a `rust` body are emitted"),
            "header must describe Tier 1 bodies:\n{code}"
        );
    }

    // --- Tier 2: neutral `expr` bodies --------------------------------------

    /// `op Book getBook(String title) { expr { books.first(b => b.title ==
    /// title) } }`: the expression's type drives the return (`Option` of the
    /// declared class id), and the body is the lowered Rust text.
    #[test]
    fn expr_body_op_lowers_to_a_typed_method() {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes.push(ClassDef::new(
            "Library",
            vec![],
            vec![Feature::new(
                "books",
                FeatureKind::Containment,
                TypeRef::Class {
                    package: "demo".to_string(),
                    name: "Book".to_string(),
                },
                Multiplicity::MANY,
            )],
        ));
        package.classes[0].operations.push(
            Operation::new(
                "getBook",
                TypeRef::Class {
                    package: "demo".to_string(),
                    name: "Book".to_string(),
                },
                vec![OperationParam {
                    name: "title".to_string(),
                    type_: TypeRef::Primitive(PrimitiveType::String),
                }],
            )
            .with_body("expr", "books.first(b => b.title == title)"),
        );
        package.classes.push(ClassDef::new(
            "Book",
            vec![],
            vec![Feature::new(
                "title",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            )],
        ));
        model.packages.push(package);

        let code = generate(&model)
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");
        assert!(
            code.contains(
                "pub fn get_book(&self, res: &Resource, title: String) -> Option<BookId> {\n        self.books.clone().iter().find(|b| (res.book(**b).map(|o| o.title.clone()).expect(\"dangling `Book` id\") == title.clone())).copied()\n    }"
            ),
            "lowered method:\n{code}"
        );
        assert!(
            code.contains("lowered from the neutral `expr` body"),
            "doc comment must mark lowered bodies:\n{code}"
        );
    }

    #[test]
    fn expr_body_type_error_fails_generation_naming_the_operation() {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package
            .classes
            .push(ClassDef::new("Library", vec![], vec![]));
        package.classes[0].operations.push(
            Operation::new("broken", TypeRef::Primitive(PrimitiveType::Int), vec![])
                .with_body("expr", "\"text\" + 1"),
        );
        package.classes.push(ClassDef::new("Book", vec![], vec![]));
        model.packages.push(package);

        let error = generate(&model).expect_err("L2 violation must fail generation");
        assert!(
            error.to_string().contains("'broken'"),
            "error must name the operation: {error:#}"
        );
        assert!(
            error
                .to_string()
                .contains("operator `+` requires numeric operands"),
            "error must carry the checker message: {error:#}"
        );
    }

    #[test]
    fn expr_body_return_type_mismatch_fails_generation() {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package
            .classes
            .push(ClassDef::new("Library", vec![], vec![]));
        package.classes[0].operations.push(
            Operation::new("wrong", TypeRef::Primitive(PrimitiveType::Int), vec![])
                .with_body("expr", "\"text\""),
        );
        package.classes.push(ClassDef::new("Book", vec![], vec![]));
        model.packages.push(package);

        let error = generate(&model).expect_err("return mismatch must fail generation");
        assert!(
            error.to_string().contains("'wrong'") && error.to_string().contains("int"),
            "error must name the operation and both types: {error:#}"
        );
    }

    #[test]
    fn conflicting_rust_and_expr_bodies_are_rejected() {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package
            .classes
            .push(ClassDef::new("Library", vec![], vec![]));
        package.classes[0].operations.push(
            Operation::new("both", TypeRef::Primitive(PrimitiveType::Int), vec![])
                .with_body("rust", " 42 ")
                .with_body("expr", "42"),
        );
        package.classes.push(ClassDef::new("Book", vec![], vec![]));
        model.packages.push(package);

        let error = generate(&model).expect_err("conflicting bodies must fail generation");
        assert!(
            error.to_string().contains("'both'"),
            "error must name the operation: {error:#}"
        );
    }

    /// A derived feature with an `expr` body lowers into an accessor on the
    /// class. The return type follows the feature's declared typing (a
    /// 0..1 derived attribute is `Option<..>`); a total expression of the
    /// inner type wraps in `Some` — it simply never yields the absent state.
    #[test]
    fn derived_expr_body_lowers_to_an_accessor() {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes.push(ClassDef::new(
            "Book",
            vec![],
            vec![
                Feature::new(
                    "title",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "pages",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Int),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "citation",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::OPTIONAL,
                )
                .derived()
                .with_body("expr", "if pages > 400 { title } else { \"short read\" }"),
            ],
        ));
        model.packages.push(package);

        let code = generate(&model)
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");
        assert!(
            code.contains(
                "pub fn citation(&self, res: &Resource) -> Option<String> {\n        Some(if (self.pages > 400i32) { self.title.clone() } else { \"short read\".to_string() })\n    }"
            ),
            "lowered accessor:\n{code}"
        );
        assert!(
            code.contains("Derived feature `citation`, lowered from the neutral `expr` body."),
            "doc comment must mark the lowered accessor:\n{code}"
        );
    }

    #[test]
    fn derived_expr_body_type_error_fails_generation_naming_the_feature() {
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes.push(ClassDef::new(
            "Book",
            vec![],
            vec![Feature::new(
                "citation",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::OPTIONAL,
            )
            .derived()
            .with_body("expr", "\"text\" + 1")],
        ));
        model.packages.push(package);

        let error = generate(&model).expect_err("L2 violation must fail generation");
        assert!(
            error.to_string().contains("'citation'"),
            "error must name the feature: {error:#}"
        );
    }

    // --- Constraint validation (runtime reports, never panics) --------------

    fn constrained(feature: Feature, constraints: rex_ir::FeatureConstraints) -> Feature {
        let mut feature = feature;
        feature.constraints = constraints;
        feature
    }

    fn string_bounds(min: u64, max: u64) -> rex_ir::FeatureConstraints {
        rex_ir::FeatureConstraints {
            min_length: Some(min),
            max_length: Some(max),
            ..Default::default()
        }
    }

    fn numeric_bounds(min: i64, max: i64) -> rex_ir::FeatureConstraints {
        rex_ir::FeatureConstraints {
            minimum: Some(min),
            maximum: Some(max),
            ..Default::default()
        }
    }

    /// The IR equivalent of the validation scenario: one class carrying every
    /// v1-supported constraint shape (string, many-valued, optional, int,
    /// enum value bounds, datatype length bounds, String-keyed and Int-keyed
    /// vocabulary bounds) plus an unconstrained sibling.
    fn validation_model() -> Model {
        let mut model = Model::new();
        let mut package = Package::new("nz.example.validation");
        package.enums.push(EnumDef::new(
            "Rank",
            vec![
                EnumLiteral::new("Low", None, 1),
                EnumLiteral::new("High", None, 9),
            ],
        ));
        package.datatypes.push(DatatypeDef::new("Meta", None));
        package.vocabularies.push(VocabularyDef {
            name: "Grade".to_string(),
            description: None,
            source: "test:grades".to_string(),
            version: None,
            key: "code".to_string(),
            facets: vec![rex_ir::VocabularyFacet {
                name: "code".to_string(),
                type_: PrimitiveType::String,
            }],
            entries: vec![
                VocabularyEntry {
                    key: "US".to_string(),
                    facets: BTreeMap::from([(
                        "code".to_string(),
                        DefaultValue::String("US".to_string()),
                    )]),
                },
                VocabularyEntry {
                    key: "EURO".to_string(),
                    facets: BTreeMap::from([(
                        "code".to_string(),
                        DefaultValue::String("EURO".to_string()),
                    )]),
                },
            ],
        });
        package.vocabularies.push(VocabularyDef {
            name: "Level".to_string(),
            description: None,
            source: "test:levels".to_string(),
            version: None,
            key: "code".to_string(),
            facets: vec![rex_ir::VocabularyFacet {
                name: "code".to_string(),
                type_: PrimitiveType::Int,
            }],
            entries: vec![
                VocabularyEntry {
                    key: "A".to_string(),
                    facets: BTreeMap::from([("code".to_string(), DefaultValue::Int(2))]),
                },
                VocabularyEntry {
                    key: "B".to_string(),
                    facets: BTreeMap::from([("code".to_string(), DefaultValue::Int(7))]),
                },
            ],
        });
        let string = || TypeRef::Primitive(PrimitiveType::String);
        package.classes.push(ClassDef::new(
            "Profile",
            vec![],
            vec![
                constrained(
                    Feature::new(
                        "name",
                        FeatureKind::Attribute,
                        string(),
                        Multiplicity::REQUIRED,
                    ),
                    string_bounds(2, 8),
                ),
                constrained(
                    Feature::new(
                        "nicknames",
                        FeatureKind::Attribute,
                        string(),
                        Multiplicity::MANY,
                    ),
                    string_bounds(2, 4),
                ),
                constrained(
                    Feature::new(
                        "score",
                        FeatureKind::Attribute,
                        TypeRef::Primitive(PrimitiveType::Int),
                        Multiplicity::REQUIRED,
                    ),
                    numeric_bounds(0, 100),
                ),
                constrained(
                    Feature::new(
                        "rank",
                        FeatureKind::Attribute,
                        TypeRef::Enum {
                            package: "nz.example.validation".to_string(),
                            name: "Rank".to_string(),
                        },
                        Multiplicity::REQUIRED,
                    ),
                    numeric_bounds(2, 10),
                ),
                constrained(
                    Feature::new(
                        "meta",
                        FeatureKind::Attribute,
                        TypeRef::Datatype {
                            package: "nz.example.validation".to_string(),
                            name: "Meta".to_string(),
                        },
                        Multiplicity::REQUIRED,
                    ),
                    string_bounds(4, 64),
                ),
                constrained(
                    Feature::new(
                        "grade",
                        FeatureKind::Attribute,
                        TypeRef::Vocabulary {
                            package: "nz.example.validation".to_string(),
                            name: "Grade".to_string(),
                        },
                        Multiplicity::REQUIRED,
                    ),
                    rex_ir::FeatureConstraints {
                        max_length: Some(3),
                        ..Default::default()
                    },
                ),
                constrained(
                    Feature::new(
                        "level",
                        FeatureKind::Attribute,
                        TypeRef::Vocabulary {
                            package: "nz.example.validation".to_string(),
                            name: "Level".to_string(),
                        },
                        Multiplicity::REQUIRED,
                    ),
                    rex_ir::FeatureConstraints {
                        minimum: Some(5),
                        ..Default::default()
                    },
                ),
                constrained(
                    Feature::new(
                        "alt_name",
                        FeatureKind::Attribute,
                        string(),
                        Multiplicity::OPTIONAL,
                    ),
                    string_bounds(2, 32),
                ),
                Feature::new(
                    "note",
                    FeatureKind::Attribute,
                    string(),
                    Multiplicity::REQUIRED,
                ),
            ],
        ));
        model.packages.push(package);
        model
    }

    fn generate_validation() -> String {
        generate(&validation_model())
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs")
    }

    #[test]
    fn constrained_class_gets_validate_reporting_bound_kinds() {
        let code = generate_validation();
        assert!(
            code.contains("pub fn validate(&self) -> Vec<rex_runtime::validation::Violation>"),
            "constrained class must carry a validate method:\n{code}"
        );
        assert!(
            code.contains("rex_runtime::validation::ConstraintKind::MinLength")
                && code.contains("rex_runtime::validation::ConstraintKind::MaxLength")
                && code.contains("rex_runtime::validation::ConstraintKind::Minimum")
                && code.contains("rex_runtime::validation::ConstraintKind::Maximum"),
            "all four bound kinds must be reported:\n{code}"
        );
        assert!(
            code.contains("feature: \"sku\".to_string()")
                || code.contains("feature: \"name\".to_string()"),
            "violations must name the declared feature:\n{code}"
        );
        assert!(
            code.contains("let len = value.chars().count();"),
            "string bounds check the element length:\n{code}"
        );
        assert!(
            code.contains("if len < 2 {") && code.contains("if len > 8 {"),
            "string bounds compare inclusively:\n{code}"
        );
        assert!(
            code.contains("detail: format!(\"length {len} is below minLength 2\")"),
            "min-length detail:\n{code}"
        );
        assert!(
            code.contains("detail: format!(\"length {len} exceeds maxLength 8\")"),
            "max-length detail:\n{code}"
        );
        assert!(
            code.contains("let number = i64::from(*value);"),
            "int bounds widen to i64 (bound literals must not overflow the field type):\n{code}"
        );
        assert!(
            code.contains("detail: format!(\"value {number} is below minimum 0\")")
                && code.contains("detail: format!(\"value {number} exceeds maximum 100\")"),
            "numeric details:\n{code}"
        );
        // The method lives inside the class's impl block.
        let impl_start = code.find("impl Profile {").expect("Profile impl");
        let validate_at = code.find("pub fn validate").expect("validate in output");
        assert!(
            validate_at > impl_start,
            "validate must be in the impl block"
        );
    }

    #[test]
    fn unconstrained_model_emits_no_validate() {
        let code = generate_currency();
        assert!(
            !code.contains("fn validate"),
            "classes without constrained features must not carry validate:\n{code}"
        );
    }

    #[test]
    fn many_valued_and_optional_attributes_check_elements() {
        let code = generate_validation();
        assert!(
            code.contains("for (index, value) in self.nicknames.iter().enumerate()"),
            "many-valued constraints apply per element:\n{code}"
        );
        assert!(
            code.contains("detail: format!(\"[{index}] length {len} is below minLength 2\")"),
            "element index must appear in the detail:\n{code}"
        );
        assert!(
            code.contains("if let Some(value) = &self.alt_name {"),
            "optional attributes check when present:\n{code}"
        );
    }

    #[test]
    fn enum_and_vocabulary_bounds_follow_value_and_key_facets() {
        let code = generate_validation();
        assert!(
            code.contains("let number = value.value();"),
            "enum numeric bounds check the literal value:\n{code}"
        );
        assert!(
            code.contains("let len = value.key().chars().count();"),
            "String-keyed vocabulary length bounds check the entry key:\n{code}"
        );
        assert!(
            code.contains("let number = value.code();"),
            "Int-keyed vocabulary numeric bounds use the key facet accessor:\n{code}"
        );
    }
}
