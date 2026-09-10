//! Tests for the comment-preserving formatter (`rex_syntax::fmt`).
//!
//! The formatter is token-stream based: it never parses, so sources with
//! syntax errors still format, but sources with *lex* errors are rejected.

use rex_syntax::fmt::{format, FormatError};

fn fmt(source: &str) -> String {
    format(source).expect("format succeeds")
}

// --- basic layout ------------------------------------------------------------

#[test]
fn empty_input_formats_to_empty_output() {
    assert_eq!(fmt(""), "");
    assert_eq!(fmt("   \n\t\n  "), "");
}

#[test]
fn normalizes_spacing_one_decl_per_line() {
    assert_eq!(
        fmt("class   Book  { int   pages }"),
        "class Book {\n    int pages\n}\n"
    );
}

#[test]
fn raw_op_body_braces_nest_on_one_line() {
    // The raw `{ ... }` body of an op is not grammar; its balanced braces
    // stay inline on the feature's line.
    assert_eq!(
        fmt("class S { op int f() { if x { y } } }"),
        "class S {\n    op int f() { if x { y } }\n}\n"
    );
}

#[test]
fn empty_class_stays_inline() {
    assert_eq!(fmt("class Empty { }"), "class Empty {}\n");
    assert_eq!(fmt("class Empty {}"), "class Empty {}\n");
    assert_eq!(fmt("class Empty {}"), fmt("class Empty {}"));
}

#[test]
fn empty_vocabulary_stays_inline() {
    assert_eq!(
        fmt("vocabulary V from \"u:v\" {}"),
        "vocabulary V from \"u:v\" {}\n"
    );
}

// --- spacing details ---------------------------------------------------------

#[test]
fn qualified_names_have_no_spaces_around_dots() {
    assert_eq!(
        fmt("class B { contains nz . example . Book [ ] b }"),
        "class B {\n    contains nz.example.Book[] b\n}\n"
    );
}

#[test]
fn extends_list_spacing() {
    assert_eq!(
        fmt("class A extends B , C { }"),
        "class A extends B, C {}\n"
    );
}

#[test]
fn op_params_spacing() {
    assert_eq!(
        fmt("class S { op Book get( String title,int pages ) }"),
        "class S {\n    op Book get(String title, int pages)\n}\n"
    );
}

#[test]
fn default_value_spacing() {
    assert_eq!(
        fmt("class B { String name=\"x\" int n = 3 }"),
        "class B {\n    String name = \"x\"\n    int n = 3\n}\n"
    );
}

#[test]
fn default_value_may_be_a_name() {
    assert_eq!(
        fmt("class B { String status = active }"),
        "class B {\n    String status = active\n}\n"
    );
}

// --- multiplicities ----------------------------------------------------------

#[test]
fn multiplicity_zero_to_star_normalizes_to_empty_brackets() {
    assert_eq!(
        fmt("class L { contains Book [0..*] books }"),
        "class L {\n    contains Book[] books\n}\n"
    );
    assert_eq!(
        fmt("class L { contains Book [ 0 .. * ] books }"),
        "class L {\n    contains Book[] books\n}\n"
    );
}

#[test]
fn other_multiplicities_are_preserved() {
    for mult in ["[3]", "[0..1]", "[1..*]", "[3..5]"] {
        let source = format!("class L {{ contains Book {mult} books }}");
        assert_eq!(
            fmt(&source),
            format!("class L {{\n    contains Book{mult} books\n}}\n"),
            "multiplicity {mult} must be preserved"
        );
    }
}

// --- modifiers ---------------------------------------------------------------

#[test]
fn modifiers_normalize_to_id_first() {
    assert_eq!(
        fmt("class B { readonly id String x }"),
        "class B {\n    id readonly String x\n}\n"
    );
    assert_eq!(
        fmt("class B { readonly readonly String y id  id String x }"),
        "class B {\n    readonly String y\n    id String x\n}\n"
    );
}

#[test]
fn all_feature_kinds_format() {
    let source = concat!(
        "class B {\n",
        "  contains Item[] parts opposite owner\n",
        "  refers Tag[0..1] tag opposite thing\n",
        "  container Box box opposite content\n",
        "  op int size(String unit)\n",
        "  derived String label\n",
        "  String name\n",
        "}\n"
    );
    let expected = concat!(
        "class B {\n",
        "    contains Item[] parts opposite owner\n",
        "    refers Tag[0..1] tag opposite thing\n",
        "    container Box box opposite content\n",
        "    op int size(String unit)\n",
        "    derived String label\n",
        "    String name\n",
        "}\n"
    );
    assert_eq!(fmt(source), expected);
}

