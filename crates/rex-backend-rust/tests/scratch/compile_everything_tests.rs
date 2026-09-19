// Behavioral tests appended to the generated coverage scratch crate's
// lib.rs (the compile-everything fixture, issue #16). Compiled together
// with the generated `models.rs` content; `use crate::*` reaches the
// generated types.
#[cfg(test)]
mod coverage_scratch_tests {
    use crate::*;
    use rex_runtime::validation::ConstraintKind;

    /// Everything the builder wires, handed back by id for targeted asserts.
    struct Built {
        res: Resource,
        e: EverythingId,
        p1: PartId,
        p2: PartId,
        core: PartId,
        peer: PeerId,
        gizmo: GizmoId,
    }

    /// Constructs an instance touching every shape of the fixture — every
    /// attribute of every primitive in every multiplicity, constraints,
    /// enum/datatype/vocabulary attributes, containment (many, single,
    /// one-sided), and every mutual-opposite pair — through the generated
    /// mutators only, so opposites are guaranteed consistent.
    fn full_resource() -> Built {
        let mut res = Resource::default();
        let e = res.new_everything();
        {
            let ev = res.everything_mut(e).expect("everything");
            ev.entity_key = "E-1".to_string();
            ev.fixed_label = "pinned-by-hand".to_string();
            ev.set_req_string("full".to_string());
            ev.set_opt_string("opt".to_string());
            ev.set_many_string(vec!["café".to_string(), "日本".to_string()]);
            ev.set_req_int(11);
            ev.set_opt_int(22);
            ev.set_many_int(vec![1, 2, 3]);
            ev.set_req_long(123_456_789_012);
            ev.set_opt_long(987_654_321_098);
            ev.set_many_long(vec![-1, 1]);
            ev.set_req_short(-300);
            ev.set_opt_short(300);
            ev.set_many_short(vec![1, -2]);
            ev.set_req_float(1.5);
            ev.set_opt_float(-2.5);
            ev.set_many_float(vec![0.5, 2.5]);
            ev.set_req_double(2.25);
            ev.set_opt_double(-0.75);
            ev.set_many_double(vec![1.25, -2.125]);
            ev.set_req_boolean(true);
            ev.set_opt_boolean(false);
            ev.set_many_boolean(vec![true, false]);
            ev.set_req_byte(-128);
            ev.set_opt_byte(127);
            ev.set_many_byte(vec![1, -1]);
            ev.set_req_char('x');
            ev.set_opt_char('Ω');
            ev.set_many_char(vec!['a', 'Ω']);
            ev.set_patterned_code("abc-12".to_string());
            ev.set_min_only_name("ok".to_string());
            ev.set_max_only_name("three".to_string());
            ev.set_opt_tagged("abc".to_string());
            ev.set_bounded_int(50);
            ev.set_min_long(0);
            ev.set_max_double(5.5);
            ev.set_bounded_short(50);
            ev.set_bounded_byte(10);
            ev.set_bounded_float(1.0);
            ev.set_many_tagged(vec!["ab".to_string(), "cde".to_string()]);
            ev.set_many_scores(vec![10, 20]);
            ev.set_vitality(Vitality::High);
            ev.set_vitalities(vec![Vitality::High]);
            ev.set_sku(Sku("FIX-1234".to_string()));
            ev.set_secret_token(Secret("s3cret".to_string()));
            ev.set_many_secrets(vec![Secret("a".to_string()), Secret("b".to_string())]);
            ev.set_favorite(Color::Green);
            ev.set_palette(vec![Color::Green, Color::Blue]);
            ev.set_spare_color(Color::Blue);
        }

        // Containment: many (parts), single (core), one-sided (gizmos).
        let p1 = res.new_part();
        {
            let part = res.part_mut(p1).expect("part");
            part.set_label("p1".to_string());
            part.set_weight(60);
        }
        let p2 = res.new_part();
        {
            let part = res.part_mut(p2).expect("part");
            part.set_label("p2".to_string());
            part.set_weight(10);
        }
        let core = res.new_part();
        {
            let part = res.part_mut(core).expect("part");
            part.set_label("core".to_string());
            part.set_weight(5);
        }
        let gizmo = res.new_gizmo();
        res.gizmo_mut(gizmo).expect("gizmo").set_tag("g1".to_string());
        res.everything_add_parts(e, p1);
        res.everything_add_parts(e, p2);
        res.everything_set_core(e, Some(core));
        res.everything_add_gizmos(e, gizmo);

        // Cross references: many-many, single-many, single-single.
        let peer = res.new_peer();
        {
            let p = res.peer_mut(peer).expect("peer");
            p.peer_key = "P-1".to_string();
            p.set_label("peer".to_string());
        }
        res.everything_add_peers(e, peer);
        res.everything_set_best_peer(e, Some(peer));
        res.peer_add_inspections(peer, p1);
        res.part_set_inspector(p2, Some(peer));

        Built {
            res,
            e,
            p1,
            p2,
            core,
            peer,
            gizmo,
        }
    }

