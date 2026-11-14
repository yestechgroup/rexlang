# Versioning and release policy

How rexlang versions, what compatibility each version line promises, and
what a release consists of. The consumer-facing drift record is
[CHANGELOG.md](../CHANGELOG.md).

## Versioning scheme

Semver-flavored: `MAJOR.MINOR.PATCH`.

- **0.x (today)**: the language is stabilizing. Breaking changes to the
  grammar, the IR shape, or the CLI are allowed — but every one of them
  must (a) land in the changelog, and (b) include its golden-regeneration
  notes so downstream consumers can see the exact drift. 0.x minor
  releases are the release ladder's rungs (0.2 → 0.3 → …); patch
  releases are fixes and dependency security bumps with no surface
  change.
- **1.0 (the commitment)**: the wire formats freeze. Within
  `formatVersion 1`, evolution is **additive only**: new optional fields
  may appear (with `#[serde(default)]` + `skip_serializing_if`, so
  body-less models stay byte-identical), but no field is renamed, no
  field changes meaning, no existing artifact stops parsing, and no
  golden changes without a `formatVersion` bump. A `formatVersion`
  bump is the only sanctioned breaking change after 1.0, and it must be
  a MAJOR release with a migration note.

## What each artifact promises

- **The Core IR / per-surface artifacts** (`rex_ir` models, the
  `rexlang ir` output): versioned by `formatVersion` (gate-checked on
  read) and stamped with `rexVersion` provenance. `rexVersion` is
  provenance, not a compatibility gate — readers gate on `formatVersion`
  only.
- **Generated code** (the Rust backend): target-language code may change
  between 0.x minors; the *runtime* crate (`rex-runtime`) that generated
  code links against is versioned with the workspace, and generated
  output pins its support-lib expectations at generation time.
- **The CLI** (`rexlang`): subcommand surface is additive within 0.x;
  script-breaking changes allowed at minor bumps with changelog notes.

## Golden fixtures are the spec

`tests/conformance/` and `examples/artifacts/` goldens are byte-compared
by the suite. The rule for touching them: regenerate with
`REX_UPDATE_FIXTURES=1 cargo test --workspace`, then **read the diff** —
goldens are the spec, not an output. Body-less/vocabulary-less models
must stay byte-identical.

## What a release consists of

1. The milestone's PRs are merged with the full local gate green
   (`scripts/ci-local.sh`; `audit` subcommand for the supply-chain
   gates) and CI green.
2. The workspace version is bumped in the release PR; every golden
   containing `rexVersion` is regenerated in the same PR, and the diff
   review confirms the only content change is the provenance line.
3. `CHANGELOG.md` gains the release entry (Keep a Changelog format,
   linked to the milestone PRs).
4. At milestone closeout: `git tag vX.Y.Z` on main, the tag is pushed,
   and a GitHub Release is published with notes drawn from the
   changelog. Every later release gets a tag + release notes; the
   changelog is the single place a consumer checks for drift.
5. `docs/VERIFICATION.md` (the proof register) is untouched by
   dependency-only releases; a toolchain bump that affects
   `rex-verify` must re-run `cargo creusot -p rex-verify` and update the
   register in the same PR.

## Dependency policy (short form)

- Security advisories (`cargo audit`) are fixed on the current minor;
  the CI `audit` and `deny` jobs keep the tree at zero.
- Git-pinned toolchains (sigil, restingstate, dsntk) move by rev, with
  the compatibility boundary named in the workspace `Cargo.toml`
  comment.
- New dependencies need a job the stdlib-ish status quo can't do, and an
  equivalence story when they replace hand-written logic (see the
  `heck`/instance-id precedent in #220).
