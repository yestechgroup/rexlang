//! rex-evidence — the proof-obligation register's data model.
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
//! # Why a dedicated crate
//!
//! The register is *evidence*, and evidence must be checkable without
//! dragging the compiler with it. The crate is deliberately
//! dependency-light (serde/serde_json only — no toml, no chrono): any
//! future consumer (a CLI subcommand, the receipts of issue #197, CI
//! tooling) can depend on it without pulling workspace machinery. Parse
//! is lenient (empty strings over missing fields); **all** semantics live
//! in [`register::validate`], so the rules have exactly one home and the
//! negative self-tests can show each one rejecting what it claims to
//! reject.
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
//! caused it.

pub mod register;

pub use register::{
    parse, render_markdown, validate, Evidence, HoldRow, Mirror, Obligation, PipelineRow, Register,
    TrustedRow, Violation,
};
