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
| [IFML.md](IFML.md) | `.ifml` | Interaction flows: views, components, events, the reference pattern library |

## For consumers and contributors

| Document | Read it when… |
|---|---|
| [../README.md](../README.md) | you want the overview: what rexlang is, status, CLI usage, integration styles |
| [../examples/README.md](../examples/README.md) | you learn from working code — the canonical examples compile one domain through every surface, CI-enforced |
| [BACKENDS.md](BACKENDS.md) | you write a backend (in or out of tree) that consumes rexlang artifacts |
| [RUNE-BODIES.md](RUNE-BODIES.md) | you care about executing imported Rune (`.rosetta`) logic — the design RFC for lowering Rune bodies |
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
- **"I want to add a surface"** → [../AGENTS.md](../AGENTS.md) (the table
  row), then any per-surface doc as the template.
