//! The tail helpers shared by `check`, `ir`, and `build`: the per-surface
//! compile-and-report pipeline all three commands share, the multi-file
//! `.mox` compile-and-report pipeline, and the artifact-JSON output tail.

use std::path::{Path, PathBuf};

use rex_driver::{
    compile_actors_str, compile_actors_str_with_designs, compile_cmmn_str, compile_ddd_str,
    compile_ddd_str_with_decisions, compile_deploy_str, compile_dmn_str, compile_evt_str,
    compile_usecase_str,
};

use crate::inputs::{
    compile_sources, is_actor_file, is_cmmn_file, is_ddd_file, is_deploy_file, is_dmn_file,
    is_evt_file, is_ifml_file, is_uc_file, read_actor_pair_in, read_cmmn_pair, read_ddd_design,
    read_deploy_pair, read_dmn_pair, read_evt_pair, read_ifml_inputs, read_sources, read_uc_pair,
};
use crate::project::{Artifact, Project};
use crate::report::{
    report_actor_diagnostics, report_cmmn_diagnostics, report_ddd_diagnostics,
    report_deploy_diagnostics, report_dmn_diagnostics, report_evt_diagnostics,
    report_ifml_diagnostics, report_multi_diagnostics, report_usecase_diagnostics,
};

/// The multi-file `.mox` pipeline: read the sources, compile them as one
/// multi-package model, and render the diagnostics grouped per file (the
/// `.mox` diagnostics first, then the `import sigil` diagnostics against
/// their rosetta sources).
pub(crate) fn compile_reported_sources(
    files: &[PathBuf],
) -> anyhow::Result<(Vec<(String, String)>, rex_driver::MultiCompilation)> {
    let sources = read_sources(files)?;
    let (compilation, sigil_sources) = compile_sources(&sources)?;
    report_multi_diagnostics(
        &sources,
        &sigil_sources.sources,
        &compilation.diagnostics,
        &compilation.sigil_diagnostics,
    );
    Ok((sources, compilation))
}

/// The `ir` emission tail: write `json` to `out`, or print it when no
/// output path was given.
pub(crate) fn emit_json(json: String, out: Option<&Path>) -> anyhow::Result<()> {
    match out {
        Some(out_path) => std::fs::write(out_path, json)?,
        None => println!("{json}"),
    }
    Ok(())
}

/// One compile outcome shared by `check`, `ir`, and `build`: the `OK`
/// lines `check` prints and the artifact JSON `ir`/`build` emit — one
/// compile feeds all three.
pub(crate) struct SurfaceOutcome {
    /// The compiling sources' driver paths; `check` prints one
    /// `OK <path>` line each (a multi-`.mox` model prints one per file).
    pub(crate) ok: Vec<String>,
    /// The artifact JSON exactly `rexlang ir` emits for the surface.
    pub(crate) json: String,
    /// How many warning-severity diagnostics were rendered. Warnings never
    /// fail a compile; `check --deny-warnings` turns them into failures.
    pub(crate) warnings: usize,
}

/// The compiled `.actor` artifacts of the project snapshot (excluding the
/// file being checked): the capability pool the design engine's rule 10
/// resolves against at compile time (rexlang#128's J3). Siblings compile
/// lookup-before-compute through the project's own readers (their imports
/// resolve map-first from the snapshot); a sibling that fails to compile
/// contributes nothing — parity with the union pass, which keeps only the
/// successful artifacts.
fn gather_sibling_actors(except: &Path, project: &mut Project) -> Vec<rex_ir::ActorModel> {
    let mut models = Vec::new();
    for key in project.keys_with_extension(".actor") {
        if Path::new(&key) == except {
            continue;
        }
        match project.peek(&key) {
            Some(Artifact::Actor(model)) => models.push(model),
            Some(_) => {}
            None => {
                let Ok(pair) = read_actor_pair_in(Path::new(&key), Some(project)) else {
                    continue;
                };
                let compilation =
                    compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
                if let Some(model) = compilation.model {
                    project.store(&key, &Artifact::Actor(model.clone()));
                    models.push(model);
                }
            }
        }
    }
    models
}

