//! The `driver.integrity` band: `X0001`–`X0099`, the rex-driver's
//! cross-surface integrity pass (`check_integrity`, issue rexlang#114 —
//! `crates/rex-driver/src/integrity.rs`). No codes are allocated yet —
//! the next free code is `X0001`. Claim one by adding a documented
//! constant plus an `Entry` to `ENTRIES`.

use crate::Entry;

/// The band's registry id ([`crate::all`] entries reference it).
pub const INTEGRITY_BAND: &str = "driver.integrity";

pub(crate) const ENTRIES: &[Entry] = &[];
