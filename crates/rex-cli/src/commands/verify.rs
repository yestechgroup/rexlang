//! `rexlang verify`: emit a verification receipt (issue #197), validate one
//! independently (`--check`, issue #206), or prove it fresh against the
//! workspace (`--check-fresh`, the CI gate of issue #207).
//!
//! A receipt is the register's shipped artifact. It records the bytes a
//! compile read, the bytes it wrote, the generator that wrote them, and the
//! proof-obligation register's rows — statuses copied, never reinterpreted.
//! `docs/RECEIPTS.md` is the reader's guide and the staleness contract.
//!
//! # Generation (`rexlang verify <files…> -o <receipt.json>`)
//!
//! Every input is compiled through the ordinary per-surface pipeline
//! ([`compile_surface`] with no project context — exactly the bytes
//! `rexlang ir` and `rexlang build <files>` write, which is what the
//! committed goldens pin), except `.ifml` inputs, which contribute their
//! declaration-backed control/route manifest (post-check, the
//! `rexlang manifest` artifact) — the surface's strongest declaration
//! artifact and the only `.ifml` artifact with a committed golden.
//!
//! Sources are the **transitive closure of every file the compile read**:
//! the recording pass below walks each input through the CLI's own readers
//! with a project snapshot, so every import fetch lands in one map
//! (`Project::keys_with_extension("")` — the empty suffix is every key).
//! Two families are read outside that funnel by their collectors
//! (`import schema`, `import sigil` and the sigil candidate pool; a `.dmn`
//! model's external decision tables), so the completion pass adds them from
//! the same AST declarations the collectors read.
//!
//! Artifacts are the exact emitted bytes, hashed. Where a committed golden
//! exists next to the receipt (`<stem>.<ext>.json`, or `<stem>.manifest.json`
//! for `ifmlManifest`), the in-memory emission must be byte-identical to it
//! or generation fails naming both hashes — that is what makes the receipt
//! stronger than "a hash of whatever was on disk". Domain and actor
//! artifacts ship no example goldens: they are recorded digest-only
//! (`"path": null`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use rex_evidence::receipt::{
    sha256_hex, ArtifactRef, Generator, Receipt, RegisterRow, RegisterSnapshot, RowEvidence,
    SourceFile, TrustedRef,
};

use crate::inputs::{
    expand_check_inputs, is_actor_file, is_cmmn_file, is_ddd_file, is_deploy_file, is_dmn_file,
    is_evt_file, is_ifml_file, is_uc_file, read_actor_pair_in, read_cmmn_pair, read_ddd_design,
    read_deploy_pair, read_dmn_pair, read_evt_pair, read_ifml_inputs, read_uc_pair,
};
use crate::project::Project;
use crate::report::report_ifml_diagnostics;

use super::pipeline::compile_surface;

/// The register the receipt reports, repo-relative (resolved from the
/// working directory, which is the repository root for every supported use
/// — the CI gate runs there, and the CLI is documented that way).
const REGISTER_PATH: &str = "docs/verification/register.json";

/// The CLI surface of `rexlang verify`. Exactly one mode runs: generation
/// (inputs + `-o`), `--check`, or `--check-fresh`.
pub(crate) fn run(
    files: Vec<PathBuf>,
    out: Option<PathBuf>,
    subject: Option<String>,
    check: Option<PathBuf>,
    check_fresh: Option<PathBuf>,
) -> anyhow::Result<ExitCode> {
    match (check, check_fresh) {
        (Some(_), Some(_)) => {
            anyhow::bail!("verify takes either --check or --check-fresh, not both")
        }
        (Some(receipt), None) => {
            require_no_generation_args(&files, &out, &subject)?;
            check_receipt(&receipt)
        }
        (None, Some(receipt)) => {
            require_no_generation_args(&files, &out, &subject)?;
            verify_fresh(&receipt)
        }
        (None, None) => generate(&files, out, subject),
    }
}

/// `--check`/`--check-fresh` read a receipt; they take no generation inputs.
fn require_no_generation_args(
    files: &[PathBuf],
    out: &Option<PathBuf>,
    subject: &Option<String>,
) -> anyhow::Result<()> {
    if !files.is_empty() || out.is_some() || subject.is_some() {
        anyhow::bail!(
            "verify validates one receipt at a time: pass only the receipt path with --check/--check-fresh"
        );
    }
    Ok(())
}

// --- generation ---------------------------------------------------------------

