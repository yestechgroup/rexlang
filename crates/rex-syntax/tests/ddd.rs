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
        Some("Manages the lending of books.\nAlso handles renewals.")
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
fn negated_flags_parse_with_the_bang_in_their_span() {
    let source = concat!(
        "application A { module m {\n",
        "    entity Book !auditable cache\n",
        "    entity Movie !optimisticLocking\n",
        "} }\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let designs = &result.ast.unwrap().application.unwrap().modules[0].designs;
    let book = &designs[0];
    let auditable = book.flags.auditable.expect("!auditable parsed");
    assert!(auditable.negated);
    assert_eq!(span_text(source, auditable.span), "!auditable");
    assert!(!book.flags.cache.expect("cache parsed").negated);

    let movie = &designs[1];
    let locking = movie
        .flags
        .optimistic_locking
        .expect("!optimisticLocking parsed");
    assert!(locking.negated);
    assert_eq!(span_text(source, locking.span), "!optimisticLocking");
}

#[test]
fn negated_flags_survive_formatting_in_canonical_position() {
    // Source order is scrambled on purpose: the negated flag re-emits with
    // its `!` attached, in the canonical flag order.
    let source = "application A { module m { entity Book cache !auditable } }\n";
    let formatted = format_ddd(source).unwrap();
    assert!(
        formatted.contains("entity Book !auditable cache"),
        "canonical order with the negation kept: {formatted:?}"
    );
    // Idempotent: formatting the formatted output is a fixpoint.
    assert_eq!(format_ddd(&formatted).unwrap(), formatted);
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
fn doc_runs_attach_to_services_and_searches() {
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
        "        /// Search doc.\n",
        "        search Lookup { }\n",
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
    let module = &result.ast.unwrap().application.unwrap().modules[0];
    let services = &module.services;
    // Contiguous `///` lines join with newlines — the `.mox` semantic.
    assert_eq!(
        services[0].doc.as_deref(),
        Some("First line.\nSecond line.")
    );
    assert_eq!(services[1].doc, None, "a non-doc comment breaks the run");
    assert_eq!(services[2].doc, None, "a blank line breaks the run");
    assert_eq!(module.searches[0].doc.as_deref(), Some("Search doc."));
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

// --- search projections ------------------------------------------------------

/// A full-featured search projection: every member kind, out of canonical
/// order (the formatter reorders), a concat document entry, a brace-bearing
/// conditional document entry, a `;` inside a string literal, per-field
/// boost/analyzer, the camelCase `tfIdf` keyword, and a multi-name
/// capability clause — the same clause grammar a service operation takes.
const SEARCH: &str = r#"
import "library.mox"

application Library {
    module media {
        // a leading line comment
        search MediaSearch {
            capability SearchMedia, SearchMediaAdmin
            pagination {
                cursor
                max 100
                limit 20
            }
            analyzer "english"
            ranking bm25
            document {
                headline = title;
                blurb = title + " - " + synopsis;
                teaser = if (runtimeMinutes > 100) { title + " (long)" } else { "a; b" };
            }
            sort {
                title
            }
            filters {
                genre
            }
            text {
                title boost 3
                synopsis analyzer "english"
            }
            entity Movie
        }
    }
}
"#;

fn expect_search<'a>(file: &'a DddFile, name: &str) -> &'a DddSearch {
    expect_module(file, "media")
        .searches
        .iter()
        .find(|search| search.name.text == name)
        .unwrap_or_else(|| panic!("expected search `{name}`"))
}

#[test]
fn search_decl_parses_with_all_members() {
    let result = parse_ddd(SEARCH);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let search = expect_search(&file, "MediaSearch");

    assert_eq!(search.entity.as_ref().expect("entity").full_name(), "Movie");
    assert_eq!(search.text.len(), 2);
    assert_eq!(search.text[0].property.full_name(), "title");
    assert_eq!(search.text[0].boost, Some(3));
    assert_eq!(search.text[0].analyzer, None);
    assert_eq!(search.text[1].property.full_name(), "synopsis");
    assert_eq!(search.text[1].boost, None);
    assert_eq!(search.text[1].analyzer.as_deref(), Some("english"));
    assert_eq!(
        search
            .filters
            .iter()
            .map(|name| name.full_name())
            .collect::<Vec<_>>(),
        vec!["genre"]
    );
    assert_eq!(
        search
            .sort
            .iter()
            .map(|name| name.full_name())
            .collect::<Vec<_>>(),
        vec!["title"]
    );
    assert_eq!(search.document.len(), 3);
    assert_eq!(search.document[0].name.text, "headline");
    assert_eq!(span_text(SEARCH, search.document[0].expr), "title");
    assert_eq!(search.document[1].name.text, "blurb");
    assert_eq!(
        span_text(SEARCH, search.document[1].expr),
        r#"title + " - " + synopsis"#
    );
    assert_eq!(search.document[2].name.text, "teaser");
    assert_eq!(
        span_text(SEARCH, search.document[2].expr),
        r#"if (runtimeMinutes > 100) { title + " (long)" } else { "a; b" }"#
    );
    assert_eq!(search.ranking, Some(DddRanking::Bm25));
    assert_eq!(search.analyzer.as_deref(), Some("english"));
    let pagination = search.pagination.as_ref().expect("pagination");
    assert_eq!(pagination.limit, Some(20));
    assert_eq!(pagination.max, Some(100));
    assert!(pagination.cursor);
    assert_eq!(
        search
            .capabilities
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["SearchMedia", "SearchMediaAdmin"]
    );
    assert!(search.doc.is_none(), "`//` comments are not doc runs");
}

#[test]
fn search_capability_clause_accepts_a_list_and_clauses_merge() {
    // The unified clause grammar: a search's `capability` clause takes the
    // same `capability <name> (, <name>)*` list a service operation takes,
    // and repeated clauses merge into one list (the `text`/`filters`
    // precedent) instead of shadowing each other.
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        search S {\n",
        "            entity Book\n",
        "            capability ReadBooks, LendBooks\n",
        "            capability RenewLoans\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "a capability list in a search must parse: {:?}",
        result.errors
    );
    let search = &result
        .ast
        .expect("expected an AST")
        .application
        .unwrap()
        .modules[0]
        .searches[0];
    assert_eq!(
        search
            .capabilities
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["ReadBooks", "LendBooks", "RenewLoans"]
    );
}

#[test]
fn search_repeated_clauses_merge_and_singles_are_idempotent() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        search S {\n",
        "            entity Book\n",
        "            text { title }\n",
        "            filters { genre }\n",
        "            entity Movie\n", // idempotent: the first wins
        "            ranking bm25\n",
        "            ranking exact\n", // idempotent: the first wins
        "            text { synopsis boost 2 }\n",
        "            sort { title }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let search = &result
        .ast
        .expect("expected an AST")
        .application
        .unwrap()
        .modules[0]
        .searches[0];
    assert_eq!(search.entity.as_ref().expect("entity").full_name(), "Book");
    assert_eq!(search.ranking, Some(DddRanking::Bm25));
    assert_eq!(
        search
            .text
            .iter()
            .map(|field| field.property.full_name())
            .collect::<Vec<_>>(),
        vec!["title", "synopsis"]
    );
    assert_eq!(search.text[1].boost, Some(2));
    assert_eq!(search.filters.len(), 1);
    assert_eq!(search.sort.len(), 1);
}

#[test]
fn search_parse_failures_are_clean_errors() {
    for (source, label) in [
        (
            "application A { module m { search { entity B } } }",
            "missing name",
        ),
        (
            "application A { module m { search S { document { h = title } } } }",
            "unterminated document entry",
        ),
        (
            "application A { module m { search S { document { h = ; } } } }",
            "empty document entry",
        ),
        (
            "application A { module m { search S { ranking } } }",
            "ranking without a strategy",
        ),
        (
            r#"application A { module m { search S { ranking custom } } }"#,
            "custom without a string",
        ),
    ] {
        let result = parse_ddd(source);
        assert!(!result.errors.is_empty(), "{label} must be an error");
    }
}

#[test]
fn search_formats_canonically_and_is_idempotent() {
    let formatted = fmt_ddd(SEARCH);
    assert_eq!(
        formatted,
        concat!(
            "import \"library.mox\";\n",
            "\n",
            "application Library {\n",
            "    module media {\n",
            "        // a leading line comment\n",
            "        search MediaSearch {\n",
            "            entity Movie\n",
            "            text {\n",
            "                title boost 3\n",
            "                synopsis analyzer \"english\"\n",
            "            }\n",
            "            filters {\n",
            "                genre\n",
            "            }\n",
            "            sort {\n",
            "                title\n",
            "            }\n",
            "            document {\n",
            "                headline = title;\n",
            "                blurb = title + \" - \" + synopsis;\n",
            "                teaser = if(runtimeMinutes > 100) { title + \" (long)\" } else { \"a; b\" };\n",
            "            }\n",
            "            ranking bm25\n",
            "            analyzer \"english\"\n",
            "            pagination {\n",
            "                cursor\n",
            "                max 100\n",
            "                limit 20\n",
            "            }\n",
            "            capability SearchMedia, SearchMediaAdmin\n",
            "        }\n",
            "    }\n",
            "}\n",
        ),
        "members must reorder into the canonical order, got:\n{formatted}"
    );
    assert_eq!(
        fmt_ddd(&formatted),
        formatted,
        "formatting must be idempotent"
    );
}

#[test]
fn search_output_round_trips_structurally() {
    let formatted = fmt_ddd(SEARCH);
    let once = parse_ddd(&formatted);
    assert!(
        once.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}",
        once.errors
    );
    let twice = parse_ddd(&fmt_ddd(&formatted));
    assert!(
        twice.errors.is_empty(),
        "re-formatted output must parse cleanly, got {:?}",
        twice.errors
    );
    assert_eq!(once.ast, twice.ast);
}

