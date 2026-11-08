//! The `.deploy` entry point: deployment compilation against pre-compiled
//! design and flow artifacts.

use crate::compilation::DeployCompilation;
use crate::deploy;
use crate::session::{parse_deploy_query, Database, SourceFile};

/// Compiles a `.deploy` deployment file in one call.
///
/// Each `import` in the file must resolve to a provided entry:
/// `designs` holds the imported `.ddd` designs and `flows` the imported
/// `.ifml` flows, each as `(path, artifact)` pairs. An import matches a
/// provided entry by exact path or by lexical resolution relative to the
/// deploy file's directory (the same rule
/// [`compile_actors_str`](crate::compile_actors_str) applies) — the CLI
/// passes resolved paths, so bindings written as `"../ddd/library.ddd"`
/// match the file that import resolves to. Extra pairs no import names are
/// ignored; an import no provided pair names is the error
/// ``imported file "<path>" was not provided``.
///
/// ```
/// let designs: Vec<(String, rex_ir::ddd::DddModel)> = Vec::new();
/// let flows: Vec<(String, rex_ir::ifml::IfmlModel)> = Vec::new();
/// let source = concat!(
///     "application Svc {\n",
///     "    component api: api { replicas: 2 }\n",
///     "    component db: database { engine: sqlite }\n",
///     "    connects api -> db\n",
///     "}\n",
///     "\n",
///     "profile local {\n",
///     "    target: standalone\n",
///     "    require (api.replicas >= 1)\n",
///     "}\n",
///     "\n",
///     "deployment dev for Svc {\n",
///     "    use local\n",
///     "}\n",
/// );
/// let compilation = rex_driver::compile_deploy_str("svc.deploy", source, &designs, &flows);
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().applications.len(), 1);
/// ```
///
/// # Normative contract
///
/// ## Entry points
///
/// - [`compile_deploy_str`] — parse and validate in one call; there is no
///   domain lowering to memoize, so the salsa layer is parse-only
///   ([`parse_deploy_query`]).
/// - [`compile_deploy_file`](crate::deploy) — the salsa-free engine the
///   entry point calls.
///
/// ## Resolution semantics
///
/// A deployment file is **self-contained for names** (applications,
/// profiles, components, deployments resolve within the file) and
/// **composition-open for content**: `import "<path>.ddd"` /
/// `import "<path>.ifml"` pull in pre-compiled design and flow artifacts
/// the host provides, and component bindings (`design "...#Module"` /
/// `flow "...#Module"`) reference them by the import path *as written*,
/// split at the last `#`. Targets are never authored: a profile names one
/// of the implementation-provided targets, whose capability, engine, and
/// setting tables live in the driver.
///
/// Resolution is **validation-only**: the artifact keeps every name,
/// setting, and policy expression exactly as authored (component baseline
/// settings stay single-segment, unqualified). The effective settings of a
/// deployment — component baselines ⊕ profile defaults ⊕ deployment
/// overrides — are computed for validation and never serialized back.
///
/// ## Validation rules
///
/// Any error-severity diagnostic drops the artifact.
///
/// 1. **Syntax** — parse errors are reported against the `.deploy` file.
/// 2. **Imports** — every `import "<path>"` must name a provided `.ddd`
///    design or `.ifml` flow, else `imported file "<path>" was not
///    provided`; other extensions are `unsupported import`.
/// 3. **Kinds** — a component's kind must be in the closed vocabulary
///    (`api`, `worker`, `database`, `queue`, `objectStore`, `frontend`).
/// 4. **Capabilities** — a `requires` name must be in the closed
///    capability vocabulary (`longRunningProcess`, `persistentFilesystem`,
///    `sqlDatabase`, `container`, `backgroundWorker`,
///    `horizontalScaling`, `edgeExecution`, `statefulProcess`).
/// 5. **Settings** — component baselines are single-segment
///    `name: value` lines from the kind's vocabulary; defaults and
///    overrides are two-segment `component.setting: value` lines naming a
///    component of the deployed application; word values must be in their
///    list (`engine: sqlite|postgres|d1`, ...); integer and text settings
///    take the matching literal kind; a path is declared once per level.
///    The vocabulary carries the operational settings (issue #135): `port`
///    (integer) on every kind, `health` (text) on `api`/`worker`, and
///    `cors` (text, comma-separated origins) on `api`/`frontend`. An
///    `env { NAME = "value" ... }` block is *not* settings: its entries
///    are opaque strings lowered verbatim, checked only for empty and
///    duplicated names per component (`duplicate env 'NAME' on component
///    'C'`), never joined into the effective settings, and never
///    policy-addressable.
/// 6. **Bindings** — a binding's path must match one of the file's
///    imports, and a `#<Module>` suffix must name a module of the imported
///    design/flow.
/// 7. **Targets** — a profile must carry a `target:` line naming a known
///    target (`standalone`, `dockerCompose`, `kubernetes`,
///    `cloudflareWorkers`).
/// 8. **Uniqueness** — application, profile, and deployment names are
///    unique within the file; component names within their application.
/// 9. **References** — `connects` endpoints name components of their
///    application; `deployment ... for` names an application and `use`
///    names a profile of this file.
/// 10. **Effective settings** — computed per deployment as component
///     baselines ⊕ profile defaults ⊕ overrides (later levels win), then
///     the target rules run against them.
/// 11. **Capabilities** — each component's intrinsic requirements (a
///     `worker` needs `longRunningProcess`, a `database`
///     `sqlDatabase`), conditional requirements (`engine: sqlite` adds
///     `persistentFilesystem`; `runtime: container` adds `container`),
///     and declared `requires` must all be provided by the target, else
///     the error names the component and target.
/// 12. **Engines** — a database's effective engine must be one the target
///     hosts (standalone: `sqlite`; compose: `sqlite`, `postgres`;
///     kubernetes: `postgres`; cloudflare-workers: `d1`) — a postgres
///     database is never silently mapped onto D1.
/// 13. **Policies** — every profile policy is parsed with rex-expr, typed
///     as `boolean` against the application's components (the synthetic
///     settings universe: one class per component holding its setting
///     vocabulary — the operational settings included — and the
///     application as the root; env entries are not features, so
///     `api.env.FOO` is a type error), and evaluated against the
///     deployment's effective settings with rules R1 (checked arithmetic)
///     and R4 (no zero divisor). A `require` condition that evaluates
///     false — including through a deployment's overrides — or a
///     `prohibit` condition that evaluates true is an error naming the
///     deployment and the policy.
///
/// Diagnostics are ordered deterministically: the file's parse diagnostics
/// first, then its semantic diagnostics in declaration order
/// (applications, profiles, deployments — each in source order).
pub fn compile_deploy_str(
    path: &str,
    source: &str,
    designs: &[(String, rex_ir::ddd::DddModel)],
    flows: &[(String, rex_ir::ifml::IfmlModel)],
) -> DeployCompilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let parsed = parse_deploy_query(&db, file);
    let (model, diagnostics) = deploy::compile_deploy_file(
        path,
        source,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        designs,
        flows,
    );
    DeployCompilation { model, diagnostics }
}
