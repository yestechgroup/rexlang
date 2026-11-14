//! Integration tests for the AST navigation index: definitions, type-ref and
//! opposite references, offset lookups, and robustness on broken sources.

use rex_driver::navigation::{FeatureSymbolKind, Lookup, NavigationIndex, Reference, SymbolKind};
use rex_driver::{Definition, Span};

const SOURCE: &str = r#"
package demo

class Library {
    contains Book[] books opposite library
}

class Book {
    container Library library opposite books
    String title
    Currency price
}

class Special extends Book {
    op int compute(int x)
    derived String label
    Book sequel
}

enum Color { Red = 0 }

type Date wraps opaque

vocabulary Currency from "iso:4217" {
    key code
    facet String code
}
"#;

/// Byte span of the `nth` (0-based) occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str, nth: usize) -> Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

fn index_of(source: &str) -> NavigationIndex {
    NavigationIndex::build_or_empty(source)
}

/// The single definition named `name`, or a panic.
fn find_def<'a>(index: &'a NavigationIndex, name: &str) -> (usize, &'a Definition) {
    let mut matches: Vec<(usize, &Definition)> = index
        .definitions()
        .filter(|(_, definition)| definition.name == name)
        .collect();
    assert!(!matches.is_empty(), "no definition named '{name}'");
    assert_eq!(matches.len(), 1, "multiple definitions named '{name}'");
    matches.remove(0)
}

/// The reference found at the start of `span` (exercising `at`), or a panic.
fn find_ref(index: &NavigationIndex, span: Span) -> &Reference {
    match index.at(span.start) {
        Lookup::Reference(reference) => {
            assert_eq!(reference.span, span, "reference span mismatch");
            reference
        }
        other => panic!("no reference at span {span:?}, got {other:?}"),
    }
}

#[test]
fn top_level_definitions_are_indexed_with_exact_name_spans() {
    let index = index_of(SOURCE);

    for (name, kind, nth) in [
        ("Library", SymbolKind::Class, 0),
        ("Book", SymbolKind::Class, 1),
        ("Special", SymbolKind::Class, 0),
        ("Color", SymbolKind::Enum, 0),
        ("Date", SymbolKind::Datatype, 0),
        ("Currency", SymbolKind::Vocabulary, 1),
    ] {
        let (id, definition) = find_def(&index, name);
        assert_eq!(definition.kind, kind, "kind of '{name}'");
        assert_eq!(definition.owner, None, "owner of '{name}'");
        assert_eq!(
            definition.name_span,
            span_of(SOURCE, name, nth),
            "span of '{name}'"
        );
        assert_eq!(index.definition(id).name, name);
    }
}

#[test]
fn features_are_indexed_with_owner_and_kind() {
    let index = index_of(SOURCE);

    let (library, _) = find_def(&index, "Library");
    let (book, _) = find_def(&index, "Book");
    let (special, _) = find_def(&index, "Special");
    let (color, _) = find_def(&index, "Color");
    let (currency, _) = find_def(&index, "Currency");

    for (name, kind, owner, nth) in [
        (
            "books",
            SymbolKind::Feature(FeatureSymbolKind::Containment),
            library,
            0,
        ),
        (
            "library",
            SymbolKind::Feature(FeatureSymbolKind::Container),
            book,
            1,
        ),
        (
            "title",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            book,
            0,
        ),
        (
            "price",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            book,
            0,
        ),
        (
            "compute",
            SymbolKind::Feature(FeatureSymbolKind::Operation),
            special,
            0,
        ),
        (
            "label",
            SymbolKind::Feature(FeatureSymbolKind::Derived),
            special,
            0,
        ),
        (
            "sequel",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            special,
            0,
        ),
        ("Red", SymbolKind::EnumLiteral, color, 0),
        (
            "code",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            currency,
            1,
        ),
    ] {
        let (_, definition) = find_def(&index, name);
        assert_eq!(definition.kind, kind, "kind of '{name}'");
        assert_eq!(definition.owner, Some(owner), "owner of '{name}'");
        assert_eq!(
            definition.name_span,
            span_of(SOURCE, name, nth),
            "span of '{name}'"
        );
    }
}

