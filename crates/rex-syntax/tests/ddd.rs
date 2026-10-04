//! Integration tests for `.ddd` files: standalone design sources made of
//! `import` declarations followed by exactly one `application` declaration
//! of modules, application services, and class designs. The keyword policy
//! is contextual throughout: the design words lex as ordinary identifiers
//! and are special only in their grammar positions.

use rex_syntax::ast::*;
use rex_syntax::{format_ddd, parse_ddd, DddBuiltinOp, DddStereotype, MultiplicityKind, Span};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

// --- parsing -----------------------------------------------------------------

/// A full-featured model exercising every grammar production: all three
/// stereotypes, an abstract entity, all five flags, builtin and declared
/// repository operations, delegated and declared service operations, a
/// multi-name capability clause, injects, base, qualified names, every
/// multiplicity form, and contextual keywords used as names.
const FULL: &str = r#"
import "shared.mox"
import "nz.example.library.mox"

application Library {

    base nz.example.library

    module catalogue {
        /// Manages the lending of books.
        /// Also handles renewals.
        service LoanService {
            inject LoanRepository;
            inject NotificationService;
            boolean borrow(Book[] books, int days) capability BorrowBooks, RenewLoans;
            save => loanRepository.persist capability AuditWrites;
            Book[] overdue(date asOf);
            String save(Book dto) capability AuditWrites;
        }
        abstract entity AbstractThing cache scaffold nonPersistent optimisticLocking auditable
        entity Book scaffold repository BookRepository {
            delete;
            findById;
            findAll;
            save;
            String[] findByTitle(String title);
        }
        value Money
        value cache
        dto LoanSummary optimisticLocking nonPersistent auditable
    }

    module notification {
        service repository {
            inject EmailGateway;
            void notify(Book dto, date asOf) capability delete;
        }
        dto DeliveryReport nonPersistent
    }
}
"#;

fn expect_module<'a>(file: &'a DddFile, name: &str) -> &'a DddModule {
    file.application
        .as_ref()
        .expect("application")
        .modules
        .iter()
        .find(|module| module.name.text == name)
        .unwrap_or_else(|| panic!("expected module `{name}`, found {file:?}"))
}

