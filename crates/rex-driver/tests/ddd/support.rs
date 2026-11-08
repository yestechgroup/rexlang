//! Shared fixtures for the `ddd` integration-test target.

//! Integration tests for `.ddd` design compilation (`compile_ddd_str` /
//! `compile_ddd_str_with_actors`): lowering to the `rex_ir::ddd::DddModel`
//! and each validation rule against the imported domain models. The
//! normative contract lives on `compile_ddd_str`; these tests pin it.

pub(crate) use rex_driver::{
    compile_ddd_str, compile_ddd_str_with_actors, DddCompilation, Diagnostic,
};
pub(crate) use rex_ir::ddd::DDD_MODEL_FORMAT_VERSION;
pub(crate) use rex_ir::{ActorModel, ActorsDef, CapabilityDef, TypeRef};

/// One domain exercising resolution, containment (Library owns Books), an
/// enum, and a spare value-object class (every feature `readonly`, per the
/// value contract — rule 12).
pub(crate) const DOMAIN: &str = r#"
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
    readonly int cents
}
"#;

/// A three-link containment chain: Library owns Books, Books own Chapters.
pub(crate) const CHAIN_DOMAIN: &str = r#"
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

/// A domain for the aggregate reference constraint (rule 13): Library owns
/// Books, Books own Chapters, and PhysicalMedia sits *outside* the aggregate
/// while referring to one of its non-root members.
pub(crate) const AGGREGATE_DOMAIN: &str = r#"
package nz.example.library

class Library {
    String name
    contains Book[] books opposite library
}

class Book {
    container Library library opposite books
    String title
    contains Chapter[] chapters opposite book
}

class Chapter {
    container Book book opposite chapters
    String heading
}

class PhysicalMedia {
    String status
    refers Chapter media
}
"#;

/// A clean design over [`DOMAIN`]: an aggregate root with a repository, a
/// contained entity without one, and a value object.
pub(crate) const CLEAN: &str = r#"
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

pub(crate) fn compile(source: &str, domains: &[(&str, &str)]) -> DddCompilation {
    let domains: Vec<(String, String)> = domains
        .iter()
        .map(|(path, source)| (path.to_string(), source.to_string()))
        .collect();
    compile_ddd_str(
        "design.ddd",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
    )
}

/// Compiles against the single [`DOMAIN`] import.
pub(crate) fn compile_domain(source: &str) -> DddCompilation {
    compile(source, &[("library.mox", DOMAIN)])
}

/// Compiles against the single [`CHAIN_DOMAIN`] import.
pub(crate) fn compile_chain(source: &str) -> DddCompilation {
    compile(source, &[("chain.mox", CHAIN_DOMAIN)])
}

pub(crate) fn actors_with(capabilities: &[&str]) -> ActorModel {
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
pub(crate) fn single<'a>(compilation: &'a DddCompilation, needle: &str) -> &'a Diagnostic {
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
pub(crate) fn span_of(source: &str, needle: &str, nth: usize) -> rex_driver::Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

pub(crate) fn assert_clean(compilation: &DddCompilation) {
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}
