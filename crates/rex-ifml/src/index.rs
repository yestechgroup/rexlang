//! The parse span index: a side-table of byte-offset sites (module
//! declarations, module uses, views, actions, actors) collected while
//! walking the same Pest parse tree the lowering walks.
//!
//! [`crate::parse_ifml`] discards spans; [`parse_ifml_indexed`] returns the
//! **same** [`IfmlModel`] plus an [`IfmlIndex`] so tooling (LSP navigation,
//! the resolver in [`crate::resolve`]) can reason about *where* things are
//! declared and used. The index is a plain in-memory struct — it is never
//! serialized and never part of the wire artifact, so artifacts stay
//! byte-identical.
//!
//! Layout: `index.files[0]` is the parsed file; every site carries the
//! `file` it belongs to (an index into `files`). Within a file, sites
//! appear in source order; files appear in compilation order (main file
//! first, then imports depth-first — see [`crate::resolve`]). A module use
//! resolved against a module declaration references it as
//! `(file index, decl index within that file)` via
//! [`ModuleUseSite::resolved_module`]; [`IfmlIndex::module_decl`] maps that
//! reference back to the site.

use rex_ir::ifml::IfmlModel;

use crate::parser::{parse_ifml_top_pair, parse_string, Rule};
use pest::iterators::Pair;

/// A file in the compilation: path and full source text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexFile {
    /// Path as the compilation keys it (the main file's `path` argument, or
    /// the import string for imported files). Empty for
    /// [`parse_ifml_indexed`], which receives bare text.
    pub path: String,
    /// The file's full source text; spans index into it.
    pub text: String,
}

/// One module input parameter as recorded by the index: name, declared
/// type, byte span of the `parameter_decl`, and whether a default literal
/// is present (the resolver's "requires input" check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexParam {
    /// Parameter name.
    pub name: String,
    /// Declared type as written (`String`, `Int`, `Uuid`, a custom
    /// identifier, ...).
    pub type_ref: String,
    /// Byte span of the whole `parameter_decl` in the file's text.
    pub span: (usize, usize),
    /// Whether the parameter declares a `= default` literal.
    pub has_default: bool,
}

/// The kind of literal an override value is (structurally): what the
/// resolver's trivial literal-vs-type sanity check consumes. Non-literal
/// values (identifiers, calls, arrays, objects) record `None`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiteralKind {
    /// A quoted string literal.
    Str,
    /// A numeric literal (optionally negated).
    Num,
    /// `true` / `false`.
    Bool,
}

/// One property override inside a `use` body, recorded for resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseOverride {
    /// Overridden property/input name.
    pub name: String,
    /// Byte span of the whole `property_assignment` in the file's text.
    pub span: (usize, usize),
    /// The literal kind of the assigned value, when it is a bare literal.
    pub literal: Option<LiteralKind>,
}

/// A `module "Name" { ... }` declaration site.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleDeclSite {
    /// File the module is declared in (index into [`IfmlIndex::files`]).
    pub file: usize,
    /// Module name (the unescaped `module "..."` string).
    pub name: String,
    /// Byte span of the `"..."` name token itself — the precise hover /
    /// selection range.
    pub name_span: (usize, usize),
    /// Byte span of the whole `module_declaration` in the file's text.
    pub span: (usize, usize),
    /// The module's `input` parameters, in source order.
    pub input_params: Vec<IndexParam>,
    /// Names of the module's own property assignments, in source order
    /// (override targets besides the input parameters).
    pub properties: Vec<String>,
}

/// A `use "Target" as alias { ... };` statement site, wherever it appears
/// (view body, container body, or module body).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModuleUseSite {
    /// File the use appears in (index into [`IfmlIndex::files`]).
    pub file: usize,
    /// The targeted module name (the `use "..."` string).
    pub target: String,
    /// Byte span of the `"..."` target token itself — the precise
    /// hover / selection range.
    pub name_span: (usize, usize),
    /// The `as` alias, when present.
    pub alias: Option<String>,
    /// Byte span of the whole `module_use_statement` in the file's text.
    pub span: (usize, usize),
    /// Property overrides inside the use body, in source order.
    pub overrides: Vec<UseOverride>,
    /// Resolution result set by the resolver
    /// ([`crate::compile_ifml_str`]): the referenced module declaration as
    /// `(file index, decl index within that file)`, or `None` while
    /// unresolved (unknown target, ambiguous target, or a use in an
    /// imported file, which the resolver does not validate).
    pub resolved_module: Option<(usize, usize)>,
}

