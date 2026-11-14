//! The node table and the resolution rules over it.
//!
//! Every node is registered here under its `(kind, name)` key while pass A
//! emits nodes; the emitters' pass B resolves references **only** through
//! this table, never by guessing. The rules mirror the compiler (see
//! `docs/GRAPH.md`):
//!
//! - a surface-qualified reference resolves by name;
//! - a bare intra-surface name resolves by the last `.`-separated segment;
//! - names pool **union-wide within one application** (block-local and
//!   module-local names are the same name across surfaces — the union
//!   integrity pass's rule) and never across applications;
//! - one match binds, several are returned as every match so the caller can
//!   emit all of them with `attributes["ambiguous"] = "true"`, and no match
//!   is no edge.
//!
//! [`Symbols`] is also the graph's declared-symbol oracle for the
//! per-surface emitters: `is_empty`-style questions ("is there any
//! deployment component at all?") keep an emitter from inventing a binding
//! when the counterpart surface is absent.

use std::collections::BTreeMap;

use rex_ir::graph::kinds;
use rex_ir::TypeRef;

/// One resolvable symbol: a node's id plus the coordinates resolution and
/// application scoping need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Symbol {
    /// The node id.
    pub(crate) id: String,
    /// The surface-qualified name.
    pub(crate) name: String,
    /// The node kind.
    pub(crate) kind: String,
    /// The declaring file (display path).
    pub(crate) file: String,
    /// The application the node belongs to.
    pub(crate) application: String,
}

/// The node table, pooled by kind and by file.
#[derive(Debug, Default)]
pub(crate) struct Symbols {
    by_kind: BTreeMap<String, BTreeMap<String, Vec<Symbol>>>,
    by_file: BTreeMap<String, Vec<Symbol>>,
}

impl Symbols {
    /// Registers a symbol. Callers dedupe by id first (the builder does).
    pub(crate) fn insert(&mut self, symbol: Symbol) {
        self.by_kind
            .entry(symbol.kind.clone())
            .or_default()
            .entry(symbol.name.clone())
            .or_default()
            .push(symbol.clone());
        self.by_file
            .entry(symbol.file.clone())
            .or_default()
            .push(symbol);
    }

