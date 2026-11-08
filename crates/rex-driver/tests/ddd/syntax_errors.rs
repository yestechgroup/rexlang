//! `ddd` tests: syntax errors.

use crate::support::*;

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