#[test]
fn type_references_resolve_to_declarations() {
    let index = index_of(SOURCE);

    let (library, _) = find_def(&index, "Library");
    let (book, _) = find_def(&index, "Book");
    let (currency, _) = find_def(&index, "Currency");

    // `contains Book[] books` — the containment type ref. (Library is
    // declared first, so this mention precedes the `class Book` declaration.)
    let reference = find_ref(&index, span_of(SOURCE, "Book", 0));
    assert_eq!(reference.target, Some(book));

    // `container Library library` — the container type ref.
    let reference = find_ref(&index, span_of(SOURCE, "Library", 1));
    assert_eq!(reference.target, Some(library));

    // `extends Book` — inheritance targets classes.
    let reference = find_ref(&index, span_of(SOURCE, "Book", 2));
    assert_eq!(reference.target, Some(book));

    // `Currency price` — an attribute typed by a vocabulary. (The attribute
    // precedes the vocabulary declaration itself in the source.)
    let reference = find_ref(&index, span_of(SOURCE, "Currency", 0));
    assert_eq!(reference.target, Some(currency));

    // Primitives resolve to None but are still recorded as references.
    for primitive_span in [
        span_of(SOURCE, "String", 0), // `String title`
        span_of(SOURCE, "String", 1), // `derived String label`
        span_of(SOURCE, "String", 2), // `facet String code`
        span_of(SOURCE, "int", 0),    // `op int compute`
        span_of(SOURCE, "int", 1),    // `int x` parameter
    ] {
        let reference = find_ref(&index, primitive_span);
        assert_eq!(reference.target, None, "primitive at {primitive_span:?}");
        assert!(index.resolve(reference).is_none());
    }
}

#[test]
fn unknown_type_reference_records_target_none() {
    let source = "package demo\n\nclass C {\n    Mystery f\n}\n";
    let index = index_of(source);
    let (c, _) = find_def(&index, "C");

    let reference = find_ref(&index, span_of(source, "Mystery", 0));
    assert_eq!(reference.target, None);
    assert!(index.resolve(reference).is_none());
    // ...and the feature itself is still a definition owned by C.
    let (_, feature) = find_def(&index, "f");
    assert_eq!(feature.owner, Some(c));
}

#[test]
fn qualified_two_segment_type_references_resolve() {
    let source = "package demo\n\nclass C {\n    demo.C me\n}\n";
    let index = index_of(source);
    let (c, _) = find_def(&index, "C");

    // The type ref span covers the whole qualified name `demo.C`.
    let reference = find_ref(&index, span_of(source, "demo.C", 0));
    assert_eq!(reference.target, Some(c));

    // A two-segment name with a foreign package head does not resolve.
    let source = "package demo\n\nclass C {\n    other.Thing me\n}\n";
    let index = index_of(source);
    let reference = find_ref(&index, span_of(source, "other.Thing", 0));
    assert_eq!(reference.target, None);
}

#[test]
fn opposite_mentions_resolve_to_the_counterpart_feature() {
    let index = index_of(SOURCE);

    // In Library: `contains Book[] books opposite library` — the opposite
    // mention `library` (occurrence 0) resolves to Book.library's definition.
    let (book_library, _) = find_def(&index, "library");
    let reference = find_ref(&index, span_of(SOURCE, "library", 0));
    assert_eq!(reference.target, Some(book_library));

    // In Book: `container Library library opposite books` — the opposite
    // mention `books` (occurrence 1) resolves to Library.books's definition.
    let (library_books, _) = find_def(&index, "books");
    let reference = find_ref(&index, span_of(SOURCE, "books", 1));
    assert_eq!(reference.target, Some(library_books));
}