/// The compiled `.dmn` artifacts of the project snapshot (rexlang#78): the
/// decision services the `.ddd` decision links resolve against at compile
/// time — rule 17's decisions bucket. A sibling that fails to compile
/// contributes nothing (the [`gather_sibling_actors`] discipline).
fn gather_sibling_decisions(
    except: &Path,
    project: &mut Project,
) -> Vec<(String, rex_ir::dmn::DmnModel)> {
    let mut models = Vec::new();
    for key in project.keys_with_extension(".dmn") {
        if Path::new(&key) == except {
            continue;
        }
        match project.peek(&key) {
            Some(Artifact::Decision(model)) => models.push((key.clone(), model.clone())),
            Some(_) => {}
            None => {
                let Ok(pair) = read_dmn_pair(Path::new(&key), Some(project)) else {
                    continue;
                };
                let compilation = compile_dmn_str(
                    &pair.pair.path,
                    &pair.pair.source,
                    &pair.pair.domains,
                    &pair.pair.imports(),
                    &pair.tables,
                    &pair.events,
                );
                if let Some(model) = compilation.model {
                    project.store(&key, &Artifact::Decision(model.clone()));
                    models.push((key.clone(), model));
                }
            }
        }
    }
    models
}

/// The compiled `.ddd` artifacts of the project snapshot (excluding the
/// file being checked): the carrier set grant coverage resolves against at
/// compile time (rexlang#128's J4). Gathered exactly like
/// [`gather_sibling_actors`].
fn gather_sibling_designs(except: &Path, project: &mut Project) -> Vec<rex_ir::ddd::DddModel> {
    let mut models = Vec::new();
    for key in project.keys_with_extension(".ddd") {
        if Path::new(&key) == except {
            continue;
        }
        match project.peek(&key) {
            Some(Artifact::Design(model)) => models.push(model),
            Some(_) => {}
            None => {
                let Ok(design) = read_ddd_design(Path::new(&key), Some(project)) else {
                    continue;
                };
                let compilation = compile_ddd_str(
                    &design.path,
                    &design.source,
                    &design.domains,
                    &design.imports(),
                );
                if let Some(model) = compilation.model {
                    project.store(&key, &Artifact::Design(model.clone()));
                    models.push(model);
                }
            }
        }
    }
    models
}

