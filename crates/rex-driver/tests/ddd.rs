//! Integration tests for `.ddd` design compilation (`compile_ddd_str` /
//! `compile_ddd_str_with_actors`): lowering to the `rex_ir::ddd::DddModel`
//! and each validation rule against the imported domain models. The
//! normative contract lives on `compile_ddd_str`; these tests pin it.

use rex_driver::{compile_ddd_str, compile_ddd_str_with_actors, DddCompilation, Diagnostic};
use rex_ir::ddd::DDD_MODEL_FORMAT_VERSION;
use rex_ir::{ActorModel, ActorsDef, CapabilityDef, TypeRef};

/// One domain exercising resolution, containment (Library owns Books), an
/// enum, and a spare value-object class.
const DOMAIN: &str = r#"
package nz.example.library

enum BookCategory {
    Fiction as "F" = 0
}

class Library {
    String name
    contains Book[] books opposite library
}

class Book {
    container Library library opposite books
    String title
    BookCategory category
}

class Money {
    int cents
}
"#;

/// A three-link containment chain: Library owns Books, Books own Chapters.
const CHAIN_DOMAIN: &str = r#"
package nz.example.library

class Library {
    String name
    contains Book[] books opposite library
}

class Book {
    container Library library opposite books
    contains Chapter[] chapters opposite book
}

class Chapter {
    container Book book opposite chapters
}
"#;

/// A clean design over [`DOMAIN`]: an aggregate root with a repository, a
/// contained entity without one, and a value object.
const CLEAN: &str = r#"
import "library.mox";

application Library {
    base nz.example.library

    module catalogue {
        entity Library repository LibraryRepository {
            findById;
        }
        entity Book
        value Money
    }
}
"#;

fn compile(source: &str, domains: &[(&str, &str)]) -> DddCompilation {
    let domains: Vec<(String, String)> = domains
        .iter()
        .map(|(path, source)| (path.to_string(), source.to_string()))
        .collect();
    compile_ddd_str("design.ddd", source, &domains)
}

/// Compiles against the single [`DOMAIN`] import.
fn compile_domain(source: &str) -> DddCompilation {
    compile(source, &[("library.mox", DOMAIN)])
}

/// Compiles against the single [`CHAIN_DOMAIN`] import.
fn compile_chain(source: &str) -> DddCompilation {
    compile(source, &[("chain.mox", CHAIN_DOMAIN)])
}

fn actors_with(capabilities: &[&str]) -> ActorModel {
    let mut block = ActorsDef::new("Ops");
    for name in capabilities {
        block = block.capability(CapabilityDef::new(
            *name,
            TypeRef::Class {
                package: "nz.example.library".to_string(),
                name: "Book".to_string(),
            },
        ));
    }
    ActorModel::new().block(block)
}

/// The single diagnostic whose message contains `needle`, or a panic.
fn single<'a>(compilation: &'a DddCompilation, needle: &str) -> &'a Diagnostic {
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.message.contains(needle))
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one diagnostic containing '{needle}', got: {:?}",
        compilation.diagnostics
    );
    matches[0]
}

/// Byte span of the `nth` (0-based) occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str, nth: usize) -> rex_driver::Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

fn assert_clean(compilation: &DddCompilation) {
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}

// --- lowering: the happy path -------------------------------------------------