#[test]
fn adversarial_search_inputs_do_not_panic() {
    let sources = [
        "search",
        "search S",
        "search S {",
        "search S { entity",
        "search S { text { } filters { } document { } }",
        "search S { document { = title; } }",
        "search S { document { h = title } document { h2 = synopsis; } }",
        "search S { document { h = \"unterminated } } }",
        "search S { document { h = (((title) ; } }",
        "search S { pagination { limit } }",
        "search S { pagination { max x } }",
        "search S { text { title boost -3 } }",
        "application A { module m { search S { entity E } search T { entity F } } }",
    ];
    for source in sources {
        let _ = parse_ddd(source);
        // Formatting may fail on lex errors (e.g. unterminated strings);
        // the contract is only that neither path panics.
        let _ = format_ddd(source);
    }
}

// --- state machines -------------------------------------------------------------

/// A machine-bearing design: repository + machine on one design, a machine on
/// a body-less design, triggered and bare transitions, an edge guard, and
/// service/repository ops guarded by machine states.
const MACHINE: &str = r#"
import "payrun.mox"

application Payroll {
    base nz.example.payroll
    module payroll {
        service PayRunService {
            boolean approve() capability ApprovePayRun when (status == AwaitingApproval);
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
            PhysicalMedia[] findByState(String state) when (status == Posted);
        }
        machine status: PayRunStatus {
            initial Draft;
            Draft -> Calculating via Calculate;
            AwaitingApproval -> Approved via Approve when (total > 0);
            Draft -> Reversed;
            terminal Reversed;
            terminal Posted;
        }
        entity AuditLog
        machine state: AuditState {
            initial Open;
            Open -> Closed via Close;
            terminal Closed;
        }
    }
}
"#;