#[test]
fn full_featured_model_parses() {
    let result = parse_ddd(FULL);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");

    // Imports keep source order; the whole-decl span covers the keyword.
    assert_eq!(
        file.imports
            .iter()
            .map(|import| import.path.as_str())
            .collect::<Vec<_>>(),
        vec!["shared.mox", "nz.example.library.mox"]
    );
    assert_eq!(
        span_text(FULL, file.imports[0].span),
        r#"import "shared.mox""#
    );

    let application = file.application.as_ref().expect("application");
    assert_eq!(application.name.text, "Library");
    assert!(span_text(FULL, application.span).starts_with("application Library"));
    assert_eq!(
        application.base.as_ref().expect("base").package.full_name(),
        "nz.example.library"
    );
    assert_eq!(
        span_text(FULL, application.base.as_ref().unwrap().span),
        "base nz.example.library"
    );
    assert_eq!(
        application
            .modules
            .iter()
            .map(|module| module.name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["catalogue", "notification"]
    );
}

#[test]
fn service_ops_capabilities_and_dependencies_parse() {
    let result = parse_ddd(FULL);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let catalogue = expect_module(&file, "catalogue");

    let service = &catalogue.services[0];
    assert_eq!(service.name.text, "LoanService");
    assert_eq!(
        service.doc.as_deref(),
        Some("Manages the lending of books. Also handles renewals.")
    );
    assert_eq!(
        service
            .dependencies
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["LoanRepository", "NotificationService"]
    );

    // Declared op with parameters, a multi-name capability clause, and a
    // whole-decl span including the `;`.
    let borrow = &service.operations[0];
    assert_eq!(borrow.name.text, "borrow");
    assert_eq!(
        borrow.return_type.as_ref().unwrap().name.full_name(),
        "boolean"
    );
    assert!(borrow.multiplicity.is_none());
    assert_eq!(borrow.params.len(), 2);
    assert_eq!(borrow.params[0].type_ref.name.full_name(), "Book");
    assert!(matches!(
        borrow.params[0].multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Unbounded
    ));
    assert_eq!(borrow.params[1].type_ref.name.full_name(), "int");
    assert_eq!(borrow.params[1].name.text, "days");
    assert_eq!(
        borrow
            .capabilities
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["BorrowBooks", "RenewLoans"]
    );
    assert_eq!(
        span_text(FULL, borrow.span),
        "boolean borrow(Book[] books, int days) capability BorrowBooks, RenewLoans;"
    );

    // A delegated op named `save` (a builtin word is a plain identifier in
    // service-member position), with its delegation split at the last
    // dotted segment.
    let delegated = &service.operations[1];
    assert_eq!(delegated.name.text, "save");
    assert!(delegated.return_type.is_none());
    assert!(delegated.params.is_empty());
    let delegation = delegated.delegation.as_ref().expect("delegation");
    assert_eq!(delegation.target.full_name(), "loanRepository");
    assert_eq!(delegation.operation.text, "persist");
    assert_eq!(span_text(FULL, delegation.span), "loanRepository.persist");
    assert_eq!(
        delegated
            .capabilities
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["AuditWrites"]
    );

    // Array return type multiplicity.
    let overdue = &service.operations[2];
    assert_eq!(
        overdue.return_type.as_ref().unwrap().name.full_name(),
        "Book"
    );
    assert!(matches!(
        overdue.multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Unbounded
    ));

    // A declared op named `save` with a parameter named `dto`.
    let declared_save = &service.operations[3];
    assert_eq!(declared_save.name.text, "save");
    assert_eq!(
        declared_save.return_type.as_ref().unwrap().name.full_name(),
        "String"
    );
    assert_eq!(declared_save.params[0].name.text, "dto");
}

#[test]
fn designs_parse_with_flags_and_repositories() {
    let result = parse_ddd(FULL);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let catalogue = expect_module(&file, "catalogue");

    assert_eq!(catalogue.designs.len(), 5);

    // Abstract entity, all five flags collected from a scrambled source
    // order into the canonical field order.
    let abstract_thing = &catalogue.designs[0];
    assert_eq!(abstract_thing.class.text, "AbstractThing");
    assert_eq!(abstract_thing.stereotype, DddStereotype::Entity);
    assert!(abstract_thing.is_abstract);
    assert!(abstract_thing.flags.scaffold.is_some());
    assert!(abstract_thing.flags.auditable.is_some());
    assert!(abstract_thing.flags.optimistic_locking.is_some());
    assert!(abstract_thing.flags.non_persistent.is_some());
    assert!(abstract_thing.flags.cache.is_some());
    assert!(abstract_thing.repository.is_none());

    // Entity with a repository of builtins (recognized by their keywords)
    // and one declared operation.
    let book = &catalogue.designs[1];
    assert_eq!(book.class.text, "Book");
    assert!(!book.is_abstract);
    let repository = book.repository.as_ref().expect("repository");
    assert_eq!(repository.name.text, "BookRepository");
    assert_eq!(repository.operations.len(), 5);
    assert_eq!(repository.operations[0].name.text, "delete");
    assert_eq!(repository.operations[0].builtin, Some(DddBuiltinOp::Delete));
    assert!(repository.operations[0].return_type.is_none());
    assert_eq!(
        repository.operations[1].builtin,
        Some(DddBuiltinOp::FindById)
    );
    assert_eq!(
        repository.operations[2].builtin,
        Some(DddBuiltinOp::FindAll)
    );
    assert_eq!(repository.operations[3].builtin, Some(DddBuiltinOp::Save));
    assert_eq!(span_text(FULL, repository.operations[1].span), "findById;");
    let declared = &repository.operations[4];
    assert!(declared.builtin.is_none());
    assert_eq!(declared.name.text, "findByTitle");
    assert_eq!(
        declared.return_type.as_ref().unwrap().name.full_name(),
        "String"
    );
    assert!(matches!(
        declared.multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Unbounded
    ));
    assert_eq!(declared.params[0].type_ref.name.full_name(), "String");

    // A plain value and a design named `cache` (flag words are ordinary
    // identifiers in name position).
    assert_eq!(catalogue.designs[2].class.text, "Money");
    assert_eq!(catalogue.designs[2].stereotype, DddStereotype::Value);
    assert!(catalogue.designs[2].flags.is_empty());
    assert_eq!(catalogue.designs[3].class.text, "cache");

    // Dto with a flag subset.
    let summary = &catalogue.designs[4];
    assert_eq!(summary.class.text, "LoanSummary");
    assert_eq!(summary.stereotype, DddStereotype::Dto);
    assert!(!summary.is_abstract);
    assert!(summary.flags.scaffold.is_none());
    assert!(summary.flags.auditable.is_some());
    assert!(summary.flags.optimistic_locking.is_some());
    assert!(summary.flags.non_persistent.is_some());
    assert!(summary.flags.cache.is_none());
}

#[test]
fn multiplicity_forms_parse() {
    let source = concat!(
        "application A { module M { service S {\n",
        "    Book[] a();\n",
        "    Book[3] b();\n",
        "    Book[1..*] c();\n",
        "    Book[2..5] d(Book[0..1] e, Book[] f);\n",
        "} } }\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let ops = &result.ast.unwrap().application.unwrap().modules[0].services[0].operations;
    assert!(matches!(
        ops[0].multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Unbounded
    ));
    assert!(matches!(
        ops[1].multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Exact(3)
    ));
    assert!(matches!(
        ops[2].multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Range(1, MultBound::Star)
    ));
    assert!(matches!(
        ops[3].multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Range(2, MultBound::Int(5))
    ));
    assert!(matches!(
        ops[3].params[0].multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Range(0, MultBound::Int(1))
    ));
    assert!(matches!(
        ops[3].params[1].multiplicity.as_ref().unwrap().kind,
        MultiplicityKind::Unbounded
    ));
}

#[test]
fn doc_runs_attach_to_services_only() {
    let source = concat!(
        "application A {\n",
        "    module M {\n",
        "        /// First line.\n",
        "        /// Second line.\n",
        "        service Documented { }\n",
        "        // not a doc comment\n",
        "        service Plain { }\n",
        "        /// Separated by a blank line.\n",
        "\n",
        "        service Detached { }\n",
        "        entity Thing\n",
        "    }\n",
        "}\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let services = &result.ast.unwrap().application.unwrap().modules[0].services;
    // Contiguous `///` lines join with a space.
    assert_eq!(services[0].doc.as_deref(), Some("First line. Second line."));
    assert_eq!(services[1].doc, None, "a non-doc comment breaks the run");
    assert_eq!(services[2].doc, None, "a blank line breaks the run");
}

#[test]
fn contextual_keywords_remain_usable_everywhere() {
    let source = concat!(
        "application application {\n",
        "    module application {\n",
        "        service repository {\n",
        "            inject inject;\n",
        "            service save(application dto) capability cache;\n",
        "            value => entity.inject;\n",
        "        }\n",
        "        entity entity\n",
        "    }\n",
        "}\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let application = file.application.as_ref().unwrap();
    assert_eq!(application.name.text, "application");
    let module = &application.modules[0];
    assert_eq!(module.name.text, "application");
    let service = &module.services[0];
    assert_eq!(service.name.text, "repository");
    assert_eq!(service.dependencies[0].text, "inject");
    // A declared op named `save`, with an `application`-typed parameter
    // named `dto` and a `cache`-named capability: all contextual words are
    // plain identifiers in name positions.
    let declared = &service.operations[0];
    assert_eq!(declared.name.text, "save");
    assert_eq!(
        declared.return_type.as_ref().unwrap().name.full_name(),
        "service"
    );
    assert_eq!(declared.params[0].type_ref.name.full_name(), "application");
    assert_eq!(declared.params[0].name.text, "dto");
    assert_eq!(declared.capabilities[0].text, "cache");
    // A delegated op named `value` targeting `entity.inject`.
    let delegated = &service.operations[1];
    assert_eq!(delegated.name.text, "value");
    let delegation = delegated.delegation.as_ref().unwrap();
    assert_eq!(delegation.target.full_name(), "entity");
    assert_eq!(delegation.operation.text, "inject");
    assert_eq!(module.designs[0].class.text, "entity");
}

#[test]
fn import_semicolon_is_optional_and_unescaped() {
    let result = parse_ddd(r#"import "a.mox"; import "b.mox" application A { }"#);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.imports[0].path, "a.mox");
    assert_eq!(file.imports[1].path, "b.mox");
}

// --- parse failures ------------------------------------------------------------

#[test]
fn missing_application_is_an_error() {
    for source in ["", "   \n\t\n  ", "module Bogus { }", "import \"a.mox\""] {
        let result = parse_ddd(source);
        assert!(
            result.errors.iter().any(|error| error
                .message
                .contains("expected an `application` declaration")),
            "{source:?}: expected the missing-application error, got {:?}",
            result.errors
        );
    }
}

#[test]
fn missing_service_body_brace_is_an_error() {
    let result = parse_ddd("application A { module M { service S { boolean borrow(); ");
    assert!(!result.errors.is_empty(), "expected a clean error");
    let _ = result.ast.expect("recovered AST must still exist");
}

#[test]
fn delegated_op_without_target_operation_is_an_error() {
    let result = parse_ddd("application A { module M { service S { renew =>; } } }");
    assert!(!result.errors.is_empty(), "expected a clean error");
    let result = parse_ddd("application A { module M { service S { renew => persist; } } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected `dependency.operation`")),
        "expected the delegation-split error, got {:?}",
        result.errors
    );
}

#[test]
fn capability_clause_without_names_is_an_error() {
    let result =
        parse_ddd("application A { module M { service S { boolean borrow() capability; } } }");
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("`capability` requires at least one capability name")),
        "expected the capability error, got {:?}",
        result.errors
    );
}

#[test]
fn flags_after_repository_are_an_error() {
    let source =
        "application A { module M { entity Book repository BookRepo { findById; } scaffold } }";
    let result = parse_ddd(source);
    assert!(!result.errors.is_empty(), "expected a clean error");
}

#[test]
fn two_applications_are_an_error() {
    let source = "application A { } application B { }";
    let result = parse_ddd(source);
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("duplicate `application` declaration")),
        "expected the duplicate-application error, got {:?}",
        result.errors
    );
    let file = result
        .ast
        .expect("recovered AST keeps the first application");
    assert_eq!(file.application.unwrap().name.text, "A");
}