/// The per-surface pipeline behind `check`, `ir`, and `build`: read the
/// file within its project context (or the explicit argument list as its
/// own model), compile, and render diagnostics. `Ok(None)` means the file
/// did not compile — the diagnostics are already rendered. `expand` is
/// `ir --expand`'s opt-in inlining of `.ifml` module uses; check and build
/// pass `false`.
///
/// Within a project, the file's compiled artifact lands in the snapshot's
/// cache ([`Project::store`]) so sibling consumers reuse it
/// lookup-before-compute; without one this is exactly the per-file
/// pipeline `check`/`ir` always ran. Within a project the `.ddd` and
/// `.actor` pipelines additionally resolve the capability joins at compile
/// time (rexlang#128) against the sibling `.actor`/`.ddd` artifacts of the
/// snapshot; without one those joins stay silent (opt-in house style).
pub(crate) fn compile_surface(
    files: &[PathBuf],
    mut project: Option<&mut Project>,
    expand: bool,
) -> anyhow::Result<Option<SurfaceOutcome>> {
    if files.len() == 1 && is_ifml_file(&files[0]) {
        let inputs = read_ifml_inputs(&files[0], project.as_deref_mut())?;
        let compilation =
            match rex_ifml::compile_ifml_str(&inputs.path, &inputs.source, &inputs.ifml_imports) {
                Ok(compilation) => compilation,
                Err(diagnostics) => {
                    report_ifml_diagnostics(&inputs.sources, &diagnostics);
                    return Ok(None);
                }
            };
        let binding =
            rex_ifml::check_ifml_with_actors(&compilation, inputs.domains.as_ref(), &inputs.actors);
        // Warnings (the `ifml.lints` band) render but never fail the file;
        // only error-severity diagnostics do. `check --deny-warnings`
        // flips the rule via the outcome's warning count.
        let warnings = binding
            .iter()
            .filter(|diagnostic| !diagnostic.severity.is_error())
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
        if warnings < binding.len() {
            return Ok(None);
        }
        if let Some(project) = project.as_deref_mut() {
            project.store(
                &files[0].display().to_string(),
                &Artifact::Flow(compilation.model.clone()),
            );
        }
        let model = if expand {
            rex_ifml::expand_ifml(&compilation, &inputs.ifml_imports)
        } else {
            compilation.model
        };
        return Ok(Some(SurfaceOutcome {
            ok: vec![inputs.path.clone()],
            json: model.to_json_pretty()?,
            warnings,
        }));
    }
    if files.len() == 1 && is_ddd_file(&files[0]) {
        let design = read_ddd_design(&files[0], project.as_deref_mut())?;
        // rexlang#128 (J3 at compile time): the capability clauses resolve
        // against every `.actor` artifact of the project snapshot — the
        // empty slice is the exact `compile_ddd_str` behavior. rexlang#78
        // (rule 17): the `invokes`/`inject decision` clauses resolve the
        // same way against every `.dmn` artifact of the snapshot.
        let (sibling_actors, sibling_decisions) = match project.as_deref_mut() {
            Some(project) => (
                gather_sibling_actors(&files[0], project),
                gather_sibling_decisions(&files[0], project),
            ),
            None => (Vec::new(), Vec::new()),
        };
        let compilation = compile_ddd_str_with_decisions(
            &design.path,
            &design.source,
            &design.domains,
            &design.imports(),
            &sibling_actors,
            &sibling_decisions,
        );
        report_ddd_diagnostics(&design, &compilation);
        let Some(design_model) = compilation.model else {
            return Ok(None);
        };
        let artifact = Artifact::Design(design_model.clone());
        if let Some(project) = project.as_deref_mut() {
            project.store(&files[0].display().to_string(), &artifact);
        }
        return Ok(Some(SurfaceOutcome {
            ok: vec![design.path.clone()],
            json: artifact.to_json_pretty()?,
            warnings: 0,
        }));
    }
    if files.len() == 1 && is_actor_file(&files[0]) {
        let pair = read_actor_pair_in(&files[0], project.as_deref_mut())?;
        // rexlang#128 (J4 at compile time): grant coverage resolves against
        // every `.ddd` artifact of the project snapshot — the empty slice
        // is the exact `compile_actors_str` behavior.
        let sibling_designs = match project.as_deref_mut() {
            Some(project) => gather_sibling_designs(&files[0], project),
            None => Vec::new(),
        };
        let compilation = compile_actors_str_with_designs(
            &pair.path,
            &pair.source,
            &pair.domains,
            &pair.imports(),
            &sibling_designs,
        );
        report_actor_diagnostics(&pair, &compilation);
        let Some(actor_model) = compilation.model else {
            return Ok(None);
        };
        let artifact = Artifact::Actor(actor_model.clone());
        if let Some(project) = project.as_deref_mut() {
            project.store(&files[0].display().to_string(), &artifact);
        }
        return Ok(Some(SurfaceOutcome {
            ok: vec![pair.path.clone()],
            json: artifact.to_json_pretty()?,
            warnings: 0,
        }));
    }
    if files.len() == 1 && is_evt_file(&files[0]) {
        let pair = read_evt_pair(&files[0], project.as_deref_mut())?;
        let compilation = compile_evt_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
        report_evt_diagnostics(&pair, &compilation);
        let Some(event_model) = compilation.model else {
            return Ok(None);
        };
        let artifact = Artifact::Event(event_model.clone());
        if let Some(project) = project.as_deref_mut() {
            project.store(&files[0].display().to_string(), &artifact);
        }
        return Ok(Some(SurfaceOutcome {
            ok: vec![pair.path.clone()],
            json: artifact.to_json_pretty()?,
            warnings: 0,
        }));
    }
    if files.len() == 1 && is_deploy_file(&files[0]) {
        let pair = read_deploy_pair(&files[0], project.as_deref_mut())?;
        let compilation = compile_deploy_str(&pair.path, &pair.source, &pair.designs, &pair.flows);
        report_deploy_diagnostics(&pair, &compilation);
        let Some(deploy_model) = compilation.model else {
            return Ok(None);
        };
        let artifact = Artifact::Deploy(deploy_model.clone());
        if let Some(project) = project.as_deref_mut() {
            project.store(&files[0].display().to_string(), &artifact);
        }
        return Ok(Some(SurfaceOutcome {
            ok: vec![pair.path.clone()],
            json: artifact.to_json_pretty()?,
            warnings: 0,
        }));
    }
    if files.len() == 1 && is_uc_file(&files[0]) {
        let pair = read_uc_pair(&files[0], project.as_deref_mut())?;
        let compilation = compile_usecase_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
        );
        report_usecase_diagnostics(&pair.pair, &compilation);
        let Some(use_case_model) = compilation.model else {
            return Ok(None);
        };
        let artifact = Artifact::UseCase(use_case_model.clone());
        if let Some(project) = project.as_deref_mut() {
            project.store(&files[0].display().to_string(), &artifact);
        }
        return Ok(Some(SurfaceOutcome {
            ok: vec![pair.pair.path.clone()],
            json: artifact.to_json_pretty()?,
            warnings: 0,
        }));
    }
    if files.len() == 1 && is_cmmn_file(&files[0]) {
        let pair = read_cmmn_pair(&files[0], project.as_deref_mut())?;
        let compilation = compile_cmmn_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.designs,
            &pair.actors,
            &pair.events,
            &pair.decisions,
        );
        report_cmmn_diagnostics(&pair.pair, &compilation);
        let Some(case_model) = compilation.model else {
            return Ok(None);
        };
        let artifact = Artifact::Case(case_model.clone());
        if let Some(project) = project.as_deref_mut() {
            project.store(&files[0].display().to_string(), &artifact);
        }
        return Ok(Some(SurfaceOutcome {
            ok: vec![pair.pair.path.clone()],
            json: artifact.to_json_pretty()?,
            warnings: 0,
        }));
    }
    if files.len() == 1 && is_dmn_file(&files[0]) {
        let pair = read_dmn_pair(&files[0], project.as_deref_mut())?;
        let compilation = compile_dmn_str(
            &pair.pair.path,
            &pair.pair.source,
            &pair.pair.domains,
            &pair.pair.imports(),
            &pair.tables,
            &pair.events,
        );
        report_dmn_diagnostics(&pair.pair, &compilation);
        let Some(decision_model) = compilation.model else {
            return Ok(None);
        };
        let artifact = Artifact::Decision(decision_model.clone());
        if let Some(project) = project.as_deref_mut() {
            project.store(&files[0].display().to_string(), &artifact);
        }
        return Ok(Some(SurfaceOutcome {
            ok: vec![pair.pair.path.clone()],
            json: artifact.to_json_pretty()?,
            warnings: 0,
        }));
    }
    let (sources, compilation) = compile_reported_sources(files)?;
    let Some(model) = compilation.model else {
        return Ok(None);
    };
    let ok = sources.iter().map(|(path, _)| path.clone()).collect();
    let json = if let [file] = files {
        let artifact = Artifact::Domain(model.clone());
        if let Some(project) = project {
            project.store(&file.display().to_string(), &artifact);
        }
        artifact.to_json_pretty()?
    } else {
        model.to_json_pretty()?
    };
    Ok(Some(SurfaceOutcome {
        ok,
        json,
        warnings: 0,
    }))
}
