//! The `ifml.lints` band: `W0150`–`W0199`, reserved for warning-severity
//! rex-ifml lints. No codes are allocated yet — the next free code is
//! `W0150`. Claim one by adding a documented constant plus an `Entry` to
//! `ENTRIES`.

use crate::Entry;

/// The band's registry id ([`crate::all`] entries reference it).
pub const LINTS_BAND: &str = "ifml.lints";

pub(crate) const ENTRIES: &[Entry] = &[];
