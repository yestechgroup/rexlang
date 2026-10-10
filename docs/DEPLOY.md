# Deployment models (.deploy)

## Overview

A deployment-projection layer over the other rexlang artifacts. The `.mox`
domain model defines the data, `.ddd` designs the application services, and
`.ifml` the interaction flows; the `.deploy` model declares the **logical
architecture** (an `application` of named components and their
`connects`), reusable **profiles** (a target plus defaults, policies, and
resource mappings), and **deployments** (one application projected through
one profile with per-application `configure` overrides) — with nothing
about containers, manifests, or clouds written down. Parsed by
**rex-syntax** (`parse_deploy`, the chumsky parser) at compile time and
lowered and validated by **rex-driver** (`compile_deploy_str`) into the
versioned `rex_ir::deploy::DeployModel` artifact (the rex-ir crate).

The separation the surface enforces is the issue-#49 discipline:

- **Architecture** — what components exist and how they relate
  (`application`). Nothing here says where anything runs.
- **Profile** — the constraints and runtime topology of a target
  (`profile`: `target`, `defaults`, `policies`, `mappings`). Profiles are
  reusable across applications.
- **Deployment** — the concrete projection (`deployment ... for ... use
  ... configure ...`). The effective configuration is *component
  baselines ⊕ profile defaults ⊕ overrides* (later levels win).

**Targets are implementation-provided, never authored**: a profile names
one of the closed targets (`standalone`, `dockerCompose`, `kubernetes`,
`cloudflareWorkers`), and the driver's capability, engine, and setting
tables define what each can host. The same logical application deploys to
every target it is compatible with — incompatibility is a **compile
error**, not a generated artifact ("validate before generating").

The `.deploy` DSL composes the other surfaces: `import "<path>.ddd"` /
`import "<path>.ifml"` pull in pre-compiled design and flow artifacts, and
component bindings (`design "<path>#<Module>"` / `flow
"<path>#<Module>"`) tie components to them by the import path *as
written*. **No in-tree backend exists yet**: compose.yaml / Kubernetes
manifest / Worker-bundle generation are the planned consumers of this
artifact (the deployment-plan resource graph is the planned intermediate);
the artifact deliberately carries authored intent only in slice 1.

The normative contract — entry points, resolution semantics, and the
validation rules — is documented on `rex_driver::compile_deploy_str`; this
page is the language reference.

---

## A worked example

```text
import "../ddd/library.ddd"
import "../ifml/app.ifml"

application Svc {
    component api: api { scaling: autoscaled }
    component lending: api { design "../ddd/library.ddd#media" }
    component web: frontend { flow "../ifml/app.ifml#Pagination" }
    component search: worker { requires longRunningProcess }
    component db: database { engine: postgres  storage: managed }
    connects web -> api
    connects lending -> db
}

profile production_k8s {
    target: kubernetes
    defaults { api.runtime: container  api.replicas: 3 }
    mappings { api -> kubernetes.deployment  database -> external.postgres }
    require (api.replicas >= 3)
    prohibit (db.storage == "local")
}

deployment production for Svc {
    use production_k8s
    configure { api.replicas: 4 }
}
```

---

## Artifact wire format

`DeployModel` is a standalone, versioned wire artifact following the
rex-ir [wire format contract](BACKENDS.md) (see also the `rex_ir::deploy`
module docs):

- Versioned independently of the domain-model marker via
  **`DEPLOY_MODEL_FORMAT_VERSION`** (= 1), the `ActorModel` precedent.
  `DeployModel::from_json` rejects absent/wrong versions with
  `IrError::UnsupportedFormatVersion` (an absent version reports as 0).
- All struct field names serialize as `camelCase` (`formatVersion`).
  Unit-only enums serialize as bare camelCase strings: component kinds
  (`"api" | "worker" | "database" | "queue" | "objectStore" | "frontend"`)
  and targets (`"standalone" | "dockerCompose" | "kubernetes" |
  "cloudflareWorkers"`).