    /// Every symbol of `kind` named `name` in `application`.
    pub(crate) fn all(&self, kind: &str, name: &str, application: &str) -> Vec<&Symbol> {
        self.by_kind
            .get(kind)
            .and_then(|names| names.get(name))
            .map(|symbols| {
                symbols
                    .iter()
                    .filter(|symbol| symbol.application == application)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The unique symbol of `kind` named `name` in `application`, or `None`
    /// when no symbol or several match.
    pub(crate) fn one(&self, kind: &str, name: &str, application: &str) -> Option<&Symbol> {
        match self.all(kind, name, application).as_slice() {
            [symbol] => Some(symbol),
            _ => None,
        }
    }

    /// Every symbol of `kind`, named `name` by its last `.`-segment, in
    /// `application` — the bare-spelling rule.
    pub(crate) fn by_last_segment<'a>(
        &'a self,
        kind: &str,
        name: &str,
        application: &str,
    ) -> Vec<&'a Symbol> {
        self.scoped(kind, application)
            .into_iter()
            .filter(|symbol| super::ids::last_segment(&symbol.name) == name)
            .collect()
    }

    /// Every symbol of `kind` in `application` whose name ends with
    /// `.<suffix>` (or equals `suffix`) — the qualified-spelling rule for
    /// cross-surface references carrying their own qualification
    /// (`support.TicketService.raiseRefund`).
    pub(crate) fn by_suffix<'a>(
        &'a self,
        kind: &str,
        suffix: &str,
        application: &str,
    ) -> Vec<&'a Symbol> {
        let dotted = format!(".{suffix}");
        self.scoped(kind, application)
            .into_iter()
            .filter(|symbol| symbol.name == suffix || symbol.name.ends_with(&dotted))
            .collect()
    }

    /// Every symbol of `kind` declared in `file`.
    pub(crate) fn in_file(&self, kind: &str, file: &str) -> Vec<&Symbol> {
        self.by_file
            .get(file)
            .map(|symbols| {
                symbols
                    .iter()
                    .filter(|symbol| symbol.kind == kind)
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Every capability or actor named `name` in `application` — the pooled
    /// `requires:` / `roles:` / `.uc` step universe.
    pub(crate) fn capability_or_actor<'a>(
        &'a self,
        name: &str,
        application: &str,
    ) -> Vec<&'a Symbol> {
        let mut matches = self.by_last_segment(kinds::CAPABILITY, name, application);
        matches.extend(self.by_last_segment(kinds::ACTOR, name, application));
        matches
    }

    /// Every service operation of `application` whose operation name (its
    /// last segment) equals `name` ASCII-case-insensitively — the #114
    /// action-binding rule.
    pub(crate) fn service_operations_named<'a>(
        &'a self,
        name: &str,
        application: &str,
    ) -> Vec<&'a Symbol> {
        self.scoped(kinds::SERVICE_OPERATION, application)
            .into_iter()
            .filter(|symbol| super::ids::last_segment(&symbol.name).eq_ignore_ascii_case(name))
            .collect()
    }

    /// Every event of `application` named `name` by its last segment.
    pub(crate) fn events_named<'a>(&'a self, name: &str, application: &str) -> Vec<&'a Symbol> {
        self.by_last_segment(kinds::EVENT, name, application)
    }

    /// The decision or decision-service universe a `Model.Service` reference
    /// resolves against: decision services named `name` by their last
    /// segment first, then decisions (DMN's rule-17 spelling).
    pub(crate) fn decision_target<'a>(&'a self, name: &str, application: &str) -> Vec<&'a Symbol> {
        let services = self.by_last_segment(kinds::DECISION_SERVICE, name, application);
        if services.is_empty() {
            self.by_last_segment(kinds::DECISION, name, application)
        } else {
            services
        }
    }

    /// Resolves a designed class reference: the authored spelling is either
    /// unqualified (`Ticket`), package-qualified with dots
    /// (`nz.example.support.Ticket` — the `.ddd` `base` convention) or
    /// qualified with `::` (the driver's `package::Class` form). Order:
    /// exact, `base`-qualified, then unique bare name.
    pub(crate) fn resolve_class<'a>(
        &'a self,
        authored: &str,
        base: Option<&str>,
        application: &str,
    ) -> Option<&'a Symbol> {
        let normalized = authored.replace("::", ".");
        if let Some(symbol) = self.one(kinds::CLASS, &normalized, application) {
            return Some(symbol);
        }
        if let Some(base) = base {
            let qualified = format!("{base}.{normalized}");
            if let Some(symbol) = self.one(kinds::CLASS, &qualified, application) {
                return Some(symbol);
            }
        }
        if let Some(base) = base {
            let qualified = format!("{base}.{}", super::ids::last_segment(&normalized));
            if let Some(symbol) = self.one(kinds::CLASS, &qualified, application) {
                return Some(symbol);
            }
        }
        // The bare name pools by last segment, exactly like a `.mox` type
        // reference inside its own package; several packages declaring the
        // same class name is unresolved rather than invented.
        match self
            .by_last_segment(
                kinds::CLASS,
                super::ids::last_segment(&normalized),
                application,
            )
            .as_slice()
        {
            [symbol] => Some(symbol),
            _ => None,
        }
    }

    /// Resolves a resolved [`TypeRef`] to its type node: classes and
    /// interfaces are `class` nodes, enums and datatypes their own kinds.
    /// Primitives and vocabularies have no node kind in v1 (no edge).
    pub(crate) fn type_target(&self, type_ref: &TypeRef, application: &str) -> Option<&Symbol> {
        let (kind, package, name) = match type_ref {
            TypeRef::Class { package, name } | TypeRef::Interface { package, name } => {
                (kinds::CLASS, package, name)
            }
            TypeRef::Enum { package, name } => (kinds::ENUM, package, name),
            TypeRef::Datatype { package, name } => (kinds::DATATYPE, package, name),
            TypeRef::Primitive(_) | TypeRef::Vocabulary { .. } => return None,
        };
        self.one(kind, &super::ids::qualify(&[package, name]), application)
    }

    /// Every symbol of `kind` in one application — the pooled universe every
    /// resolver starts from. The items borrow `self` only, so a caller may
    /// filter by a local spelling without tying the result to it.
    fn scoped<'a>(&'a self, kind: &str, application: &str) -> Vec<&'a Symbol> {
        self.by_kind
            .get(kind)
            .into_iter()
            .flat_map(|names| names.values())
            .flatten()
            .filter(|symbol| symbol.application == application)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn symbol(kind: &str, name: &str, file: &str, application: &str) -> Symbol {
        Symbol {
            id: format!("{kind}:{file}#{name}"),
            name: name.to_string(),
            kind: kind.to_string(),
            file: file.to_string(),
            application: application.to_string(),
        }
    }

    fn table() -> Symbols {
        let mut symbols = Symbols::default();
        symbols.insert(symbol(
            kinds::CLASS,
            "nz.example.support.Ticket",
            "support.mox",
            "a",
        ));
        symbols.insert(symbol(
            kinds::CAPABILITY,
            "Support.RaiseRefund",
            "support.actor",
            "a",
        ));
        symbols.insert(symbol(kinds::ACTOR, "Support.Agent", "support.actor", "a"));
        symbols.insert(symbol(
            kinds::SERVICE_OPERATION,
            "SupportDesk.desk.TicketService.raiseRefund",
            "support.ddd",
            "a",
        ));
        symbols.insert(symbol(kinds::CLASS, "other.Ticket", "other.mox", "b"));
        symbols
    }

    #[test]
    fn lookups_never_cross_applications() {
        let symbols = table();
        assert_eq!(
            symbols.by_last_segment(kinds::CLASS, "Ticket", "a").len(),
            1
        );
        assert_eq!(
            symbols.by_last_segment(kinds::CLASS, "Ticket", "b").len(),
            1
        );
        assert!(symbols
            .by_last_segment(kinds::CLASS, "Ticket", "c")
            .is_empty());
    }

    #[test]
    fn class_resolution_prefers_exact_then_base_then_bare() {
        let symbols = table();
        assert_eq!(
            symbols
                .resolve_class("Ticket", Some("nz.example.support"), "a")
                .map(|symbol| symbol.name.as_str()),
            Some("nz.example.support.Ticket")
        );
        assert_eq!(
            symbols
                .resolve_class("nz.example.support::Ticket", None, "a")
                .map(|symbol| symbol.name.as_str()),
            Some("nz.example.support.Ticket")
        );
        assert!(symbols.resolve_class("Nothing", None, "a").is_none());
    }

    #[test]
    fn action_binding_is_case_insensitive_on_the_operation_segment() {
        let symbols = table();
        let matches = symbols.service_operations_named("ApproveRefund", "a");
        assert!(matches.is_empty());
        let matches = symbols.service_operations_named("RaiseRefund", "a");
        assert_eq!(matches.len(), 1);
    }

    #[test]
    fn pooled_names_include_capabilities_and_actors() {
        let symbols = table();
        assert_eq!(symbols.capability_or_actor("RaiseRefund", "a").len(), 1);
        assert_eq!(symbols.capability_or_actor("Agent", "a").len(), 1);
        assert!(symbols.capability_or_actor("RaiseRefund", "b").is_empty());
    }
}