    #[test]
    fn declared_defaults_apply() {
        let e = Everything::default();
        assert_eq!(e.entity_key, "");
        assert_eq!(e.fixed_label, "pinned");
        assert_eq!(e.req_string, "base");
        assert_eq!(e.req_int, 42);
        assert_eq!(e.req_long, 7);
        assert!(e.req_boolean);
        assert_eq!(e.vitality, Vitality::High);
        assert_eq!(e.sku, Sku("FIX-0001".to_string()));
        assert_eq!(e.favorite, Color::Red);
        assert_eq!(e.opt_string, None);
        assert!(e.many_int.is_empty());
        assert_eq!(e.core, None);
        assert!(e.parts.is_empty());
        assert_eq!(Part::default().label, "unlabeled");
    }

    #[test]
    fn vocabulary_surface_keys_facets_and_lookup() {
        assert_eq!(Color::Green.key(), "Green");
        assert_eq!(Color::Green.hex(), "#00ff00");
        assert_eq!(Color::Green.name(), "Green");
        assert_eq!(Color::Green.to_string(), "Green");
        assert_eq!(Color::try_from("Blue"), Ok(Color::Blue));
        assert!(Color::try_from("Mauve").is_err());
        assert_eq!(Vitality::Low.value(), 1);
        assert_eq!(Vitality::Low.label(), "L");
        assert_eq!(Vitality::High.value(), 9);
    }

    #[test]
    fn clean_resource_has_no_violations() {
        let built = full_resource();
        let violations = built.res.everything(built.e).expect("e").validate();
        assert!(violations.is_empty(), "violations: {violations:#?}");
    }