/// Generates the receipt for `files` and writes it to `out`.
fn generate(
    files: &[PathBuf],
    out: Option<PathBuf>,
    subject: Option<String>,
) -> anyhow::Result<ExitCode> {
    let Some(out) = out else {
        anyhow::bail!(
            "verify needs -o <receipt.json> to write the receipt (or --check/--check-fresh to validate one)"
        );
    };
    let inputs = expand_check_inputs(files)?;
    let directory = receipt_dir(&out);
    let subject = match subject {
        Some(subject) => subject,
        None => default_subject(&inputs, &out),
    };
    let text = receipt_text(&inputs, &directory, &subject)?;
    std::fs::write(&out, &text)
        .map_err(|error| anyhow::anyhow!("cannot write {}: {error}", out.display()))?;
    Ok(ExitCode::SUCCESS)
}

/// Builds the receipt's canonical bytes for `inputs`, resolving artifact
/// files next to `directory` (the receipt's own directory) and asserting
/// byte-equality with any committed golden found there. Writes nothing —
/// `--check-fresh` compares this against the committed receipt.
fn receipt_text(inputs: &[PathBuf], directory: &Path, subject: &str) -> anyhow::Result<String> {
    let emissions = emit_artifacts(inputs)?;
    let sources = collect_sources(inputs)?;
    let artifacts = emissions
        .iter()
        .map(|emission| artifact_entry(emission, directory))
        .collect::<anyhow::Result<Vec<ArtifactRef>>>()?;
    let (register, trusted) = register_snapshot()?;

    let mut receipt = Receipt {
        format_version: rex_evidence::receipt::FORMAT_VERSION,
        kind: rex_evidence::receipt::KIND.to_string(),
        subject: subject.to_string(),
        generator: Generator {
            name: "rexlang".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        },
        inputs: inputs
            .iter()
            .map(|input| repo_relative(input))
            .collect::<anyhow::Result<Vec<String>>>()?,
        sources,
        artifacts,
        register,
        trusted,
        limits: rex_evidence::receipt::LIMITS
            .iter()
            .map(|limit| (*limit).to_string())
            .collect(),
    };
    receipt.inputs.sort();
    receipt.inputs.dedup();
    receipt.artifacts.sort_by_key(artifact_key);
    let text = receipt.to_json_pretty()?;
    // A receipt that does not pass its own validator must never be written:
    // the emitter and the verifier share one contract.
    let mut read = |relative: &str| std::fs::read(directory.join(relative)).ok();
    rex_evidence::receipt::check(&text, &mut read).map_err(|violations| {
        anyhow::anyhow!(
            "the generated receipt fails its own validation: {}",
            violations
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<String>>()
                .join("; ")
        )
    })?;
    Ok(text)
}

/// One input's emitted artifact: the input path, its receipt-local kind, and
/// the exact bytes the CLI writes.
struct Emission {
    input: PathBuf,
    kind: &'static str,
    json: String,
}

/// Compiles every input and collects its emitted artifact bytes.
fn emit_artifacts(inputs: &[PathBuf]) -> anyhow::Result<Vec<Emission>> {
    let mut emissions = Vec::new();
    let mut failed = 0usize;
    for input in inputs {
        let kind = artifact_kind(input)?;
        let json = if is_ifml_file(input) {
            match ifml_manifest_json(input)? {
                Some(json) => json,
                None => {
                    failed += 1;
                    continue;
                }
            }
        } else {
            match compile_surface(std::slice::from_ref(input), None, false)? {
                Some(outcome) => outcome.json,
                None => {
                    failed += 1;
                    continue;
                }
            }
        };
        emissions.push(Emission {
            input: input.clone(),
            kind,
            json,
        });
    }
    if failed > 0 {
        anyhow::bail!(
            "{failed} of {} inputs failed; no receipt written",
            inputs.len()
        );
    }
    Ok(emissions)
}

/// The receipt-local kind for an input's extension (the closed
/// [`rex_evidence::receipt::ARTIFACT_KINDS`] vocabulary).
fn artifact_kind(input: &Path) -> anyhow::Result<&'static str> {
    let extension = input
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    Ok(match extension {
        "mox" => "irModel",
        "actor" => "actorModel",
        "ddd" => "dddDesign",
        "evt" => "eventContract",
        "uc" => "useCaseModel",
        "deploy" => "deployModel",
        "dmn" => "decisionModel",
        "cmmn" => "cmmnModel",
        "ifml" => "ifmlManifest",
        other => anyhow::bail!(
            "verify does not support .{other} inputs ({}); pass model surfaces",
            input.display()
        ),
    })
}