#[test]
fn escaped_identifiers_are_preserved_verbatim() {
    assert_eq!(
        fmt("class ^class { String ^id }"),
        "class ^class {\n    String ^id\n}\n"
    );
}

// --- enums -------------------------------------------------------------------

#[test]
fn enum_literals_normalize_and_keep_optional_parts_only_if_present() {
    assert_eq!(
        fmt("enum E { A as \"a\" = 1   B   C as \"c\" }"),
        "enum E {\n    A as \"a\" = 1\n    B\n    C as \"c\"\n}\n"
    );
}

// --- type / interface / annotation / package ---------------------------------

#[test]
fn datatype_with_bindings_body() {
    assert_eq!(
        fmt("type Date wraps opaque { rust   \"chrono::NaiveDate\" csharp \"System.DateOnly\" }"),
        concat!(
            "type Date wraps opaque {\n",
            "    rust \"chrono::NaiveDate\"\n",
            "    csharp \"System.DateOnly\"\n",
            "}\n"
        )
    );
}

#[test]
fn datatype_wrapping_a_named_type_has_no_body() {
    assert_eq!(fmt("type Foo wraps a.Bar"), "type Foo wraps a.Bar\n");
}

#[test]
fn interface_bindings_one_per_line() {
    assert_eq!(
        fmt("interface I { rust \"x\" java \"y\" }"),
        "interface I {\n    rust \"x\"\n    java \"y\"\n}\n"
    );
}

#[test]
fn annotation_decl() {
    assert_eq!(
        fmt("annotation \"deprecated\" as Old"),
        "annotation \"deprecated\" as Old\n"
    );
    assert_eq!(fmt("annotation \"x\""), "annotation \"x\"\n");
}

// --- vocabulary --------------------------------------------------------------

#[test]
fn vocabulary_body_items_one_per_line() {
    assert_eq!(
        fmt(concat!(
            "vocabulary  Currency  from  \"iso:4217\" { ",
            "version \"2024-01-01\" key alpha3 facet String symbol facet int minorUnits }"
        )),
        concat!(
            "vocabulary Currency from \"iso:4217\" {\n",
            "    version \"2024-01-01\"\n",
            "    key alpha3\n",
            "    facet String symbol\n",
            "    facet int minorUnits\n",
            "}\n"
        )
    );
}

// --- blank lines -------------------------------------------------------------

#[test]
fn blank_lines_collapse_between_top_level_decls() {
    assert_eq!(
        fmt("package a\n\n\n\nclass B { int x }\n\n\nclass C {}\n\n\n"),
        "package a\n\nclass B {\n    int x\n}\n\nclass C {}\n"
    );
}

#[test]
fn no_blank_line_at_document_start_and_single_newline_at_eof() {
    assert_eq!(fmt("\n\npackage a\n"), "package a\n");
}

#[test]
fn blank_lines_inside_bodies_are_removed() {
    assert_eq!(
        fmt("class B {\n    int x\n\n\n    int y\n\n}"),
        "class B {\n    int x\n    int y\n}\n"
    );
}

// --- comments ----------------------------------------------------------------

#[test]
fn own_line_comment_attaches_to_following_line_indent() {
    assert_eq!(
        fmt("class B {\n  // the page count\n  int   pages\n}"),
        "class B {\n    // the page count\n    int pages\n}\n"
    );
}

#[test]
fn trailing_comment_stays_on_the_same_line() {
    assert_eq!(
        fmt("class B {\n    int pages // inline note\n}"),
        "class B {\n    int pages // inline note\n}\n"
    );
}

#[test]
fn consecutive_own_line_comments_form_a_block() {
    assert_eq!(
        fmt("class B {\n    // one\n\n\n    // two\n    int x\n}"),
        "class B {\n    // one\n    // two\n    int x\n}\n"
    );
}

#[test]
fn multiline_block_comment_keeps_internal_newlines_and_indents_first_line_only() {
    let source = "class B {\n    /* spans\n   several\n lines */\n    int x\n}";
    assert_eq!(
        fmt(source),
        "class B {\n    /* spans\n   several\n lines */\n    int x\n}\n"
    );
}