#[test]
fn happy_path_lowers_the_expected_design_model() {
    let source = r#"
import "library.mox"

application Library {
    base nz.example.library

    module catalogue {
        /// Coordinates lending.
        service LoanService {
            inject LibraryRepository;
            boolean borrow(Book book) capability BorrowBooks;
            lend => LibraryRepository.findById;
        }
        abstract entity Library scaffold cache repository LibraryRepository {
            findById;
            findAll;
            Book byTitle(String title);
        }
        entity Book
        value Money nonPersistent
    }
}
"#;
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let model = compilation.model.expect("design artifact lowered");
    assert_eq!(model.format_version, DDD_MODEL_FORMAT_VERSION);
    let application = model.application.as_ref().expect("application");
    assert_eq!(application.name, "Library");
    assert_eq!(application.base.as_deref(), Some("nz.example.library"));
    assert_eq!(model.modules.len(), 1);

    let module = &model.modules[0];
    assert_eq!(module.name, "catalogue");

    let service = &module.services[0];
    assert_eq!(service.name, "LoanService");
    assert_eq!(service.description.as_deref(), Some("Coordinates lending."));
    assert_eq!(service.dependencies, ["LibraryRepository"]);
    assert_eq!(service.operations.len(), 2);

    let borrow = &service.operations[0];
    assert_eq!(borrow.name, "borrow");
    assert_eq!(
        borrow.return_type,
        Some(rex_ir::TypeRef::Primitive(rex_ir::PrimitiveType::Boolean))
    );
    assert_eq!(borrow.params.len(), 1);
    assert_eq!(
        borrow.params[0].type_,
        TypeRef::Class {
            package: "nz.example.library".to_string(),
            name: "Book".to_string()
        }
    );
    assert_eq!(borrow.capabilities, ["BorrowBooks"]);
    assert!(borrow.delegation.is_none());

    let lend = &service.operations[1];
    assert_eq!(lend.name, "lend");
    assert!(lend.return_type.is_none());
    assert!(lend.params.is_empty());
    let delegation = lend.delegation.as_ref().expect("delegation");
    assert_eq!(delegation.target, "LibraryRepository");
    assert_eq!(delegation.operation, "findById");
    assert!(lend.capabilities.is_empty());

    assert_eq!(module.designs.len(), 3);
    let library = &module.designs[0];
    assert_eq!(library.class, "Library");
    assert!(library.is_abstract);
    assert!(library.flags.scaffold && library.flags.cache);
    let repository = library.repository.as_ref().expect("repository");
    assert_eq!(repository.name, "LibraryRepository");
    assert_eq!(repository.operations.len(), 3);
    assert_eq!(
        repository.operations[0].builtin,
        Some(rex_ir::ddd::BuiltinRepositoryOp::FindById)
    );
    assert!(repository.operations[0].return_type.is_none());
    let declared = &repository.operations[2];
    assert_eq!(declared.name, "byTitle");
    assert!(declared.builtin.is_none());
    assert!(declared.return_type.is_some());
    assert_eq!(declared.params[0].name, "title");

    let book = &module.designs[1];
    assert_eq!(book.class, "Book");
    assert!(!book.is_abstract);
    assert!(book.repository.is_none());

    let money = &module.designs[2];
    assert!(money.flags.non_persistent);

    // The resolved domain union rides along for consumers.
    let domains_model = compilation.domains_model.expect("domains lowered");
    assert_eq!(domains_model.packages.len(), 1);
}

#[test]
fn the_shipped_conformance_design_compiles_clean() {
    let compilation = compile_domain(CLEAN);
    assert_clean(&compilation);
}

// --- rule 1: base -------------------------------------------------------------

#[test]
fn unknown_base_package_is_an_error() {
    let source =
        "import \"library.mox\"\napplication A { base nz.example.typo module m { value Money } }";
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown base package 'nz.example.typo'");
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "base nz.example.typo", 0))
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the imported domains declare: nz.example.library"),
        "help lists the declared packages: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn base_without_any_imported_domain_is_an_error() {
    let compilation = compile("application A { base nz.example.library }", &[]);
    let diagnostic = single(&compilation, "unknown base package 'nz.example.library'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("no domains are imported"),
        "help points at the missing imports: {:?}",
        diagnostic.help
    );
}

#[test]
fn import_must_be_provided() {
    let compilation = compile(
        "import \"missing.mox\"\n\napplication A { module m { value Money } }",
        &[("library.mox", DOMAIN)],
    );
    let diagnostic = single(
        &compilation,
        "imported file \"missing.mox\" was not provided",
    );
    assert!(diagnostic.is_error());
    assert!(compilation.model.is_none());
}