    #[test]
    fn violating_resource_reports_each_offended_bound() {
        let mut built = full_resource();
        {
            let ev = built.res.everything_mut(built.e).expect("e");
            ev.set_patterned_code("abc".to_string()); // 3 < minLength 5 (pattern never checked)
            ev.set_min_only_name("x".to_string()); // 1 < minLength 2
            ev.set_max_only_name("toolong".to_string()); // 7 > maxLength 5
            ev.set_opt_tagged("toolong".to_string()); // 7 > maxLength 4
            ev.set_bounded_int(101); // > maximum 100
            ev.set_min_long(-6); // < minimum -5
            ev.set_max_double(10.5); // > maximum 10
            ev.set_many_tagged(vec!["a".to_string(), "toolong".to_string()]);
            ev.set_many_scores(vec![5, 25]);
            ev.set_vitality(Vitality::Low); // value 1 < minimum 2
            ev.set_vitalities(vec![Vitality::Low]); // [0] value 1 < minimum 5
            ev.set_sku(Sku("abc".to_string())); // 3 < minLength 4
            ev.set_favorite(Color::Red); // key "Red" length 3 < minLength 4
            ev.set_palette(vec![Color::Red]); // [0] key length 3 < minLength 4
        }
        let violations = built.res.everything(built.e).expect("e").validate();
        assert_eq!(violations.len(), 15, "violations: {violations:#?}");

        let names_kind = |feature: &str, kind: ConstraintKind| {
            violations
                .iter()
                .any(|v| v.feature == feature && v.kind == kind)
        };
        assert!(names_kind("patternedCode", ConstraintKind::MinLength));
        assert!(names_kind("minOnlyName", ConstraintKind::MinLength));
        assert!(names_kind("maxOnlyName", ConstraintKind::MaxLength));
        assert!(names_kind("optTagged", ConstraintKind::MaxLength));
        assert!(names_kind("boundedInt", ConstraintKind::Maximum));
        assert!(names_kind("minLong", ConstraintKind::Minimum));
        assert!(names_kind("manyTagged", ConstraintKind::MinLength));
        assert!(names_kind("manyTagged", ConstraintKind::MaxLength));
        assert!(names_kind("manyScores", ConstraintKind::Minimum));
        assert!(names_kind("manyScores", ConstraintKind::Maximum));
        assert!(names_kind("vitality", ConstraintKind::Minimum));
        assert!(names_kind("vitalities", ConstraintKind::Minimum));
        assert!(names_kind("sku", ConstraintKind::MinLength));
        assert!(names_kind("favorite", ConstraintKind::MinLength));
        assert!(names_kind("palette", ConstraintKind::MinLength));

        // Many-valued violations carry the element index in the detail.
        let tagged: Vec<&str> = violations
            .iter()
            .filter(|v| v.feature == "manyTagged")
            .map(|v| v.detail.as_str())
            .collect();
        assert!(tagged.iter().any(|d| d.starts_with("[0] ")), "{tagged:?}");
        assert!(tagged.iter().any(|d| d.starts_with("[1] ")), "{tagged:?}");

        // Descriptive bounds (short/byte/float/double — including the
        // offending 10.5 set above) and the pattern never report anything.
        for silent in ["boundedShort", "boundedByte", "boundedFloat", "maxDouble"] {
            assert!(
                violations.iter().all(|v| v.feature != silent),
                "{silent} must never be reported"
            );
        }
    }

    #[test]
    fn operations_and_derived_accessors_evaluate() {
        let mut built = full_resource();
        let res = &mut built.res;
        {
            let e = res.everything(built.e).expect("e");
            // expr bodies.
            assert_eq!(e.total_part_weight(res), 70); // parts only: 60 + 10
            assert_eq!(e.find_part(res, "p1".to_string()), Some(built.p1));
            assert_eq!(e.find_part(res, "core".to_string()), None); // core is not in parts
            assert_eq!(e.find_part(res, "nope".to_string()), None);
            assert_eq!(e.part_count(res), Some(2)); // derived defaults to 0..1
            assert!(e.has_heavy_part(res)); // p1 weighs 60 > 50; declared [1..1]
            // rust body.
            assert!(e.has_core(res));
            // navigation over the containment ends.
            assert_eq!(e.core(res).expect("core").label, "core");
            assert_eq!(e.parts(res).len(), 2);
            assert_eq!(e.peers(res).len(), 1);
            assert_eq!(e.best_peer(res).expect("best").label, "peer");
        }
        {
            let peer = res.peer(built.peer).expect("peer");
            assert_eq!(peer.inspections(res).len(), 2);
            assert_eq!(peer.best_of(res).map(|owner| owner.fixed_label.clone()),
                       Some("pinned-by-hand".to_string()));
        }
        assert_eq!(
            res.part(built.p1)
                .expect("p1")
                .inspector(res)
                .expect("inspector")
                .label,
            "peer"
        );
        // The single-containment child carries the owner on holderOf — and
        // nothing on holder.
        assert_eq!(res.part(built.core).expect("core part").holder_of, Some(built.e));
        assert_eq!(res.part(built.core).expect("core part").holder, None);

        // The required derived accessor tracks the data.
        res.part_mut(built.p1).expect("p1").set_weight(10);
        assert!(!res.everything(built.e).expect("e").has_heavy_part(res));
    }

