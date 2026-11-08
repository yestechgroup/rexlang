//! `ddd` tests: rule 11 searches.

use crate::support::*;

// --- rule 11: searches --------------------------------------------------------

/// A domain with inheritance, an enum, an int feature, and a class-typed
/// feature — everything the search rules distinguish.
const SEARCH_DOMAIN: &str = r#"
package nz.example.library

enum Genre {
    Drama as "D" = 0
}

class Media {
    String title
}

class Movie extends Media {
    Genre genre
    String synopsis
    int runtimeMinutes
    contains Scene[] scenes opposite movie
}

class Scene {
    container Movie movie opposite scenes
    int startAt
}
"#;

fn compile_search(source: &str) -> DddCompilation {
    compile(
        source,
        &[("search.mox", SEARCH_DOMAIN), ("library.mox", DOMAIN)],
    )
}

#[test]
fn search_lowers_with_all_members() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            text {\n",
        "                title boost 3\n",
        "                synopsis analyzer \"english\"\n",
        "            }\n",
        "            filters {\n",
        "                genre\n",
        "            }\n",
        "            sort {\n",
        "                synopsis\n",
        "            }\n",
        "            document {\n",
        "                blurb = title + \" - \" + synopsis;\n",
        "            }\n",
        "            ranking custom \"myRanker\"\n",
        "            pagination {\n",
        "                limit 10\n",
        "                max 50\n",
        "                cursor\n",
        "            }\n",
        "            capability SearchMovies\n",
        "        }\n",
        "        service S {\n",
        "            inject MovieSearch;\n",
        "            find => MovieSearch.search;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    assert_clean(&compilation);
    let model = compilation.model.expect("clean compile lowers a model");
    let module = &model.modules[0];
    let search = &module.searches[0];
    assert_eq!(search.name, "MovieSearch");
    assert_eq!(search.entity, "Movie");
    assert_eq!(search.text.len(), 2);
    assert_eq!(search.text[0].property, "title");
    assert_eq!(search.text[0].boost, Some(3.0.into()));
    assert_eq!(search.text[1].analyzer.as_deref(), Some("english"));
    assert_eq!(search.filters[0].property, "genre");
    assert_eq!(search.sort[0].property, "synopsis");
    assert_eq!(search.document.len(), 1);
    assert_eq!(search.document[0].expr, "title + \" - \" + synopsis");
    assert_eq!(
        search.ranking,
        Some(rex_ir::ddd::RankingStrategy::Custom("myRanker".to_string()))
    );
    let pagination = search.pagination.as_ref().expect("pagination");
    assert_eq!(pagination.limit, Some(10));
    assert_eq!(pagination.max_limit, Some(50));
    assert!(pagination.cursor);
    assert_eq!(search.capabilities, vec!["SearchMovies".to_string()]);
    // The service ties in: the search is a dependency and a delegation target.
    let service = &module.services[0];
    assert_eq!(service.dependencies, vec!["MovieSearch".to_string()]);
    assert_eq!(
        service.operations[0]
            .delegation
            .as_ref()
            .map(|d| d.target.clone()),
        Some("MovieSearch".to_string())
    );
    assert_eq!(
        service.operations[0]
            .delegation
            .as_ref()
            .map(|d| d.operation.clone()),
        Some("search".to_string())
    );
}

#[test]
fn search_names_are_unique_application_wide() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch { entity Movie }\n",
        "    }\n",
        "    module n {\n",
        "        entity Movie\n",
        "        search MovieSearch { entity Movie }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    let diagnostic = single(&compilation, "duplicate search 'MovieSearch'");
    assert_eq!(diagnostic.span, Some(span_of(source, "MovieSearch", 1)));
}

#[test]
fn search_requires_an_entity_line() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            text { title }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    single(
        &compilation,
        "search 'MovieSearch' does not declare an entity",
    );
}

#[test]
fn search_entity_must_resolve_to_a_class() {
    let source = concat!(
        "import \"library.mox\"\n",
        "\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library\n",
        "        search MovieSearch {\n",
        "            entity BookCategory\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    single(&compilation, "is an enum, not a class");
}

#[test]
fn search_entity_must_be_designed_in_the_same_module() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        search MovieSearch { entity Movie }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    let diagnostic = single(
        &compilation,
        "search 'MovieSearch' targets 'Movie', which has no entity design in module 'm'",
    );
    assert!(diagnostic
        .help
        .as_deref()
        .unwrap_or_default()
        .contains("add an `entity Movie` design to module 'm' first"));
}

#[test]
fn search_entity_designed_in_another_module_names_it() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        search MovieSearch { entity Movie }\n",
        "    }\n",
        "    module n {\n",
        "        entity Movie\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    let diagnostic = single(
        &compilation,
        "search 'MovieSearch' targets 'Movie', which has no entity design in module 'm'",
    );
    assert!(diagnostic
        .help
        .as_deref()
        .unwrap_or_default()
        .contains("'Movie' is designed as an entity in module 'n'"));
}

#[test]
fn search_text_fields_resolve_with_inheritance() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            text { title }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    assert_clean(&compilation);
    assert_eq!(
        compilation.model.expect("model").modules[0].searches[0].text[0].property,
        "title"
    );
}

#[test]
fn search_text_field_rules() {
    // Unknown feature.
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            text { bogus }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    let diagnostic = single(&compilation, "unknown text field 'bogus' on 'Movie'");
    assert!(diagnostic
        .help
        .as_deref()
        .unwrap_or_default()
        .contains("'Movie' declares:"));

    // A path, not a direct feature.
    let source = source.replace("text { bogus }", "text { scenes.startAt }");
    let compilation = compile_search(&source);
    single(
        &compilation,
        "text field 'scenes.startAt' must name a feature of 'Movie' directly",
    );

    // Not text-like.
    let source = source.replace("text { scenes.startAt }", "text { runtimeMinutes }");
    let compilation = compile_search(&source);
    single(&compilation, "text field 'runtimeMinutes' is not text-like");

    // Boost below 1.
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            text { title boost 0 }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    single(
        &compilation,
        "boost of text field 'title' must be at least 1",
    );
}

