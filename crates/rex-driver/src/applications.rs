//! Import-connected application clustering (issue rexlang#241, the
//! semantic-graph slice): the union-find relation `check`'s integrity pass
//! and the graph builder both need, hoisted into the driver so the two
//! never drift.
//!
//! Two files share an application when an import of one resolves to the
//! other — transitively. A `.mox` domain every surface imports is a
//! **bridge node**: it joins its importers into one component without any
//! special casing (an edge through it is just two edges).
//!
//! The relation is a **pure function of path strings**: the caller
//! normalizes (and canonicalizes) every path and every import target first.
//! An import naming no member of the set contributes no edge, and a file no
//! import reaches is its own component.
//!
//! ```
//! use rex_driver::applications::{applications, ApplicationFile};
//!
//! let files = [
//!     ApplicationFile { path: "/app/desk.ddd".into(), imports: vec!["/app/app.mox".into()] },
//!     ApplicationFile { path: "/app/other.ddd".into(), imports: vec!["/app/app.mox".into()] },
//!     ApplicationFile { path: "/app/app.mox".into(), imports: Vec::new() },
//! ];
//! let components = applications(&files);
//! assert_eq!(components, vec![vec![0, 1, 2]]);
//! ```

use std::collections::BTreeMap;

/// One file's clustering input: its host-normalized path and the
/// host-normalized targets of its imports.
///
/// The driver never touches the filesystem: the caller canonicalizes both
/// the paths and the resolved import targets (the CLI's snapshot code does
/// this at the point it reads the files, the LSP at the point it keys
/// documents). Paths are used for matching and ordering only — they are
/// never displayed, so nothing leaks an absolute path out of a component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApplicationFile {
    /// The file's host-normalized path (canonical where the host has one).
    pub path: String,
    /// The file's import targets, normalized the same way as [`Self::path`].
    pub imports: Vec<String>,
}

/// Clusters files into import-connected applications.
///
/// Returns the components as member indices into `files`: each component's
/// members are sorted ascending, and components are sorted by their
/// smallest member path — the stable order the union integrity pass renders
/// diagnostics in and the graph builder numbers applications by.
///
/// # Precondition
///
/// Every `path` and every import target is host-normalized (the caller
/// canonicalizes): the relation is plain string equality. An import that
/// names no member of the set contributes no edge; duplicate paths are one
/// node (they union together).
///
/// ```
/// use rex_driver::applications::{applications, ApplicationFile};
///
/// let files = [
///     ApplicationFile { path: "b/ddd.ddd".into(), imports: Vec::new() },
///     ApplicationFile { path: "a/mox.mox".into(), imports: Vec::new() },
/// ];
/// // Unrelated files are separate components, ordered by smallest path.
/// assert_eq!(applications(&files), vec![vec![1], vec![0]]);
/// ```
pub fn applications(files: &[ApplicationFile]) -> Vec<Vec<usize>> {
    // Node identity is exact path equality; several entries naming one file
    // (a caller passing the same path twice) union into one node.
    let mut nodes: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
    for (index, file) in files.iter().enumerate() {
        nodes.entry(file.path.as_str()).or_default().push(index);
    }
    let mut parent: Vec<usize> = (0..files.len()).collect();
    for members in nodes.values() {
        for &member in &members[1..] {
            union(&mut parent, members[0], member);
        }
    }
    // Import edges join a file to whatever member lives at the target.
    for (from, file) in files.iter().enumerate() {
        for import in &file.imports {
            let Some(targets) = nodes.get(import.as_str()) else {
                continue;
            };
            for &to in targets {
                union(&mut parent, from, to);
            }
        }
    }
    // Components keyed by root; members land in ascending node order because
    // the loop is ascending, so no per-component sort is needed.
    let mut components: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for node in 0..files.len() {
        let root = find_root(&mut parent, node);
        components.entry(root).or_default().push(node);
    }
    let mut ordered: Vec<Vec<usize>> = components.into_values().collect();
    ordered.sort_by(|a, b| smallest_path(files, a).cmp(smallest_path(files, b)));
    ordered
}

/// The smallest member path of a component (its canonical ordering key and,
/// for the graph, its application id).
fn smallest_path<'a>(files: &'a [ApplicationFile], members: &[usize]) -> &'a str {
    members
        .iter()
        .map(|&member| files[member].path.as_str())
        .min()
        .expect("a component is never empty")
}

/// Follows union-find parent links with path halving.
fn find_root(parent: &mut [usize], mut node: usize) -> usize {
    while parent[node] != node {
        parent[node] = parent[parent[node]];
        node = parent[node];
    }
    node
}

/// Joins the sets containing `a` and `b`.
fn union(parent: &mut [usize], a: usize, b: usize) {
    let a = find_root(parent, a);
    let b = find_root(parent, b);
    if a != b {
        parent[a] = b;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str, imports: &[&str]) -> ApplicationFile {
        ApplicationFile {
            path: path.to_string(),
            imports: imports.iter().map(|import| import.to_string()).collect(),
        }
    }

    #[test]
    fn unrelated_files_are_separate_components() {
        let files = [
            file("/app/library.mox", &[]),
            file("/app/pricing.mox", &[]),
            file("/app/iot.mox", &[]),
        ];
        assert_eq!(applications(&files), vec![vec![2], vec![0], vec![1]]);
    }

    #[test]
    fn a_mox_bridge_merges_design_and_policy() {
        // The design and the policy import nothing of each other; the
        // domain both import is the bridge node.
        let files = [
            file("/app/support.ddd", &["/app/support.mox"]),
            file("/app/support.actor", &["/app/support.mox"]),
            file("/app/support.mox", &[]),
            file("/app/orders.ddd", &["/app/ecommerce.mox"]),
            file("/app/ecommerce.mox", &[]),
        ];
        assert_eq!(
            applications(&files),
            vec![
                // ecommerce.mox sorts before support.mox.
                vec![3, 4],
                vec![0, 1, 2],
            ]
        );
    }

    #[test]
    fn components_sort_by_smallest_member_path() {
        let files = [
            file("/app/z/zeta.mox", &[]),
            file("/app/a/alpha.ddd", &["/app/z/zeta.mox"]),
            file("/app/m/middle.mox", &[]),
        ];
        // The alpha/zeta component's smallest path is /app/a/alpha.ddd.
        assert_eq!(applications(&files), vec![vec![0, 1], vec![2]]);
    }

    #[test]
    fn an_import_outside_the_set_contributes_no_edge() {
        let files = [
            file("/app/desk.ddd", &["/app/app.mox", "/elsewhere/missing.mox"]),
            file("/app/other.mox", &[]),
        ];
        assert_eq!(applications(&files), vec![vec![0], vec![1]]);
    }

    #[test]
    fn member_indices_are_sorted() {
        // Imports arrive in arbitrary order; members always read ascending.
        let files = [
            file("/app/c.mox", &["/app/a.mox"]),
            file("/app/b.mox", &["/app/c.mox"]),
            file("/app/a.mox", &[]),
        ];
        let components = applications(&files);
        assert_eq!(components.len(), 1);
        assert_eq!(components[0], vec![0, 1, 2]);
    }

    #[test]
    fn duplicate_paths_are_one_node() {
        let files = [
            file("/app/a.mox", &[]),
            file("/app/b.mox", &["/app/a.mox"]),
            file("/app/a.mox", &[]),
        ];
        assert_eq!(applications(&files), vec![vec![0, 1, 2]]);
    }

    #[test]
    fn the_empty_set_has_no_components() {
        assert!(applications(&[]).is_empty());
    }
}