- Setting values are adjacently tagged:
  `{"type": "int" | "text" | "word", "value": <primitive>}` — the
  wire-contract rule-2 shape.
- **Closed semantic core, extensible platform surface**: kinds and targets
  are the closed enums; capability names, setting paths, policy
  expressions, and mapping resources (`kubernetes.deployment`,
  `cloudflare.d1`, `docker.volume`, `external.postgres`) are plain
  strings, so new platform resources need no schema change.
- Policy expressions are **verbatim source text** between the policy's
  parentheses — the IR never parses or reformats them (the
  `when`-condition rule).
- Resolution is **validation-only**: the artifact keeps every name,
  setting, and binding as authored (component baselines stay
  single-segment). Effective settings are computed for validation and
  never serialized back.
- Unknown fields are ignored on deserialize; every field added after v1
  must keep the `#[serde(default)]` + skip discipline.

Minimal artifact: an empty model serializes as exactly
`{"formatVersion":1}`.

```rust
let designs: Vec<(String, rex_ir::ddd::DddModel)> = vec![];
let flows: Vec<(String, rex_ir::ifml::IfmlModel)> = vec![];
let compilation = rex_driver::compile_deploy_str("svc.deploy", source, &designs, &flows);
let json = compilation.model.unwrap().to_json_pretty()?; // versioned artifact
let model = rex_ir::deploy::DeployModel::from_json(&json)?; // version-gated reload
```

---

## Grammar

The normative EBNF for the `.deploy` surface lives exactly once, in the
parser module docs under [*`.deploy` deployment
sources*](https://docs.rs/rex-syntax/latest/rex_syntax/parser/index.html)
(`crates/rex-syntax/src/parser.rs`) — the single authority for
`deploy_file`, `application_decl`, `component_decl`, `profile_decl`,
`policy_decl`, `deployment_decl`, and the setting/mapping productions;
this page keeps only the language-level rules. The `.deploy` keyword
tables in `crates/rex-syntax/src/deploy.rs` are the keyword authority the
parser and the formatter share.

Language-level rules:

- **`application <name> { component <name>: <kind> { ... } ... connects a
  -> b ... }`** — the logical architecture. Kinds are the closed
  vocabulary `api`, `worker`, `database`, `queue`, `objectStore`,
  `frontend`. A component body carries `name: value` baseline settings,
  `requires <capability>, ...` clauses, and `design`/`flow` bindings.
- **`profile <name> { target: <target> ... }`** — a reusable
  configuration. `defaults { component.setting: value }` apply to every
  deployment selecting the profile; `mappings { kind ->
  platform.resource }` say how a kind translates into platform resources;
  `require (...)`/`prohibit (...)` policies constrain every such
  deployment.
- **`deployment <name> for <application> { use <profile> configure {
  ... } }`** — one projection. `configure` overrides beat profile
  defaults, which beat component baselines.
- **Policies are rex-expr conditions** over the effective settings:
  `api.replicas >= 3`, `db.storage == "local"`, `&&`/`||`/`!`, integer
  arithmetic. They are parsed and type-checked against the deployed
  application's components (an unknown component or setting is a type
  error) and **evaluated** against each deployment's effective settings —
  a violated `require` or satisfied `prohibit` (including through
  overrides) is a compile error. The expression rules R1 (checked
  arithmetic) and R4 (no zero divisor) apply. An unset setting referenced
  by a policy is an evaluation error: give the component a baseline or
  the profile a default.
- **`design`/`flow` bindings** are `"<path>"` or `"<path>#<Module>"` —
  the path must match one of the file's imports *as written* and the
  module must exist in the imported design/flow.
- **Every `.deploy` word is a contextual identifier** (the `.ddd`
  discipline): special only in its grammar position, an ordinary name
  everywhere else, escapable with `^`. The kind and target *vocabularies*
  are deliberately not syntax — the driver validates them, so a word like
  `container` (a `.mox` keyword) works as a setting value.
