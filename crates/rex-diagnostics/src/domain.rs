//! The `driver.domain` band: `E0300`–`E0349`, the rex-driver `.mox`
//! domain-surface checks (issue rexlang#177 —
//! `crates/rex-driver/src/lower/names.rs`). Every check prefixes its code
//! onto the diagnostic message (`E0300 stripped module name …`), the same
//! discipline the `dmn.check` band follows. The band's first occupant is
//! the stripped-name uniqueness check: two classes or enums in one package
//! that strip to the same module identifier, or map to the same effective
//! table name, would collide silently downstream (one generated module path
//! or pg table overwrites the other), so the compiler rejects them. Claim a
//! new code by adding a documented constant plus an `Entry` to `ENTRIES`.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const DOMAIN_BAND: &str = "driver.domain";

/// `stripped module name '<name>' is shared by <kind> '<a>' and <kind>
/// '<b>'`.
pub const E_STRIPPED_NAME_COLLISION: &str = "E0300";
/// `table name '<name>' is shared by class '<a>' and class '<b>'`.
pub const E_TABLE_NAME_COLLISION: &str = "E0301";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: E_STRIPPED_NAME_COLLISION,
        band: DOMAIN_BAND,
        severity: Severity::Error,
        title: "Stripped-name collision",
        meaning: "`stripped module name '<name>' is shared by <kind> '<a>' and <kind> '<b>'` — two declarations in one package (classes and/or enums) whose names strip to the same module identifier: one trailing `Type` suffix removed, then snake_case (`PayRun` and `PayRunType` both strip to `pay_run`; `TypeWriter` keeps its name). Consumers generate the same module path for both and one overwrites the other. The diagnostic spans the later declaration and names both. Pairs where either class is a containment target (some class declares a `contains` feature targeting it) are exempt — contained value objects share their owner's module downstream by design.",
        fix: "Rename one of the declarations, or give one class a distinct `table \"…\"` override when the collision is a value-object/table pairing.",
    },
    Entry {
        code: E_TABLE_NAME_COLLISION,
        band: DOMAIN_BAND,
        severity: Severity::Error,
        title: "Table-name collision",
        meaning: "`table name '<name>' is shared by class '<a>' and class '<b>'` — two classes in one package map to the same effective table name (a declared `table \"…\"` override, else the derived default), so both would generate the same pg table and one overwrites the other. The diagnostic spans the later declaration and names both. Pairs where either class is a containment target are exempt.",
        fix: "Give one of the classes a distinct `table \"…\"` override, or rename it.",
    },
];