#[test]
fn invalid_opposite_resolves_to_none() {
    let source = r#"
package demo

class Node {
    contains Leaf[] kids opposite nowhere
}

class Leaf {
    container Node parent opposite kids
}
"#;
    let index = index_of(source);

    // The typo'd opposite mention `nowhere` is a reference with no target.
    let reference = find_ref(&index, span_of(source, "nowhere", 0));
    assert_eq!(reference.target, None);
    assert!(index.resolve(reference).is_none());

    // The valid reverse mention still resolves.
    let (leaf_kids, _) = find_def(&index, "kids");
    let reference = find_ref(&index, span_of(source, "kids", 1));
    assert_eq!(reference.target, Some(leaf_kids));
}

#[test]
fn at_returns_definitions_and_references() {
    let index = index_of(SOURCE);
    let (book, book_def) = find_def(&index, "Book");

    // Offset inside the class name `Book` → its definition.
    let name_span = span_of(SOURCE, "Book", 1);
    match index.at(name_span.start + 1) {
        Lookup::Definition(id, definition) => {
            assert_eq!(id, book);
            assert_eq!(definition, book_def);
        }
        other => panic!("expected a definition, got {other:?}"),
    }

    // Offset inside the type ref `Book` in `contains Book[] books` → reference.
    let type_span = span_of(SOURCE, "Book", 0);
    match index.at(type_span.start + 1) {
        Lookup::Reference(reference) => assert_eq!(reference.target, Some(book)),
        other => panic!("expected a reference, got {other:?}"),
    }

    // Offset inside the opposite mention `library` → reference.
    let opposite_span = span_of(SOURCE, "library", 0);
    match index.at(opposite_span.start + 1) {
        Lookup::Reference(reference) => {
            let (book_library, _) = find_def(&index, "library");
            assert_eq!(reference.target, Some(book_library));
        }
        other => panic!("expected a reference, got {other:?}"),
    }

    // An offset that hits nothing → None.
    assert_eq!(index.at(0), Lookup::None);
}

#[test]
fn references_to_finds_all_mentions() {
    let index = index_of(SOURCE);
    let (book, _) = find_def(&index, "Book");

    // Every mention of Book except the definition itself: the `contains`
    // type ref, the `extends` clause, and the `Book sequel` feature type.
    let mut spans = index.references_to(book);
    spans.sort_by_key(|span| span.start);
    assert_eq!(
        spans,
        vec![
            span_of(SOURCE, "Book", 0),
            span_of(SOURCE, "Book", 2),
            span_of(SOURCE, "Book", 3),
        ]
    );

    let (library, _) = find_def(&index, "Library");
    assert_eq!(
        index.references_to(library),
        vec![span_of(SOURCE, "Library", 1)]
    );
}

#[test]
fn definitions_iterate_in_source_order() {
    let index = index_of(SOURCE);
    let names: Vec<&str> = index.definitions().map(|(_, d)| d.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Library", "books", "Book", "library", "title", "price", "Special", "compute", "label",
            "sequel", "Color", "Red", "Date", "Currency", "code",
        ]
    );
}

#[test]
fn build_or_empty_handles_garbage_without_panicking() {
    // Garbage tokens: either no AST or only junk declarations.
    let index = index_of("%%% never a valid model :( %%%");
    assert_eq!(index.definitions().count(), 0);
    assert_eq!(index.at(0), Lookup::None);

    // A lex-level failure (no AST at all) is fine too.
    let index = index_of("\"unterminated");
    assert_eq!(index.definitions().count(), 0);
}

#[test]
fn resolve_id_returns_the_target_id_of_a_reference() {
    let source = "package demo\n\n\
        class Library {\n\
        \x20   contains Book[] books opposite library\n\
        \x20   Date ^when\n\
        }\n\n\
        class Book { container Library library opposite books }\n";
    let index = index_of(source);
    let (library_id, _) = index
        .definitions()
        .find(|(_, d)| d.name == "library" && d.owner.is_some())
        .expect("feature `library`");

    // The `opposite library` mention resolves to that feature's id.
    let (_, reference) = index
        .references()
        .find(|(_, reference)| reference.target == Some(library_id))
        .expect("a reference to `library`");
    assert_eq!(index.resolve_id(reference), Some(library_id));

    // `Date` is not declared: the reference is indexed but unresolved.
    let (_, date_ref) = index
        .references()
        .find(|(span, _)| source[span.start..span.end] == *"Date")
        .expect("the Date type reference");
    assert_eq!(date_ref.target, None);
    assert_eq!(index.resolve_id(date_ref), None);
}

