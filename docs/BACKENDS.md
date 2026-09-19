# Writing a backend for rexlang

rexlang is a DSL for **modelling**. Backends are consumers of the Core IR —
not features of the compiler. Candidate backends (ACTUS cashflow mapping,
Accord/TemplateMark document generation, additional codegen targets) live as
**separate, out-of-tree crates** that consume the IR; nothing registers with
the compiler. This guide is the contract for writing one. The wire-format
rules summarized here are **normative in the [`rex-ir` crate
docs](https://docs.rs/rex-ir)** (also rendered by `cargo doc -p rex-ir`).

## The boundary

- A backend consumes **only** `rex_ir::Model` (plus `rex_ir::ActorModel` if
  it deals with the authorization dimension) or the serialized artifact.
  Never the AST: parsing, name resolution, validation, and lowering into the
  IR happen upstream in `rex-driver`.
- Backends **register nothing and emit anything** — Rust code, JSON Schema,
  Cedar policies, documents, tables.
- Two integration modes:
  1. **Library**: depend on the `rex-ir` crate, deserialize with
     `Model::from_json` / `ActorModel::from_json` (or build the IR
     programmatically for tests).
  2. **Artifact**: run `rexlang ir <files> -o out.rex.json` and read the
     JSON. A `.mox` compile emits the `Model` artifact; a `.actor` compile
     emits the standalone `ActorModel` artifact.
- The in-tree backends (`rex-backend-rust`, `rex-backend-jsonschema`,
  `rex-backend-cedar`) are reference implementations of exactly this
  discipline; reading them is the fastest way to learn the shapes.

## The serialized-artifact contract (summary)

Root shapes, as actually serialized:

| Artifact | Root |
| --- | --- |
| Domain IR (`Model`) | `{"formatVersion": N, "rexVersion"?: "…", "packages": […]}` |
| Actor policy (`ActorModel`) | `{"formatVersion": N, "blocks"?: […]}` — block-less serializes as exactly `{"formatVersion": N}` |
| Canonical instance | `{"$type": "rex.instance", "formatVersion": N, "model": "…", "objects": […]}` |

- **Casing.** All field names and bare enum tags are `camelCase`
  (`formatVersion`, `isReadOnly`, `"crossReference"`). Note that `$type` /
  `$id` / `$ref` belong to the *instance* format; IR artifacts tag their
  enums **adjacently** as `{"type": "<variant>", "value": <payload>}`.
- **Version gate.** Readers accept exactly their own
  `FORMAT_VERSION` (domain artifacts) or `ACTOR_MODEL_FORMAT_VERSION`
  (actor artifacts) and reject anything else with an error naming the
  found and expected versions (`IrError::UnsupportedFormatVersion`).
  Canonical instances are versioned independently
  (`rex_runtime::INSTANCE_FORMAT_VERSION`).
- **Additive evolution.** Every field added after v1 is
  `#[serde(default)]`, so older artifacts keep loading; fields empty in the
  common case also `skip_serializing_if`, so artifacts for models that do
  not use a feature (no bodies, no vocabularies, no actors, no descriptions)
  stay **byte-identical** to older output. Unknown fields are ignored on
  read, never denied.
- **Stable feature ids.** `Feature::id` equals its 0-based declaration index
  at publication time and is frozen for wire formats; backends key on it.
- **Golden byte-identity.** The goldens under `tests/conformance/` are the
  spec: an emitter change that shifts a byte is a contract change. When
  developing *in* this workspace, `REX_UPDATE_FIXTURES=1` regenerates them
  and the diff is reviewed as a spec change.

## The `preserve_order` gotcha

`serde_json::Map` is a `BTreeMap` **unless any crate in the build graph
enables serde_json's `preserve_order` feature**, which flips it to insertion
order. `cedar-policy` (a dev-dependency of `rex-backend-cedar`) enables it,
so a workspace that pulls in Cedar codegen inherits insertion order
everywhere. Emitters that rely on implicit map ordering will emit keys in
whatever order that build happened to produce. **Canonicalize key order
yourself before serializing** — see `canonical_key_order` in
`crates/rex-backend-jsonschema/src/lib.rs`, which recursively re-inserts
keys in a pinned order.

## Semver and `formatVersion` policy

The workspace is at **0.1.0 — pre-1.0**, so the usual "breaking = major"
rule does not apply yet:

- **Until 1.0, a breaking wire change bumps the MINOR crate version**
  (0.2.0, 0.3.0, …) *and* the affected `formatVersion` marker. Purely
  additive changes keep the format version and ride a normal minor/patch.
- Readers **accept their own version and reject any higher version with an
  error naming the version** — they never guess. Lower versions are equally
  rejected: there is no migration path inside one crate version.
- **Compatibility-matrix expectation.** Every rex-ir release documents the
  format versions it reads and writes, and out-of-tree backends declare the
  `formatVersion`(s) they accept, so consumers can pin both sides. The
  matrix so far:

| rex-ir | reads (domain) | writes (domain) | reads/writes (actor) | reads/writes (instance) |
| --- | --- | --- | --- | --- |
| 0.1.0 | 1 | 1 | 1 | 1 |

- `rexVersion` inside an artifact is provenance only — consumers that must
  not care can ignore it.

## The test kit: `rexlang artifact check`

Third-party backends can validate any serialized artifact against the wire
format without the originating model:

```
rexlang artifact check out.rex.json support.actors.rex.json instance.json
```

Per file it checks the invariants below, prints `OK <path>`, and exits `1`
naming the file and violation when anything fails:

- the root shape matches a known artifact kind (IR artifact, actor
  artifact, or `$type: "rex.instance"` canonical instance);
- `formatVersion` is present, an unsigned integer, and supported — a higher
  version is rejected with an error naming it;
- structural keys are camelCase (a snake_case spelling of a known key is
  reported with the correct spelling; model-defined keys — feature names,
  target names — are exempt);
- instance sanity: `$id`s are well-formed (`<class>/<ordinal>`) and unique,
  every object carries `$type`, `$ref` objects carry nothing else, and
  every `$ref` resolves within the document.

The checker is dogfooded in CI over every golden artifact under
`tests/conformance/artifacts/`, so it stays honest as the format evolves.

## Backend-author checklist

1. **Consume** only `rex_ir::Model` / `ActorModel` (or the artifact) —
   never the AST; declare which `formatVersion`s you accept.
2. **Emit** deterministically; canonicalize `serde_json::Map` key order
   yourself (`preserve_order` gotcha above).
3. **Pin golden byte-identity**: keep golden outputs under version control
   and byte-compare them in your tests, the way this workspace does.
4. **Run `rexlang artifact check`** over your consumed IR artifacts and any
   instance documents you produce.
5. **Pin the `rex-ir` version** and record the compatibility matrix row for
   the format versions you read and write.
