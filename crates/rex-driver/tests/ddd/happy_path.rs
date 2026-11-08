//! `ddd` tests: happy path.

use crate::support::*;

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