#[test]
fn inline_block_comment_stays_inline() {
    assert_eq!(
        fmt("class B { int /* count */ pages }"),
        "class B {\n    int /* count */ pages\n}\n"
    );
}

#[test]
fn own_line_block_comment_gets_its_own_line() {
    assert_eq!(
        fmt("class B {\n    /* note */\n    int x\n}"),
        "class B {\n    /* note */\n    int x\n}\n"
    );
}

#[test]
fn comment_content_is_preserved_verbatim() {
    assert_eq!(
        fmt("class B {\n    //   spaced    out   *\n    int x\n}"),
        "class B {\n    //   spaced    out   *\n    int x\n}\n"
    );
}

#[test]
fn comment_block_before_a_top_level_decl_sits_after_the_blank_line() {
    assert_eq!(
        fmt("package a\n\n// Books are things\nclass B { int x }"),
        "package a\n\n// Books are things\nclass B {\n    int x\n}\n"
    );
}

#[test]
fn comment_at_document_start_gets_no_leading_blank() {
    assert_eq!(fmt("// header\npackage a\n"), "// header\npackage a\n");
}

#[test]
fn comment_before_closing_brace_keeps_body_indent() {
    assert_eq!(
        fmt("class B {\n    int x\n    // end of class\n}"),
        "class B {\n    int x\n    // end of class\n}\n"
    );
}

#[test]
fn comment_at_eof_is_separated_by_a_blank_line() {
    assert_eq!(
        fmt("class B { int x }\n// bye"),
        "class B {\n    int x\n}\n\n// bye\n"
    );
}

#[test]
fn trailing_comment_after_closing_brace_line() {
    assert_eq!(
        fmt("class B { int x } // trailing\n"),
        "class B {\n    int x\n} // trailing\n"
    );
}

// --- robustness (token-based, no parsing) ------------------------------------

#[test]
fn parse_errors_still_format() {
    // `int` alone is not a valid feature (missing name) but lexes fine.
    assert_eq!(fmt("class B { int }"), "class B {\n    int\n}\n");
}

#[test]
fn lex_errors_are_reported() {
    // The `Other` catch-all makes almost everything lexable; the one true
    // lex error is an integer literal that overflows `i64`.
    let result = format("class B { int x = 99999999999999999999999 }");
    assert!(matches!(result, Err(FormatError::Lex(_))), "got {result:?}");
}

// --- idempotency -------------------------------------------------------------

fn assert_idempotent(source: &str) {
    let once = fmt(source);
    let twice = fmt(&once);
    assert_eq!(once, twice, "formatting must be a fixpoint for:\n{source}");
}

#[test]
fn idempotent_on_conformance_library() {
    assert_idempotent(include_str!(
        "../../../tests/conformance/models/library.mox"
    ));
}

#[test]
fn idempotent_on_conformance_currency() {
    assert_idempotent(include_str!(
        "../../../tests/conformance/models/currency.mox"
    ));
}

#[test]
fn idempotent_on_examples_library() {
    assert_idempotent(include_str!("../../../examples/library.mox"));
}

#[test]
fn idempotent_gnarly_weird_spacing() {
    assert_idempotent("package   a.b\n\n\nclass    Book{\n\nint    pages\n   String  title\n}\n\n\nclass  Empty{}\n");
}

#[test]
fn idempotent_gnarly_comments_everywhere() {
    assert_idempotent(concat!(
        "// doc header\n",
        "package a // trailing after package\n",
        "\n",
        "/* block\n   before class */\n",
        "class B { // after brace\n",
        "    // own line\n",
        "    /* inline */ int x // trailing\n",
        "    int y\n",
        "    // before close\n",
        "}\n",
        "// at eof\n"
    ));
}

#[test]
fn idempotent_gnarly_all_features_with_modifiers() {
    assert_idempotent(concat!(
        "class B {\n",
        "  readonly id String a = \"x\"\n",
        "  id contains Item[0..*] parts opposite owner\n",
        "  readonly refers Tag[0..1] tag\n",
        "  container Box box opposite content\n",
        "  id readonly op int size(String u, int n) { return n }\n",
        "  derived String label\n",
        "  id int n = 3\n",
        "}\n"
    ));
}

