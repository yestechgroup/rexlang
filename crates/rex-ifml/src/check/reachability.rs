//! The instantiation reachability rule shared by the interaction and
//! authorization passes: which module bodies an entry file's `use` sites
//! make part of the app's surface.

use crate::index::{IfmlIndex, ModuleDeclSite};

/// The module bodies the entry file instantiates: seeded by the entry
/// file's own `use` sites and closed over module composition — a reached
/// module's body `use` statements (those textually contained in its
/// declaration span) reach their targets. Identity is the module
/// declaration site itself, so sibling modules of the same imported file
/// stay inert: instantiating `Inbox` does not instantiate `Checkout`.
pub(super) fn instantiated_modules(index: &IfmlIndex) -> Vec<&ModuleDeclSite> {
    let mut reached: Vec<&ModuleDeclSite> = Vec::new();
    let mut queue: Vec<&ModuleDeclSite> = Vec::new();
    for site in &index.module_uses {
        if site.file != 0 {
            continue;
        }
        if let Some(reference) = site.resolved_module {
            let decl = index
                .module_decl(reference)
                .expect("resolved module exists");
            if !reached.contains(&decl) {
                reached.push(decl);
                queue.push(decl);
            }
        }
    }
    while let Some(decl) = queue.pop() {
        for site in &index.module_uses {
            if site.file != decl.file {
                continue;
            }
            let contained = decl.span.0 <= site.span.0 && site.span.1 <= decl.span.1;
            if !contained {
                continue;
            }
            if let Some(reference) = site.resolved_module {
                let target = index
                    .module_decl(reference)
                    .expect("resolved module exists");
                if !reached.contains(&target) {
                    reached.push(target);
                    queue.push(target);
                }
            }
        }
    }
    reached
}

/// Whether a site belongs to the app's surface: the entry file itself, or
/// a reached module body (its span textually inside the module's
/// declaration).
pub(super) fn site_is_instantiated(
    reached: &[&ModuleDeclSite],
    file: usize,
    span: (usize, usize),
) -> bool {
    if file == 0 {
        return true;
    }
    reached
        .iter()
        .any(|decl| decl.file == file && decl.span.0 <= span.0 && span.1 <= decl.span.1)
}
