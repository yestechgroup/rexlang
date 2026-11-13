//! The check-layer test suite: one module per concern; the shop-model
//! fixtures and `diags` helpers live in `support`.

use super::*;

mod support;

mod authorization;
mod decisions;
mod instantiated_modules;
mod interaction_reference;
mod lints;