#[test]
fn idempotent_gnarly_vocabulary_with_facets() {
    assert_idempotent(concat!(
        "vocabulary Currency from \"iso:4217\" {\n",
        "  version   \"2024-01-01\"\n",
        "  key   alpha3\n",
        "  facet String symbol\n",
        "  facet int minorUnits\n",
        "}\n"
    ));
}

#[test]
fn idempotent_gnarly_enum_with_labels_and_values() {
    assert_idempotent("enum E {  A as \"a\" =  1 B as \"b\" C = 2 D }\n");
}

#[test]
fn idempotent_gnarly_multiplicity_shapes() {
    assert_idempotent(concat!(
        "class L {\n",
        "  contains Book[0..*] a\n",
        "  contains Book [3] b\n",
        "  refers W [0..1] c\n",
        "  refers W [1..*] d\n",
        "  refers W [3..5] e\n",
        "}\n"
    ));
}

#[test]
fn idempotent_gnarly_multiline_block_comment() {
    assert_idempotent(concat!(
        "class L {\n",
        "  /* first\n",
        "     second\n",
        "   third */\n",
        "  int x\n",
        "}\n"
    ));
}

#[test]
fn idempotent_gnarly_blank_line_chaos() {
    assert_idempotent("\n\n\npackage a\n\n\n\n\nannotation \"x\"\n\n\n\nclass B { }\n\n\n\n");
}

#[test]
fn idempotent_gnarly_qualified_names_and_escapes() {
    assert_idempotent("class B {\n  contains nz.example.deep . Book [] b\n  String ^type\n}\n");
}

#[test]
fn idempotent_gnarly_extends_and_type_decls() {
    assert_idempotent(concat!(
        "class A extends B , nz.example.C {}\n",
        "type D wraps opaque {\n",
        "  rust \"a::B\"\n",
        "}\n",
        "type E wraps f.G\n"
    ));
}

// --- Tier 1: target-tagged op bodies and datatype create/convert blocks ------
//
// Body formatting rule (see the module docs in src/fmt.rs): the op's `{`
// closes its line; each `<target> {` sits at the next indent. A single-line
// body renders inline; a multi-line body's inner bytes are kept VERBATIM
// (only ends trimmed) between the `<target> {` line and a `}` at the
// `<target>` indent. Verbatim bytes make re-formatting a fixpoint.

#[test]
fn op_single_line_target_body_renders_inline() {
    let formatted = fmt("class S { op Book get(String t) { rust { find(t) } } }");
    assert_eq!(
        formatted,
        "class S {\n    op Book get(String t) {\n        rust { find(t) }\n    }\n}\n"
    );
    assert_eq!(fmt(&formatted), formatted, "not a fixpoint");
}

#[test]
fn op_multiline_target_body_keeps_bytes_verbatim() {
    let source = "class S {\n\
                  \x20   op Book get(String t) {\n\
                  \x20       rust {\n\
                  \x20           res.books.iter().find_map(|b| {\n\
                  \x20               (b.title == t).then_some(*b)\n\
                  \x20           })\n\
                  \x20       }\n\
                  \x20   }\n\
                  }";
    let formatted = fmt(source);
    assert_eq!(
        formatted,
        "class S {\n\
         \x20   op Book get(String t) {\n\
         \x20       rust {\n\
         \x20           res.books.iter().find_map(|b| {\n\
         \x20               (b.title == t).then_some(*b)\n\
         \x20           })\n\
         \x20       }\n\
         \x20   }\n\
         }\n"
    );
    assert_eq!(fmt(&formatted), formatted, "not a fixpoint");
}

#[test]
fn op_multiple_target_bodies_each_get_their_own_block() {
    let source = "class S { op Book get(String t) { rust { find(t) } java { jfind(t) } } }";
    let formatted = fmt(source);
    assert_eq!(
        formatted,
        "class S {\n\
         \x20   op Book get(String t) {\n\
         \x20       rust { find(t) }\n\
         \x20       java { jfind(t) }\n\
         \x20   }\n\
         }\n"
    );
    assert_eq!(fmt(&formatted), formatted, "not a fixpoint");
}

#[test]
fn bare_op_body_still_renders_on_one_line() {
    // Bare (untagged) bodies are a driver error, but formatting must be
    // stable regardless.
    let formatted = fmt("class S { op int f() { if x { y } } }");
    assert_eq!(formatted, "class S {\n    op int f() { if x { y } }\n}\n");
    assert_eq!(fmt(&formatted), formatted);
}

