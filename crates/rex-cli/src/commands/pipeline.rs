//! The tail helpers shared by `check`, `ir`, and `build`: the per-surface
//! compile-and-report pipeline all three commands share, the multi-file
//! `.mox` compile-and-report pipeline, and the artifact-JSON output tail.

use std::path::{Path, PathBuf};

use rex_driver::{
    compile_actors_str, compile_cmmn_str, compile_ddd_str, compile_deploy_str, compile_evt_str,
    compile_usecase_str,
};

use crate::inputs::{
    compile_sources, is_actor_file, is_cmmn_file, is_ddd_file, is_deploy_file, is_evt_file,
    is_ifml_file, is_uc_file, read_actor_pair_in, read_cmmn_pair, read_ddd_design,
    read_deploy_pair, read_evt_pair, read_ifml_inputs, read_sources, read_uc_pair,
};
use crate::project::{Artifact, Project};
use crate::report::{
    report_actor_diagnostics, report_cmmn_diagnostics, report_ddd_diagnostics,
    report_deploy_diagnostics, report_evt_diagnostics, report_ifml_diagnostics,
    report_multi_diagnostics, report_usecase_diagnostics,
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
/// pipeline `check`/`ir` always ran.
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
        if !binding.is_empty() {
            let sources: Vec<(String, String)> = compilation
                .index
                .files
                .iter()
                .map(|file| (file.path.clone(), file.text.clone()))
                .collect();
            report_ifml_diagnostics(&sources, &binding);
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
        }));
    }
    if files.len() == 1 && is_ddd_file(&files[0]) {
        let design = read_ddd_design(&files[0], project.as_deref_mut())?;
        let compilation = compile_ddd_str(
            &design.path,
            &design.source,
            &design.domains,
            &design.imports(),
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
        }));
    }
    if files.len() == 1 && is_actor_file(&files[0]) {
        let pair = read_actor_pair_in(&files[0], project.as_deref_mut())?;
        let compilation =
            compile_actors_str(&pair.path, &pair.source, &pair.domains, &pair.imports());
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
    Ok(Some(SurfaceOutcome { ok, json }))
}
