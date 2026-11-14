//! rex-evidence — the proof-obligation register's data model and the
//! verification receipts that report it.
//!
//! `docs/VERIFICATION.md`'s tables are **generated** from one
//! machine-readable source, [`docs/verification/register.json`](https://github.com/yestechgroup/rexlang/blob/main/docs/verification/register.json);
//! this crate owns everything about that source except its content: the
//! data model ([`register::Register`]), the parse
//! ([`register::parse`]), the semantic validation
//! ([`register::validate`]), and the markdown renderer
//! ([`register::render_markdown`]). The consumer is the drift gate in
//! `crates/rex-verify/tests/trusted_register.rs`, which loads the JSON,
//! validates it, resolves every evidence pointer against the workspace,
//! and byte-compares the rendered block against
//! `docs/VERIFICATION.md` (regenerate with `REX_UPDATE_FIXTURES=1 cargo
//! test -p rex-verify` — the same pattern as `rex-diagnostics` and
//! `docs/DIAGNOSTICS.md`).
//!
//! # The second half: receipts
//!
//! Issue #197's **verification receipt** is the register's shipped
//! artifact: `rexlang verify` compiles a model set, hashes every source it
//! read and every artifact it emitted, embeds the register's rows, and
//! writes one canonical JSON document; `rexlang verify --check` validates
//! that document with no register, no models, no toolchain, and no network
//! (issue #206). Everything the receipt *is* lives in [`receipt`] — the
//! wire contract, the closed vocabularies, the validator, and the hashing
//! helper. [docs/RECEIPTS.md](https://github.com/yestechgroup/rexlang/blob/main/docs/RECEIPTS.md)
//! is the reader's guide.
//!
//! # Why a dedicated crate
//!
//! The register is *evidence*, and evidence must be checkable without
//! dragging the compiler with it. The crate is deliberately
//! dependency-light (serde/serde_json plus sha2 for the receipt's hashes —
//! no toml, no chrono): any future consumer (the CLI subcommand, CI
//! tooling, the v0.6 MCP server) can depend on it without pulling
//! workspace machinery. Parse is lenient (empty strings over missing
//! fields); **all** semantics live in [`register::validate`] and
//! [`receipt::check`], so the rules have exactly one home and the negative
//! self-tests can show each one rejecting what it claims to reject.
//!
//! # The validation contract
//!
//! [`register::validate`] enforces the closed vocabularies (statuses
//! `proved`/`tested`/`argued`/`assumed` — the value `"verified"` is never
//! legal — and evidence kinds `proof`/`test`/`mirror`/`doc`/`code`), the
//! pointer grammar, the review-schedule discipline (argued/assumed/mirror
//! rows carry `trigger` + ISO `review` ≥ `reviewedAt`), the status↔evidence
//! joins (`proved` needs a proof-or-mirror pointer, `tested` needs a test
//! pointer), hold-row completeness, and trusted-row references. Every
//! violation names its rule, so a failing gate points at the edit that
//! caused it. [`receipt::check`] enforces the receipt's own closed sets
//! (artifact kinds, limits), its canonical byte form, and re-hashes every
//! artifact — every violation names its **field path**
//! (`register.rows[2].status`), so a tampered receipt fails at the exact
//! key that was changed.

pub mod receipt;
pub mod register;

pub use receipt::sha256_hex;
pub use register::{
    parse, render_markdown, validate, Evidence, HoldRow, Mirror, Obligation, PipelineRow, Register,
    TrustedRow, Violation,
};