    #[test]
    fn canonical_instance_shapes() {
        let built = full_resource();
        let json = built.res.to_instance_json();

        // Contract invariants: containers never serialized (either end of
        // the Part pair), cross references are links, enums serialize the
        // literal name, datatypes and vocabulary attributes their string.
        assert!(!json.contains("\"holder\""), "container key leaked:\n{json}");
        assert!(!json.contains("\"holderOf\""), "container key leaked:\n{json}");
        assert!(json.contains("\"$ref\": \"peer/0\""));
        assert!(json.contains("\"$ref\": \"everything/0\""));
        assert!(json.contains("\"$ref\": \"part/0\""));
        assert!(json.contains("\"vitality\": \"High\""));
        assert!(json.contains("\"sku\": \"FIX-1234\""));
        assert!(json.contains("\"favorite\": \"Green\""));
        assert!(json.contains("\"optChar\": \"Ω\""));
        assert!(json.contains("\"$id\": \"everything/0\""));
        assert!(json.contains("\"$id\": \"part/2\""));
        assert!(json.contains("\"$type\": \"Gizmo\""));
        assert!(json.contains("\"reqChar\": \"x\""));
    }

    #[test]
    fn optional_attributes_are_omitted_when_unset() {
        let mut res = Resource::default();
        let _e = res.new_everything();
        let _peer = res.new_peer();
        let json = res.to_instance_json();

        for optional in [
            "optString", "optInt", "optLong", "optShort", "optFloat", "optDouble",
            "optBoolean", "optByte", "optChar", "optTagged", "secretToken",
            "spareColor", "core", "bestPeer",
        ] {
            assert!(
                !json.contains(&format!("\"{optional}\"")),
                "unset optional {optional} must be omitted:\n{json}"
            );
        }
        // Required defaults are always written — including the NUL char.
        assert!(json.contains("\"reqChar\": \"\\u0000\""), "{json}");
        assert!(json.contains("\"vitality\": \"High\""));
        assert!(json.contains("\"sku\": \"FIX-0001\""));
        assert!(json.contains("\"favorite\": \"Red\""));
    }

    #[test]
    fn round_trip_full_resource() {
        let built = full_resource();
        let json = built.res.to_instance_json();
        let loaded = Resource::from_instance_json(&json)
            .unwrap_or_else(|error| panic!("load failed: {error}"));
        assert_eq!(loaded, built.res, "load(save(x)) != x");
        assert_eq!(loaded.to_instance_json(), json, "re-saved bytes drifted");

        // The single-containment child carries TWO container ends back to
        // the owner; each must reconstruct onto its own opposite by name.
        assert_eq!(
            loaded.part(built.core).expect("core part").holder_of,
            Some(built.e),
            "core's holderOf end was not reconstructed"
        );
        assert_eq!(
            loaded.part(built.core).expect("core part").holder,
            None,
            "core must not gain a holder it never had"
        );
        assert_eq!(loaded.part(built.p1).expect("p1").holder, Some(built.e));
        assert_eq!(loaded.part(built.p1).expect("p1").inspector, Some(built.peer));
        assert_eq!(loaded.everything(built.e).expect("e").core, Some(built.core));
        assert_eq!(
            loaded.everything(built.e).expect("e").fixed_label,
            "pinned-by-hand"
        );
    }

    #[test]
    fn round_trip_sparse_resource() {
        let mut res = Resource::default();
        let e = res.new_everything();
        let peer = res.new_peer();
        res.everything_add_peers(e, peer);
        let json = res.to_instance_json();
        let loaded = Resource::from_instance_json(&json)
            .unwrap_or_else(|error| panic!("load failed: {error}"));
        assert_eq!(loaded, res, "load(save(x)) != x");
        assert_eq!(loaded.to_instance_json(), json);
    }

    #[test]
    fn round_trip_empty_resource() {
        let res = Resource::default();
        let json = res.to_instance_json();
        assert!(json.contains("\"objects\": []"), "{json}");
        let loaded = Resource::from_instance_json(&json).expect("empty resource loads");
        assert_eq!(loaded, res);
        assert_eq!(loaded.to_instance_json(), json);
    }

    #[test]
    fn load_rejects_serialized_container() {
        let built = full_resource();
        let json = built.res.to_instance_json();
        let tampered = json.replace(
            "\"label\": \"core\"",
            "\"label\": \"core\", \"holderOf\": null",
        );
        assert!(
            Resource::from_instance_json(&tampered).is_err(),
            "a serialized container key must be rejected"
        );
    }
}