/// A named top-level declaration site: a view, an action, or an actor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedSite {
    /// File the declaration appears in (index into [`IfmlIndex::files`]).
    pub file: usize,
    /// Declaration name (the unescaped `"..."` string).
    pub name: String,
    /// Byte span of the `"..."` name token itself — the precise hover /
    /// selection range.
    pub name_span: (usize, usize),
    /// Byte span of the whole declaration in the file's text.
    pub span: (usize, usize),
}

/// What [`IfmlIndex::at`] found at a byte offset: one of the site kinds
/// the index tracks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AtSite<'a> {
    /// A `module "Name" { ... }` declaration.
    ModuleDecl(&'a ModuleDeclSite),
    /// A named `view`/`action`/`actor` declaration.
    Named(&'a NamedSite),
    /// A `use "Target" ...` statement site.
    Use(&'a ModuleUseSite),
}

/// The span side-table for one compilation: every file, every module
/// declaration/use, and every named view/action/actor, with byte-offset
/// spans into each file's text. Plain struct — never serialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IfmlIndex {
    /// Every file in the compilation, in compilation order (main file
    /// first, then imports depth-first in source order).
    pub files: Vec<IndexFile>,
    /// Module declaration sites across all files, in compilation order
    /// (contiguous per file, source order within a file).
    pub module_decls: Vec<ModuleDeclSite>,
    /// Module use sites across all files, in compilation order (contiguous
    /// per file, source order within a file).
    pub module_uses: Vec<ModuleUseSite>,
    /// View declaration sites.
    pub views: Vec<NamedSite>,
    /// Action declaration sites.
    pub actions: Vec<NamedSite>,
    /// Actor declaration sites.
    pub actors: Vec<NamedSite>,
}

impl IfmlIndex {
    /// An index holding exactly one file with no sites.
    pub(crate) fn single_file(path: &str, text: &str) -> Self {
        Self {
            files: vec![IndexFile {
                path: path.to_string(),
                text: text.to_string(),
            }],
            module_decls: Vec::new(),
            module_uses: Vec::new(),
            views: Vec::new(),
            actions: Vec::new(),
            actors: Vec::new(),
        }
    }

    /// Merges a parsed import's index (always exactly one file, relabeled
    /// from index 0 to `file`) into this compilation index.
    pub(crate) fn absorb(&mut self, file: usize, other: IfmlIndex) {
        self.files.extend(other.files);
        for mut decl in other.module_decls {
            decl.file = file;
            self.module_decls.push(decl);
        }
        for mut use_site in other.module_uses {
            use_site.file = file;
            self.module_uses.push(use_site);
        }
        for mut named in other.views {
            named.file = file;
            self.views.push(named);
        }
        for mut named in other.actions {
            named.file = file;
            self.actions.push(named);
        }
        for mut named in other.actors {
            named.file = file;
            self.actors.push(named);
        }
    }

    /// The path of file `file`, if it exists in the compilation.
    pub fn file_path(&self, file: usize) -> Option<&str> {
        self.files.get(file).map(|f| f.path.as_str())
    }

    /// The declaration or use site whose span contains `offset` in `file`'s
    /// text, if any — the single lookup behind hover and go-to-definition.
    /// The narrowest containing span wins, so a `use` inside a view
    /// resolves to the use, not the view.
    pub fn at(&self, file: usize, offset: usize) -> Option<AtSite<'_>> {
        let mut best: Option<AtSite<'_>> = None;
        let mut best_width = usize::MAX;
        for site in &self.module_decls {
            if site.file == file {
                consider_site(
                    &mut best,
                    &mut best_width,
                    AtSite::ModuleDecl(site),
                    site.span,
                    offset,
                );
            }
        }
        for site in self.views.iter().chain(&self.actions).chain(&self.actors) {
            if site.file == file {
                consider_site(
                    &mut best,
                    &mut best_width,
                    AtSite::Named(site),
                    site.span,
                    offset,
                );
            }
        }
        for site in &self.module_uses {
            if site.file == file {
                consider_site(
                    &mut best,
                    &mut best_width,
                    AtSite::Use(site),
                    site.span,
                    offset,
                );
            }
        }
        best
    }

    /// Maps a resolution reference — `(file index, decl index within that
    /// file)` — back to the module declaration site, or `None` when the
    /// reference is out of bounds.
    pub fn module_decl(&self, reference: (usize, usize)) -> Option<&ModuleDeclSite> {
        let (file, decl) = reference;
        let mut seen = 0usize;
        for site in &self.module_decls {
            if site.file != file {
                continue;
            }
            if seen == decl {
                return Some(site);
            }
            seen += 1;
        }
        None
    }
}

