//! `import schema` declarations: the nominal, feature-less class a
//! schema import contributes to the package namespace (v1 is opaque —
//! JSON content is validated, never lowered into features).

use std::collections::HashMap;
use std::path::Path;

use rex_ir as ir;
use rex_syntax::ast as mox;

use crate::diagnostic::Diagnostic;

use super::*;
/// The name an `import schema` declaration joins its package's namespace
/// as: the `as` alias, or the import path's file stem when the alias is
/// absent (`"deep/dir/todo_item.json"` imports as `todo_item`).
pub(crate) fn imported_name(decl: &mox::ImportSchemaDecl) -> String {
    if let Some(alias) = &decl.alias {
        return alias.text.clone();
    }
    Path::new(&decl.path)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// One validated `import schema` declaration: the name it joins the
/// namespace as (alias or file stem).
#[derive(Debug, Clone)]
pub(crate) struct SchemaImport {
    /// The namespace name (alias, or the path's file stem).
    pub name: String,
}

/// `true` for a valid `.mox` identifier shape (`[A-Za-z_][A-Za-z0-9_]*`),
/// which a stem-derived import name must have to be referencable.
pub(crate) fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Collects and validates the `import schema` declarations of one file.
///
/// v1 lowering is deliberately **opaque**: an import registers a nominal,
/// feature-less class — no features, enums, or datatypes are derived from
/// the JSON content (structural lowering is future work). The content is
/// still validated: it must be provided (the driver is filesystem-free —
/// the host passes JSON text through [`crate::SchemaImports`]) and it must
/// parse as JSON.
///
/// Collisions are errors naming both sources: an alias (or stem) that
/// matches a declaration of the same package, the same name imported twice,
/// and two stem-derived names that collide. Validated imports are returned
/// in source order; collided ones are skipped so the namespace stays clean
/// for the remaining diagnostics.
pub(crate) fn collect_schema_imports(
    file_path: &str,
    ast: &mox::Model,
    provided: &crate::SchemaImports,
    declared: &HashMap<String, TopKind>,
    diags: &mut Vec<Diagnostic>,
) -> Vec<SchemaImport> {
    let mut imports: Vec<SchemaImport> = Vec::new();
    let mut seen: HashMap<String, String> = HashMap::new();
    for decl in ast.declarations.iter() {
        let mox::Decl::ImportSchema(decl) = decl else {
            continue;
        };
        // `import sigil` declarations are lowered by the sigil module.
        if decl.kind == mox::ImportKind::Sigil {
            continue;
        }
        let name = imported_name(decl);
        let span = decl.span;
        if let Some(kind) = declared.get(&name).copied() {
            diags.push(
                Diagnostic::error(
                    format!(
                        "import schema '{}' declares '{}', which is already declared in this \
                         package as a {}",
                        decl.path,
                        name,
                        kind.label()
                    ),
                    Some(span),
                )
                .with_help(format!(
                    "rename the import: `import schema \"{}\" as <OtherName>`",
                    decl.path
                )),
            );
            continue;
        }
        if let Some(previous) = seen.get(&name) {
            diags.push(
                Diagnostic::error(
                    format!(
                        "duplicate import name '{name}': imported from '{previous}' and '{}'",
                        decl.path
                    ),
                    Some(span),
                )
                .with_help("import each schema under a distinct `as` name"),
            );
            continue;
        }
        if name.is_empty() {
            diags.push(Diagnostic::error(
                format!(
                    "import schema '{}' has no file stem to name the import",
                    decl.path
                ),
                Some(span),
            ));
            continue;
        }
        if decl.alias.is_none() && !is_identifier(&name) {
            diags.push(
                Diagnostic::error(
                    format!("imported schema name '{name}' is not a valid identifier"),
                    Some(span),
                )
                .with_help(format!(
                    "add an alias: `import schema \"{}\" as <Name>`",
                    decl.path
                )),
            );
            continue;
        }
        match provided.get(file_path, &decl.path) {
            None => {
                diags.push(
                    Diagnostic::error(
                        format!("imported schema '{}' was not provided", decl.path),
                        Some(span),
                    )
                    .with_help(
                        "import schema content is provided by the host: the CLI reads the \
                         JSON next to the model; embedded callers pass it through \
                         SchemaImports",
                    ),
                );
                continue;
            }
            Some(json) => {
                if let Err(error) = serde_json::from_str::<serde_json::Value>(json) {
                    diags.push(Diagnostic::error(
                        format!("imported schema '{}' is not valid JSON: {error}", decl.path),
                        Some(span),
                    ));
                    continue;
                }
            }
        }
        seen.insert(name.clone(), decl.path.clone());
        imports.push(SchemaImport { name });
    }
    imports
}

/// The nominal IR class one validated import lowers into: a feature-less,
/// empty-extends `ClassDef` (v1 is opaque; structural lowering is future
/// work).
pub(crate) fn import_class_def(import: &SchemaImport) -> ir::ClassDef {
    ir::ClassDef::new(import.name.clone(), Vec::new(), Vec::new())
}