#[test]
fn machine_blocks_parse() {
    let result = parse_ddd(MACHINE);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let module = expect_module(&file, "payroll");

    let payrun = module
        .designs
        .iter()
        .find(|design| design.class.text == "PayRun")
        .expect("the PayRun design");
    let machine = payrun.machine.as_ref().expect("the PayRun machine");
    assert_eq!(machine.field.text, "status");
    assert_eq!(machine.type_.name.full_name(), "PayRunStatus");
    assert_eq!(machine.initial.as_ref().expect("initial").text, "Draft");
    assert_eq!(
        machine
            .terminals
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Reversed", "Posted"]
    );
    assert_eq!(machine.transitions.len(), 3);
    let approve = &machine.transitions[1];
    assert_eq!(approve.from.text, "AwaitingApproval");
    assert_eq!(approve.to.text, "Approved");
    assert_eq!(approve.trigger.as_ref().expect("trigger").text, "Approve");
    assert_eq!(
        span_text(MACHINE, approve.guard.expect("guard")),
        "(total > 0)"
    );
    let bare = &machine.transitions[2];
    assert_eq!(bare.from.text, "Draft");
    assert_eq!(bare.to.text, "Reversed");
    assert!(bare.trigger.is_none());
    assert!(bare.guard.is_none());

    // A machine on a body-less design parses the same way.
    let audit = module
        .designs
        .iter()
        .find(|design| design.class.text == "AuditLog")
        .expect("the AuditLog design");
    assert!(audit.repository.is_none());
    let audit_machine = audit.machine.as_ref().expect("the AuditLog machine");
    assert_eq!(audit_machine.field.text, "state");
    assert_eq!(audit_machine.transitions.len(), 1);
}

#[test]
fn service_op_when_clause_parses() {
    let result = parse_ddd(MACHINE);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let module = expect_module(&file, "payroll");
    let service = module
        .services
        .iter()
        .find(|service| service.name.text == "PayRunService")
        .expect("the PayRunService");
    let approve = service
        .operations
        .iter()
        .find(|op| op.name.text == "approve")
        .expect("the approve operation");
    assert_eq!(
        span_text(MACHINE, approve.condition.expect("condition")),
        "(status == AwaitingApproval)"
    );

    // A declared repository operation takes a `when` clause but no
    // `capability` clause (the documented slice-1).
    let payrun = module
        .designs
        .iter()
        .find(|design| design.class.text == "PayRun")
        .expect("the PayRun design");
    let repository = payrun.repository.as_ref().expect("repository");
    let by_state = repository
        .operations
        .iter()
        .find(|op| op.name.text == "findByState")
        .expect("the findByState operation");
    assert_eq!(
        span_text(MACHINE, by_state.condition.expect("condition")),
        "(status == Posted)"
    );
}

/// A service operation carrying the `emits` clause: a comma list of event
/// names, written between the `capability` clause and the `when` guard
/// (the canonical clause order).
const EMITS: &str = r#"
import "payrun.mox"

application Payroll {
    base nz.example.payroll
    module payroll {
        service PayRunService {
            boolean approve(String approver) capability ApprovePayRun emits PayRunStateChanged, PayRunSubmitted when (status == AwaitingApproval);
            save => payRunRepository.save emits PayRunStateChanged;
            void purge() emits PayRunPurged;
            void replicate();
        }
    }
}
"#;

#[test]
fn service_op_emits_clause_parses() {
    let result = parse_ddd(EMITS);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let service = &expect_module(&file, "payroll").services[0];
    assert_eq!(service.operations.len(), 4);

    // A declared op: emits names collected in source order, the whole-decl
    // span covering the clause and the `;`.
    let approve = &service.operations[0];
    assert_eq!(approve.name.text, "approve");
    assert_eq!(
        approve
            .capabilities
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["ApprovePayRun"]
    );
    assert_eq!(
        approve
            .emits
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["PayRunStateChanged", "PayRunSubmitted"]
    );
    assert_eq!(
        span_text(EMITS, approve.condition.expect("condition")),
        "(status == AwaitingApproval)"
    );
    assert_eq!(
        span_text(EMITS, approve.span),
        "boolean approve(String approver) capability ApprovePayRun emits PayRunStateChanged, \
         PayRunSubmitted when (status == AwaitingApproval);"
    );

    // A delegated op carries the clause too (it is a service operation).
    let delegated = &service.operations[1];
    assert!(delegated.delegation.is_some());
    assert_eq!(
        delegated
            .emits
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["PayRunStateChanged"]
    );

    // An op with no clause emits nothing (the AST default).
    let replicate = &service.operations[3];
    assert_eq!(replicate.name.text, "replicate");
    assert!(replicate.emits.is_empty());
}

#[test]
fn emits_clause_requires_at_least_one_event_name() {
    let result = parse_ddd("application A { module M { service S { boolean approve() emits; } } }");
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("`emits` requires at least one event name")),
        "expected the emits error, got {:?}",
        result.errors
    );
}

#[test]
fn emits_formats_in_canonical_clause_order() {
    // The source is already canonical (capability → emits → when); the
    // formatter re-emits it byte-identically and is idempotent.
    let formatted = fmt_ddd(EMITS);
    assert_eq!(
        formatted,
        concat!(
            "import \"payrun.mox\";\n",
            "\n",
            "application Payroll {\n",
            "    base nz.example.payroll\n",
            "    module payroll {\n",
            "        service PayRunService {\n",
            "            boolean approve(String approver) capability ApprovePayRun emits PayRunStateChanged, PayRunSubmitted when (status == AwaitingApproval);\n",
            "            save => payRunRepository.save emits PayRunStateChanged;\n",
            "            void purge() emits PayRunPurged;\n",
            "            void replicate();\n",
            "        }\n",
            "    }\n",
            "}\n",
        ),
        "canonical clause order capability → emits → when, got:\n{formatted}"
    );
    assert_eq!(
        fmt_ddd(&formatted),
        formatted,
        "formatting must be idempotent"
    );

    // The formatted output parses cleanly.
    let once = parse_ddd(&formatted);
    assert!(
        once.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}",
        once.errors
    );
}