// --- rule 2: uniqueness -------------------------------------------------------

#[test]
fn duplicate_module_names_are_rejected() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m { value Money }\n",
        "    module m { entity Book }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "duplicate module 'm'");
    let name_start = span_of(source, "m { entity", 0).start;
    assert_eq!(diagnostic.span, Some((name_start..name_start + 1).into()));
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("'m' is already declared in this application"),
        "help names the first declaration: {:?}",
        diagnostic.help
    );
}

#[test]
fn duplicate_service_names_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module a { service S { } }\n",
        "    module b { service S { } }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate service 'S'");
}

#[test]
fn duplicate_designs_for_one_class_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module a { value Money }\n",
        "    module b { dto Money }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate design for class 'Money'");
}

#[test]
fn duplicate_repository_names_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module a { entity Library repository R { findById; } }\n",
        "    module b { value Money repository R { findAll; } }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate repository 'R'");
}

#[test]
fn duplicate_operation_names_in_one_service_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a(Book book);\n",
        "            String a();\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate operation 'a' in service 'S'");
}

#[test]
fn duplicate_operation_names_in_one_repository_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        entity Library repository R {\n",
        "            findById;\n",
        "            String findById(String title);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(
        &compilation,
        "duplicate operation 'findById' in repository 'R'",
    );
}

// --- rule 3: stereotype targets ----------------------------------------------

#[test]
fn design_target_must_resolve_to_a_class() {
    let source = "application A { module m { value NoSuchClass } }";
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown type 'NoSuchClass'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the design of 'NoSuchClass' in module 'm'"),
        "the failure names the design: {:?}",
        diagnostic.help
    );
}

#[test]
fn design_target_must_be_a_class() {
    let source = "import \"library.mox\"\napplication A { module m { value BookCategory } }";
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "design target 'BookCategory' is an enum, not a class",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "BookCategory", 0)));
}

// --- rule 4: flag/stereotype compatibility -----------------------------------

#[test]
fn entity_only_flags_are_rejected_on_value_designs() {
    for flag in ["scaffold", "auditable", "optimisticLocking"] {
        let source = format!("application A {{ module m {{ value Money {flag} }} }}");
        let compilation = compile_domain(&source);
        let diagnostic = single(
            &compilation,
            &format!("flag '{flag}' requires an entity design; 'Money' is designed as a value"),
        );
        assert_eq!(diagnostic.span, Some(span_of(&source, flag, 0)));
    }
}

#[test]
fn non_persistent_is_rejected_on_entity_designs() {
    let source = "application A { module m { entity Book nonPersistent } }";
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "flag 'nonPersistent' requires a value or dto design; 'Book' is designed as an entity",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "nonPersistent", 0)));
}

#[test]
fn cache_is_allowed_on_any_design() {
    for design in ["value Money cache", "dto Money cache", "entity Book cache"] {
        let source = format!("import \"library.mox\"\napplication A {{ module m {{ {design} }} }}");
        let compilation = compile_domain(&source);
        assert_clean(&compilation);
    }
}

// --- rule 5: repository placement --------------------------------------------

#[test]
fn repository_on_non_entity_design_is_rejected() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        value Money repository MoneyRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "repository 'MoneyRepository' requires an entity design; 'Money' is designed as a value",
    );
    assert_eq!(
        diagnostic.span,
        Some(span_of(
            source,
            "repository MoneyRepository { findById; }",
            0
        ))
    );
}

// --- rule 6: aggregate boundary ----------------------------------------------

