//! The per-invocation project snapshot (Phase C of #117): every supported
//! file under the command-line directory roots gathered into one
//! `resolved key → text` map, plus the compile-once [`Artifact`] cache.
//!
//! `rexlang check <dir>` expands its argument into the supported-surface
//! listing ([`crate::inputs::expand_check_inputs`]) and hands the listing
//! to [`Project::gather`]: every file is read once into the snapshot. The
//! per-surface readers in [`crate::inputs`] then fetch imports
//! **map-first, disk-second** — the CLI's analog of the LSP's
//! open-documents-first rule. An import that resolves inside the project
//! is served from the map; one that resolves outside the root is read from
//! disk relative to its importer **and inserted into the map**, so a later
//! consumer of the same file reuses it. Read errors keep today's
//! byte-identical messages: fetches return [`std::io::Result`] and each
//! reader's side channel still renders its own
//! `cannot read imported file … (imported by …)` text.
//!
//! The snapshot also owns the artifact cache: every CLI-level compile of a
//! project file lands in it under the file's resolved key, and consumers
//! compile sibling imports **lookup-before-compute** — an `.ifml` flow
//! whose `.actor` import the batch already checked reuses that compiled
//! [`Artifact::Actor`] instead of recompiling. The surface import graph is
//! acyclic by grammar (`.mox` is the root; `.ifml` imports only
//! `.ifml`/`.mox`/`.actor` — intra-`.ifml` cycles are the resolver's
//! E0003), so a plain map with a lookup before each compute suffices.
//! Compiles that happen inside `rex_driver`'s gatherers (a `.deploy`
//! file's imported designs and flows, a `.uc` file's four buckets) consume
//! the map's texts but cannot consult this cache — the cache boundary is
//! the CLI's own readers. Top-level per-file checks always compile and
//! render (the byte-identical `OK`/warning output is the contract); the
//! cache dedupes the import-side compiles.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// One compiled surface artifact — the same JSON `rexlang ir` emits for
/// the surface, kept typed so consumers can pull the model they need.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Artifact {
    /// A `.mox` domain model (the Core IR `Model`).
    Domain(rex_ir::Model),
    /// An `.actor` policy (the standalone `ActorModel`).
    Actor(rex_ir::ActorModel),
    /// A `.ddd` design (the standalone `DddModel`).
    Design(rex_ir::ddd::DddModel),
    /// An `.evt` contract (the standalone `EventModel`).
    Event(rex_ir::events::EventModel),
    /// A `.uc` use case pattern (the standalone `UseCaseModel`).
    UseCase(rex_ir::usecases::UseCaseModel),
    /// A `.cmmn` case (the standalone `CmmnModel`).
    Case(rex_ir::cmmn::CmmnModel),
    /// A `.deploy` deployment (the standalone `DeployModel`).
    Deploy(rex_ir::deploy::DeployModel),
    /// An `.ifml` flow (the standalone `IfmlModel`).
    Flow(rex_ir::ifml::IfmlModel),
}

impl Artifact {
    /// The artifact JSON exactly `rexlang ir` emits for the surface.
    pub(crate) fn to_json_pretty(&self) -> anyhow::Result<String> {
        match self {
            Artifact::Domain(model) => Ok(model.to_json_pretty()?),
            Artifact::Actor(model) => Ok(model.to_json_pretty()?),
            Artifact::Design(model) => Ok(model.to_json_pretty()?),
            Artifact::Event(model) => Ok(model.to_json_pretty()?),
            Artifact::UseCase(model) => Ok(model.to_json_pretty()?),
            Artifact::Case(model) => Ok(model.to_json_pretty()?),
            Artifact::Deploy(model) => Ok(model.to_json_pretty()?),
            Artifact::Flow(model) => Ok(model.to_json_pretty()?),
        }
    }
}

/// The in-memory snapshot of every supported file under the command-line
/// roots, plus the compile-once artifact cache.
#[derive(Debug, Default)]
pub(crate) struct Project {
    /// Resolved key → source text. Keys are the resolved disk-path strings
    /// the readers compute (`dir_of(importer).join(import)` displayed) —
    /// the same strings the driver receives as file paths.
    sources: BTreeMap<String, String>,
    /// Resolved key → compiled artifact (lookup-before-compute).
    artifacts: BTreeMap<String, Artifact>,
    /// How many compiles the cache has performed — the hit/miss observer
    /// the unit tests pin.
    computes: usize,
}

impl Project {
    /// Reads every file of an `expand_check_inputs`-style listing into the
    /// snapshot. Unreadable entries are skipped: their per-file check
    /// re-reads them through today's reader path and fails there, with
    /// today's message, at today's point in the batch output.
    pub(crate) fn gather(expanded: &[PathBuf]) -> Project {
        let mut project = Project::default();
        for file in expanded {
            if let Ok(text) = std::fs::read_to_string(file) {
                project.sources.insert(file.display().to_string(), text);
            }
        }
        project
    }

    /// The snapshot's text for a resolved key, without touching disk.
    #[cfg(test)]
    pub(crate) fn text(&self, key: &str) -> Option<&str> {
        self.sources.get(key).map(String::as_str)
    }

