//! Name resolution: the `.mox` type namespace walk behind [`Scope`] —
//! single-package resolution for plain compiles, union-namespace
//! resolution for multi-file compiles and the imported domains of the
//! artifact surfaces, plus the cross-surface [`resolve_ddd_type`] /
//! [`resolve_ddd_class`] entry points the `.ddd`/`.evt`/`.uc`/`.cmmn`
//! engines share (never re-implemented there).

use std::collections::HashMap;

use rex_ir as ir;
use rex_syntax::ast as mox;

use crate::diagnostic::Diagnostic;

use super::*;
/// Resolves within a single package: primitives and single-segment names are
/// package-local; the only qualified form accepted is `<package>.<Name>`.
pub(crate) fn resolve_single(
    type_ref: &mox::TypeRef,
    package: &str,
    kinds: &HashMap<String, TopKind>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Resolution> {
    let segments = &type_ref.name.segments;
    let full_name = type_ref.name.full_name();

    let local = match segments.len() {
        1 => &segments[0].text,
        2 if segments[0].text == *package => &segments[1].text,
        _ => {
            diags.push(Diagnostic::error(
                "cross-package type references are not supported yet",
                Some(type_ref.span),
            ));
            return None;
        }
    };

    if segments.len() == 1 {
        // A declaration shadows the like-named built-in: a model's own
        // `type Date wraps opaque` keeps resolving to the datatype after the
        // `date` primitive exists (issue #9).
        if kinds.contains_key(local.as_str()) {
            let resolved = classify(
                &full_name,
                local,
                type_ref.span,
                kinds.get(local.as_str()),
                diags,
            )?;
            return Some(Resolution {
                kind: resolved,
                package: package.to_string(),
                name: local.clone(),
            });
        }
        if let Some(primitive) = primitive_type(local) {
            return Some(Resolution {
                kind: Resolved::Primitive(primitive),
                package: package.to_string(),
                name: local.clone(),
            });
        }
    }

    let resolved = classify(
        &full_name,
        local,
        type_ref.span,
        kinds.get(local.as_str()),
        diags,
    )?;
    Some(Resolution {
        kind: resolved,
        package: package.to_string(),
        name: local.clone(),
    })
}

/// Resolves against the combined namespace of the imported domain packages:
/// a single-segment name is looked up in every package (ambiguous when found
/// in more than one); a qualified name's leading segments must name exactly
/// one package.
pub(crate) fn resolve_domains(
    type_ref: &mox::TypeRef,
    packages: &[DomainPackage],
    diags: &mut Vec<Diagnostic>,
) -> Option<Resolution> {
    let segments = &type_ref.name.segments;
    let full_name = type_ref.name.full_name();

    if segments.len() >= 2 {
        let package_name = segments[..segments.len() - 1]
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join(".");
        let local = &segments[segments.len() - 1].text;
        if let Some(package) = packages.iter().find(|package| package.name == package_name) {
            let resolved = classify(
                &full_name,
                local,
                type_ref.span,
                package.kinds.get(local.as_str()),
                diags,
            )?;
            return Some(Resolution {
                kind: resolved,
                package: package.name.clone(),
                name: local.clone(),
            });
        }
        diags.push(Diagnostic::error(
            format!("unknown type '{full_name}'"),
            Some(type_ref.span),
        ));
        return None;
    }

    let local = &segments[0].text;
    // Declared types shadow like-named primitives (issue #9); ambiguity
    // between several declaring packages stands.
    let matches: Vec<&DomainPackage> = packages
        .iter()
        .filter(|package| package.kinds.contains_key(local.as_str()))
        .collect();
    match matches.len() {
        0 => {
            if let Some(primitive) = primitive_type(local) {
                return Some(Resolution {
                    kind: Resolved::Primitive(primitive),
                    package: String::new(),
                    name: local.clone(),
                });
            }
            diags.push(Diagnostic::error(
                format!("unknown type '{full_name}'"),
                Some(type_ref.span),
            ));
            None
        }
        1 => {
            let package = matches[0];
            let resolved = classify(
                &full_name,
                local,
                type_ref.span,
                package.kinds.get(local.as_str()),
                diags,
            )?;
            Some(Resolution {
                kind: resolved,
                package: package.name.clone(),
                name: local.clone(),
            })
        }
        _ => {
            let mut names: Vec<&str> = matches
                .iter()
                .map(|package| package.name.as_str())
                .collect();
            names.sort();
            diags.push(
                Diagnostic::error(
                    format!(
                        "ambiguous type `{local}`; qualify as `{}.{local}`",
                        names[0]
                    ),
                    Some(type_ref.span),
                )
                .with_help(format!(
                    "matching types: {}",
                    names
                        .iter()
                        .map(|name| format!("{name}.{local}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            );
            None
        }
    }
}

/// Resolves a type reference against the combined domain namespace, with an
/// optional preferred (base) package: a bare single-segment name that the
/// base package declares resolves there immediately, before the
/// unique-across-all-packages lookup runs. Qualified names ignore the base.
/// This is the `.ddd` resolution rule (see `compile_ddd_str`); on failure the
/// standard cross-package diagnostics are emitted.
pub(crate) fn resolve_preferred(
    type_ref: &mox::TypeRef,
    packages: &[DomainPackage],
    base: Option<&str>,
    diags: &mut Vec<Diagnostic>,
) -> Option<Resolution> {
    if type_ref.name.segments.len() == 1 {
        if let Some(index) =
            base.and_then(|base| packages.iter().position(|package| package.name == base))
        {
            if packages[index]
                .kinds
                .contains_key(type_ref.name.segments[0].text.as_str())
            {
                return resolve_domains(type_ref, &packages[index..=index], diags);
            }
        }
    }
    resolve_domains(type_ref, packages, diags)
}

/// Resolves a `.ddd` signature type to its IR type reference: any declared
/// kind resolves (classes, enums, datatypes, interfaces, vocabularies) plus
/// the primitives; a failed resolution reports through `diags` and yields a
/// placeholder (the artifact is discarded whenever errors exist).
pub(crate) fn resolve_ddd_type(
    type_ref: &mox::TypeRef,
    packages: &[DomainPackage],
    base: Option<&str>,
    diags: &mut Vec<Diagnostic>,
) -> ir::TypeRef {
    let resolution = resolve_preferred(type_ref, packages, base, diags);
    ir_type_of(
        resolution.as_ref(),
        base.unwrap_or_else(|| {
            packages
                .first()
                .map(|package| package.name.as_str())
                .unwrap_or("")
        }),
        type_ref,
    )
}

/// Resolves a `.ddd` design's class target: only a class resolves. On
/// failure the resolution diagnostics stand, enriched with `design_help`
/// when they do not already carry a help line; a non-class resolution gets
/// its own error naming the found kind.
pub(crate) fn resolve_ddd_class(
    type_ref: &mox::TypeRef,
    packages: &[DomainPackage],
    base: Option<&str>,
    design_help: &str,
    diags: &mut Vec<Diagnostic>,
) -> Option<(String, String)> {
    let before = diags.len();
    let resolution = resolve_preferred(type_ref, packages, base, diags);
    let Some(resolution) = resolution else {
        if let Some(diagnostic) = diags.get_mut(before) {
            if diagnostic.help.is_none() {
                diagnostic.help = Some(design_help.to_string());
            }
        }
        return None;
    };
    if resolution.kind == Resolved::Class {
        return Some((resolution.package, resolution.name));
    }
    let label = resolution.kind.label();
    diags.push(
        Diagnostic::error(
            format!(
                "design target '{}' is {} {label}, not a class",
                type_ref.name.full_name(),
                article(label)
            ),
            Some(type_ref.span),
        )
        .with_help(design_help),
    );
    None
}