#[test]
fn contained_entity_cannot_declare_a_repository() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "        entity Book repository BookRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "entity 'Book' is contained by 'Library'; only aggregate roots may declare a repository",
    );
    assert_eq!(
        diagnostic.span,
        Some(span_of(
            source,
            "repository BookRepository { findById; }",
            0
        ))
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("remove the repository from design 'Book'"),
        "help offers the two outs: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn containment_is_transitive_over_the_closure() {
    // Only Library and Chapter are designed: Book carries no design, yet
    // Chapter is transitively contained by the stereotyped entity Library
    // (Library → Book → Chapter) and may not declare a repository.
    let source = concat!(
        "import \"chain.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "        entity Chapter repository ChapterRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_chain(source);
    single(
        &compilation,
        "entity 'Chapter' is contained by 'Library'; only aggregate roots may declare a repository",
    );
}

#[test]
fn containment_without_an_entity_design_allows_a_repository() {
    // Library is not designed as an entity, so no stereotyped entity
    // contains Book and its repository is legitimate.
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Book repository BookRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    assert!(compilation.model.is_some());
}

// --- rule 7: signature type-check --------------------------------------------

#[test]
fn signature_types_must_resolve() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean borrow(NoSuchType thing);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "unknown type 'NoSuchType'");
}

#[test]
fn signature_types_accept_primitives_enums_and_classes() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        service S {\n",
        "            Book categorize(String title, int pages, BookCategory category, date asOf);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
}

#[test]
fn multiplicity_annotations_lower_into_cardinality_slots() {
    let source = concat!(
        "import \"library.mox\";\n",
        "\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        service S {\n",
        "            Book[] books(Book[] input, int count);\n",
        "        }\n",
        "        entity Library repository R {\n",
        "            Book[] byTitle(String title);\n",
        "            save;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let model = compilation.model.expect("clean compile lowers a model");
    let service = &model.modules[0].services[0];
    let books = &service.operations[0];
    assert_eq!(
        books.return_multiplicity,
        Some(rex_ir::Multiplicity::MANY),
        "[] lowers as 0..*"
    );
    assert_eq!(
        books.params[0].multiplicity,
        Some(rex_ir::Multiplicity::MANY)
    );
    assert_eq!(books.params[1].multiplicity, None);
    let repository = model.modules[0].designs[0]
        .repository
        .as_ref()
        .expect("the design declares a repository");
    let by_title = &repository.operations[0];
    assert_eq!(
        by_title.return_multiplicity,
        Some(rex_ir::Multiplicity::MANY)
    );
    let save = &repository.operations[1];
    assert_eq!(
        save.return_multiplicity, None,
        "built-ins carry no signature"
    );
}

// --- rule 8: delegations -----------------------------------------------------

#[test]
fn delegation_target_must_be_a_service_or_repository() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            lend => Money.findById;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown delegation target 'Money'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("delegate to an injected dependency"),
        "help names the fix: {:?}",
        diagnostic.help
    );
}

#[test]
fn delegation_operation_must_exist_on_the_target() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            lend => LibraryRepository.renew;\n",
        "        }\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(
        &compilation,
        "repository 'LibraryRepository' has no operation 'renew'",
    );
}

#[test]
fn service_to_service_delegation_across_modules_is_allowed() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module media {\n",
        "        service MediaService {\n",
        "            registerBorrower => PersonService.register;\n",
        "        }\n",
        "    }\n",
        "\n",
        "    module person {\n",
        "        service PersonService {\n",
        "            boolean register();\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
}

#[test]
fn service_to_repository_delegation_across_modules_is_rejected() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module media {\n",
        "        service MediaService {\n",
        "            lend => LibraryRepository.findById;\n",
        "        }\n",
        "    }\n",
        "\n",
        "    module person {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "interaction between a Service in one Module and a Repository in another Module \
         is not allowed; go via a Service",
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("repository 'LibraryRepository' lives in module 'person'"),
        "help names both modules: {:?}",
        diagnostic.help
    );
}

// --- rule 9: inject ----------------------------------------------------------

#[test]
fn inject_must_resolve_to_a_service_or_repository() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            inject MoneyRepository;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown dependency 'MoneyRepository'");
    assert_eq!(diagnostic.span, Some(span_of(source, "MoneyRepository", 0)));
}

