# Changelog

All notable changes to rexlang are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.0.0/); versioning policy
in [docs/VERSIONING.md](docs/VERSIONING.md).

## [Unreleased]

"Proof-carrying models" (epic #202, phases P0 and P1's receipts core): the
evidence chain becomes mechanical. No grammar, IR, artifact, or wire-format
change — the receipts are a new document kind, and every existing golden is
untouched.

### Added

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

### Changed

- The `#[trusted]` register gate (issue #89) is now the full **register
  drift gate** (#203): a dead evidence pointer, a status outside the closed
  vocabulary (`"verified"` is rejected by name), an `argued`/`assumed`/
  mirrored row without a trigger and review date, or a `#[trusted]` that is
  not bidirectionally registered all fail `cargo test`.

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