/// One `.ifml` input's control/route manifest JSON — the `rexlang manifest`
/// flow: compile, gate on the checker's errors, then derive the manifest
/// from the checked compilation. Warnings render but never fail the file
/// (the `check`/`build` discipline).
fn ifml_manifest_json(file: &Path) -> anyhow::Result<Option<String>> {
    let inputs = read_ifml_inputs(file, None)?;
    let compilation =
        match rex_ifml::compile_ifml_str(&inputs.path, &inputs.source, &inputs.ifml_imports) {
            Ok(compilation) => compilation,
            Err(diagnostics) => {
                report_ifml_diagnostics(&inputs.sources, &diagnostics);
                return Ok(None);
            }
        };
    let binding = rex_ifml::check_ifml_with_decisions(
        &compilation,
        inputs.domains.as_ref(),
        &inputs.actors,
        &inputs.decisions,
    );
    let errors = binding
        .iter()
        .filter(|diagnostic| diagnostic.severity.is_error())
        .count();
    if !binding.is_empty() {
        let sources: Vec<(String, String)> = compilation
            .index
            .files
            .iter()
            .map(|file| (file.path.clone(), file.text.clone()))
            .collect();
        report_ifml_diagnostics(&sources, &binding);
    }
    if errors > 0 {
        return Ok(None);
    }
    let manifest =
        rex_ifml::manifest_ifml(&compilation, &inputs.ifml_imports, inputs.domains.as_ref());
    Ok(Some(manifest.to_json_pretty()?))
}

/// The receipt entry for one emission: hash the exact emitted bytes, assert
/// byte-equality with the committed golden of the same name next to the
/// receipt when one exists, and record the artifact's own `formatVersion`.
fn artifact_entry(emission: &Emission, directory: &Path) -> anyhow::Result<ArtifactRef> {
    let Emission { input, kind, json } = emission;
    let sha256 = sha256_hex(json.as_bytes());
    let golden = golden_name(input, kind)?;
    let path = match std::fs::read(directory.join(&golden)) {
        Ok(committed) => {
            if committed != json.as_bytes() {
                anyhow::bail!(
                    "the in-memory {kind} artifact for {} hashes to {sha256}, but the committed \
                     golden {} hashes to {} — the artifact drifted; regenerate it \
                     (REX_UPDATE_FIXTURES=1) or fix the model",
                    input.display(),
                    directory.join(&golden).display(),
                    sha256_hex(&committed),
                );
            }
            Some(golden)
        }
        Err(_) => None,
    };
    Ok(ArtifactRef {
        path,
        kind: (*kind).to_string(),
        format_version: artifact_format_version(json)?,
        sha256,
    })
}

/// The committed-golden file name for one input: `<stem>.<ext>.json` — the
/// `examples/artifacts/` convention — and `<stem>.manifest.json` for an
/// `.ifml` flow's manifest. A file that does not exist next to the receipt
/// makes the artifact digest-only.
fn golden_name(input: &Path, kind: &str) -> anyhow::Result<String> {
    let stem = input
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or_else(|| anyhow::anyhow!("{} has no file stem", input.display()))?;
    if kind == "ifmlManifest" {
        return Ok(format!("{stem}.manifest.json"));
    }
    let extension = input
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    Ok(format!("{stem}.{extension}.json"))
}

/// The artifact's own wire `formatVersion` (readers gate on it; absent is
/// reported as 0 so the validator names it).
fn artifact_format_version(json: &str) -> anyhow::Result<u32> {
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|error| anyhow::anyhow!("emitted artifact is not JSON: {error}"))?;
    let version = value
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(0);
    Ok(u32::try_from(version).unwrap_or(u32::MAX))
}

/// The artifact sort key — `(kind, path, sha256)`, which the validator
/// re-checks so a reader can recompute the document's order.
fn artifact_key(artifact: &ArtifactRef) -> (String, String, String) {
    (
        artifact.kind.clone(),
        artifact.path.clone().unwrap_or_default(),
        artifact.sha256.clone(),
    )
}

// --- the source closure -------------------------------------------------------

