//! Property tests for the `.ifml` formatter: generated sources must
//! survive the canonical formatting with their IR intact, and formatting
//! must be idempotent. Mirrors the `parse(print(e))` discipline of the
//! sigil pretty-printer: the formatter is whitespace/comment-preserving
//! re-emission, so the serialized artifact is the compared identity:
//! `to_json(format(x)) == to_json(x)` (parse spans differ by construction
//! and are skipped on serialize, so wire-equality is the invariant that
//! matters).

use proptest::prelude::*;

use rex_ifml::{format_ifml, parse_ifml};

const FIXTURE_SEEDS: &[&str] = &[
    include_str!("../../../tests/conformance/ifml/app.ifml"),
    include_str!("../../../patterns/ifml/navigation.ifml"),
    include_str!("../../../patterns/ifml/commerce.ifml"),
    include_str!("../../../patterns/ifml/overlays.ifml"),
];

// --- source model -------------------------------------------------------------

#[derive(Clone, Debug)]
enum Stmt {
    Property(String, String),
    Container(String, Vec<Stmt>),
    Component(String, Vec<Stmt>),
    Event(String),
    Use(String, Option<String>),
    Condition,
}

#[derive(Clone, Debug)]
enum Decl {
    View(String, Vec<Stmt>),
    Module(String, Vec<Stmt>),
    Actor(String, Vec<Stmt>),
    Action(String, Vec<Stmt>),
}

fn arb_name() -> impl Strategy<Value = String> {
    "[A-Z][a-z]{1,10}"
}

fn arb_key() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("type".to_string()),
        Just("data".to_string()),
        Just("label".to_string()),
        Just("filter".to_string()),
        Just("paginated".to_string()),
    ]
}

fn arb_value() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("list".to_string()),
        Just("Item".to_string()),
        Just("\"leaf\"".to_string()),
        Just("true".to_string()),
        Just("12".to_string()),
        Just("[a, b]".to_string()),
    ]
}

fn arb_property() -> impl Strategy<Value = Stmt> {
    (arb_key(), arb_value()).prop_map(|(key, value)| Stmt::Property(key, value))
}

/// Statements legal in a `view` (or container) body.
fn arb_view_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![
        arb_property(),
        (arb_name(), arb_name()).prop_map(|(target, alias)| Stmt::Use(target, Some(alias))),
        arb_name().prop_map(|target| Stmt::Use(target, None)),
        Just(Stmt::Condition),
        arb_name().prop_map(Stmt::Event),
    ]
}

/// Statements legal in a `module` body: containers, components, events,
/// uses — no bare conditions (grammar: module_declaration).
fn arb_module_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![
        arb_property(),
        (arb_name(), arb_name()).prop_map(|(target, alias)| Stmt::Use(target, Some(alias))),
        arb_name().prop_map(|target| Stmt::Use(target, None)),
        arb_name().prop_map(Stmt::Event),
    ]
}

/// Statements legal in an `action` body: properties and events.
fn arb_action_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![arb_property(), arb_name().prop_map(Stmt::Event),]
}

/// Statements legal in a `component` body — components are FLAT: no `use`,
/// no nested containers/components (grammar: component_body).
fn arb_component_stmt() -> impl Strategy<Value = Stmt> {
    prop_oneof![
        arb_property(),
        Just(Stmt::Condition),
        arb_name().prop_map(Stmt::Event),
    ]
}

fn arb_stmt() -> impl Strategy<Value = Stmt> {
    arb_view_stmt()
        .prop_recursive(3, 16, 4, |inner| {
            prop_oneof![
                (arb_name(), proptest::collection::vec(inner, 0..3))
                    .prop_map(|(name, body)| Stmt::Container(name, body)),
                (
                    arb_name(),
                    proptest::collection::vec(arb_component_stmt(), 0..4)
                )
                    .prop_map(|(name, body)| Stmt::Component(name, body)),
            ]
        })
        .boxed()
}

fn arb_decl() -> impl Strategy<Value = Decl> {
    prop_oneof![
        (arb_name(), proptest::collection::vec(arb_stmt(), 0..5))
            .prop_map(|(name, body)| Decl::View(name, body)),
        (
            arb_name(),
            proptest::collection::vec(arb_module_stmt(), 0..4)
        )
            .prop_map(|(name, body)| Decl::Module(name, body)),
        (arb_name(), proptest::collection::vec(arb_property(), 0..3))
            .prop_map(|(name, body)| Decl::Actor(name, body)),
        (
            arb_name(),
            proptest::collection::vec(arb_action_stmt(), 0..3)
        )
            .prop_map(|(name, body)| Decl::Action(name, body)),
    ]
}

fn arb_source() -> impl Strategy<Value = Vec<Decl>> {
    proptest::collection::vec(arb_decl(), 1..4)
}

// --- rendering ----------------------------------------------------------------

