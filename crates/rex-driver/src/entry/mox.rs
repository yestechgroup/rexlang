//! The `.mox` entry points: single-file and multi-file compilation of
//! domain models to the Core IR.

use salsa::Database as Db;

use crate::compilation::{Compilation, Compiled, MultiCompilation};
use crate::imports::DomainImports;
use crate::lower;
use crate::session::{parse_query, Database, ParseOutput, SourceFile};
use crate::Diagnostic;

/// Compiles a [`SourceFile`] to the Core IR (memoized by salsa):
/// parse → resolve/validate → lower. The tracked surface compiles with an
/// empty import bundle — every import declaration is a not-provided error;
/// the one-shot [`compile_str`] carries caller-provided content.
#[salsa::tracked]
pub fn compile(db: &dyn Db, file: SourceFile) -> Compiled {
    compile_file(db, file, &DomainImports::default())
}

/// [`compile`] with explicit import content — the salsa-tracked query the
/// LSP and other hosts use after gathering imports with
/// [`workspace_imports`](crate::workspace_imports). Imports enter the memo key, so an edit to the
/// document recompiles; import *content* changes are the host's
/// responsibility to re-drive.
pub fn compile_with_imports(db: &dyn Db, file: SourceFile, imports: &DomainImports) -> Compiled {
    compile_file(db, file, imports)
}

/// The body of [`compile`], parameterized over the provided import content.
///
/// Imports enter the salsa layer through this one borrowed bundle: the
/// tracked queries compile with an empty bundle (their only import shape),
/// and the one-shot entry points own a caller-provided bundle on the stack
/// and pass it straight through. The next import kind joins
/// [`DomainImports`] — no signature here changes.
fn compile_file(db: &dyn Db, file: SourceFile, imports: &DomainImports) -> Compiled {
    let source = file.text(db);
    let parsed = parse_query(db, file);
    let mut diagnostics = parsed.diagnostics;
    let lowered = parsed
        .ast
        .as_ref()
        .map(|ast| lower::compile(&file.path(db), &source, ast, imports));
    let sigil_diagnostics = lowered
        .as_ref()
        .map(|(_, _, sigil)| sigil.clone())
        .unwrap_or_default();
    if let Some((_, semantic, _)) = &lowered {
        diagnostics.extend(semantic.iter().cloned());
    }
    let blocked = diagnostics.iter().any(Diagnostic::is_error)
        || sigil_diagnostics.iter().any(|(_, d)| d.is_error());
    let model = if blocked {
        None
    } else {
        lowered.and_then(|(model, _, _)| model)
    };
    Compiled {
        model,
        diagnostics,
        sigil_diagnostics,
    }
}
/// Compiles several `.mox` files — one package per file — to a single
/// [`rex_ir::Model`] (memoized by salsa): every file lowers against the
/// union namespace of all packages, so declarations may reference types
/// across packages. `first` is the compilation's anchor file; `rest` holds
/// the remaining files in input order.
#[salsa::tracked]
pub(crate) fn compile_multi(
    db: &dyn Db,
    first: SourceFile,
    rest: Vec<SourceFile>,
) -> MultiCompilation {
    compile_multi_files(db, first, rest, &DomainImports::default())
}

/// The body of [`compile_multi`], parameterized over the provided
/// import content (see [`compile_file`] for the shape).
fn compile_multi_files(
    db: &dyn Db,
    first: SourceFile,
    rest: Vec<SourceFile>,
    imports: &DomainImports,
) -> MultiCompilation {
    let mut files: Vec<SourceFile> = vec![first];
    files.extend(rest);
    let mut paths: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut parse_outputs: Vec<ParseOutput> = Vec::new();
    for file in &files {
        paths.push(file.path(db));
        sources.push(file.text(db));
        parse_outputs.push(parse_query(db, *file));
    }
    let units: Vec<lower::MultiFile<'_>> = files
        .iter()
        .enumerate()
        .map(|(index, _)| lower::MultiFile {
            path: &paths[index],
            source: &sources[index],
            ast: parse_outputs[index].ast.as_ref(),
            parse_diagnostics: &parse_outputs[index].diagnostics,
        })
        .collect();
    let (model, diagnostics, sigil_diagnostics) = lower::compile_multi(&units, imports);
    MultiCompilation {
        model,
        diagnostics,
        sigil_diagnostics,
    }
}

/// Compiles multiple in-memory sources — one package per file — in one call.
///
/// The files' `import schema` and `import sigil` declarations resolve
/// through `imports`: every declared import must have an entry for its
/// `(mox path, import path)` pair or the compilation errors. The empty
/// bundle ([`DomainImports::default`]) is the no-import compilation.
///
/// ```
/// let files = [
///     "package a\n\nclass Book { String title }".to_string(),
///     "package b\n\nclass Shelf { refers a.Book[] links }".to_string(),
/// ];
/// let paths: Vec<(String, String)> = files
///     .into_iter()
///     .enumerate()
///     .map(|(index, source)| (format!("{index}.mox"), source))
///     .collect();
/// let compilation = rex_driver::compile_files(&paths, &rex_driver::DomainImports::default());
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().packages.len(), 2);
/// ```
pub fn compile_files(files: &[(String, String)], imports: &DomainImports) -> MultiCompilation {
    let db = Database::new();
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|(path, source)| SourceFile::new(&db, path.clone(), source.clone()))
        .collect();
    let Some(first) = sources.first().copied() else {
        return MultiCompilation::default();
    };
    compile_multi_files(&db, first, sources[1..].to_vec(), imports)
}
/// Compiles in-memory source text in one call.
///
/// The source's `import schema` and `import sigil` declarations resolve
/// through `imports`. The empty bundle ([`DomainImports::default`]) is the
/// no-import compilation.
///
/// ```
/// let compilation =
///     rex_driver::compile_str("m.mox", "package demo", &rex_driver::DomainImports::default());
/// assert!(compilation.model.is_some());
/// ```
pub fn compile_str(path: &str, source: &str, imports: &DomainImports) -> Compilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let compiled = compile_file(&db, file, imports);
    Compilation {
        model: compiled.model,
        diagnostics: compiled.diagnostics,
        sigil_diagnostics: compiled.sigil_diagnostics,
    }
}
