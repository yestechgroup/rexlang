//! Property-based contract for Tier 1 operation bodies: an operation body is
//! an arbitrary (balanced-brace, unicode-hostile) verbatim string. The body
//! must survive the IR JSON round trip and reach the generated Rust code
//! BYTE-IDENTICALLY — the codegen embeds the string as-is, in-process on
//! `generate()` output (no scratch compile needed here; compilation and
//! behavior are proven by `codegen_end_to_end.rs`).

use proptest::prelude::*;

use rex_ir::{
    ClassDef, DatatypeDef, Feature, FeatureKind, Model, Operation, OperationParam, Package,
    PrimitiveType, TypeRef,
};

const PKG: &str = "nz.example.bodies";

/// Balanced-brace text: atoms carry hostile content (unicode, `=>`, comments
/// and string literals containing braces, newlines) but no raw braces; the
/// recursive level wraps inner text in balanced `{ ... }` groups, so every
/// generated body is embeddable between an opening and closing brace.
fn balanced_text() -> impl Strategy<Value = String> {
    let atom = prop::sample::select(vec![
        "".to_string(),
        "x => y ? a : b".to_string(),
        "λf.(λx.f(x)) üñí ©ødè".to_string(),
        "// comment with a brace: } and {".to_string(),
        "let s = \"}\"){(\";".to_string(),
        "line1\nline2\twith\r\nbreaks".to_string(),
        "emoji \u{1F600} escaped \"quotes\" \\backslash".to_string(),
        "a; b; c".to_string(),
    ]);
    atom.prop_recursive(4, 64, 8, |inner| {
        (inner.clone(), inner.clone(), inner.clone(), inner).prop_map(
            |(pre, first, second, post)| format!("{pre} {{ {first} }} mid {{ {second} }} {post}"),
        )
    })
}

fn body_model(rust_body: String, create: String, convert: String) -> Model {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.datatypes.push(
        DatatypeDef::new("Date", None)
            .with_body("create", "rust", create)
            .with_body("convert", "rust", convert),
    );
    let mut class = ClassDef::new(
        "Library",
        vec![],
        vec![Feature::new(
            "name",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            rex_ir::Multiplicity::REQUIRED,
        )],
    );
    class.operations.push(
        Operation::new(
            "getBook",
            TypeRef::Class {
                package: PKG.to_string(),
                name: "Book".to_string(),
            },
            vec![OperationParam {
                name: "title".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::String),
            }],
        )
        .with_body("rust", rust_body),
    );
    package.classes.push(class);
    model.packages.push(package);
    model
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn bodies_round_trip_json_and_embed_byte_identically(
        rust_body in balanced_text(),
        create in balanced_text(),
        convert in balanced_text(),
    ) {
        let model = body_model(rust_body.clone(), create.clone(), convert.clone());

        // IR JSON round trip preserves bodies exactly.
        let json = model.to_json().expect("serialize");
        let parsed = Model::from_json(&json).expect("deserialize");
        prop_assert_eq!(&parsed, &model, "IR JSON round trip changed the model");

        // The generated code embeds each body byte-identically.
        let code = rex_backend_rust::generate(&parsed)
            .expect("generate")
            .remove("models.rs")
            .expect("models.rs");
        prop_assert!(
            code.contains(&rust_body),
            "operation body not embedded byte-identically:\nbody: {rust_body:?}\ncode:\n{code}"
        );
        prop_assert!(
            code.contains(&format!("pub fn create(it: String) -> Self {{\n{create}\n    }}")),
            "create body not embedded byte-identically:\nbody: {create:?}\ncode:\n{code}"
        );
        prop_assert!(
            code.contains(&format!("pub fn convert(self) -> String {{\n{convert}\n    }}")),
            "convert body not embedded byte-identically:\nbody: {convert:?}\ncode:\n{code}"
        );
    }
}
