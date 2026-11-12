//! The domain-type index every surface feeds before checking:
//! [`DomainTypes`], [`ClassInfo`], and [`FunctionSignature`].

use std::collections::BTreeMap;

use rex_ir::{Feature, Model, Multiplicity, Operation, OperationParam, TypeRef};

use crate::types::NamedKind;

/// The resolvable members of one class.
#[derive(Debug, Clone, Default)]
pub struct ClassInfo {
    pub extends: Vec<TypeRef>,
    pub features: Vec<Feature>,
    pub operations: Vec<Operation>,
}

/// The signature of one sigil-lowered package function (`rex_ir::Package::functions`),
/// registered under its bare name. Several packages may declare the same
/// bare name: their signatures coexist in insertion order, and a call
/// resolving to more than one is an ambiguity error naming every package.
#[derive(Debug, Clone)]
pub struct FunctionSignature {
    /// The declaring package's dotted name.
    pub package: String,
    /// The resolved return type.
    pub return_type: TypeRef,
    /// The declared return cardinality; `None` is single-valued. Lifts the
    /// call's type the same way a parameter's multiplicity lifts an
    /// argument's (`return_result_ty`).
    pub return_multiplicity: Option<Multiplicity>,
    /// Parameters in declaration order.
    pub params: Vec<OperationParam>,
}

/// The unified domain-object index: the generic binding seam every surface
/// lowers into. Classes carry their resolvable members (features, operations,
/// supertypes); every other named type is registered by kind; enums may
/// additionally register their declared literal names — the closed
/// vocabulary the enum-literal namespace and the membership check read
/// (rule L3). Built from a
/// [`rex_ir::Model`] — which is what `.mox` compiles to and what sigil
/// imports, schema imports, `.ddd`, and `.evt` all resolve against — or
/// assembled incrementally with [`DomainTypes::insert_class`],
/// [`DomainTypes::insert_named`], and [`DomainTypes::insert_enum`], so
/// surfaces without a full `Model` can still contribute bindable domain
/// objects.
///
/// The member registry is scoped to enums. Vocabularies are deferred
/// deliberately: their entries are lockfile-backed facets whose member
/// semantics differ (entry keys, not nominal literals), and an entry set can
/// be legitimately empty before a snapshot is vendored — a closed-set check
/// over an absent snapshot would false-positive. Register one with
/// [`DomainTypes::insert_named`] as today; its members stay unchecked.
#[derive(Debug, Clone, Default)]
pub struct DomainTypes {
    classes: BTreeMap<(String, String), ClassInfo>,
    pub(crate) named: BTreeMap<(String, String), NamedKind>,
    pub(crate) functions: BTreeMap<String, Vec<FunctionSignature>>,
    members: BTreeMap<(String, String), Vec<String>>,
}

impl DomainTypes {
    /// Builds a domain-type index from a resolved model.
    pub fn from_model(model: &Model) -> Self {
        let mut context = DomainTypes::default();
        for package in &model.packages {
            for class in &package.classes {
                context.insert_class(
                    &package.name,
                    &class.name,
                    class.extends.clone(),
                    class.features.clone(),
                    class.operations.clone(),
                );
            }
            for enum_ in &package.enums {
                context.insert_enum(
                    &package.name,
                    &enum_.name,
                    enum_
                        .literals
                        .iter()
                        .map(|literal| literal.name.clone())
                        .collect(),
                );
            }
            for datatype in &package.datatypes {
                context.insert_named(&package.name, &datatype.name, NamedKind::Datatype);
            }
            for interface in &package.interfaces {
                context.insert_named(&package.name, &interface.name, NamedKind::Interface);
            }
            for vocabulary in &package.vocabularies {
                context.insert_named(&package.name, &vocabulary.name, NamedKind::Vocabulary);
            }
            for function in &package.functions {
                context.insert_function(
                    &function.name,
                    FunctionSignature {
                        package: package.name.clone(),
                        return_type: function.return_type.clone(),
                        return_multiplicity: function.return_multiplicity,
                        params: function.params.clone(),
                    },
                );
            }
        }
        context
    }

    /// Registers one class with its resolvable members, replacing any
    /// previous entry (and its kind) for the same (package, name).
    pub fn insert_class(
        &mut self,
        package: &str,
        name: &str,
        extends: Vec<TypeRef>,
        features: Vec<Feature>,
        operations: Vec<Operation>,
    ) {
        let key = (package.to_string(), name.to_string());
        self.named.insert(key.clone(), NamedKind::Class);
        self.classes.insert(
            key,
            ClassInfo {
                extends,
                features,
                operations,
            },
        );
    }

    /// Registers a non-class named type (datatype, interface, vocabulary,
    /// or a member-less enum) by kind.
    pub fn insert_named(&mut self, package: &str, name: &str, kind: NamedKind) {
        self.named
            .insert((package.to_string(), name.to_string()), kind);
    }

    /// Registers one enum with its declared literal names, replacing any
    /// previous entry (and its kind) for the same (package, name). The
    /// members are the closed set the enum-literal namespace resolves
    /// against (rule L3 of `docs/EXPRESSIONS.md`); a host that registers
    /// only the kind via [`DomainTypes::insert_named`] leaves the set open
    /// — the namespace and the membership check then defer silently.
    pub fn insert_enum(&mut self, package: &str, name: &str, members: Vec<String>) {
        let key = (package.to_string(), name.to_string());
        self.named.insert(key.clone(), NamedKind::Enum);
        self.members.insert(key, members);
    }

    /// The declared literal names of one registered enum, in declaration
    /// order; `None` when the enum was registered without members.
    pub fn enum_members(&self, package: &str, name: &str) -> Option<&[String]> {
        self.members
            .get(&(package.to_string(), name.to_string()))
            .map(|members| members.as_slice())
    }

    /// Every member-registered enum whose name matches `name`, as
    /// `(package, name)` keys. The bare-name resolution input of the
    /// enum-literal namespace: exactly one match resolves, more are
    /// ambiguous (never guessed), none leaves the name to the ordinary
    /// scope rules.
    pub fn enums_named(&self, name: &str) -> Vec<(String, String)> {
        self.members
            .keys()
            .filter(|(_, enum_name)| enum_name == name)
            .cloned()
            .collect()
    }

    /// Registers one free-function signature under its bare name. Further
    /// insertions under the same name (same or different package) coexist
    /// in insertion order; a call is ambiguous when more than one remains.
    pub fn insert_function(&mut self, name: &str, signature: FunctionSignature) {
        self.functions
            .entry(name.to_string())
            .or_default()
            .push(signature);
    }

    /// Merges `other` into `self`; `other`'s entries win on collision. This
    /// is how domain unions are assembled: one [`DomainTypes`] per
    /// contributing surface, combined before binding. The function tables
    /// union instead: per-name signature vecs append, so neither side's
    /// candidates are dropped (map-level `extend` would drop the left vec).
    pub fn merge(&mut self, other: DomainTypes) {
        self.classes.extend(other.classes);
        self.named.extend(other.named);
        self.members.extend(other.members);
        for (name, signatures) in other.functions {
            self.functions.entry(name).or_default().extend(signatures);
        }
    }

    /// Looks up a class's resolvable members.
    pub fn class(&self, package: &str, name: &str) -> Option<&ClassInfo> {
        self.classes.get(&(package.to_string(), name.to_string()))
    }

    /// Looks up a named type's kind.
    pub fn named_kind(&self, package: &str, name: &str) -> Option<NamedKind> {
        self.named
            .get(&(package.to_string(), name.to_string()))
            .copied()
    }
}
