# Verification receipts

A **receipt** is the proof-obligation register's shipped artifact: one
canonical, versioned JSON document that says *what was compiled, from which
bytes, into which artifact bytes, by which generator, and what the register
states about it* — and nothing more. `rexlang verify` writes it;
`rexlang verify --check` validates it on a machine with no workspace
knowledge (issue #206); `rexlang verify --check-fresh` regenerates it and
byte-compares (the CI freshness gate, issue #207).

The epic's non-goal is the document's spine: **receipts never say
"verified"**. A receipt reports the register's statuses — `proved`,
`tested`, `argued`, `assumed` — verbatim, with the evidence pointers that
back them. The word `verified` is not a status in the closed vocabulary,
and `receipt::check` rejects it by name (the register's own discipline:
[VERIFICATION.md](VERIFICATION.md), [SEMANTICS.md](SEMANTICS.md)).

## What a receipt is not

- **Not a verdict.** It does not upgrade an `argued` row, does not merge
  statuses, and does not summarize. It carries the rows.
- **Not self-authenticating.** `--check` proves the document is
  well-formed, internally consistent, and that its artifact bytes are the
  bytes next to it. That the embedded rows still match *today's* register
  is `--check-fresh`'s job (it re-reads the register and re-hashes it).
  That the artifacts still correspond to their committed goldens is also
  `--check-fresh`'s job (generation asserts byte-equality first).
- **Not a claim about source truth.** A source file is hashed as bytes: a
  comment-only edit invalidates the receipt. That is deliberate — fail
  closed, never silently keep a stale claim.
- **Not a proof of the impossible-to-record.** The `limits` array names
  what the receipt explicitly does not know (see below).

## The interim mutant fact (`mutantKilled: null`)

Issue #197 asks for a `mutantKilled` fact per `tested` row: evidence that
the row's tests are falsifiable rather than merely green. The anti-vacuity
mutant pipeline that produces that fact is **#208**, and it does not exist
yet. Until it lands, every row carries `"mutantKilled": null` and the
receipt declares the gap out loud:

```json
"limits": ["mutants-unrecorded"]
```

`limits` is a closed vocabulary of stable lowercase slugs; the interim set
is exactly one entry:

| Slug | Meaning | Retired by |
|---|---|---|
| `mutants-unrecorded` | No mutant kill data exists for this receipt's rows; every `mutantKilled` is `null`. | #208 |

The validator couples the two: a non-null `mutantKilled` while `limits`
still declares `mutants-unrecorded` is a failure at
`register.rows[i].mutantKilled`. Dropping the limit (when #208 lands) is the
only way to record mutant facts.

## Schema

camelCase; alphabetical key order at every level; two-space indentation; no
trailing newline; no timestamps, no absolute paths, no environment. Two
runs over the same inputs produce byte-identical files (the CLI test suite
pins it).

```jsonc
{
  "formatVersion": 1,                       // the only compatibility gate
  "kind": "verificationReceipt",            // closed: exactly this string
  "subject": "support",                     // --subject, else the inputs' common stem
  "generator": { "name": "rexlang", "version": "0.2.0" },   // provenance (CARGO_PKG_VERSION)
  "inputs":  [ "examples/support.actor", … ],               // sorted, repo-relative
  "sources": [ { "path": "…", "sha256": "…" }, … ],         // sorted, repo-relative
  "artifacts": [
    { "kind": "dddDesign", "formatVersion": 1,
      "path": "support.ddd.json",           // relative TO THE RECEIPT, null = digest-only
      "sha256": "…" }
  ],
  "register": {
    "source": "docs/verification/register.json",
    "sha256": "…",
    "reviewedAt": "2026-11-14",
    "rows": [
      { "id": "V3", "claim": "…", "status": "argued",
        "evidence": [ { "kind": "proof", "pointer": "…" } ],
        "trigger": "…", "review": "…", "mutantKilled": null }
    ]
  },
  "trusted": [ { "item": "push_containment", "row": "V9" } ],
  "limits":  [ "mutants-unrecorded" ]
}
```

### Closed vocabularies

| Vocabulary | Values | Where enforced |
|---|---|---|
| `kind` | `verificationReceipt` | `receipt::check` |
| obligation `status` | `proved` \| `tested` \| `argued` \| `assumed` (`verified` rejected by name) | `receipt::check` |
| evidence `kind` | `proof` \| `test` \| `mirror` \| `doc` \| `code` | `receipt::check` |
| artifact `kind` | `irModel` \| `actorModel` \| `dddDesign` \| `eventContract` \| `useCaseModel` \| `deployModel` \| `decisionModel` \| `cmmnModel` \| `ifmlManifest` | `receipt::check` |
| `limits` | `mutants-unrecorded` | `receipt::check` |

The artifact vocabulary is **receipt-local**, not `rexlang artifact check`'s
shape-sniffed classification: an empty `IfmlModel` is shape-ambiguous with a
DDD design (see `crates/rex-cli/src/artifact_check/mod.rs`), but a receipt
knows what it just compiled. `ifmlManifest` names the control/route manifest
(`rex_ir::ifml_manifest`) an `.ifml` input contributes — the flow's
declaration-backed artifact and the only `.ifml` artifact with a committed
golden.

### Consistency rules the validator adds

Beyond the vocabularies:

- `formatVersion` must equal 1; a higher version is rejected naming the
  upgrade ("a newer receipt needs a newer rex-evidence").
- Every `sources`/`inputs`/`register.source` path is relative, normalized,
  and free of `..`; every `artifacts` path is relative to the receipt.
- `sources` and `inputs` are sorted and duplicate-free; `artifacts` are
  sorted by `(kind, path, sha256)` — a reader can recompute the order and
  detect reordering.
- Every `artifacts` entry with a `path` is **re-hashed** from the file next
  to the receipt: a missing file fails at `artifacts[i].path`, a mismatch at
  `artifacts[i].sha256` (naming both hashes). `path: null` entries are
  digest-only — the hash pins the emitted bytes but there is no sibling file
  to re-hash (domain and actor artifacts ship no example golden).
- Status↔evidence joins are carried from the register: `proved` needs a
  `proof` or `mirror` entry, `tested` needs a `test`.
- `argued`/`assumed` rows carry a `trigger` and an ISO `review` ≥
  `reviewedAt`; every `trusted` entry references a row the receipt carries.
- The document must be in canonical byte form (re-serialization must
  reproduce the file exactly).

## Commands

```sh
# Emit: compile the surfaces, hash what was read and written, embed the register.
rexlang verify examples/support.mox examples/support.actor examples/support.ddd \
  examples/support.evt examples/support.uc examples/support.deploy \
  examples/support.dmn examples/support.cmmn examples/support.ifml \
  -o examples/artifacts/support.receipt.json

# Validate standalone (no register, no models, no toolchain, no network).
rexlang verify --check examples/artifacts/support.receipt.json

# Prove it fresh (the CI gate): regenerate from the receipt's own inputs.
rexlang verify --check-fresh examples/artifacts/support.receipt.json
```

Exit discipline (consumable by CI and the future MCP server):

| Invocation | Exit 0 | Exit non-zero |
|---|---|---|
| `verify … -o R` | receipt written | a surface failed to compile (diagnostics rendered) or the emitted artifact drifted from its committed golden (both hashes named) |
| `verify --check R` | every rule holds; `OK <path> (<subject>)` | the first failing field first, one line per violation: `<receipt>: <field>: <message>` |
| `verify --check-fresh R` | regenerated bytes equal the committed file; `OK <path> is fresh` | `stale receipt: …` plus the exact `rexlang verify … -o …` command to run |

Generation never writes an invalid receipt: it runs `receipt::check` on its
own output before writing (with `--check`'s filesystem closure), so the
emitter and the verifier share one contract.

### How `--check` works offline

`--check` needs three things and no more: the receipt file, the artifact
files whose names it records (read from the receipt's own directory), and
`rex-evidence` itself. It does not read the register, parse a model, invoke
a backend, call Creusot, or touch the network. Copy the receipt and the
artifacts it names into any directory and it validates there; that is the
auditor's story (`docs/VERIFICATION.md` points here for the shipped
artifact).

## Staleness

The receipt is a golden. What invalidates it:

| Change | Why the receipt moves |
|---|---|
| a source model in `sources` changes (any surface, including a comment) | its `sha256` changes — sources are hashed as bytes |
| an import / `patterns/ifml/` module / `.dmn` decision table changes | it is in `sources`; the compiled artifact likely changes too |
| a backend, generator, or `rexVersion` changes | the emitted artifact bytes change (and `generator.version`); the committed goldens must be regenerated first |
| an artifact file changes | `artifacts[i].sha256` no longer matches; generation refuses to record a hash that disagrees with its golden |
| the register changes (a row, a status, `reviewedAt`) | `register.sha256` and the embedded rows change |

What does **not** invalidate it:

- documentation, comments in non-input files, or formatting of anything that
  is not one of the receipt's `sources`;
- regenerating the same inputs with the same generator and register (byte
  identity is the promise);
- where the receipt sits — as long as its artifact siblings move with it.

Regenerate with the command `--check-fresh` prints (it is derived from the
receipt's own `inputs`, so the receipt tells you how to remake itself), then
read the diff: the receipt is the spec of what was verified, not an output
to accept blindly.

### Source closure: what is recorded

`sources` is the transitive closure of every file the compile actually read,
sorted and repo-relative. For the flagship `support` receipt the closure is:

```
examples/support.{actor,cmmn,ddd,deploy,dmn,evt,ifml,mox,uc}
examples/tables/refund-authority.md      (support.dmn's `logic table`)
patterns/ifml/{communication,workflow}.ifml          (support.ifml's imports)
patterns/ifml/{collections,data,navigation}.ifml     (their transitive imports)
docs/verification/register.json is provenance (register.source), not a source
```

Paths are resolved from the working directory, which is the repository root:
run `rexlang verify` there. A path outside it cannot be receipted (receipts
never record an environment); an absolute input is spelled repo-relative by
stripping the canonical working directory.

## Where the receipt lives

`examples/artifacts/support.receipt.json`, next to the artifacts it names —
so `--check` re-hashes its siblings and the whole bundle is copy-able. The
`support` example is the flagship: nine surfaces, the cross-surface
capability joins, the pattern library, and an external decision table, which
is what makes it worth a receipt.

Regenerate the artifacts and the receipt together when a golden moves:

```sh
REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test examples_artifacts
REX_UPDATE_FIXTURES=1 cargo test -p rex-ifml
rexlang verify examples/support.actor examples/support.cmmn examples/support.ddd \
  examples/support.deploy examples/support.dmn examples/support.evt \
  examples/support.ifml examples/support.mox examples/support.uc \
  -o examples/artifacts/support.receipt.json
```

## The CI gate

`scripts/ci-local.sh receipt` is the freshness check (and part of the
default `all` sequence); `.github/workflows/ci.yml` invokes that subcommand
rather than restating the command — the script is the single source of truth
for the gate sequence (issue #121). A source or register change without
receipt regeneration is CI-red with the command to run above.

## Known limits (beyond `limits`)

- The register snapshot is validated at generation time with
  `rex_evidence::register::validate`; an invalid register refuses to become
  a receipt.
- `sources` includes the `import sigil` named file *and* the rosetta
  candidate pool the collector reads from the same directory tree (dot and
  `target` directories skipped, depth-capped) — a sibling `.rosetta` change
  moves the receipt.
- `--check` cannot tell that a `path: null` artifact *should* have had a
  named golden; `--check-fresh` re-derives that from the receipt's inputs.
  A hand-nulled artifact path is therefore a freshness failure, not a
  `--check` failure.