#[test]
fn datatype_create_convert_blocks_render_and_fixpoint() {
    let source = "type D wraps opaque {\n\
                  \x20   rust \"a::D\"\n\
                  \x20   create { rust { D(it) } csharp { new D(it) } }\n\
                  \x20   convert { rust { self.0.clone() } }\n\
                  }";
    let formatted = fmt(source);
    assert_eq!(
        formatted,
        "type D wraps opaque {\n\
         \x20   rust \"a::D\"\n\
         \x20   create {\n\
         \x20       rust { D(it) }\n\
         \x20       csharp { new D(it) }\n\
         \x20   }\n\
         \x20   convert {\n\
         \x20       rust { self.0.clone() }\n\
         \x20   }\n\
         }\n"
    );
    assert_eq!(fmt(&formatted), formatted, "not a fixpoint");
}

#[test]
fn idempotent_gnarly_target_bodies_with_odd_indentation() {
    // The body bytes (including their original indentation) survive
    // verbatim; the surrounding scaffolding is normalized once and then
    // never changes again.
    let source = "class S {\n\
                  \x20 op Book get(String t) { rust {\n\
                  \x20     if x {\n\
                  \x20   y\n\
                  \x20     }\n\
                  \x20 } }\n\
                  }";
    let formatted = fmt(source);
    assert_eq!(fmt(&formatted), formatted, "not a fixpoint");
    assert!(
        formatted.contains("      if x {\n    y\n      }\n"),
        "body bytes must survive verbatim (only ends trimmed):\n{formatted}"
    );
}

// --- stability: output parses when the input did -----------------------------

fn assert_still_parses(source: &str) {
    let before = rex_syntax::parse(source);
    let output = fmt(source);
    let after = rex_syntax::parse(&output);
    assert!(
        after.errors.is_empty(),
        "formatted output of a parseable input must parse cleanly, got {:?}\n--- formatted ---\n{output}",
        after.errors
    );
    // The declarational facts must survive: format(parse(x)) is a fixpoint.
    assert_eq!(fmt(&output), output, "formatted output is not a fixpoint");
    let _ = before; // `source` is parseable by construction in callers
}

#[test]
fn stable_on_conformance_models() {
    assert_still_parses(include_str!(
        "../../../tests/conformance/models/library.mox"
    ));
    assert_still_parses(include_str!(
        "../../../tests/conformance/models/currency.mox"
    ));
}

#[test]
fn stable_on_inline_models() {
    assert_still_parses("package a.b\nclass Book { int pages }");
    assert_still_parses("class B { readonly id String a contains Item[] items opposite owner }");
    assert_still_parses("enum E { A as \"a\" = 1 B }");
    assert_still_parses("vocabulary V from \"u:v\" { version \"1\" key k facet int f }");
    assert_still_parses("type D wraps opaque { rust \"a::B\" }");
    assert_still_parses("interface I { rust \"x\" }");
    assert_still_parses("annotation \"dep\" as Old");
    assert_still_parses("class A extends B, C { op int get(String k) }");
}

// --- golden fixtures ---------------------------------------------------------

const GOLDENS: &[(&str, &str)] = &[
    (
        "tests/conformance/models/library.mox",
        "tests/conformance/fmt/library.fmt.mox",
    ),
    (
        "tests/conformance/models/currency.mox",
        "tests/conformance/fmt/currency.fmt.mox",
    ),
];

/// Workspace root (two levels above the rex-syntax crate).
fn workspace_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn conformance_models_match_golden_formatting() {
    for (model, golden) in GOLDENS {
        let formatted = fmt(&model_source(model));
        let golden_path = workspace_root().join(golden);
        if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
            std::fs::create_dir_all(golden_path.parent().unwrap()).unwrap();
            std::fs::write(&golden_path, &formatted).unwrap();
            eprintln!("updated {}", golden_path.display());
            continue;
        }
        let expected = std::fs::read_to_string(&golden_path).unwrap_or_else(|_| {
            panic!(
                "missing golden formatting {}; regenerate with REX_UPDATE_FIXTURES=1",
                golden_path.display()
            )
        });
        assert_eq!(
            formatted, expected,
            "formatting of {model} drifted from the golden file"
        );
    }
}

/// Reads a workspace-relative fixture path at runtime.
fn model_source(relative: &str) -> String {
    std::fs::read_to_string(workspace_root().join(relative)).expect("read conformance model")
}