/// A delegated service operation carrying the `transition` clause (issue
/// rexlang#176): the explicit transition binding, unqualified or qualified
/// by the machine's enum type name.
const TRANSITION: &str = r#"
import "payrun.mox"

application Payroll {
    base nz.example.payroll
    module payroll {
        service PayRunService {
            inject PayRunRepository;
            submit => PayRunRepository.save transition Submit when (status == Draft);
            reverse => PayRunRepository.save transition PayRunStatus.Reverse;
            protected post => PayRunRepository.save capability PostPayRun emits PayRunPosted transition Post;
        }
    }
}
"#;

#[test]
fn transition_clause_parses() {
    let result = parse_ddd(TRANSITION);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let service = &expect_module(&file, "payroll").services[0];
    assert_eq!(service.operations.len(), 3);

    // Unqualified: the trigger only, `machine` none; the guard rides after
    // the binding in the canonical clause order.
    let submit = &service.operations[0];
    let transition = submit.transition.as_ref().expect("the transition binding");
    assert_eq!(transition.trigger.text, "Submit");
    assert!(transition.machine.is_none());
    assert_eq!(span_text(TRANSITION, transition.span), "transition Submit");
    assert_eq!(
        span_text(TRANSITION, submit.condition.expect("condition")),
        "(status == Draft)"
    );

    // Qualified: the machine qualifier is the enum type name, split at the
    // dot.
    let reverse = &service.operations[1];
    let qualified = reverse.transition.as_ref().expect("the transition binding");
    assert_eq!(qualified.trigger.text, "Reverse");
    assert_eq!(
        qualified.machine.as_ref().expect("the qualifier").text,
        "PayRunStatus"
    );
    assert_eq!(
        span_text(TRANSITION, qualified.span),
        "transition PayRunStatus.Reverse"
    );

    // The binding composes with the other clauses, in the canonical order:
    // capability, emits, transition.
    let post = &service.operations[2];
    assert!(post.is_protected);
    assert_eq!(post.capabilities[0].text, "PostPayRun");
    assert_eq!(post.emits[0].text, "PayRunPosted");
    let bound = post.transition.as_ref().expect("the transition binding");
    assert_eq!(bound.trigger.text, "Post");
    assert!(bound.machine.is_none());
    assert!(post.condition.is_none());

    // An op without the binding carries none (the AST default).
    let plain = r#"
application A { module m { service S {
    void go();
    finish => T.save;
} } }
"#;
    let result = parse_ddd(plain);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let ops = &expect_module(&file, "m").services[0].operations;
    assert!(ops[0].transition.is_none());
    assert!(ops[1].transition.is_none());
}

#[test]
fn transition_clause_requires_a_trigger_name() {
    let result =
        parse_ddd("application A { module m { service S { finish => T.save transition; } } }");
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("`transition` requires a trigger name")),
        "expected the transition error, got {:?}",
        result.errors
    );
}

#[test]
fn transition_clause_order_is_pinned() {
    // The clause order is fixed: `capability? invokes? emits? transition?
    // when?`. A violation is a syntax error, not a formatting opportunity.
    for source in [
        // `transition` before `emits`.
        "application A { module m { service S { finish => T.save transition Submit emits PayRunPosted; } } }",
        // `transition` after the guard.
        "application A { module m { service S { finish => T.save when (total > 0) transition Submit; } } }",
        // A third dotted segment is not part of the spelling.
        "application A { module m { service S { finish => T.save transition pkg.PayRunStatus.Reverse; } } }",
    ] {
        let result = parse_ddd(source);
        assert!(
            !result.errors.is_empty(),
            "expected a syntax error for {source}"
        );
    }
}

#[test]
fn transition_formats_in_canonical_clause_order() {
    // The source is already canonical (emits → transition → when); the
    // formatter re-emits it byte-identically and is idempotent.
    let formatted = fmt_ddd(TRANSITION);
    assert_eq!(
        formatted,
        concat!(
            "import \"payrun.mox\";\n",
            "\n",
            "application Payroll {\n",
            "    base nz.example.payroll\n",
            "    module payroll {\n",
            "        service PayRunService {\n",
            "            inject PayRunRepository;\n",
            "            submit => PayRunRepository.save transition Submit when (status == Draft);\n",
            "            reverse => PayRunRepository.save transition PayRunStatus.Reverse;\n",
            "            protected post => PayRunRepository.save capability PostPayRun emits PayRunPosted transition Post;\n",
            "        }\n",
            "    }\n",
            "}\n",
        ),
        "canonical clause order emits → transition → when, got:\n{formatted}"
    );
    assert_eq!(
        fmt_ddd(&formatted),
        formatted,
        "formatting must be idempotent"
    );

    // The formatted output parses cleanly.
    let once = parse_ddd(&formatted);
    assert!(
        once.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}",
        once.errors
    );
}