/// Renders a statement body with the grammar's ordering: properties first,
/// then every other statement kind (view/container/component bodies close
/// the property run once a nested declaration appears).
fn render_body(stmts: &[Stmt], indent: usize, out: &mut String) {
    for stmt in stmts
        .iter()
        .filter(|stmt| matches!(stmt, Stmt::Property(..)))
    {
        render_stmt(stmt, indent, out);
    }
    for stmt in stmts
        .iter()
        .filter(|stmt| !matches!(stmt, Stmt::Property(..)))
    {
        render_stmt(stmt, indent, out);
    }
}

fn render_stmt(stmt: &Stmt, indent: usize, out: &mut String) {
    let pad = "    ".repeat(indent);
    match stmt {
        Stmt::Property(key, value) => {
            out.push_str(&format!("{pad}{key}: {value};\n"));
        }
        Stmt::Container(name, body) | Stmt::Component(name, body) => {
            let keyword = match stmt {
                Stmt::Container(..) => "container",
                _ => "component",
            };
            out.push_str(&format!("{pad}{keyword} \"{name}\" {{\n"));
            render_body(body, indent + 1, out);
            out.push_str(&format!("{pad}}}\n"));
        }
        Stmt::Event(target) => {
            out.push_str(&format!(
                "{pad}on select(row) -> navigate(\"{target}\", {{\n{pad}    id: row.id\n{pad}}});\n"
            ));
        }
        Stmt::Use(target, alias) => match alias {
            Some(alias) => out.push_str(&format!(
                "{pad}use \"{target}\" as {alias} {{ item: 1; }};\n"
            )),
            None => out.push_str(&format!("{pad}use \"{target}\";\n")),
        },
        Stmt::Condition => out.push_str(&format!("{pad}if ready == true;\n")),
    }
}

fn render_decl(decl: &Decl, out: &mut String) {
    let (keyword, name, body) = match decl {
        Decl::View(name, body) => ("view", name, body),
        Decl::Module(name, body) => ("module", name, body),
        Decl::Actor(name, body) => ("actor", name, body),
        Decl::Action(name, body) => ("action", name, body),
    };
    out.push_str(&format!("{keyword} \"{name}\" {{\n"));
    if matches!(decl, Decl::Module(..)) {
        // Grammar-mandated: a module carries both parameter blocks.
        out.push_str("    input { page: Int = 1, size: Int }\n");
        out.push_str("    output { total: Int }\n");
    }
    render_body(body, 1, out);
    out.push_str("}\n");
}

/// Canonical rendering, then scrambling: whitespace and comments are
/// inserted only at boundaries the renderer controls, so the scrambled
/// source parses to the same IR by construction.
fn scramble(source: &str, rng: &mut proptest::test_runner::TestRng) -> String {
    let mut scrambled = String::new();
    for (index, line) in source.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        // Random blank lines before statements.
        if index > 0 && rng.random_bool(0.2) {
            scrambled.push('\n');
        }
        // Random comment lines.
        if rng.random_bool(0.1) {
            scrambled.push_str("// scattered\n");
        }
        // Random indentation widths (including tabs).
        let indent = line.len() - trimmed.len();
        let pad = if rng.random_bool(0.15) {
            "\t".repeat(indent / 4 + 1)
        } else {
            " ".repeat(indent + rng.random_range(0..=4usize))
        };
        scrambled.push_str(&pad);
        scrambled.push_str(trimmed);
        // Random trailing spaces after `{` and `;`.
        if (trimmed.ends_with('{') || trimmed.ends_with(';')) && rng.random_bool(0.25) {
            scrambled.push_str("  ");
        }
        scrambled.push('\n');
    }
    scrambled
}

// --- properties ---------------------------------------------------------------

fn assert_round_trip(source: &str) {
    let formatted = format_ifml(source);
    let before = parse_ifml(source).unwrap_or_else(|error| {
        panic!("generated source must parse: {error}\n--- source debug ---\n{source:?}")
    });
    let after = parse_ifml(&formatted)
        .unwrap_or_else(|error| panic!("formatted source must parse: {error}\n{formatted}"));
    let before = before
        .to_json_pretty()
        .expect("generated source serializes");
    let after = after.to_json_pretty().expect("formatted source serializes");
    assert_eq!(
        before, after,
        "formatting must preserve the wire artifact\n--- source ---\n{source}\n--- formatted ---\n{formatted}"
    );
    let again = format_ifml(&formatted);
    assert_eq!(
        again, formatted,
        "formatting must be idempotent\n--- first ---\n{formatted}\n--- second ---\n{again}"
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn formatting_preserves_the_ir_and_is_idempotent(
        decls in arb_source(),
        seed in prop::collection::vec(any::<u8>(), 32),
    ) {
        let mut canonical = String::new();
        for decl in &decls {
            render_decl(decl, &mut canonical);
        }
        let mut rng = proptest::test_runner::TestRng::from_seed(
            proptest::test_runner::RngAlgorithm::default(),
            &seed,
        );
        let scrambled = scramble(&canonical, &mut rng);
        assert_round_trip(&scrambled);
    }
}

#[test]
fn fixtures_round_trip_and_are_idempotent() {
    for fixture in FIXTURE_SEEDS {
        assert_round_trip(fixture);
    }
}
