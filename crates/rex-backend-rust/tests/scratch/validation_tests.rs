// Behavioral tests appended to the generated validation scratch crate's
// lib.rs. Compiled together with the generated `models.rs` content for the
// constrained catalog model above; `use crate::*` reaches the generated
// types.
#[cfg(test)]
mod validation_scratch_tests {
    use crate::*;
    use rex_runtime::validation::ConstraintKind;

    /// A profile satisfying every declared bound.
    fn valid_profile() -> Profile {
        Profile {
            name: "Alice".to_string(),
            nicknames: vec!["ab".to_string(), "cdef".to_string()],
            score: 50,
            rank: Rank::High,
            meta: Meta("abcd".to_string()),
            grade: Grade::US,
            level: Level::B,
            alt_name: Some("okay".to_string()),
            note: "unconstrained".to_string(),
        }
    }

    #[test]
    fn valid_profile_has_no_violations() {
        let violations = valid_profile().validate();
        assert!(violations.is_empty(), "violations: {violations:#?}");
    }

    #[test]
    fn violating_profile_reports_each_offended_bound() {
        let mut profile = valid_profile();
        profile.name = "A".to_string(); // length 1 < minLength 2
        profile.nicknames = vec!["a".to_string(), "abcdef".to_string(), "xyz".to_string()];
        profile.score = 150; // > maximum 100
        profile.rank = Rank::Low; // literal value 1 < minimum 2
        profile.meta = Meta("ab".to_string()); // length 2 < minLength 4
        profile.grade = Grade::EURO; // key length 4 > maxLength 3
        profile.level = Level::A; // key facet code 2 < minimum 5
        profile.alt_name = Some("x".to_string()); // length 1 < minLength 2

        let violations = profile.validate();
        assert!(!violations.is_empty(), "violations must be reported");
        assert_eq!(violations.len(), 9, "violations: {violations:#?}");

        let names_kind = |feature: &str, kind: ConstraintKind| {
            violations
                .iter()
                .any(|v| v.feature == feature && v.kind == kind)
        };
        assert!(names_kind("name", ConstraintKind::MinLength), "{violations:#?}");
        assert!(
            names_kind("nicknames", ConstraintKind::MinLength),
            "{violations:#?}"
        );
        assert!(
            names_kind("nicknames", ConstraintKind::MaxLength),
            "{violations:#?}"
        );
        assert!(names_kind("score", ConstraintKind::Maximum), "{violations:#?}");
        assert!(names_kind("rank", ConstraintKind::Minimum), "{violations:#?}");
        assert!(names_kind("meta", ConstraintKind::MinLength), "{violations:#?}");
        assert!(names_kind("grade", ConstraintKind::MaxLength), "{violations:#?}");
        assert!(names_kind("level", ConstraintKind::Minimum), "{violations:#?}");
        assert!(
            names_kind("alt_name", ConstraintKind::MinLength),
            "{violations:#?}"
        );

        // Many-valued violations carry the element index in the detail.
        let nickname_details: Vec<&str> = violations
            .iter()
            .filter(|v| v.feature == "nicknames")
            .map(|v| v.detail.as_str())
            .collect();
        assert!(
            nickname_details.iter().any(|d| d.starts_with("[0] ")),
            "element index must appear in the detail: {nickname_details:?}"
        );
        assert!(
            nickname_details.iter().any(|d| d.starts_with("[1] ")),
            "element index must appear in the detail: {nickname_details:?}"
        );
    }

    #[test]
    fn default_profile_reports_its_own_defaults() {
        // The default profile violates several bounds at once (empty name,
        // first enum literal below the minimum, empty meta, first vocabulary
        // entry code below the minimum) — validation is a report, not a
        // panic, and the unconstrained `note` never appears.
        let violations = Profile::default().validate();
        assert!(!violations.is_empty(), "violations: {violations:#?}");
        assert!(
            violations.iter().all(|v| v.feature != "note"),
            "unconstrained features must never be reported: {violations:#?}"
        );
    }

    #[test]
    fn violation_display_names_feature_and_kind() {
        let violations = Profile::default().validate();
        let name = violations
            .iter()
            .find(|v| v.feature == "name")
            .expect("default name violates minLength");
        let display = name.to_string();
        assert!(
            display.contains("name") && display.contains("MinLength"),
            "display must name the feature and the kind: {display}"
        );
    }
}