#[test]
fn transition_word_stays_contextual() {
    // `transition` is special only in the clause position between a service
    // operation's `emits` clause and its `when` guard; everywhere else it
    // is an ordinary identifier: service name, op name, class name — and
    // `^transition` escapes as any contextual word. A machine transition
    // line may also be named `transition` (the from-state position).
    let source = concat!(
        "application transition { module m {\n",
        "    service transition {\n",
        "        void transition();\n",
        "    }\n",
        "    entity transition\n",
        "    entity ^transition\n",
        "    machine transition: Status {\n",
        "        initial Draft;\n",
        "        transition -> Posted;\n",
        "        terminal Posted;\n",
        "    }\n",
        "} }\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let module = expect_module(&file, "m");
    assert_eq!(module.designs[0].class.text, "transition");
    assert_eq!(module.designs[1].class.text, "transition");
    assert_eq!(module.services[0].name.text, "transition");
    // The machine block attaches to the preceding design (the `^transition`
    // one), and its transition line may itself be named `transition`.
    let machine = module.designs[1].machine.as_ref().expect("the machine");
    assert_eq!(machine.field.text, "transition");
    assert_eq!(machine.transitions[0].from.text, "transition");
}

#[test]
fn emits_word_stays_contextual() {
    // `emits` is special only between a service operation's capability and
    // guard clauses; everywhere else it is an ordinary identifier: service
    // name, inject target, op name, parameter type, parameter name,
    // capability name, emit name, class name — and `^emits` escapes as any
    // contextual word.
    let source = concat!(
        "application A { module m {\n",
        "    service emits {\n",
        "        inject emits;\n",
        "        boolean emits(emits emits) capability emits;\n",
        "        value => entity.emits emits emits;\n",
        "    }\n",
        "    entity emits\n",
        "    entity ^emits\n",
        "} }\n",
    );
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let module = expect_module(&file, "m");
    assert_eq!(module.designs[0].class.text, "emits");
    assert_eq!(module.designs[1].class.text, "emits");
    let service = &module.services[0];
    assert_eq!(service.name.text, "emits");
    assert_eq!(service.dependencies[0].text, "emits");
    let declared = &service.operations[0];
    assert_eq!(declared.name.text, "emits");
    assert_eq!(
        declared.return_type.as_ref().unwrap().name.full_name(),
        "boolean"
    );
    assert_eq!(declared.params[0].type_ref.name.full_name(), "emits");
    assert_eq!(declared.params[0].name.text, "emits");
    assert_eq!(declared.capabilities[0].text, "emits");
    // A delegated op named `value` targeting `entity.emits`, emitting the
    // event `emits`: the keyword position is behind the delegation target.
    let delegated = &service.operations[1];
    let delegation = delegated.delegation.as_ref().unwrap();
    assert_eq!(delegation.target.full_name(), "entity");
    assert_eq!(delegation.operation.text, "emits");
    assert_eq!(delegated.emits[0].text, "emits");
}

#[test]
fn machine_words_stay_contextual() {
    // `machine`, `initial`, `terminal`, and `via` are ordinary identifiers in
    // every other position: class names, field names, enum names, state
    // names, and escaped forms.
    let source = r#"
application A {
    module m {
        entity machine
        machine initial: terminal {
            initial terminal;
            terminal -> initial via via;
            terminal initial;
        }
        entity ^machine
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let module = expect_module(&file, "m");
    assert_eq!(module.designs[0].class.text, "machine");
    let machine = module.designs[0].machine.as_ref().expect("machine");
    assert_eq!(machine.field.text, "initial");
    assert_eq!(machine.type_.name.full_name(), "terminal");
    assert_eq!(machine.initial.as_ref().expect("initial").text, "terminal");
    assert_eq!(machine.terminals[0].text, "initial");
    assert_eq!(machine.transitions[0].from.text, "terminal");
    assert_eq!(machine.transitions[0].to.text, "initial");
    assert_eq!(machine.transitions[0].trigger.as_ref().unwrap().text, "via");
    assert_eq!(module.designs[1].class.text, "machine");
}

#[test]
fn machine_parse_failures_are_clean_errors() {
    for (source, label) in [
        (
            "application A { module m { machine { initial Open; } } }",
            "missing field and type",
        ),
        (
            "application A { module m { entity E machine status: S { Draft; } } }",
            "transition without an arrow",
        ),
        (
            "application A { module m { entity E machine status: S { initial; } } }",
            "initial without a state",
        ),
        (
            "application A { module m { entity E machine status: S { Draft -> Done via; } } }",
            "via without a trigger",
        ),
        (
            "application A { module m { entity E machine status: S { Open -> Done when total; } } }",
            "when without parens",
        ),
        (
            "application A { module m { entity E machine status: S { Open -> Done, Cancelled; } } }",
            "multi-target arrow",
        ),
    ] {
        let result = parse_ddd(source);
        assert!(!result.errors.is_empty(), "{label} must be an error");
    }
}

#[test]
fn machine_recovered_errors_keep_parsing_the_design() {
    let source = r#"
application A {
    module m {
        entity E
        machine status: S {
            initial Open;
            Open -> Done via;
            terminal Done;
        }
        entity F repository FRepository {
            findById;
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        !result.errors.is_empty(),
        "the `via` without a trigger must be an error"
    );
    let file = result.ast.expect("a partial AST");
    let module = expect_module(&file, "m");
    assert!(
        module.designs.iter().any(|design| design.class.text == "F"),
        "the design after the broken machine must still parse"
    );
}

#[test]
fn machines_format_canonically_and_are_idempotent() {
    let formatted = fmt_ddd(MACHINE);
    assert_eq!(
        formatted,
        concat!(
            "import \"payrun.mox\";\n",
            "\n",
            "application Payroll {\n",
            "    base nz.example.payroll\n",
            "    module payroll {\n",
            "        service PayRunService {\n",
            "            boolean approve() capability ApprovePayRun when (status == AwaitingApproval);\n",
            "        }\n",
            "        entity PayRun repository PayRunRepository {\n",
            "            findById;\n",
            "            save;\n",
            "            PhysicalMedia[] findByState(String state) when (status == Posted);\n",
            "        }\n",
            "        machine status: PayRunStatus {\n",
            "            initial Draft;\n",
            "            Draft -> Calculating via Calculate;\n",
            "            AwaitingApproval -> Approved via Approve when (total > 0);\n",
            "            Draft -> Reversed;\n",
            "            terminal Reversed;\n",
            "            terminal Posted;\n",
            "        }\n",
            "        entity AuditLog\n",
            "        machine state: AuditState {\n",
            "            initial Open;\n",
            "            Open -> Closed via Close;\n",
            "            terminal Closed;\n",
            "        }\n",
            "    }\n",
            "}\n",
        ),
        "the machine source is already canonical, got:\n{formatted}"
    );
    assert_eq!(
        fmt_ddd(&formatted),
        formatted,
        "formatting must be idempotent"
    );
}

#[test]
fn machine_output_round_trips_structurally() {
    let formatted = fmt_ddd(MACHINE);
    let once = parse_ddd(&formatted);
    assert!(
        once.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}",
        once.errors
    );
    let twice = parse_ddd(&fmt_ddd(&formatted));
    assert!(
        twice.errors.is_empty(),
        "re-formatted output must parse cleanly, got {:?}",
        twice.errors
    );
    assert_eq!(once.ast, twice.ast);
}

#[test]
fn adversarial_machine_inputs_do_not_panic() {
    let sources = [
        "machine",
        "machine status",
        "machine status:",
        "machine status: S",
        "machine status: S {",
        "machine status: S { initial",
        "machine status: S { Open ->",
        "machine status: S { Open -> Done when }",
        "machine status: S { Open -> Done when ( }",
        "application A { module m { entity E machine status: S { initial Open; } machine other: T { initial X; } } }",
    ];
    for source in sources {
        let _ = parse_ddd(source);
        // Formatting may fail on lex errors; the contract is only that
        // neither path panics.
        let _ = format_ddd(source);
    }
}

// --- decision links (`invokes` clause, `inject decision`) ----------------------

/// A service exercising the decision-link surface: plain and decision
/// injects, and the `invokes` clause on declared and delegated operations.
const DECISION_LINKS: &str = r#"
application A {
    module m {
        service LoanService {
            inject LoanRepository;
            inject decision underwriting.RiskScore;
            inject decision FraudScreen;
            boolean approve(int amount) capability ApproveLoans invokes RiskScore, underwriting.FraudScreen emits LoanApproved when (true);
            route => riskEngine.evaluate invokes CreditBureau;
        }
    }
}
"#;

#[test]
fn invokes_clauses_parse_on_declared_and_delegated_ops() {
    let result = parse_ddd(DECISION_LINKS);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let service = &expect_module(&file, "m").services[0];

    let approve = &service.operations[0];
    assert_eq!(approve.name.text, "approve");
    let invokes: Vec<String> = approve
        .invokes
        .iter()
        .map(|entry| entry.name.full_name())
        .collect();
    assert_eq!(invokes, vec!["RiskScore", "underwriting.FraudScreen"]);
    // The entries carry no pin here; the spans cover the references
    // themselves, source order kept.
    assert!(approve.invokes.iter().all(|entry| entry.pin.is_none()));
    assert_eq!(
        span_text(DECISION_LINKS, approve.invokes[0].name.span),
        "RiskScore"
    );
    assert_eq!(
        span_text(DECISION_LINKS, approve.invokes[1].name.span),
        "underwriting.FraudScreen"
    );

    // A delegated op carries the clause too (it is a service operation).
    let route = &service.operations[1];
    assert!(route.delegation.is_some());
    assert_eq!(
        route
            .invokes
            .iter()
            .map(|entry| entry.name.full_name())
            .collect::<Vec<_>>(),
        vec!["CreditBureau"]
    );
    assert_eq!(
        span_text(DECISION_LINKS, route.invokes[0].name.span),
        "CreditBureau"
    );
}

#[test]
fn inject_decision_parses_beside_the_plain_form() {
    let result = parse_ddd(DECISION_LINKS);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let service = &expect_module(&file, "m").services[0];

    // Plain `inject` is unchanged; `inject decision` collects its own list,
    // both in source order.
    assert_eq!(
        service
            .dependencies
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["LoanRepository"]
    );
    assert_eq!(
        service
            .decision_dependencies
            .iter()
            .map(|entry| entry.name.full_name())
            .collect::<Vec<_>>(),
        vec!["underwriting.RiskScore", "FraudScreen"]
    );
    assert!(service
        .decision_dependencies
        .iter()
        .all(|entry| entry.pin.is_none()));
}

/// The `version` pins (rexlang#154): `compatible` and the exact string,
/// per `invokes` entry and on the `inject decision` line, and the
/// committed-missing-value error.
#[test]
fn version_pins_parse_on_invokes_and_inject_decision() {
    let source = r#"
application A {
    module m {
        service LoanService {
            inject decision underwriting.RiskScore version compatible;
            inject decision FraudScreen version "1.4.2";
            boolean approve() invokes RiskScore version compatible, underwriting.FraudScreen version "1.4.2", CreditBureau;
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let service = &expect_module(&file, "m").services[0];

    use rex_syntax::ast::DddVersionPin;
    let injects = &service.decision_dependencies;
    assert_eq!(
        injects
            .iter()
            .map(|entry| entry.name.full_name())
            .collect::<Vec<_>>(),
        vec!["underwriting.RiskScore", "FraudScreen"]
    );
    assert_eq!(injects[0].pin, Some(DddVersionPin::Compatible));
    assert_eq!(injects[1].pin, Some(DddVersionPin::Exact("1.4.2".into())));

    let invokes = &service.operations[0].invokes;
    assert_eq!(invokes.len(), 3);
    assert_eq!(invokes[0].pin, Some(DddVersionPin::Compatible));
    assert_eq!(invokes[1].pin, Some(DddVersionPin::Exact("1.4.2".into())));
    assert_eq!(
        invokes[2].pin, None,
        "the pin-less entry keeps today's shape"
    );
}

#[test]
fn a_bare_version_pin_recovers_with_an_error() {
    // The pin is committed once `version` matched: a missing value is a
    // syntax error, and the clause still recovers.
    let source = r#"
application A {
    module m {
        service S {
            boolean broken() invokes RiskScore version;
            boolean fine();
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        !result.errors.is_empty(),
        "a bare `version` pin is a reported error"
    );
    assert!(result.ast.is_some(), "the service still recovers");
}

#[test]
fn a_missing_invokes_name_recovers_with_an_error() {
    // The clause is committed once the keyword matched: a missing name list
    // is a syntax error, and the rest of the service still parses.
    let source = r#"
application A {
    module m {
        service S {
            boolean broken() invokes;
            boolean fine();
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        !result.errors.is_empty(),
        "the empty `invokes` clause must be an error"
    );
    let file = result.ast.expect("a partial AST");
    let service = &expect_module(&file, "m").services[0];
    assert!(
        service
            .operations
            .iter()
            .any(|op| op.name.text == "fine" && op.invokes.is_empty()),
        "the operation after the broken clause must still parse"
    );
}

#[test]
fn decision_links_format_canonically_and_round_trip() {
    // The canonical clause order is `capability ... invokes ... emits ...
    // when ...`; the decision inject re-emits with its discriminator.
    let canonical = concat!(
        "application A {\n",
        "    module m {\n",
        "        service LoanService {\n",
        "            inject LoanRepository;\n",
        "            inject decision underwriting.RiskScore;\n",
        "            inject decision FraudScreen;\n",
        "            boolean approve(int amount) capability ApproveLoans invokes RiskScore, underwriting.FraudScreen emits LoanApproved when (true);\n",
        "            route => riskEngine.evaluate invokes CreditBureau;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_ddd(DECISION_LINKS), canonical);
    // Idempotence: the canonical output is a fixpoint.
    assert_eq!(fmt_ddd(canonical), canonical);
    // The formatted output parses back to the same AST.
    let once = parse_ddd(&fmt_ddd(DECISION_LINKS));
    let twice = parse_ddd(&fmt_ddd(&fmt_ddd(DECISION_LINKS)));
    assert!(once.errors.is_empty(), "{:?}", once.errors);
    assert!(twice.errors.is_empty(), "{:?}", twice.errors);
    assert_eq!(once.ast, twice.ast);
}

#[test]
fn invokes_and_decision_stay_contextual_identifiers() {
    // `^invokes` is an ordinary (escaped) operation name; `^decision` is a
    // plain dependency named `decision`; neither parses as its keyword.
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            inject ^decision;\n",
        "            boolean ^invokes();\n",
        "        }\n",
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
    let service = &expect_module(&file, "m").services[0];
    assert_eq!(
        service
            .dependencies
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["decision"]
    );
    assert!(service.decision_dependencies.is_empty());
    assert_eq!(service.operations[0].name.text, "invokes");
    assert!(service.operations[0].invokes.is_empty());
    // The escaped spellings survive formatting (the escape is load-bearing).
    let formatted = fmt_ddd(source);
    assert!(formatted.contains("inject ^decision;"), "{formatted}");
    assert!(formatted.contains("boolean ^invokes();"), "{formatted}");
}

// --- child-scoped extend blocks (issue rexlang#179) ---------------------------

/// A module with two `extend` blocks over one aggregate root: a collection
/// child with `add`/`remove`/`save` actions (one `protected`, one guarded),
/// and a scalar child with `save`.
const EXTEND: &str = r#"
import "payroll.mox";

application Payroll {

    base nz.example.payroll

    module payroll {
        entity PayRun repository PayRunRepository {
            findById;
        }
        extend PayRun {
            boolean addLine(PayRunLineDraft line) => lines.add;
            protected boolean removeLine(String lineId) => lines.remove when (lineId != "");
        }
        extend PayRun {
            boolean saveLine(PayRunLineDraft line) => lines.save;
        }
    }
}
"#;

#[test]
fn extend_blocks_parse_with_child_operations() {
    let result = parse_ddd(EXTEND);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let module = expect_module(&file, "payroll");
    assert_eq!(module.extends.len(), 2, "both blocks are kept in order");

    let first = &module.extends[0];
    assert_eq!(first.class.text, "PayRun");
    assert_eq!(first.operations.len(), 2);

    let add = &first.operations[0];
    assert_eq!(add.name.text, "addLine");
    assert_eq!(add.return_type.name.full_name(), "boolean");
    assert_eq!(add.params[0].name.text, "line");
    assert_eq!(add.params[0].type_ref.name.full_name(), "PayRunLineDraft");
    assert_eq!(add.feature.text, "lines");
    assert_eq!(add.action.text, "add");
    assert!(!add.is_protected);
    assert!(add.condition.is_none());

    let remove = &first.operations[1];
    assert!(remove.is_protected);
    assert_eq!(remove.action.text, "remove");
    assert_eq!(
        span_text(EXTEND, remove.condition.expect("the guard")),
        "(lineId != \"\")"
    );

    assert_eq!(module.extends[1].operations[0].action.text, "save");
}

#[test]
fn extend_formats_canonically_and_is_idempotent() {
    let canonical = concat!(
        "import \"payroll.mox\";\n",
        "\n",
        "application Payroll {\n",
        "    base nz.example.payroll\n",
        "    module payroll {\n",
        "        entity PayRun repository PayRunRepository {\n",
        "            findById;\n",
        "        }\n",
        "        extend PayRun {\n",
        "            boolean addLine(PayRunLineDraft line) => lines.add;\n",
        "            protected boolean removeLine(String lineId) => lines.remove when (lineId != \"\");\n",
        "        }\n",
        "        extend PayRun {\n",
        "            boolean saveLine(PayRunLineDraft line) => lines.save;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let formatted = fmt_ddd(EXTEND);
    assert_eq!(formatted, canonical, "extend layout drifted");
    assert_eq!(fmt_ddd(&formatted), formatted, "formatting is a fixpoint");
    assert_eq!(
        parse_ddd(&formatted).errors.len(),
        0,
        "the canonical form parses"
    );
}

#[test]
fn child_op_without_an_action_is_an_error() {
    let source = r#"
import "payroll.mox"

application Payroll {
    base nz.example.payroll
    module payroll {
        entity PayRun repository PayRunRepository {
            findById;
        }
        extend PayRun {
            boolean addLine(PayRunLineDraft line) => lines;
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected `feature.action`")),
        "a one-segment child target is a syntax error: {:?}",
        result.errors
    );
}

#[test]
fn extend_remains_a_contextual_identifier() {
    // `extend` is an ordinary name everywhere but module-member position.
    let source = r#"
application Payroll {
    module payroll {
        entity extend
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        result.errors.is_empty(),
        "a design may be named `extend`: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(
        expect_module(&file, "payroll").designs[0].class.text,
        "extend"
    );
}

#[test]
fn a_junk_child_operation_recovers_within_the_extend_block() {
    let source = r#"
import "payroll.mox"

application Payroll {
    base nz.example.payroll
    module payroll {
        extend PayRun {
            nope;
            boolean addLine(PayRunLineDraft line) => lines.add;
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        !result.errors.is_empty(),
        "the junk line is reported, not silently dropped"
    );
    let file = result.ast.expect("the file still parses");
    let module = expect_module(&file, "payroll");
    assert_eq!(
        module.extends[0]
            .operations
            .iter()
            .map(|operation| operation.name.text.as_str())
            .collect::<Vec<_>>(),
        ["addLine"],
        "the valid operation after the junk still parses"
    );
}

// --- executable operation bodies (issue #178) --------------------------------

const BODIES: &str = r#"
import "payroll.mox"

application Payroll {
    base nz.example.payroll
    module payroll {
        service PayRunService {
            inject PayRunRepository;
            submit(PayRunDraft draft) capability SubmitPayRun when (draft.quantity > 0) => PayRunRepository {
                let run=self.findById("x") else error NotFound("PayRun","x");
                guard run.total>=0 else error Conflict("total");
                run.total=draft.quantity;
                if (run.total>0) { let positive=true; } else { let positive=false; }
            }
            boolean approve() => PayRunRepository {
                return true;
            }
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
            int totalQuantity(String runId) when (runId != "") {
                let run = self.findById(runId) else error NotFound("PayRun", runId);
                return run.lines.sum(line => line.quantity);
            }
        }
    }
}
"#;

#[test]
fn service_and_repository_bodies_parse_into_statements() {
    let result = parse_ddd(BODIES);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let module = expect_module(&file, "payroll");
    let service = &module.services[0];
    let submit = &service.operations[0];
    assert_eq!(submit.name.text, "submit");
    assert_eq!(
        submit.body_receiver.as_ref().map(|name| name.text.as_str()),
        Some("PayRunRepository")
    );
    let body = submit.body.as_ref().expect("the service body parsed");
    assert_eq!(body.stmts.len(), 4, "let, guard, assign, if");

    let DddStmt::Let {
        name,
        expr,
        raise: Some(raise),
    } = &body.stmts[0]
    else {
        panic!("expected a let-else, got {:?}", body.stmts[0]);
    };
    assert_eq!(name.text, "run");
    assert_eq!(span_text(BODIES, *expr), r#"self.findById("x")"#);
    assert_eq!(raise.variant.text, "NotFound");
    assert_eq!(
        raise
            .args
            .iter()
            .map(|arg| span_text(BODIES, *arg))
            .collect::<Vec<_>>(),
        [r#""PayRun""#, r#""x""#],
    );

    assert!(matches!(&body.stmts[1], DddStmt::Guard { .. }));
    let DddStmt::Assign { path, expr } = &body.stmts[2] else {
        panic!("expected an assignment, got {:?}", body.stmts[2]);
    };
    assert_eq!(
        path.iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        ["run", "total"]
    );
    assert_eq!(span_text(BODIES, *expr), "draft.quantity");

    let DddStmt::If { then, else_, .. } = &body.stmts[3] else {
        panic!("expected an if statement, got {:?}", body.stmts[3]);
    };
    assert_eq!(then.len(), 1);
    assert_eq!(else_.len(), 1);

    let approve = &service.operations[1];
    assert!(matches!(
        approve.body.as_ref().unwrap().stmts[0],
        DddStmt::Return { expr: Some(_) }
    ));

    // The repository operation's declared body carries no receiver.
    let repository = module.designs[0].repository.as_ref().expect("repository");
    assert_eq!(
        repository.operations[2].body.as_ref().unwrap().stmts.len(),
        2
    );
}

#[test]
fn service_and_repository_bodies_format_canonically() {
    let formatted = fmt_ddd(BODIES);
    let expected = r#"import "payroll.mox";

application Payroll {
    base nz.example.payroll
    module payroll {
        service PayRunService {
            inject PayRunRepository;
            submit(PayRunDraft draft) capability SubmitPayRun when (draft.quantity > 0) => PayRunRepository {
                let run = self.findById("x") else error NotFound("PayRun", "x");
                guard run.total >= 0 else error Conflict("total");
                run.total = draft.quantity;
                if (run.total > 0) {
                    let positive = true;
                } else {
                    let positive = false;
                }
            }
            boolean approve() => PayRunRepository {
                return true;
            }
        }
        entity PayRun repository PayRunRepository {
            findById;
            save;
            int totalQuantity(String runId) when (runId != "") {
                let run = self.findById(runId) else error NotFound("PayRun", runId);
                return run.lines.sum(line => line.quantity);
            }
        }
    }
}
"#;
    assert_eq!(formatted, expected);
    assert_eq!(fmt_ddd(&formatted), formatted, "formatting is a fixpoint");
}

#[test]
fn unsupported_statement_names_the_construct() {
    let source = r#"
application A {
    module M {
        service S {
            boolean submit() => R {
                while (true) { return true; }
            }
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("unsupported statement 'while' in a service body")),
        "expected the named refusal, got: {:?}",
        result.errors
    );
}

#[test]
fn builtin_repository_body_is_refused() {
    let source = r#"
application A {
    module M {
        entity E repository R {
            save { let x = 1; }
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("built-in repository operation 'save' takes no body")),
        "expected the builtin-body refusal, got: {:?}",
        result.errors
    );
}

#[test]
fn multi_segment_body_receiver_is_refused() {
    let source = r#"
application A {
    module M {
        service S {
            boolean submit() => a.b.RunRepository {
                return true;
            }
        }
    }
}
"#;
    let result = parse_ddd(source);
    assert!(
        result.errors.iter().any(|error| error
            .message
            .contains("body receiver must be a single repository name")),
        "expected the receiver refusal, got: {:?}",
        result.errors
    );
}
