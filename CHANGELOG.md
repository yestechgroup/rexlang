# Changelog

All notable changes to rexlang are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versioning policy
in [docs/VERSIONING.md](docs/VERSIONING.md).

## [Unreleased]

"Proof-carrying models" (epic #202, phases P0 and P1's receipts core): the
evidence chain becomes mechanical. No grammar, IR, artifact, or wire-format
change — the receipts are a new document kind, and every existing golden is
untouched.

"The unified semantic graph" (epic #194): every surface now compiles into
**one** queryable artifact — applications, nodes, and edges — that impact
analysis, `graph query`, the cross-policy analytics, the editor's
cross-surface navigation, and (in later milestones) the MCP server and
Studio all consume. The graph is a new *additive, independently versioned*
wire artifact (`rex_ir::graph::GraphModel`, marker
`GRAPH_MODEL_FORMAT_VERSION`): no existing artifact, golden, or emitted
byte moves.

### Added

- **`rex_ir::graph::GraphModel`** (issue #239, the semantic-graph slice
  1/8): the unified, versioned semantic-graph wire artifact — applications,
  nodes (`class`, `serviceOperation`, `decision`, `event`, `view`,
  `planItem`, …), and edges (`imports`, `permits`, `binds`, `emits`,
  `executes`, `deploys`, …). Node and edge kinds are an **open,
  string-typed vocabulary** so the artifact can grow additively before the
  v0.7 freeze; an empty graph serializes as exactly `{"formatVersion":1}`,
  unknown kinds round-trip, and there is no `rexVersion` stamp (goldens
  never churn on release bumps). Normative tables in `docs/GRAPH.md`.
- **`rex_driver::graph` + `rex_driver::applications`** (issue #241, slice
  2/8): the pure post-compile builder over all nine surfaces — `.mox`,
  `.actor`, `.ddd` (state machines included), `.evt`, `.dmn`, `.ifml`,
  `.deploy`, `.uc`, `.cmmn` — with cross-surface resolution mirroring the
  compiler (union-wide capability/actor pools, the ASCII-case-insensitive
  `action → operation` rule, ambiguity flagged, unresolved references emit
  **no** edge). The import-connected-application union-find moved out of
  the CLI into the driver and `check`'s integrity pass now consumes it
  (its diagnostics are byte-identical).
- **`GraphIndex` + `rexlang graph query`** (issue #242, slice 4/8): the
  artifact-only query surface — `node`, `list`, `affected-by`,
  `dependencies` — each in one versioned envelope, running from a committed
  `graph.json` with no sources and no recompile. `rexlang artifact check`
  classifies `GraphModel` artifacts (before the deploy rule) and names
  violations precisely.
- **Spanned `SurfaceIndex` navigation indexes for all seven artifact
  surfaces** (issues #245/#244, slices 6/8 and 7/8): `.actor`, `.ddd`,
  `.evt`, `.deploy`, `.uc`, `.cmmn`, `.dmn` each publish definitions and
  references with byte spans, and every definition's `(kind, name)` **is**
  the graph builder's node key — pinned by the anti-drift equality test
  over all nine surfaces. The `.mox` index moved verbatim into
  `navigation/mox.rs` (behavior frozen).
- **`rexlang graph`** (issue #240, the semantic-graph slice 3/8): compile a
  tree of model files into **one** deterministic, versioned
  `rex_ir::graph::GraphModel` — the import-connected applications, the full
  node/edge vocabulary, and the semantic edges between every surface —
  printed to stdout or `-o`. Directories enter project mode like
  `check`/`build`; display paths are root-relative POSIX (base = the input
  roots' common ancestor), so the artifact never carries a machine path.
  Emission is best-effort over the compiling subset (the `build` precedent).
  The pipeline's `SurfaceOutcome` now carries each file's typed artifact and
  resolved imports (and `IfmlInputs` its imports), so the graph re-reads and
  re-compiles nothing; `check`/`ir`/`build` output is unchanged. Pinned by
  `examples/artifacts/examples.graph.json` (the whole `examples/` tree, base
  `examples/`) and its `crates/rex-cli/tests/cli/graph.rs` suite.
- **`rexlang graph query … separation-of-duty|monotonicity|dead-capabilities|reachability`**
  (issue #246, the semantic-graph slice 5/8): the four cross-policy analytics
  run on the artifact **alone** and answer in the same versioned envelope as
  the structural queries — per application (unrelated applications never
  cross-check), conservative may-analysis (`when` guards are satisfiable), and
  an explicit `unknown` for opaque `cedar` grants, which never become a
  definite yes or no. `--fail-on-findings` exits `1` only on a definite
  finding (a violated pair, a monotonicity gap, a `dead` capability, an
  unreachable target), so CI can gate without making a policy finding a
  language error. The compiler's `lineage`/`effective_permits` semantics are
  rederived from the artifact (`crates/rex-driver/src/graph/analytics.rs`).
  Pinned by the self-contained `tests/conformance/graph/` fixture — `app.*`
  across all nine surfaces plus the unrelated `other.*` application (its
  `cedar` policy is the `unknown` case) — with `app.graph.json` and one
  `queries/app.*` golden per analytic.
- **Cross-surface LSP navigation over the graph index** (issue #243, the
  semantic-graph slice 8/8): every surface now answers
  definition/references/hover/document-symbols from one semantic index. The
  server builds a **graph snapshot** from the open document, its
  open-document siblings, and the disk siblings of its directory (open
  buffers win over disk; a non-compiling sibling contributes nothing),
  resolving names with the graph's `GraphIndex` and landing on spans from
  each file's `SurfaceIndex` — `.actor`→`.mox` class, `.ddd` capability →
  `.actor`, `invokes` → `.dmn`, `emits` → `.evt`, `.evt` consumer →
  `.deploy` component, `.ifml` `requires:`/`action(...)` → capability /
  bound design operation, deploy bindings → module, `.uc`/.cmmn` →
  designed operation, `.dmn` emits → event, and inline `.mox` actors. Rename
  is declaration-owned and document-scoped: a reference to another file's
  declaration is refused with a message naming the owner. `NavigationIndex`
  (`.mox`) and `IfmlIndex` (`.ifml`) gained `SurfaceIndex` projections, and
  the anti-drift equality test now covers all nine surfaces. No artifact,
  wire-format, or golden change.
- **`rex-evidence`**, the evidence-spine crate (issue #203): the
  machine-readable proof-obligation register's data model, canonical parse,
  semantic validation, and markdown rendering. Pure — serde/serde_json only,
  no filesystem or compiler access.
- **`docs/verification/register.json`** (issue #203): the register's source
  of truth — 17 obligation rows with closed statuses
  (`proved`/`tested`/`argued`/`assumed`), resolvable evidence pointers,
  triggers and review dates on every open or assumed row, the four #173
  Tier-3 holds, and the bidirectional `#[trusted]` inventory. The tables in
  `docs/VERIFICATION.md` are now **generated** from it.
- **`docs/SEMANTICS.md`** (issue #205): the claim ladder (C1–C5), the
  `WF`/`WT`/`Secure`/`Consistent` judgment forms, theorem **S** with its
  named assumptions, the explicit non-goals with reversal triggers, and the
  complete #189 section mapping. Supersedes #189.
- **`rexlang verify`** (issue #197) and the versioned **verification
  receipt** (`formatVersion` 1): one canonical, deterministic JSON document
  per model set — the source closure hashed, every artifact hashed (pinned
  against its committed golden), the generator version, and the register's
  rows exactly as the register states them. `--check` validates a receipt
  standalone (issue #206: no register, models, toolchain, or network);
  `--check-fresh` regenerates and byte-compares (issue #207). Receipts
  **never** say "verified": the four-word register vocabulary is closed and
  `verified` is rejected by name; `mutantKilled: null` +
  `limits: ["mutants-unrecorded"]` records the #208 gap out loud.
- **`examples/artifacts/support.receipt.json`**: the flagship receipt for
  the `support` example — nine surfaces, the pattern-library closure, and
  the external decision table.
- **`docs/RECEIPTS.md`**: what a receipt is and is not, its schema and
  closed vocabularies, the commands, and the staleness classes.

### Changed

- The `#[trusted]` register gate (issue #89) is now the full **register
  drift gate** (#203): a dead evidence pointer, a status outside the closed
  vocabulary (`"verified"` is rejected by name), an `argued`/`assumed`/
  mirrored row without a trigger and review date, or a `#[trusted]` that is
  not bidirectionally registered all fail `cargo test`.
- `scripts/ci-local.sh` gains the `receipt` step (CI 5/6) and
  `.github/workflows/ci.yml` invokes it: a stale flagship receipt — a source,
  artifact, or register change without regeneration — is CI-red with the
  exact command to run (issue #207).

## [0.2.0] — 2026-11-14

"Foundation & trust": the security/dependency foundation, the release
ladder's base. No language, grammar, wire-format, or artifact changes —
the nine surfaces are frozen as a set; the only golden diff this release
is `rexVersion` provenance moving `0.1.0` → `0.2.0`.

### Security

- **`cargo audit` is fully clean** and enforced in CI (see *Added*).
  Advisories fixed this release:
  - RUSTSEC-2026-0308 — salsa 0.23.0 → 0.28.5 (use-after-free in interned
    values/cached results; salsa backs the LSP, which processes untrusted
    editor input). Migration: `returns(clone)` on the tracked queries and
    the `SourceFile` input getters for 0.28's references-by-default
    (#213). Zero behavioral change — navigation, compile, and the full
    in-process LSP suites pass byte-for-byte.
  - RUSTSEC-2026-0386 — tokio 1.53.1 → 1.54.1 (AsyncFd I/O safety;
    rexlang does not use AsyncFd) (#211).
  - RUSTSEC-2026-0486 — rustix 1.1.4 → 1.1.6 via the proptest/tempfile
    dev graph (xattr zero-length buffer) (#209; advisory postdated the
    #173 survey).
- `cargo deny` (advisories, licenses, sources) gates every PR with a
  permissive license allow-list and scoped LGPL exceptions for the
  Creusot proof stdlib (`deny.toml`) (#215).

### Changed

- **The language server runs on the maintained fork**: `tower-lsp` 0.20
  (dormant since 2023) → `tower-lsp-server` 0.23 stable — new `ls_types`
  types (the `fluent_uri`-backed `Uri` replacing `url::Url`), native
  async-in-trait, `offset_encoding` left absent so positions stay UTF-16.
  Dispatch contract unchanged; LSP suite 91/91 plus a real-stdio smoke
  test (#214).
- Dev/test dependency refresh (#209, #216): pest 2.9.6 (stricter grammar
  validation, Unicode 18, stack guard), serde_json 1.0.154, clap 4.6.7,
  sha2 0.11.1, proptest 1.12, ureq 3.5.2, thiserror 2.0.21;
  jsonschema 0.55 → 0.61 (wire-schema conformance suites green),
  ariadne 0.5 → 0.6 (**zero rendering churn** — goldens byte-identical),
  cedar-policy 4.12 → 4.14.1 (dev-only; the backend stays unlinked).
  `creusot-std` deliberately held at 0.14 pending the matching Creusot
  toolchain (#220).
- The unused `askama` workspace entry is removed; `cargo-machete` is the
  recommended recurring check for this class (#209, #220).

### Added

- `rexlang fmt` and `fmt --check` accept directories with `check`'s exact
  input semantics — recursive scan over every supported surface, sorted,
  formatter-golden `X.fmt.<ext>` outputs skipped; the natural CI pairing
  `check model/ && fmt --check model/` now works (#217, closes #136).
- CI parity: `.github/workflows/ci.yml` invokes `scripts/ci-local.sh` for
  every gate — the script is the single source of truth for the gate
  sequence and the fixture-glob list, so local and remote can never
  drift, and a new fixture directory is a one-file edit (#218, closes
  #121).
- Two fast CI supply-chain jobs: `cargo audit` and `cargo deny check`;
  `scripts/ci-local.sh audit` replicates both locally (#215).
- `rexlang completions <shell>` (bash/zsh/fish/powershell/elvish) and
  `rexlang man` — rendered from the CLI's own clap definition, so they
  cannot drift from it (#219).
- Directory scans (check/build/fmt/ir) run on `walkdir` with
  `follow_links(false)`; deterministic order and symlink-loop immunity
  pinned by tests (#219).

### Fixed

- `rexlang fmt --check <dir>` no longer errors with "Is a directory"
  (closes #136, above).

### Deferred (tracked in #220)

- `line-index`/`text-size` behind the LSP position mapping;
  `assert_cmd`/`predicates`/`tempfile` for the CLI tests; the
  chumsky 0.13 / logos 0.16 dedup spikes; check-time `regex-syntax`
  validation of `.ifml` regex literals; `jiff`/`heck` trials;
  `creusot-std` 0.15 (toolchain-coupled).

### Earlier history

rexlang 0.1.0 was a version stamp, not a tagged release — no tags or
releases exist before 0.2.0. For everything prior to this entry, read the
commit log (`git log v0.2.0` ancestry); the per-surface references under
`docs/` describe each surface as shipped.

[0.2.0]: https://github.com/yestechgroup/rexlang/releases/tag/v0.2.0