#[test]
fn base_after_a_module_is_an_error() {
    let source = "application A { module M { } base nz.example.library }";
    let result = parse_ddd(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("`base` must be the first member")),
        "expected the base-ordering error, got {:?}",
        result.errors
    );
}

#[test]
fn duplicate_base_is_an_error() {
    let source = "application A { base one base two }";
    let result = parse_ddd(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("duplicate `base` declaration")),
        "expected the duplicate-base error, got {:?}",
        result.errors
    );
}

#[test]
fn import_after_application_is_an_error() {
    let source = "application A { } import \"late.mox\";";
    let result = parse_ddd(source);
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("`import` after the `application` declaration")),
        "expected the late-import error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("recovered AST");
    assert!(file.imports.is_empty(), "the late import is dropped");
}

#[test]
fn junk_regions_recover_to_the_next_import_or_application() {
    let source = "bogus garbage 42 import \"x.mox\" application A { module M { entity Book } }";
    let result = parse_ddd(source);
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.imports.len(), 1);
    assert_eq!(
        file.application.unwrap().modules[0].designs[0].class.text,
        "Book"
    );

    // A `.mox`-style declaration is junk in a `.ddd` file.
    let result = parse_ddd("class Bogus { int pages } application A { }");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    assert!(result
        .ast
        .expect("recovered")
        .application
        .unwrap()
        .modules
        .is_empty());

    // An unterminated application consumes the rest as junk; the recovered
    // file has no application at all.
    let result = parse_ddd("application A { module M { service");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("recovered AST");
    assert!(file.imports.is_empty());
    assert!(file.application.is_none());
}