/// Parses an IFML source and returns the model together with its span
/// index — the same model [`crate::parse_ifml`] produces, plus the
/// [`IfmlIndex`] side-table. The single file is recorded with an empty
/// path; [`crate::compile_ifml_str`] is the path-aware entry point.
pub fn parse_ifml_indexed(text: &str) -> Result<(IfmlModel, IfmlIndex), crate::IfmlParseError> {
    parse_ifml_indexed_with_path("", text)
}

/// Path-aware index parse: the compilation's one file is recorded under
/// `path`. Used by [`crate::compile_ifml_str`] for the main file and every
/// imported file.
pub(crate) fn parse_ifml_indexed_with_path(
    path: &str,
    text: &str,
) -> Result<(IfmlModel, IfmlIndex), crate::IfmlParseError> {
    let top = parse_ifml_top_pair(text)?;
    let Some(top) = top else {
        return Ok((IfmlModel::default(), IfmlIndex::single_file(path, text)));
    };
    let model = crate::parser::parse_ifml_model(top.clone().into_inner())?;
    let mut index = IfmlIndex::single_file(path, text);
    index_top_pair(&top, 0, &mut index);
    Ok((model, index))
}

/// Walks the `ifml_model` parse tree, collecting declaration and use sites
/// into `index`. A parallel walk (not woven into the lowering) keeps the
/// lowering untouched.
fn index_top_pair(top: &Pair<'_, Rule>, file: usize, index: &mut IfmlIndex) {
    for child in top.clone().into_inner() {
        index_pair(&child, file, index);
    }
}

