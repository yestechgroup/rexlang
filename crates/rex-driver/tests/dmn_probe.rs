use rex_syntax::{format_dmn, parse_dmn};
use std::path::Path;

#[test]
fn normalize() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    for path in [
        "tests/conformance/dmn/kiwisaver.dmn",
        "tests/conformance/dmn/minimal.dmn",
        "examples/lending.dmn",
        "examples/support.dmn",
    ] {
        let absolute = root.join(path);
        let source = std::fs::read_to_string(&absolute).unwrap();
        assert!(parse_dmn(&source).errors.is_empty(), "{path} must parse");
        let formatted = format_dmn(&source).expect("fmt");
        std::fs::write(&absolute, &formatted).unwrap();
        let again = format_dmn(&formatted).expect("fmt twice");
        assert_eq!(formatted, again, "{path} fmt fixpoint");
        assert!(
            parse_dmn(&formatted).errors.is_empty(),
            "{path} formatted must parse"
        );
        eprintln!("RESULT normalized {path}");
    }
}