/// Every file the compile read, hashed and repo-relative, sorted.
///
/// The recording pass re-runs each input through the CLI's own readers with
/// a [`Project`] snapshot: every import fetch (`read_*_pair`,
/// `read_ifml_inputs`) lands a resolved key in the snapshot, which is then
/// the closure's spine. The completion pass adds the three families those
/// collectors read outside the snapshot funnel, from the same AST
/// declarations they read: `import schema`/`import sigil` targets (plus the
/// rosetta candidate pool sigil discovery reads) and a `.dmn` model's
/// external decision tables.
fn collect_sources(inputs: &[PathBuf]) -> anyhow::Result<Vec<SourceFile>> {
    let mut project = Project::gather(inputs);
    for input in inputs {
        record_imports(input, &mut project)?;
    }
    let mut paths: BTreeSet<PathBuf> = inputs.iter().cloned().collect();
    paths.extend(
        project
            .keys_with_extension("")
            .into_iter()
            .map(PathBuf::from),
    );
    let mut discovered = BTreeSet::new();
    for path in &paths {
        complete_closure(path, &mut discovered);
    }
    paths.extend(discovered);

    let mut hashed: BTreeMap<String, String> = BTreeMap::new();
    for path in paths {
        let relative = repo_relative(&path)?;
        let bytes = std::fs::read(&path)
            .map_err(|error| anyhow::anyhow!("cannot read source {}: {error}", path.display()))?;
        hashed.entry(relative).or_insert_with(|| sha256_hex(&bytes));
    }
    Ok(hashed
        .into_iter()
        .map(|(path, sha256)| SourceFile { path, sha256 })
        .collect())
}

/// Reads one input through the CLI's own reader so its import fetches land
/// in the recording snapshot. `.mox` has no domain imports (its schema and
/// sigil targets are added by [`complete_closure`]).
fn record_imports(input: &Path, project: &mut Project) -> anyhow::Result<()> {
    if is_ifml_file(input) {
        read_ifml_inputs(input, Some(project))?;
    } else if is_ddd_file(input) {
        read_ddd_design(input, Some(project))?;
    } else if is_actor_file(input) {
        read_actor_pair_in(input, Some(project))?;
    } else if is_evt_file(input) {
        read_evt_pair(input, Some(project))?;
    } else if is_uc_file(input) {
        read_uc_pair(input, Some(project))?;
    } else if is_cmmn_file(input) {
        read_cmmn_pair(input, Some(project))?;
    } else if is_deploy_file(input) {
        read_deploy_pair(input, Some(project))?;
    } else if is_dmn_file(input) {
        read_dmn_pair(input, Some(project))?;
    }
    Ok(())
}