#[test]
fn feature_definitions_carry_their_modifiers() {
    let source = "package demo\n\n\
        class Person {\n\
        \x20   id String email\n\
        \x20   readonly id String handle\n\
        \x20   String plain\n\
        }\n";
    let index = NavigationIndex::build_or_empty(source);
    let find = |name: &str| {
        index
            .definitions()
            .find(|(_, d)| d.name == name && d.owner.is_some())
            .map(|(_, d)| d)
            .unwrap_or_else(|| panic!("no feature '{name}'"))
    };
    assert!(find("email").modifiers.is_id(), "email carries `id`");
    assert!(!find("email").modifiers.is_read_only());
    assert!(find("handle").modifiers.is_id());
    assert!(find("handle").modifiers.is_read_only());
    assert!(find("plain").modifiers.is_empty(), "plain stays empty");
}

#[test]
fn register_declarations_index_as_classes_with_their_attributes() {
    // A `register` declaration navigates like a class (issue rexlang#181):
    // the declaration is a Class symbol carrying its doc, its attributes are
    // owned feature definitions, and references to the register class name
    // resolve onto the declaration.
    let source = r#"
package demo

/// A role in the organisation.
register Role {
    /// The role's display name.
    String display_name
    refers does not parse here
}

class Person {
    refers Role[0..*] roles
}
"#;
    let source = source.replace("    refers does not parse here\n", "    int level\n");
    let index = index_of(&source);

    let (role, definition) = find_def(&index, "Role");
    assert_eq!(definition.kind, SymbolKind::Class, "a register is a class");
    assert_eq!(
        definition.doc.as_deref(),
        Some("A role in the organisation.")
    );

    let (_, display_name) = find_def(&index, "display_name");
    assert_eq!(
        display_name.kind,
        SymbolKind::Feature(FeatureSymbolKind::Attribute)
    );
    assert_eq!(display_name.owner, Some(role));
    assert_eq!(
        display_name.doc.as_deref(),
        Some("The role's display name.")
    );
    let (_, level) = find_def(&index, "level");
    assert_eq!(level.owner, Some(role));

    // `refers Role[0..*] roles` — the type reference targets the register
    // declaration (occurrence 1; occurrence 0 is the declaration's name).
    let reference = find_ref(&index, span_of(&source, "Role", 1));
    assert_eq!(reference.target, Some(role));
}

// ---------------------------------------------------------------------------
// The shared spanned indexes vs. the semantic graph (issue rexlang#245)
// ---------------------------------------------------------------------------
//
// The graph artifact is the one node table; a `SurfaceIndex` is the per-surface
// span table. These tests are the anti-drift gate: every declaration the graph
// emits a node for must have exactly one spanned definition site in its
// surface's index, under the same `(kind, name)` key.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::graph::{build, GraphFile, GraphInputs};
use rex_driver::navigation::{
    actors_index, cmmn_index, ddd_index, deploy_index, dmn_index, evt_index, ifml_surface,
    mox_index, uc_index, SurfaceIndex,
};
use rex_ir::graph::{kinds, GraphModel};