fn index_pair(pair: &Pair<'_, Rule>, file: usize, index: &mut IfmlIndex) {
    let span = pair_span(pair);
    match pair.as_rule() {
        Rule::view_declaration => {
            if let Some((name, name_span)) = first_string_site(pair) {
                index.views.push(NamedSite {
                    file,
                    name,
                    name_span,
                    span,
                });
            }
            recurse(pair, file, index);
        }
        Rule::action_declaration | Rule::actor_declaration => {
            if let Some((name, name_span)) = first_string_site(pair) {
                let site = NamedSite {
                    file,
                    name,
                    name_span,
                    span,
                };
                if pair.as_rule() == Rule::action_declaration {
                    index.actions.push(site);
                } else {
                    index.actors.push(site);
                }
            }
            recurse(pair, file, index);
        }
        Rule::module_declaration => {
            let (name, name_span) = first_string_site(pair).unwrap_or_default();
            let input_params = first_parameter_block(pair);
            let properties: Vec<String> = pair
                .clone()
                .into_inner()
                .filter(|child| child.as_rule() == Rule::property_assignment)
                .filter_map(|child| child.clone().into_inner().next())
                .map(|key| key.as_str().to_string())
                .collect();
            index.module_decls.push(ModuleDeclSite {
                file,
                name,
                name_span,
                span,
                input_params,
                properties,
            });
            recurse(pair, file, index);
        }
        Rule::module_use_statement => {
            let (target, name_span) = first_string_site(pair).unwrap_or_default();
            let alias = pair
                .clone()
                .into_inner()
                .find(|child| child.as_rule() == Rule::identifier)
                .map(|child| child.as_str().to_string());
            let overrides = pair
                .clone()
                .into_inner()
                .find(|child| child.as_rule() == Rule::module_use_body)
                .map(|body| {
                    body.clone()
                        .into_inner()
                        .filter(|child| child.as_rule() == Rule::property_assignment)
                        .map(|assignment| {
                            let mut inner = assignment.clone().into_inner();
                            let name = inner
                                .next()
                                .map(|key| key.as_str().to_string())
                                .unwrap_or_default();
                            let literal = inner.next().and_then(|value| literal_kind(&value));
                            UseOverride {
                                name,
                                span: pair_span(&assignment),
                                literal,
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            index.module_uses.push(ModuleUseSite {
                file,
                target,
                name_span,
                alias,
                span,
                overrides,
                resolved_module: None,
            });
        }
        _ => recurse(pair, file, index),
    }
}

/// `at`'s merge step: keeps the narrowest containing span.
fn consider_site<'a>(
    best: &mut Option<AtSite<'a>>,
    best_width: &mut usize,
    site: AtSite<'a>,
    span: (usize, usize),
    offset: usize,
) {
    if span.0 <= offset && offset < span.1 && span.1 - span.0 < *best_width {
        *best = Some(site);
        *best_width = span.1 - span.0;
    }
}

fn recurse(pair: &Pair<'_, Rule>, file: usize, index: &mut IfmlIndex) {
    for child in pair.clone().into_inner() {
        index_pair(&child, file, index);
    }
}

fn pair_span(pair: &Pair<'_, Rule>) -> (usize, usize) {
    let span = pair.as_span();
    (span.start(), span.end())
}

/// The first `string` child as `(unescaped text, byte span of the token)`.
fn first_string_site(pair: &Pair<'_, Rule>) -> Option<(String, (usize, usize))> {
    pair.clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::string)
        .map(|child| (parse_string(&child), pair_span(&child)))
}

/// The module's first (input) `parameter_block`, lowered to [`IndexParam`]s.
fn first_parameter_block(pair: &Pair<'_, Rule>) -> Vec<IndexParam> {
    pair.clone()
        .into_inner()
        .find(|child| child.as_rule() == Rule::parameter_block)
        .map(|block| {
            block
                .clone()
                .into_inner()
                .filter(|decl| decl.as_rule() == Rule::parameter_decl)
                .map(|decl| {
                    let mut inner = decl.clone().into_inner();
                    let name = inner
                        .next()
                        .map(|part| part.as_str().to_string())
                        .unwrap_or_default();
                    let type_ref = inner
                        .next()
                        .map(|part| part.as_str().to_string())
                        .unwrap_or_default();
                    let has_default = inner
                        .next()
                        .is_some_and(|part| part.as_rule() == Rule::param_default);
                    IndexParam {
                        name,
                        type_ref,
                        span: pair_span(&decl),
                        has_default,
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The structural literal kind of a value expression: descend the operator
/// chain while it has exactly one child; recognize bare (optionally
/// negated) string/number/boolean leaves. Containers (arrays, objects) and
/// non-literals (identifiers, calls, field access) are `None`.
fn literal_kind(pair: &Pair<'_, Rule>) -> Option<LiteralKind> {
    match pair.as_rule() {
        Rule::string => Some(LiteralKind::Str),
        Rule::number => Some(LiteralKind::Num),
        Rule::boolean => Some(LiteralKind::Bool),
        Rule::value_expression
        | Rule::expression
        | Rule::logical_or
        | Rule::logical_and
        | Rule::comparison
        | Rule::addition
        | Rule::multiplication
        | Rule::unary
        | Rule::unary_not
        | Rule::unary_neg
        | Rule::primary
        | Rule::group_expr => {
            let mut inner = pair.clone().into_inner();
            let single = inner.next()?;
            if inner.next().is_some() {
                return None;
            }
            literal_kind(&single)
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_source_indexes_one_file_and_no_sites() {
        let (model, index) = parse_ifml_indexed("").unwrap();
        assert_eq!(model, IfmlModel::default());
        assert_eq!(index.files.len(), 1);
        assert_eq!(index.files[0].path, "");
        assert!(index.module_decls.is_empty());
        assert!(index.module_uses.is_empty());
        assert!(index.views.is_empty());
        assert!(index.actions.is_empty());
        assert!(index.actors.is_empty());
    }

    #[test]
    fn indexes_views_actions_actors_modules_and_uses_with_spans() {
        let text = r#"
view "List" {
    use "Pagination" as pager { page_size: 25; };
    container "Step" {
        use "Footer";
    }
    component "grid" { type: list; data: Item; }
}

action "DoIt" {
    target: Customer;
}

actor "Admin" {
    role: admin;
}

module "Pagination" {
    input { page: Int = 1, pageSize: Int }
    output { total: Int }

    title: "Pager";

    use "Footer" as foot;

    component "pager" { type: list; data: Item; }
}
"#;
        let (_, index) = parse_ifml_indexed(text).unwrap();

        assert_eq!(index.views.len(), 1);
        assert_eq!(index.views[0].name, "List");
        assert_eq!(index.actions.len(), 1);
        assert_eq!(index.actions[0].name, "DoIt");
        assert_eq!(index.actors.len(), 1);
        assert_eq!(index.actors[0].name, "Admin");

        assert_eq!(index.module_decls.len(), 1);
        let decl = &index.module_decls[0];
        assert_eq!(decl.name, "Pagination");
        assert_eq!(decl.input_params.len(), 2);
        assert_eq!(decl.input_params[0].name, "page");
        assert_eq!(decl.input_params[0].type_ref, "Int");
        assert!(decl.input_params[0].has_default);
        assert_eq!(decl.input_params[1].name, "pageSize");
        assert!(!decl.input_params[1].has_default);
        assert_eq!(decl.properties, vec!["title".to_string()]);

        // view use, container use, module-internal use — in source order
        assert_eq!(index.module_uses.len(), 3);
        assert_eq!(index.module_uses[0].target, "Pagination");
        assert_eq!(index.module_uses[0].alias.as_deref(), Some("pager"));
        assert!(index.module_uses[0].resolved_module.is_none());
        assert_eq!(index.module_uses[1].target, "Footer");
        assert_eq!(index.module_uses[1].alias, None);
        assert_eq!(index.module_uses[2].target, "Footer");
        assert_eq!(index.module_uses[2].alias.as_deref(), Some("foot"));

        // spans are byte offsets that slice back to the expected source
        for site in &index.module_uses {
            let slice = &text[site.span.0..site.span.1];
            assert!(slice.starts_with("use "), "{slice:?}");
        }
        let decl_slice = &text[decl.span.0..decl.span.1];
        assert!(
            decl_slice.starts_with("module \"Pagination\""),
            "{decl_slice:?}"
        );
        let param = &decl.input_params[1];
        // pest spans absorb trailing inter-token whitespace
        assert_eq!(text[param.span.0..param.span.1].trim_end(), "pageSize: Int");

        // overrides record literal kinds; non-literals record None
        let overrides = &index.module_uses[0].overrides;
        assert_eq!(overrides.len(), 1);
        assert_eq!(overrides[0].name, "page_size");
        assert_eq!(overrides[0].literal, Some(LiteralKind::Num));
        assert_eq!(
            &text[overrides[0].span.0..overrides[0].span.1],
            "page_size: 25;"
        );
    }

    #[test]
    fn module_decl_maps_resolution_references_back_to_sites() {
        let (_, index) = parse_ifml_indexed(
            "module \"A\" { input { x: Int } output { y: Int } }\nmodule \"B\" { input { x: Int } output { y: Int } }",
        )
        .unwrap();
        assert_eq!(index.module_decl((0, 0)).unwrap().name, "A");
        assert_eq!(index.module_decl((0, 1)).unwrap().name, "B");
        assert!(index.module_decl((0, 2)).is_none());
        assert!(index.module_decl((1, 0)).is_none());
    }
}

#[cfg(test)]
mod at_tests {
    use super::*;

    #[test]
    fn at_finds_narrowest_site_and_name_spans() {
        let source = r#"import "pager.ifml";

view "Catalogue" {
    use "Pager" as pager { };
}

module "Pager" {
    input { pageSize: Int = 25 }
    output { total: Int }
}
"#;
        let (_model, index) = parse_ifml_indexed(source).unwrap();
        assert_eq!(index.views.len(), 1, "views indexed");
        assert_eq!(index.module_decls.len(), 1, "modules indexed");
        assert_eq!(index.module_uses.len(), 1, "uses indexed");
        let view = &index.views[0];
        assert_eq!(view.file, 0);
        assert_eq!(view.name, "Catalogue");
        assert!(view.name_span.0 < view.name_span.1);
        // The use site is inside the view span.
        let use_site = &index.module_uses[0];
        assert!(use_site.span.0 >= view.span.0 && use_site.span.1 <= view.span.1);
        // `at` picks the use (narrower) inside the view.
        let mid_use = (use_site.span.0 + use_site.span.1) / 2;
        assert!(matches!(index.at(0, mid_use), Some(AtSite::Use(_))));
        // `at` picks the view outside the use but inside the view span.
        let view_name_mid = (view.name_span.0 + view.name_span.1) / 2;
        assert!(matches!(index.at(0, view_name_mid), Some(AtSite::Named(_))));
    }
}