- **Imports come first**: `import "<path>"` (no trailing `;`, the
  `.actor` production) must precede the first declaration; a later
  `import` is a reported, recovered syntax error. Only `.ddd` and `.ifml`
  imports are supported.
- The three declaration kinds may be **interleaved** in any order; the
  artifact groups them per kind and the formatter canonicalizes to
  import → application → profile → deployment order (source order kept
  within a kind).
- The `->` arrow of `connects` and `mappings` is a real lexer token of
  the shared chumsky lexer; it never appears in a parsed grammar position
  of the other surfaces (raw bodies are span-sliced verbatim).

---

## Resolution & validation

A deployment file is self-contained for names (applications, profiles,
components, deployments resolve within the file) and composition-open for
content (design/flow artifacts arrive pre-compiled — the CLI gathers and
compiles them exactly like `rexlang check` on those surfaces). The driver
stays filesystem-free.

Validation rules (any error-severity diagnostic drops the artifact;
warnings do not):

1. **Syntax** — parse errors are reported against the `.deploy` file.
2. **Imports** — every import must name a provided `.ddd` design or
   `.ifml` flow, else `imported file "<path>" was not provided`; other
   extensions are `unsupported import`.
3. **Kinds and capabilities** — component kinds and `requires` names must
   be in the closed vocabularies (typos are errors, not vacuous
   requirements).
4. **Settings** — component baselines are single-segment `name: value`
   lines from the kind's vocabulary; defaults and overrides are
   two-segment `component.setting: value` lines naming a component of the
   deployed application; word values must be in their list; integer and
   text settings take the matching literal kind; a path is declared once
   per level.
5. **Bindings** — a binding's path must match an import, and a
   `#<Module>` suffix must name a module of the imported design/flow.
6. **Targets** — a profile must carry a `target:` line naming a known
   target.
7. **References** — `connects` endpoints name components of their
   application; `deployment ... for` names an application and `use` names
   a profile of this file.
8. **Capabilities** — per component: intrinsic requirements (a `worker`
   needs `longRunningProcess`, a `database` `sqlDatabase`), conditional
   requirements (`engine: sqlite` adds `persistentFilesystem`;
   `runtime: container` adds `container`), and declared `requires` must
   all be provided by the target. Target capability tables:
   - `standalone` — longRunningProcess, persistentFilesystem,
     sqlDatabase, backgroundWorker, statefulProcess;
   - `dockerCompose`, `kubernetes` — those plus `container` and
     `horizontalScaling`;
   - `cloudflareWorkers` — sqlDatabase, horizontalScaling,
     edgeExecution (no long-running processes, no local filesystem).
9. **Engines** — a database's effective engine must be one the target
   hosts (`standalone`: sqlite; `dockerCompose`: sqlite, postgres;
   `kubernetes`: postgres; `cloudflareWorkers`: d1). A postgres database
   is never silently mapped onto D1.
10. **Policies** — see above: parsed, typed boolean, evaluated against
    effective settings; violations are errors naming the deployment and
    the policy.

Diagnostics are ordered deterministically: parse diagnostics first, then
the file's semantic diagnostics in declaration order (applications,
profiles, deployments — each in source order).

---

## Tooling

- `rexlang check <file>.deploy` — compile and render diagnostics
  (including the imported designs/flows, which are gathered and compiled
  exactly like `rexlang check` on them; a non-compiling import is a clean
  error).
- `rexlang ir <file>.deploy [-o out]` — emit the versioned
  `DeployModel` JSON.
- `rexlang fmt <file>.deploy` — the token-based formatter (canonical
  section order, one member per line, verbatim policy conditions).
- `rexlang artifact check svc.deploy.json` — wire-format validation; the
  artifact classifies as the `DeployModel` kind.
- The LSP does not publish `.deploy` diagnostics yet (the `.evt`
  precedent): the surface compiles through the same one-shot entry
  points, so editor wiring is a follow-up, not a contract change.
