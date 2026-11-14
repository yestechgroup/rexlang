# rexlang documentation

rexlang is a multi-surface modeling language: every surface follows the same
pipeline (`check` → `ir` → backends) and compiles to its own versioned JSON
artifact. This index routes to the right document.

## The language

| Document | Surface | What it covers |
|---|---|---|
| [LANGUAGE.md](LANGUAGE.md) | `.mox` / `.actor` | The normative reference: the structural domain model (packages, classes, features, operations, enums, datatypes, vocabularies), the `.actor` authorization dimension, `import schema`/`import sigil`, canonical instance JSON, tooling |
| [EXPRESSIONS.md](EXPRESSIONS.md) | all surfaces | The expression language: grammar, type system, and the numbered semantic rules (R1–R9, A1–A6, U1, L1/L2) that diagnostics cite |
| [DDD.md](DDD.md) | `.ddd` | Sculptor-style designs: applications, modules, services, repositories, searches, entity state machines (lowered to xstate-ir) |
| [EVENTS.md](EVENTS.md) | `.evt` | Event contracts: versioned events, channels, subscriptions |
| [DEPLOY.md](DEPLOY.md) | `.deploy` | Deployment projections: components, profiles, targets, policies |
| [USECASES.md](USECASES.md) | `.uc` | SysML v2 use case patterns: subjects, actors, messages, successions, `realizes` |
| [CMMN.md](CMMN.md) | `.cmmn` | CMMN 1.1 case management: cases, case files, plans, sentry criteria — and the cmmn-rs seam |
| [DMN.md](DMN.md) | `.dmn` | DMN 1.6-style decision models: the decision-requirements graph, external markdown decision tables, decision services — and the `.ddd` decision link |
| [IFML.md](IFML.md) | `.ifml` | Interaction flows: views, components, events, the reference pattern library |
| [VERSIONING.md](VERSIONING.md) | all | The versioning/release policy: what 0.x promises, what 1.0 freezes, what a release consists of |

## For consumers and contributors

| Document | Read it when… |
|---|---|
| [../README.md](../README.md) | you want the overview: what rexlang is, status, CLI usage, integration styles |
| [../examples/README.md](../examples/README.md) | you learn from working code — the canonical examples compile one domain through every surface, CI-enforced |
| [BACKENDS.md](BACKENDS.md) | you write a backend (in or out of tree) that consumes rexlang artifacts |
| [GRAPH.md](GRAPH.md) | you want one queryable artifact spanning every surface — the semantic graph's node/edge vocabulary, its import-connected applications, `rexlang graph`, and the artifact-only `graph query` surface |
| [RUNE-BODIES.md](RUNE-BODIES.md) | you care about executing imported Rune (`.rosetta`) logic — the design RFC for lowering Rune bodies |
| [SEMANTICS.md](SEMANTICS.md) | you want the claim ladder: what the program claims, the per-surface judgments, theorem S and its assumptions, and the explicit non-goals with their reversal triggers |
| [VERIFICATION.md](VERIFICATION.md) | you need the proof-obligation register: every claim, its status (`proved`/`tested`/`argued`/`assumed`), its evidence pointer and review date — generated from `docs/verification/register.json`, gated by CI |
| [../AGENTS.md](../AGENTS.md) | you change the compiler itself: crate ownership, commands, testing quirks, gotchas |

## Reading paths

- **"I want to model a domain"** → [LANGUAGE.md](LANGUAGE.md), then the
  `.mox` examples in [../examples/](../examples/). Add behavior with
  [DDD.md](DDD.md), contracts with [EVENTS.md](EVENTS.md).
- **"I want to generate a system"** → the support-desk stack in
  [../examples/](../examples/) walks domain → policy → design → events →
  use cases → flows → deployment → case; the README's walkthrough compiles
  each layer by hand.
- **"I want to consume rexlang from my tool"** → [BACKENDS.md](BACKENDS.md)
  (artifact contract), or the root README's integration styles.
- **"I want evidence, not assurances"** → [SEMANTICS.md](SEMANTICS.md) for
  what is claimed and refused, [VERIFICATION.md](VERIFICATION.md) for each
  row's status, evidence pointer and review date, and [RECEIPTS.md](RECEIPTS.md)
  for checking a release's evidence without the prover toolchain.
- **"I want to add a surface"** → [../AGENTS.md](../AGENTS.md) (the table
  row), then any per-surface doc as the template.
