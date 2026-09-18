//! Runtime constraint validation for generated models.
//!
//! Classes with constrained attributes (`minLength`/`maxLength`/
//! `minimum`/`maximum`) are generated with a `validate` method that reports
//! violations instead of panicking: generated applications round-trip data
//! and need to *report* bad values, not die on them. `pattern` stays
//! descriptive in this milestone — generated projects take no regex
//! dependency.

/// The constraint family of a [`Violation`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConstraintKind {
    /// Inclusive minimum string length (`minLength`).
    MinLength,
    /// Inclusive maximum string length (`maxLength`).
    MaxLength,
    /// Inclusive minimum numeric value (`minimum`).
    Minimum,
    /// Inclusive maximum numeric value (`maximum`).
    Maximum,
}

/// One violated constraint on one feature of an object, as reported by a
/// generated `validate` method.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("constraint violation on feature '{feature}': {kind:?} ({detail})")]
pub struct Violation {
    /// The declared feature name, e.g. `"sku"`.
    pub feature: String,
    /// The violated constraint family.
    pub kind: ConstraintKind,
    /// Human-readable specifics: the offending length or value, the bound,
    /// and (for many-valued features) the element index in `[i]` form.
    pub detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn violation_constructs_and_displays() {
        let violation = Violation {
            feature: "sku".to_string(),
            kind: ConstraintKind::MinLength,
            detail: "length 1 is below minLength 2".to_string(),
        };
        assert_eq!(violation.feature, "sku");
        assert_eq!(violation.kind, ConstraintKind::MinLength);
        assert_eq!(violation.detail, "length 1 is below minLength 2");
        assert_eq!(
            violation.to_string(),
            "constraint violation on feature 'sku': MinLength (length 1 is below minLength 2)"
        );
    }

    #[test]
    fn constraint_kinds_are_distinct_and_debug_named() {
        let kinds = [
            ConstraintKind::MinLength,
            ConstraintKind::MaxLength,
            ConstraintKind::Minimum,
            ConstraintKind::Maximum,
        ];
        for (index, kind) in kinds.iter().enumerate() {
            assert_eq!(
                format!("{kind:?}"),
                ["MinLength", "MaxLength", "Minimum", "Maximum"][index],
            );
            for other in kinds.iter().skip(index + 1) {
                assert_ne!(kind, other);
            }
        }
    }
}