#[test]
fn search_filters_and_sorts_reject_class_references() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Scene\n",
        "        search SceneSearch {\n",
        "            entity Scene\n",
        "            filters { movie }\n",
        "            sort { startAt }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    single(&compilation, "filter 'movie' is a class reference");
}

#[test]
fn search_document_expressions_type_check_with_the_entity() {
    // A valid concat over inherited and own features lowers verbatim.
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            document {\n",
        "                blurb = title + \" - \" + synopsis;\n",
        "                runtime = runtimeMinutes;\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    assert_clean(&compilation);
    let document = &compilation.model.expect("model").modules[0].searches[0].document;
    assert_eq!(document[0].expr, "title + \" - \" + synopsis");
    assert_eq!(document[1].expr, "runtimeMinutes");

    // An R9 violation is a compile-time diagnostic.
    let source = source.replace("blurb = title + \" - \" + synopsis;", "blurb = title + 1;");
    let compilation = compile_search(&source);
    single(&compilation, "invalid document expression in field 'blurb'");

    // So is an unknown name.
    let source = source.replace("blurb = title + 1;", "blurb = bogus;");
    let compilation = compile_search(&source);
    single(&compilation, "invalid document expression in field 'blurb'");
}

/// A document expression may call a sigil-lowered package function: the
/// domain lowers through the sigil pipeline, so `Package::functions`
/// populate the `DomainTypes` the document-expression checker builds with
/// `TypeContext::from_model`.
#[test]
fn a_ddd_document_expression_may_call_a_package_function() {
    let mox = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/conformance/sigil/funcs.mox"),
    )
    .expect("read funcs.mox");
    let rosetta = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/conformance/sigil/funcs.rosetta"),
    )
    .expect("read funcs.rosetta");
    let domains = vec![("funcs.mox".to_string(), mox)];
    let imports = rex_driver::DomainImports {
        sigil: rex_driver::SigilImports::new().provide("funcs.mox", "funcs.rosetta", rosetta),
        ..rex_driver::DomainImports::default()
    };
    let source = concat!(
        "import \"funcs.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Underlier\n",
        "        search UnderlierSearch {\n",
        "            entity Underlier\n",
        "            document {\n",
        "                rate = calcRate(1.0, [\"a\"]);\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_ddd_str("design.ddd", source, &domains, &imports);
    assert_clean(&compilation);
    let document = &compilation.model.expect("model").modules[0].searches[0].document;
    assert_eq!(document[0].expr, "calcRate(1.0, [\"a\"])");

    // An unknown callee keeps the standard document-expression diagnostic
    // shape, wrapping the checker's unknown-function message.
    let source = source.replace("rate = calcRate(1.0, [\"a\"]);", "rate = unknownCall(1);");
    let compilation = compile_ddd_str("design.ddd", &source, &domains, &imports);
    let diagnostic = single(&compilation, "invalid document expression in field 'rate'");
    assert!(
        diagnostic.message.contains("unknown function"),
        "expected the unknown-function message, got: {}",
        diagnostic.message
    );
}

#[test]
fn search_document_field_names_are_unique() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            document {\n",
        "                blurb = title;\n",
        "                blurb = synopsis;\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    single(&compilation, "duplicate document field 'blurb'");
}

#[test]
fn search_pagination_bounds_are_validated() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            pagination {\n",
        "                limit 0\n",
        "                max 50\n",
        "            }\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    single(&compilation, "pagination limit must be at least 1");

    let source = source.replace("limit 0", "limit 100");
    let compilation = compile_search(&source);
    single(&compilation, "pagination limit 100 exceeds the max 50");
}

#[test]
fn service_delegations_to_searches_follow_the_coupling_rule() {
    // Same module: legal.
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch { entity Movie }\n",
        "        service S {\n",
        "            find => MovieSearch.search;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    assert_clean(&compilation);

    // Cross-module: the coupling error.
    let source = source.replace(
        "        service S {\n            find => MovieSearch.search;\n        }\n    }",
        "    }\n    module n {\n        service S {\n            find => MovieSearch.search;\n        }\n    }",
    );
    let compilation = compile_search(&source);
    single(
        &compilation,
        "interaction between a Service in one Module and a Search in another Module \
         is not allowed; go via a Service",
    );

    // The virtual operation is named `search`.
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch { entity Movie }\n",
        "        service S {\n",
        "            find => MovieSearch.bogus;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_search(source);
    single(
        &compilation,
        "search 'MovieSearch' exposes only the virtual operation 'search'",
    );
}

#[test]
fn search_capabilities_validate_against_the_actor_model() {
    let source = concat!(
        "import \"search.mox\"\n",
        "\n",
        "application A {\n",
        "    module m {\n",
        "        entity Movie\n",
        "        search MovieSearch {\n",
        "            entity Movie\n",
        "            capability SearchMovies\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &[("search.mox".to_string(), SEARCH_DOMAIN.to_string())],
        &rex_driver::DomainImports::default(),
        &[actors_with(&["SearchMovies"])],
    );
    assert_clean(&compilation);

    let compilation = compile_ddd_str_with_actors(
        "design.ddd",
        source,
        &[("search.mox".to_string(), SEARCH_DOMAIN.to_string())],
        &rex_driver::DomainImports::default(),
        &[actors_with(&["Unrelated"])],
    );
    single(
        &compilation,
        "unknown capability 'SearchMovies' on search 'MovieSearch'",
    );
}