// --- rule 10: capabilities ---------------------------------------------------

#[test]
fn capabilities_are_recorded_unvalidated_without_actors() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a() capability NobodyDeclaredThis;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let model = compilation.model.expect("artifact lowered");
    assert_eq!(
        model.modules[0].services[0].operations[0].capabilities,
        ["NobodyDeclaredThis"]
    );
}

#[test]
fn capabilities_must_exist_in_the_actor_model() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service LoanService {\n",
        "            boolean borrow(Book book) capability BorrowBooks, RenewLoans;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &domains,
        &actors_with(&["BorrowBooks"]),
    );
    let diagnostic = single(
        &compilation,
        "unknown capability 'RenewLoans' on operation 'borrow' of service 'LoanService'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "RenewLoans", 0)));
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the actor model declares: BorrowBooks"),
        "help lists the declared capabilities: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn declared_capabilities_are_accepted_by_the_actors_variant() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a() capability Declared;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation =
        compile_ddd_str_with_actors("design.ddd", source, &domains, &actors_with(&["Declared"]));
    assert_clean(&compilation);
}

#[test]
fn capabilities_against_an_empty_actor_model_list_nothing() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a() capability Anything;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let domains = [("library.mox".to_string(), DOMAIN.to_string())];
    let compilation =
        compile_ddd_str_with_actors("design.ddd", source, &domains, &ActorModel::new());
    let diagnostic = single(&compilation, "unknown capability 'Anything'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the actor model declares no capabilities"),
        "help explains the empty union: {:?}",
        diagnostic.help
    );
}

// --- resolution semantics ----------------------------------------------------

const PACKAGE_A: &str = "package a\n\nclass Money {\n    int cents\n}";
const PACKAGE_B: &str = "package b\n\nclass Money {\n    int cents\n}";

#[test]
fn ambiguous_bare_names_require_qualification() {
    let source = "import \"a.mox\"\nimport \"b.mox\"\napplication A { module m { value Money } }";
    let compilation = compile(source, &[("a.mox", PACKAGE_A), ("b.mox", PACKAGE_B)]);
    let diagnostic = single(&compilation, "ambiguous type `Money`; qualify as `a.Money`");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("matching types: a.Money, b.Money"),
        "help lists the matching packages: {:?}",
        diagnostic.help
    );
}

#[test]
fn base_package_disambiguates_bare_names() {
    let source = concat!(
        "import \"a.mox\"\n",
        "import \"b.mox\"\n",
        "application A {\n",
        "    base a\n",
        "\n",
        "    module m {\n",
        "        value Money\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("a.mox", PACKAGE_A), ("b.mox", PACKAGE_B)]);
    assert_clean(&compilation);
    // Resolution is validation-only: the name stays authored.
    let model = compilation.model.expect("artifact lowered");
    assert_eq!(model.modules[0].designs[0].class, "Money");
}

#[test]
fn qualified_names_resolve_across_packages() {
    let source = concat!(
        "import \"a.mox\"\n",
        "import \"b.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            b.Money price(b.Money input);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("a.mox", PACKAGE_A), ("b.mox", PACKAGE_B)]);
    assert_clean(&compilation);
    let model = compilation.model.expect("artifact lowered");
    let operation = &model.modules[0].services[0].operations[0];
    assert_eq!(
        operation.return_type,
        Some(TypeRef::Class {
            package: "b".to_string(),
            name: "Money".to_string()
        })
    );
}

// --- syntax errors surface ---------------------------------------------------

#[test]
fn parse_errors_surface_as_diagnostics() {
    let compilation = compile_domain("application A { module m { service S { === } } }");
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(path, diagnostic)| path == "design.ddd" && diagnostic.is_error()),
        "parse errors are tagged with the design file: {:?}",
        compilation.diagnostics
    );
}