    /// Map-first, disk-second fetch: the snapshot's text for `key`, or a
    /// disk read of `path` — filled into the map on success, so a later
    /// consumer of the same file never re-reads. `Err` carries the io
    /// error so callers render today's message byte-for-byte.
    pub(crate) fn fetch_path(&mut self, key: &str, path: &Path) -> std::io::Result<String> {
        if let Some(text) = self.sources.get(key) {
            return Ok(text.clone());
        }
        let text = std::fs::read_to_string(path)?;
        self.sources.insert(key.to_string(), text.clone());
        Ok(text)
    }

    /// The path-keyed fetch shape the driver gatherers take
    /// (`FnMut(&str) -> Option<String>`): the resolved key **is** the disk
    /// path, so the key locates both the map slot and the fallback read.
    pub(crate) fn fetch(&mut self, key: &str) -> std::io::Result<String> {
        self.fetch_path(key, Path::new(key))
    }

    /// Records a freshly compiled artifact under `key`.
    pub(crate) fn store(&mut self, key: &str, artifact: &Artifact) {
        self.computes += 1;
        self.artifacts.insert(key.to_string(), artifact.clone());
    }

    /// Lookup-before-compute: the cached artifact for `key`, or
    /// `compute()` run exactly once and stored. The import graph is
    /// acyclic by grammar, so a plain map suffices — no cycle machinery.
    pub(crate) fn artifact(
        &mut self,
        key: &str,
        compute: impl FnOnce() -> anyhow::Result<Artifact>,
    ) -> anyhow::Result<Artifact> {
        if let Some(hit) = self.artifacts.get(key) {
            return Ok(hit.clone());
        }
        let artifact = compute()?;
        self.computes += 1;
        self.artifacts.insert(key.to_string(), artifact.clone());
        Ok(artifact)
    }

    /// How many compiles the cache has performed.
    #[cfg(test)]
    pub(crate) fn computes(&self) -> usize {
        self.computes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh scratch directory per test, so concurrently running tests
    /// never share files (the integration-suite rule, in-process).
    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rex-cli-project-tests-{}-{}",
            std::process::id(),
            tag
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    const MOX: &str = "package demo\n\nclass Book { String title }\n";

    fn domain_artifact() -> Artifact {
        let compilation =
            rex_driver::compile_str("t.mox", MOX, &rex_driver::DomainImports::default());
        Artifact::Domain(compilation.model.expect("compiles"))
    }

    #[test]
    fn gather_builds_a_map_from_a_listing() {
        let dir = scratch("gather");
        let a = dir.join("a.mox");
        std::fs::write(&a, MOX).expect("write a");
        let b = dir.join("nested/b.mox");
        std::fs::create_dir_all(b.parent().unwrap()).expect("mkdir nested");
        std::fs::write(&b, MOX).expect("write b");
        let missing = dir.join("missing.mox");

        let project = Project::gather(&[a.clone(), b.clone(), missing.clone()]);

        assert_eq!(project.text(&a.display().to_string()), Some(MOX));
        assert_eq!(project.text(&b.display().to_string()), Some(MOX));
        assert_eq!(
            project.text(&missing.display().to_string()),
            None,
            "unreadable entries are skipped; their per-file check fails with today's message"
        );
    }

    #[test]
    fn fetch_serves_the_map_before_disk() {
        let dir = scratch("map-first");
        let path = dir.join("a.mox");
        std::fs::write(&path, MOX).expect("write a");
        let mut project = Project::gather(std::slice::from_ref(&path));
        std::fs::remove_file(&path).expect("remove from disk");

        let key = path.display().to_string();
        assert_eq!(
            project.fetch(&key).expect("map hit"),
            MOX,
            "a project file is served from the snapshot even after it leaves disk"
        );
    }

    #[test]
    fn fetch_falls_back_to_disk_and_fills_the_map() {
        let dir = scratch("disk-fallback");
        let listed = dir.join("a.mox");
        std::fs::write(&listed, MOX).expect("write listed");
        // An import resolving outside the gathered root.
        let outside = dir.join("outside.mox");
        std::fs::write(&outside, MOX).expect("write outside");
        let mut project = Project::gather(&[listed]);

        let key = outside.display().to_string();
        assert_eq!(
            project.fetch(&key).expect("disk fallback"),
            MOX,
            "an out-of-root import is read from disk"
        );
        std::fs::remove_file(&outside).expect("remove from disk");
        assert_eq!(
            project.fetch(&key).expect("map hit after fill"),
            MOX,
            "the disk fallback filled the map"
        );
        assert_eq!(project.text(&key), Some(MOX));
    }

    #[test]
    fn cache_compiles_once_per_key() {
        let mut project = Project::default();
        let recomputed = std::cell::Cell::new(false);

        let first = project
            .artifact("a.mox", || Ok(domain_artifact()))
            .expect("first compute");
        let second = project
            .artifact("a.mox", || {
                recomputed.set(true);
                Ok(domain_artifact())
            })
            .expect("second lookup");

        assert_eq!(first, second);
        assert_eq!(project.computes(), 1, "hit: no recompute");
        assert!(!recomputed.get(), "the second compute closure never ran");
    }

    #[test]
    fn cache_misses_compute_and_insert_per_key() {
        let mut project = Project::default();

        let _ = project
            .artifact("a.mox", || Ok(domain_artifact()))
            .expect("a");
        let _ = project
            .artifact("b.mox", || Ok(domain_artifact()))
            .expect("b");
        assert_eq!(project.computes(), 2, "distinct keys compute once each");

        let _ = project
            .artifact("a.mox", || Ok(domain_artifact()))
            .expect("a again");
        assert_eq!(project.computes(), 2, "a.mox stays cached");
    }
}