#[test]
fn adversarial_inputs_do_not_panic() {
    let nasty = [
        "",
        "   \n\t",
        "import",
        "import \"",
        "import \"unterminated",
        "import import import",
        "application",
        "application A",
        "application A {",
        "application A { base",
        "application A { base base two }",
        "application A { module",
        "application A { module M {",
        "application A { module M { service",
        "application A { module M { service S {",
        "application A { module M { service S { inject",
        "application A { module M { service S { => x.y; } } }",
        "application A { module M { service S { save => ; } } }",
        "application A { module M { service S { a.b.c => d.e.f; } } }",
        "application A { module M { service S { => x.y; } } }",
        "application A { module M { entity } }",
        "application A { module M { entity abstract } }",
        "application A { module M { abstract abstract entity T } }",
        "application A { module M { entity T repository R { findById findById } } }",
        "application A { module M { entity T repository R { String f(String } } }",
        "application A { module M { entity T scaffold scaffold scaffold } }",
        "application A { module M { service S { a => b } } }",
        "application A { application B { } }",
        "@@@ ### :",
        "^application A { module ^module { entity ^entity } }",
        "application A { module M { service S { boolean borrow(String[] a, Book[3] b); } }",
    ];
    for input in nasty {
        let result = parse_ddd(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
        let _ = format_ddd(input); // must not panic either
    }
}

// --- formatting ----------------------------------------------------------------

fn fmt_ddd(source: &str) -> String {
    format_ddd(source).expect("format succeeds")
}

#[test]
fn full_model_formats_canonically_and_is_idempotent() {
    let canonical = concat!(
        "import \"shared.mox\";\n",
        "import \"nz.example.library.mox\";\n",
        "\n",
        "application Library {\n",
        "    base nz.example.library\n",
        "    module catalogue {\n",
        "        /// Manages the lending of books.\n",
        "        /// Also handles renewals.\n",
        "        service LoanService {\n",
        "            inject LoanRepository;\n",
        "            inject NotificationService;\n",
        "            boolean borrow(Book[] books, int days) capability BorrowBooks, RenewLoans;\n",
        "            save => loanRepository.persist capability AuditWrites;\n",
        "            Book[] overdue(date asOf);\n",
        "            String save(Book dto) capability AuditWrites;\n",
        "        }\n",
        "        abstract entity AbstractThing scaffold auditable optimisticLocking nonPersistent cache\n",
        "        entity Book scaffold repository BookRepository {\n",
        "            delete;\n",
        "            findById;\n",
        "            findAll;\n",
        "            save;\n",
        "            String[] findByTitle(String title);\n",
        "        }\n",
        "        value Money\n",
        "        value cache\n",
        "        dto LoanSummary auditable optimisticLocking nonPersistent\n",
        "    }\n",
        "    module notification {\n",
        "        service repository {\n",
        "            inject EmailGateway;\n",
        "            void notify(Book dto, date asOf) capability delete;\n",
        "        }\n",
        "        dto DeliveryReport nonPersistent\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_ddd(FULL), canonical);
    // Idempotence: the canonical output is a fixpoint.
    assert_eq!(fmt_ddd(canonical), canonical);
}

#[test]
fn empty_bodies_stay_inline() {
    assert_eq!(fmt_ddd("application A { }"), "application A {}\n");
    assert_eq!(
        fmt_ddd("application A { module M { service S { } } }"),
        "application A {\n    module M {\n        service S {}\n    }\n}\n"
    );
}

#[test]
fn design_flags_canonicalize_to_a_fixed_order() {
    assert_eq!(
        fmt_ddd("application A { module M { entity T cache nonPersistent optimisticLocking auditable scaffold } }"),
        concat!(
            "application A {\n",
            "    module M {\n",
            "        entity T scaffold auditable optimisticLocking nonPersistent cache\n",
            "    }\n",
            "}\n",
        )
    );
    // Repeats collapse.
    assert_eq!(
        fmt_ddd("application A { module M { entity T cache cache } }"),
        "application A {\n    module M {\n        entity T cache\n    }\n}\n"
    );
}

#[test]
fn comments_are_preserved() {
    let source = concat!(
        "// header\n",
        "import \"a.mox\" // trailing\n",
        "/* block */\n",
        "application A { // inner\n",
        "    module M {\n",
        "        /// doc line\n",
        "        service S { // member\n",
        "            inject R; // dependency\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let expected = concat!(
        "// header\n",
        "import \"a.mox\"; // trailing\n",
        "\n",
        "/* block */\n",
        "application A { // inner\n",
        "    module M {\n",
        "        /// doc line\n",
        "        service S { // member\n",
        "            inject R; // dependency\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_ddd(source), expected);
    assert_eq!(fmt_ddd(expected), expected);
}

#[test]
fn imports_are_hoisted_ahead_of_the_application() {
    let source = concat!(
        "application A { module M { } }\n",
        "\n",
        "\n",
        "import \"b.mox\"\n",
        "import \"a.mox\";\n",
    );
    let expected = concat!(
        "import \"b.mox\";\n",
        "import \"a.mox\";\n",
        "\n",
        "application A {\n",
        "    module M {}\n",
        "}\n",
    );
    assert_eq!(fmt_ddd(source), expected);
    assert_eq!(fmt_ddd(expected), expected);
}

#[test]
fn interior_blank_lines_are_dropped() {
    assert_eq!(
        fmt_ddd("application A {\n    module M { }\n\n\n    module N { }\n}"),
        concat!(
            "application A {\n",
            "    module M {}\n",
            "    module N {}\n",
            "}\n",
        )
    );
}

#[test]
fn parse_errors_still_format() {
    assert_eq!(fmt_ddd("application"), "application\n");
    assert_eq!(
        fmt_ddd("@@@ application A { }"),
        "@ @ @\n\napplication A {}\n"
    );
}

#[test]
fn formatted_output_round_trips_structurally() {
    let formatted = fmt_ddd(FULL);
    let once = parse_ddd(&formatted);
    assert!(
        once.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}",
        once.errors
    );
    let twice_formatted = fmt_ddd(&formatted);
    let twice = parse_ddd(&twice_formatted);
    assert!(
        twice.errors.is_empty(),
        "re-formatted output must parse cleanly, got {:?}",
        twice.errors
    );
    assert_eq!(once.ast, twice.ast);
}