/// Adds the files a surface's collectors read directly from disk (see
/// [`collect_sources`]): a `.mox` model's `import schema`/`import sigil`
/// targets and the rosetta candidates sigil discovery reads, and a `.dmn`
/// model's `logic table` files.
fn complete_closure(path: &Path, out: &mut BTreeSet<PathBuf>) {
    let extension = path.extension().and_then(|extension| extension.to_str());
    if extension != Some("mox") && extension != Some("dmn") {
        return;
    }
    let Ok(source) = std::fs::read_to_string(path) else {
        return;
    };
    let directory = dir_of(path);
    if extension == Some("mox") {
        let declared = rex_syntax::parse(&source)
            .ast
            .map(|ast| {
                ast.declarations
                    .iter()
                    .filter_map(|decl| match decl {
                        rex_syntax::ast::Decl::ImportSchema(import) => {
                            Some((import.path.clone(), import.kind))
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for (import, kind) in declared {
            let resolved = directory.join(&import);
            if resolved.is_file() {
                out.insert(resolved.clone());
            }
            if kind == rex_syntax::ast::ImportKind::Sigil {
                collect_rosetta_candidates(&resolved, out);
            }
        }
        return;
    }
    let tables = rex_syntax::parse_dmn(&source)
        .ast
        .and_then(|ast| ast.model)
        .map(|model| {
            model
                .decisions
                .iter()
                .filter_map(|decision| match &decision.logic {
                    Some(rex_syntax::DmnLogic::Table { source, .. }) => Some(source.clone()),
                    _ => None,
                })
                .collect::<Vec<String>>()
        })
        .unwrap_or_default();
    for table in tables {
        let resolved = directory.join(&table);
        if resolved.is_file() {
            out.insert(resolved);
        }
    }
}

/// The rosetta candidate pool `import sigil` discovery reads: every
/// `*.rosetta` file under the named import's directory, dot and `target`
/// directories skipped, depth-capped (the collector's documented policy).
/// A sibling candidate changes the lowering, so it is a source like any
/// other.
fn collect_rosetta_candidates(named: &Path, out: &mut BTreeSet<PathBuf>) {
    const MAX_DEPTH: usize = 16;
    fn walk(dir: &Path, depth: usize, named: &Path, out: &mut BTreeSet<PathBuf>) {
        if depth > MAX_DEPTH {
            return;
        }
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<std::fs::DirEntry> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        for entry in entries {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if name.starts_with('.') || name == "target" {
                continue;
            }
            if path.is_dir() {
                walk(&path, depth + 1, named, out);
            } else if name.ends_with(".rosetta") && path != named {
                out.insert(path);
            }
        }
    }
    walk(&dir_of(named), 0, named, out);
}

/// The importing file's directory, `"."` for a bare file name (the import
/// resolution rule every reader follows).
fn dir_of(path: &Path) -> PathBuf {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."))
        .to_path_buf()
}

/// A path as the receipt spells it: lexically normalized (`.`, `..` and
/// duplicate separators resolved, no filesystem access) and **relative to
/// the working directory** — the repository root. An absolute input is
/// stripped of the canonical working directory; one that lives outside it
/// cannot be receipted (the receipt never records an environment).
fn repo_relative(path: &Path) -> anyhow::Result<String> {
    let normalized = normalize(path);
    let relative = if normalized.is_absolute() {
        let root = std::env::current_dir()
            .and_then(|cwd| cwd.canonicalize())
            .map_err(|error| anyhow::anyhow!("cannot resolve the working directory: {error}"))?;
        let canonical = normalized
            .canonicalize()
            .map_err(|error| anyhow::anyhow!("cannot resolve {}: {error}", normalized.display()))?;
        canonical
            .strip_prefix(&root)
            .map_err(|_| {
                anyhow::anyhow!(
                    "{} is outside the working directory ({}); run rexlang from the repository root",
                    normalized.display(),
                    root.display()
                )
            })?
            .to_path_buf()
    } else {
        normalized
    };
    let mut out = String::new();
    for component in relative.components() {
        match component {
            Component::Normal(part) => {
                if !out.is_empty() {
                    out.push('/');
                }
                out.push_str(&part.to_string_lossy());
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                anyhow::bail!(
                    "{} cannot be spelled repo-relative; receipts never record paths outside the repository",
                    path.display()
                );
            }
        }
    }
    if out.is_empty() {
        anyhow::bail!("{} has no repo-relative spelling", path.display());
    }
    Ok(out)
}

/// Lexical path normalization: `a/./b` → `a/b`, `a/x/../b` → `a/b`; the
/// filesystem is not consulted (the path may not exist yet).
fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

// --- the register snapshot ----------------------------------------------------

/// The register the receipt reports, read from [`REGISTER_PATH`] and
/// validated before it is embedded: a receipt must never ship a register
/// that violates its own rules. Returns the snapshot and the `#[trusted]`
/// assumption list.
fn register_snapshot() -> anyhow::Result<(RegisterSnapshot, Vec<TrustedRef>)> {
    let bytes = std::fs::read(REGISTER_PATH).map_err(|error| {
        anyhow::anyhow!(
            "cannot read the proof-obligation register {REGISTER_PATH}: {error} — run rexlang from the repository root"
        )
    })?;
    let text = String::from_utf8(bytes.clone())
        .map_err(|error| anyhow::anyhow!("{REGISTER_PATH} is not UTF-8: {error}"))?;
    let register = rex_evidence::register::parse(&text)
        .map_err(|error| anyhow::anyhow!("{REGISTER_PATH} does not parse: {error}"))?;
    let violations = rex_evidence::register::validate(&register);
    if let Some(violation) = violations.first() {
        anyhow::bail!(
            "the proof-obligation register is invalid ({violation}); fix {REGISTER_PATH} before emitting a receipt"
        );
    }
    let mut trusted: Vec<TrustedRef> = register
        .trusted
        .iter()
        .map(|row| TrustedRef {
            item: row.item.clone(),
            row: row.row.clone(),
        })
        .collect();
    trusted.sort_by(|left, right| (&left.item, &left.row).cmp(&(&right.item, &right.row)));
    let snapshot = RegisterSnapshot {
        source: REGISTER_PATH.to_string(),
        sha256: sha256_hex(&bytes),
        reviewed_at: register.reviewed_at,
        rows: register
            .obligations
            .iter()
            .map(|obligation| RegisterRow {
                id: obligation.id.clone(),
                claim: obligation.claim.clone(),
                status: obligation.status.clone(),
                evidence: obligation
                    .evidence
                    .iter()
                    .map(|evidence| RowEvidence {
                        kind: evidence.kind.clone(),
                        pointer: evidence.pointer.clone(),
                    })
                    .collect(),
                trigger: obligation.trigger.clone(),
                review: obligation.review.clone(),
                // Issue #197 asks for a mutant fact per tested row; the
                // anti-vacuity mutant pipeline (#208) that produces it does
                // not exist yet, so the honest interim is `null` plus the
                // `mutants-unrecorded` limit — never a silent `true`.
                mutant_killed: None,
            })
            .collect(),
    };
    Ok((snapshot, trusted))
}

// --- validation modes ---------------------------------------------------------

/// `--check`: validate the receipt standalone. No register file, no models,
/// no toolchain, no network — artifacts are re-hashed from the files next
/// to the receipt, everything else is schema and vocabulary.
fn check_receipt(path: &Path) -> anyhow::Result<ExitCode> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", path.display()))?;
    let directory = receipt_dir(path);
    let mut read = |relative: &str| std::fs::read(directory.join(relative)).ok();
    match rex_evidence::receipt::check(&text, &mut read) {
        Ok(receipt) => {
            println!("OK {} ({})", path.display(), receipt.subject);
            Ok(ExitCode::SUCCESS)
        }
        Err(violations) => {
            for violation in violations {
                eprintln!("{}: {violation}", path.display());
            }
            Ok(ExitCode::FAILURE)
        }
    }
}

/// `--check-fresh`: regenerate from the receipt's own `inputs` and
/// byte-compare against the committed file. The CI gate (issue #207): a
/// source, artifact, or register change without regeneration fails here with
/// the exact command to run.
fn verify_fresh(path: &Path) -> anyhow::Result<ExitCode> {
    let text = std::fs::read_to_string(path)
        .map_err(|error| anyhow::anyhow!("cannot read {}: {error}", path.display()))?;
    let receipt = Receipt::from_json(&text)
        .map_err(|error| anyhow::anyhow!("{}: not a valid receipt: {error}", path.display()))?;
    let inputs: Vec<PathBuf> = receipt
        .inputs
        .iter()
        .map(PathBuf::from)
        .collect::<Vec<PathBuf>>();
    if inputs.is_empty() {
        anyhow::bail!(
            "{}: the receipt names no inputs to regenerate from",
            path.display()
        );
    }
    let directory = receipt_dir(path);
    let regenerate = format!(
        "rexlang verify {} -o {}",
        receipt.inputs.join(" "),
        path.display()
    );
    match receipt_text(&inputs, &directory, &receipt.subject) {
        Ok(fresh) if fresh == text => {
            println!("OK {} is fresh", path.display());
            Ok(ExitCode::SUCCESS)
        }
        Ok(_) => {
            eprintln!(
                "stale receipt: {} does not match a fresh generation from its sources.",
                path.display()
            );
            eprintln!("regenerate it with:\n  {regenerate}");
            Ok(ExitCode::FAILURE)
        }
        Err(error) => {
            eprintln!(
                "stale receipt: {} cannot be regenerated: {error:#}",
                path.display()
            );
            eprintln!("regenerate it with:\n  {regenerate}");
            Ok(ExitCode::FAILURE)
        }
    }
}

// --- small helpers ------------------------------------------------------------

/// The receipt's own directory (`"."` for a bare file name): artifact paths
/// in the receipt are relative to it, so a receipt plus its siblings is a
/// self-contained bundle.
fn receipt_dir(path: &Path) -> PathBuf {
    dir_of(path)
}

/// The default subject: the inputs' common file stem (`support` for
/// `examples/support.*`), else the receipt file's stem with a trailing
/// `.receipt` removed (`support` for `examples/artifacts/support.receipt.json`).
fn default_subject(inputs: &[PathBuf], receipt: &Path) -> String {
    let stems: BTreeSet<String> = inputs
        .iter()
        .filter_map(|input| input.file_stem())
        .filter_map(|stem| stem.to_str())
        .map(str::to_string)
        .collect();
    if stems.len() == 1 {
        if let Some(stem) = stems.into_iter().next() {
            return stem;
        }
    }
    let stem = receipt
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("receipt");
    stem.strip_suffix(".receipt").unwrap_or(stem).to_string()
}