/// Every spanned surface, each with the domain its compile needs (empty for
/// the artifact surfaces that resolve their own imports from the support
/// stack).
///
/// `payrun.ddd` and `childops.ddd` are the conformance designs the support
/// fixture does not exercise — a state machine with transitions, and
/// module-level `extend` child operations — so the equality covers them too.
/// `models/actors.mox` carries an inline `actors` block (the `.mox`
/// surface's own actor/capability nodes); `support.ifml` is the interaction
/// flow, compiled through the support stack like the other artifact
/// surfaces.
const SURFACES: [(&str, &str); 12] = [
    ("examples/support.actor", "examples/support.mox"),
    ("examples/support.ddd", "examples/support.mox"),
    ("examples/support.evt", "examples/support.mox"),
    ("examples/support.deploy", ""),
    ("examples/support.uc", ""),
    ("examples/support.cmmn", ""),
    ("examples/support.dmn", ""),
    ("examples/support.ifml", ""),
    ("examples/support.mox", ""),
    ("tests/conformance/models/actors.mox", ""),
    (
        "tests/conformance/ddd/payrun.ddd",
        "tests/conformance/ddd/payrun.mox",
    ),
    (
        "tests/conformance/ddd/childops.ddd",
        "tests/conformance/ddd/childops.mox",
    ),
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

/// The canonical path of a root-relative display path.
fn disk(display: &str) -> PathBuf {
    std::fs::canonicalize(workspace_root().join(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

fn fixture(display: &str) -> String {
    std::fs::read_to_string(disk(display))
        .unwrap_or_else(|error| panic!("missing fixture file {display}: {error}"))
}

/// The `import "..."` strings of a source, in order.
fn import_strings(source: &str) -> Vec<String> {
    let mut imports = Vec::new();
    for line in source.lines() {
        let line = line.trim_start();
        let Some(rest) = line.strip_prefix("import ") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('"') else {
            continue;
        };
        if let Some(end) = rest.find('"') {
            imports.push(rest[..end].to_string());
        }
    }
    imports
}

/// One graph input file: display path, normalized key, disk-resolved imports.
fn graph_file<M>(display: &str, model: M) -> GraphFile<M> {
    let path = disk(display);
    let imports = import_strings(&fixture(display))
        .into_iter()
        .filter_map(|import| std::fs::canonicalize(path.parent()?.join(import)).ok())
        .map(|path| path.display().to_string())
        .collect();
    GraphFile::new(display, path.display().to_string(), imports, model)
}

/// The graph over one fixture file, compiled exactly the way `tests/graph.rs`
/// compiles the examples (imports resolved from disk next to the file).
///
/// The four artifact surfaces (`deploy`, `uc`, `cmmn`, `dmn`) cannot compile
/// alone — their imports are pre-compiled artifacts — so they read the
/// support stack's single graph instead (see [`support_graph`]).
fn single_file_graph(display: &str, domain: &str) -> GraphModel {
    let extension = display.rsplit_once('.').map(|(_, extension)| extension);
    if matches!(extension, Some("deploy" | "uc" | "cmmn" | "dmn" | "ifml")) {
        return support_graph().clone();
    }
    let source = fixture(display);
    if display.ends_with(".mox") {
        let path = disk(display).display().to_string();
        let compilation =
            rex_driver::compile_str(&path, &source, &rex_driver::DomainImports::default());
        let model = lowered(display, compilation.model, &compilation.diagnostics);
        let mut inputs = GraphInputs::default();
        inputs.domains.push(graph_file(display, model));
        return build(&inputs);
    }
    let path = disk(display).display().to_string();
    let mut inputs = GraphInputs::default();
    if display.ends_with(".actor") {
        // The `.actor` compile keys its imports by the string as written.
        let directory = disk(display).parent().expect("a directory").to_path_buf();
        let domains: Vec<(String, String)> = import_strings(&source)
            .into_iter()
            .filter_map(|import| {
                std::fs::read_to_string(directory.join(&import))
                    .ok()
                    .map(|text| (import, text))
            })
            .collect();
        let compilation = rex_driver::compile_actors_str(
            &path,
            &source,
            &domains,
            &rex_driver::DomainImports::default(),
        );
        let model = compilation
            .model
            .unwrap_or_else(|| panic!("{display} must compile: {:?}", compilation.diagnostics));
        inputs.actors.push(graph_file(display, model));
    } else {
        // The `.ddd`/`.evt` compiles key their imports by canonical path.
        let domains = [(disk(domain).display().to_string(), fixture(domain))];
        if display.ends_with(".ddd") {
            let compilation = rex_driver::compile_ddd_str(
                &path,
                &source,
                &domains,
                &rex_driver::DomainImports::default(),
            );
            let model = compilation
                .model
                .unwrap_or_else(|| panic!("{display} must compile: {:?}", compilation.diagnostics));
            inputs.designs.push(graph_file(display, model));
        } else {
            let compilation = rex_driver::compile_evt_str(
                &path,
                &source,
                &domains,
                &rex_driver::DomainImports::default(),
            );
            let model = compilation
                .model
                .unwrap_or_else(|| panic!("{display} must compile: {:?}", compilation.diagnostics));
            inputs.events.push(graph_file(display, model));
        }
    }
    build(&inputs)
}

/// The surface's spanned index, built from the fixture's source text.
fn surface_index(display: &str) -> (String, SurfaceIndex) {
    let source = fixture(display);
    let index = match display.rsplit_once('.').map(|(_, extension)| extension) {
        Some("actor") => actors_index(&source),
        Some("ddd") => ddd_index(&source),
        Some("evt") => evt_index(&source, display),
        Some("deploy") => deploy_index(&source),
        Some("uc") => uc_index(&source),
        Some("cmmn") => cmmn_index(&source),
        Some("dmn") => dmn_index(&source, display),
        Some("mox") => mox_index(&source),
        Some("ifml") => {
            let (model, index) = rex_ifml::parse_ifml_indexed(&source)
                .unwrap_or_else(|error| panic!("{display} must parse: {error:?}"));
            ifml_surface(&model, &index)
        }
        _ => panic!("no index builder for {display}"),
    };
    (source, index)
}

// ---------------------------------------------------------------------------
// The support stack: the reference application, every surface compiled
// ---------------------------------------------------------------------------
//
// The four artifact surfaces import pre-compiled artifacts, so their graph
// needs the whole stack. It is compiled once (the fixture pattern of
// `tests/graph.rs`) and shared.

/// The pattern files `support.ifml` instantiates (its `walk_ifml_imports`
/// closure), hardcoded per the house convention: a missing file fails loudly.
const PATTERN_FLOWS: [&str; 6] = [
    "patterns/ifml/collections.ifml",
    "patterns/ifml/communication.ifml",
    "patterns/ifml/data.ifml",
    "patterns/ifml/input.ifml",
    "patterns/ifml/navigation.ifml",
    "patterns/ifml/workflow.ifml",
];

/// One file's compiled artifact, panicking with its diagnostics when it does
/// not lower.
fn lowered<T, D: std::fmt::Debug>(display: &str, model: Option<T>, diagnostics: &D) -> T {
    model.unwrap_or_else(|| panic!("{display} must compile: {diagnostics:?}"))
}

/// One `.ifml` file compiled with its `.ifml` import closure gathered from
/// disk through the shared traversal.
fn flow_model(display: &str) -> rex_ir::ifml::IfmlModel {
    let path = disk(display);
    let key = path.display().to_string();
    let source = fixture(display);
    let mut imports = rex_ifml::IfmlImports::default();
    let mut disk_paths = std::collections::BTreeMap::new();
    disk_paths.insert(key.clone(), path.clone());
    rex_ifml::walk_ifml_imports(&key, &source, &mut |importer, import| {
        if !import.ends_with(".ifml") {
            return None;
        }
        let importer_path = disk_paths.get(importer)?;
        let resolved = importer_path.parent()?.join(import);
        let imported = std::fs::read_to_string(&resolved).ok()?;
        let resolved_key = resolved.display().to_string();
        disk_paths.insert(import.to_string(), resolved);
        imports.provide(importer, import, imported.clone());
        Some((resolved_key, imported))
    });
    rex_ifml::compile_ifml_str(&key, &source, &imports)
        .unwrap_or_else(|diagnostics| panic!("{display} must compile: {diagnostics:?}"))
        .model
}

/// The whole support stack compiled into one graph: the domain, the policy,
/// the design, the events, the decision model, the flow (with its pattern
/// library), the use cases, the case, and the deployment.
fn support_graph() -> &'static GraphModel {
    static GRAPH: OnceLock<GraphModel> = OnceLock::new();
    GRAPH.get_or_init(|| {
        let domain_display = "examples/support.mox";
        let domain_key = disk(domain_display).display().to_string();
        let domain_source = fixture(domain_display);
        let compilation = rex_driver::compile_str(
            &domain_key,
            &domain_source,
            &rex_driver::DomainImports::default(),
        );
        let domain = lowered(domain_display, compilation.model, &compilation.diagnostics);
        let domains = [(domain_key, domain_source.clone())];

        let actor_display = "examples/support.actor";
        let compilation = rex_driver::compile_actors_str(
            &disk(actor_display).display().to_string(),
            &fixture(actor_display),
            // The `.actor` compile keys its imports by the string as written.
            &[("support.mox".to_string(), domain_source)],
            &rex_driver::DomainImports::default(),
        );
        let actor = lowered(actor_display, compilation.model, &compilation.diagnostics);

        let ddd_display = "examples/support.ddd";
        let compilation = rex_driver::compile_ddd_str(
            &disk(ddd_display).display().to_string(),
            &fixture(ddd_display),
            &domains,
            &rex_driver::DomainImports::default(),
        );
        let design = lowered(ddd_display, compilation.model, &compilation.diagnostics);

        let evt_display = "examples/support.evt";
        let compilation = rex_driver::compile_evt_str(
            &disk(evt_display).display().to_string(),
            &fixture(evt_display),
            &domains,
            &rex_driver::DomainImports::default(),
        );
        let events = lowered(evt_display, compilation.model, &compilation.diagnostics);

        let dmn_display = "examples/support.dmn";
        let compilation = rex_driver::compile_dmn_str(
            &disk(dmn_display).display().to_string(),
            &fixture(dmn_display),
            &domains,
            &rex_driver::DomainImports::default(),
            &[(
                "./tables/refund-authority.md".to_string(),
                fixture("examples/tables/refund-authority.md"),
            )],
            &[("support.evt".to_string(), events.clone())],
        );
        let decision = lowered(dmn_display, compilation.model, &compilation.diagnostics);

        let ifml_display = "examples/support.ifml";
        let ifml_key = disk(ifml_display).display().to_string();
        let flow = flow_model(ifml_display);

        let uc_display = "examples/support.uc";
        let compilation = rex_driver::compile_usecase_str(
            &disk(uc_display).display().to_string(),
            &fixture(uc_display),
            &domains,
            &rex_driver::DomainImports::default(),
            &[(disk(ddd_display).display().to_string(), design.clone())],
            &[(disk(actor_display).display().to_string(), actor.clone())],
            &[(disk(evt_display).display().to_string(), events.clone())],
        );
        let use_cases = lowered(uc_display, compilation.model, &compilation.diagnostics);

        let cmmn_display = "examples/support.cmmn";
        let compilation = rex_driver::compile_cmmn_str(
            &disk(cmmn_display).display().to_string(),
            &fixture(cmmn_display),
            &domains,
            &rex_driver::DomainImports::default(),
            &[(disk(ddd_display).display().to_string(), design.clone())],
            &[(disk(actor_display).display().to_string(), actor.clone())],
            &[(disk(evt_display).display().to_string(), events.clone())],
            &[(disk(dmn_display).display().to_string(), decision.clone())],
        );
        let case = lowered(cmmn_display, compilation.model, &compilation.diagnostics);

        let deploy_display = "examples/support.deploy";
        let compilation = rex_driver::compile_deploy_str(
            &disk(deploy_display).display().to_string(),
            &fixture(deploy_display),
            &[(disk(ddd_display).display().to_string(), design.clone())],
            &[(ifml_key, flow.clone())],
        );
        let deployment = lowered(deploy_display, compilation.model, &compilation.diagnostics);

        let mut inputs = GraphInputs::default();
        inputs.domains.push(graph_file(domain_display, domain));
        inputs.actors.push(graph_file(actor_display, actor));
        inputs.designs.push(graph_file(ddd_display, design));
        inputs.events.push(graph_file(evt_display, events));
        inputs.decisions.push(graph_file(dmn_display, decision));
        inputs.flows.push(graph_file(ifml_display, flow));
        inputs.use_cases.push(graph_file(uc_display, use_cases));
        inputs.cases.push(graph_file(cmmn_display, case));
        inputs.deploys.push(graph_file(deploy_display, deployment));
        for display in PATTERN_FLOWS {
            inputs.flows.push(graph_file(display, flow_model(display)));
        }
        build(&inputs)
    })
}

/// The graph's node keys for the indexed file, `file` nodes excluded.
///
/// A `file` node's name is the compiled file's display path — a fact only the
/// host (the compiler driver) knows. A source-only span index keys
/// declarations, not files, so file nodes are the one documented exclusion
/// from the equality.
fn graph_definition_keys(graph: &GraphModel, display: &str) -> BTreeSet<(String, String)> {
    graph
        .nodes
        .iter()
        .filter(|node| node.file == display && node.kind != kinds::FILE)
        // An `.ifml` `flow` node is named by the file stem — a host fact with
        // no written declaration site, the same exclusion as `file` nodes.
        .filter(|node| !(display.ends_with(".ifml") && node.kind == kinds::FLOW))
        .map(|node| (node.kind.clone(), node.name.clone()))
        .collect()
}

/// The index's declaration keys, asserted unique (one definition site per
/// graph node).
fn index_definition_keys(index: &SurfaceIndex, display: &str) -> BTreeSet<(String, String)> {
    let keys: BTreeSet<(String, String)> = index
        .definitions
        .iter()
        .map(|definition| (definition.kind.clone(), definition.name.clone()))
        .collect();
    assert_eq!(
        keys.len(),
        index.definitions.len(),
        "{display}: duplicate (kind, name) definitions — a graph node has exactly one site"
    );
    keys
}

#[test]
fn every_indexed_definition_maps_to_exactly_one_graph_node() {
    for (display, domain) in SURFACES {
        let (_, index) = surface_index(display);
        let graph = single_file_graph(display, domain);
        let expected = graph_definition_keys(&graph, display);
        let actual = index_definition_keys(&index, display);
        assert_eq!(
            actual, expected,
            "{display}: the span table's (kind, name) set must equal the graph's node set"
        );
        assert_eq!(
            index.definitions.len(),
            expected.len(),
            "{display}: exactly one spanned definition per graph node"
        );
        for definition in &index.definitions {
            assert_eq!(
                index
                    .definitions_of(&definition.kind, &definition.name)
                    .len(),
                1,
                "{display}: {} '{}' has more than one definition site",
                definition.kind,
                definition.name
            );
        }
    }
}

#[test]
fn span_slices_equal_the_written_name() {
    for (display, _) in SURFACES {
        let (source, index) = surface_index(display);
        for definition in &index.definitions {
            let span = definition.name_span;
            assert!(span.start < span.end, "{display}: empty name span");
            assert!(
                span.end <= source.len(),
                "{display}: name span past the end of the source"
            );
            // A span is a byte offset pair into the *original* source: slicing
            // it must never land inside a multi-byte character.
            assert!(
                source.get(span.start..span.end).is_some(),
                "{display}: the name span must slice on character boundaries"
            );
            assert!(
                definition.full_span.start <= span.start && span.end <= definition.full_span.end,
                "{display}: the name span must sit inside the declaration span"
            );
            // A transition's node key is a derived composite
            // (`<machine>.<from>-><to>`, `~n`-suffixed when a pair repeats):
            // its span covers the written `from -> to` pair, not one token,
            // so it has no last-segment equality (see the `.ddd` unit test).
            if definition.name.contains("->") {
                continue;
            }
            let written = &source[span.start..span.end];
            // `.ifml` names are free strings: their name span is the quoted
            // `"…"` token (the token rename replaces).
            let written = written
                .strip_prefix('"')
                .and_then(|rest| rest.strip_suffix('"'))
                .unwrap_or(written);
            let segment = definition
                .name
                .rsplit('.')
                .next()
                .unwrap_or(&definition.name);
            assert_eq!(
                written, segment,
                "{display}: {} '{}' — the name span must cover the written name",
                definition.kind, definition.name
            );
        }
    }
}
