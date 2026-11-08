    use super::*;
    use rex_ir::{
        ActorDef, ActorModel, CapabilityDef, DelegationDef, GrantDef, GrantEntry, Multiplicity,
        NeverBothDef, Package,
    };

    fn attribute(name: &str, primitive: PrimitiveType) -> Feature {
        Feature::new(
            name,
            FeatureKind::Attribute,
            TypeRef::Primitive(primitive),
            Multiplicity::REQUIRED,
        )
    }

    fn class_ref(package: &str, name: &str) -> TypeRef {
        TypeRef::Class {
            package: package.to_string(),
            name: name.to_string(),
        }
    }

    /// `class <name> { <features> }`.
    fn class(name: &str, features: Vec<Feature>) -> ClassDef {
        ClassDef::new(name, vec![], features)
    }

    /// The fixture block `Support`: Customer / Agent extends Customer /
    /// Manager, ReadTicket + RaiseRefund on Ticket, plus the given grants.
    fn fixture_actors(grants: Vec<GrantDef>) -> ActorsDef {
        let mut actors = ActorsDef::new("Support");
        for name in ["Customer", "Agent", "Manager"] {
            actors = actors.actor(ActorDef::new(name));
        }
        actors.actors[1] = ActorDef::new("Agent").extends("Customer");
        for name in ["ReadTicket", "RaiseRefund"] {
            actors = actors.capability(CapabilityDef::new(name, class_ref("demo", "Ticket")));
        }
        for grant in grants {
            actors = actors.grant(grant);
        }
        actors
    }

    /// A pair over `Ticket { id: String, title: String, internal: boolean,
    /// amount: int, refers User assignee, derived fee: double }`: the actors
    /// block lives only in the [`ActorModel`] dimension; the domain model
    /// carries the classes (no inline actors at all).
    fn fixture_pair(actors: ActorsDef) -> (ActorModel, Model) {
        let mut ticket = class(
            "Ticket",
            vec![
                attribute("id", PrimitiveType::String),
                attribute("title", PrimitiveType::String),
                attribute("internal", PrimitiveType::Boolean),
                attribute("amount", PrimitiveType::Int),
            ],
        );
        ticket.features.push(Feature::new(
            "assignee",
            FeatureKind::CrossReference,
            class_ref("demo", "User"),
            Multiplicity::OPTIONAL,
        ));
        ticket.features.push(
            Feature::new(
                "fee",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::Double),
                Multiplicity::OPTIONAL,
            )
            .derived(),
        );
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes = vec![ticket];
        model.packages.push(package);
        (ActorModel::new().block(actors), model)
    }

    fn grant(actor: &str, entries: Vec<GrantEntry>) -> GrantDef {
        let mut grant = GrantDef::new(actor);
        for entry in entries {
            grant = grant.entry(entry);
        }
        grant
    }

    /// `<delegation <name>: from -> to { entries }>`.
    fn delegation(name: &str, from: &str, to: &str, entries: Vec<GrantEntry>) -> DelegationDef {
        let mut delegation = DelegationDef::new(name, from, to);
        for entry in entries {
            delegation = delegation.entry(entry);
        }
        delegation
    }

    /// Generates and returns the `<Block>.cedar` contents.
    fn cedar_of(pair: &(ActorModel, Model)) -> String {
        generate(&pair.0, &pair.1)
            .expect("generate")
            .remove("Support.cedar")
            .expect("policies file")
    }

    /// Generates and returns the schema JSON as parsed JSON.
    fn schema_of(pair: &(ActorModel, Model)) -> serde_json::Value {
        let files = generate(&pair.0, &pair.1).expect("generate");
        let schema = files.get("Support.cedarschema.json").expect("schema file");
        serde_json::from_str(schema).expect("valid JSON")
    }

    // -- schema mapping ------------------------------------------------------

    #[test]
    fn primitive_types_map_to_cedar_types() {
        let mut actors = ActorsDef::new("N");
        actors = actors.actor(ActorDef::new("A"));
        actors = actors.capability(CapabilityDef::new("Cap", class_ref("demo", "Wide")));
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes = vec![class(
            "Wide",
            vec![
                attribute("s", PrimitiveType::String),
                attribute("c", PrimitiveType::Char),
                attribute("i", PrimitiveType::Int),
                attribute("l", PrimitiveType::Long),
                attribute("sh", PrimitiveType::Short),
                attribute("b", PrimitiveType::Byte),
                attribute("f", PrimitiveType::Float),
                attribute("d", PrimitiveType::Double),
                attribute("bo", PrimitiveType::Boolean),
            ],
        )];
        model.packages.push(package);
        let actors = ActorModel::new().block(actors);
        let files = generate(&actors, &model).expect("generate");
        let schema: serde_json::Value =
            serde_json::from_str(files.get("N.cedarschema.json").expect("schema")).expect("json");
        let attributes = &schema["N"]["entityTypes"]["Wide"]["shape"]["attributes"];
        assert_eq!(attributes["s"], serde_json::json!({"type": "String"}));
        assert_eq!(attributes["c"], serde_json::json!({"type": "String"}));
        assert_eq!(attributes["i"], serde_json::json!({"type": "Long"}));
        assert_eq!(attributes["l"], serde_json::json!({"type": "Long"}));
        assert_eq!(attributes["sh"], serde_json::json!({"type": "Long"}));
        assert_eq!(attributes["b"], serde_json::json!({"type": "Long"}));
        assert_eq!(attributes["f"], serde_json::json!({"type": "Double"}));
        assert_eq!(attributes["d"], serde_json::json!({"type": "Double"}));
        assert_eq!(attributes["bo"], serde_json::json!({"type": "Boolean"}));
    }

    #[test]
    fn date_attributes_have_no_cedar_representation() {
        // Cedar has no date type: a date attribute is not a schema
        // attribute, so a `when` condition over it is refused by name (see
        // the policy-mapping refusals below).
        let mut actors = ActorsDef::new("N");
        actors = actors.actor(ActorDef::new("A"));
        actors = actors.capability(CapabilityDef::new("Cap", class_ref("demo", "Timed")));
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes = vec![class(
            "Timed",
            vec![
                attribute("dueDate", PrimitiveType::Date),
                attribute("s", PrimitiveType::String),
            ],
        )];
        model.packages.push(package);
        let actors = ActorModel::new().block(actors);
        let files = generate(&actors, &model).expect("generate");
        let schema: serde_json::Value =
            serde_json::from_str(files.get("N.cedarschema.json").expect("schema")).expect("json");
        let attributes = &schema["N"]["entityTypes"]["Timed"]["shape"]["attributes"];
        assert_eq!(
            attributes
                .as_object()
                .expect("attributes")
                .keys()
                .collect::<Vec<_>>(),
            ["s"],
            "the date attribute is omitted from the Cedar schema"
        );
    }

    #[test]
    fn non_primitive_features_are_not_schema_attributes() {
        let pair = fixture_pair(fixture_actors(vec![]));
        let schema = schema_of(&pair);
        let attributes = &schema["Support"]["entityTypes"]["Ticket"]["shape"]["attributes"];
        assert_eq!(
            attributes
                .as_object()
                .expect("attributes")
                .keys()
                .collect::<Vec<_>>(),
            ["amount", "id", "internal", "title"],
            "canonical key order; ref + derived features are excluded"
        );
    }

    #[test]
    fn actor_extends_becomes_member_of_types_and_roots_have_none() {
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        let schema = schema_of(&fixture_pair(actors));
        let entity_types = &schema["Support"]["entityTypes"];
        assert_eq!(
            entity_types["Agent"],
            serde_json::json!({"memberOfTypes": ["Support::Customer"]})
        );
        assert_eq!(entity_types["Customer"], serde_json::json!({}));
        assert!(
            entity_types["Customer"].get("memberOfTypes").is_none(),
            "roots carry no memberOfTypes key"
        );
    }

    #[test]
    fn principal_types_follow_actor_declaration_order_and_count_forbids() {
        // Grants appear in reverse declaration order; the forbid on Manager
        // must still place Manager after Customer (declaration order). Agent
        // is granted nothing directly, but extends Customer — whose permit
        // reaches it through the extends chain (rexlang#132).
        let actors = fixture_actors(vec![
            grant("Manager", vec![GrantEntry::forbid("ReadTicket")]),
            grant("Customer", vec![GrantEntry::permit("ReadTicket")]),
        ]);
        let schema = schema_of(&fixture_pair(actors));
        let applies_to = &schema["Support"]["actions"]["ReadTicket"]["appliesTo"];
        assert_eq!(
            applies_to["principalTypes"],
            serde_json::json!(["Support::Customer", "Support::Agent", "Support::Manager"]),
            "declaration order, forbid counts, inherited permits widen"
        );
        assert_eq!(
            applies_to["resourceTypes"],
            serde_json::json!(["Support::Ticket"])
        );
        assert_eq!(
            applies_to["context"],
            serde_json::json!({"type": "Record", "attributes": {}})
        );
    }

    #[test]
    fn inherited_grants_widen_action_principal_types() {
        // The rexlang#132 repro: only the root actor carries a grant, yet the
        // agent's inherited authority makes its principal type valid for the
        // action — strict request validation must not reject it before the
        // policy set (`principal is Customer`) gets to match it via the
        // memberOf hierarchy the same schema declares.
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        let schema = schema_of(&fixture_pair(actors));
        assert_eq!(
            schema["Support"]["actions"]["ReadTicket"]["appliesTo"]["principalTypes"],
            serde_json::json!(["Support::Customer", "Support::Agent"]),
            "the agent inherits the permit along its extends chain"
        );
    }

    #[test]
    fn an_extends_chain_widens_principal_types_transitively() {
        let mut actors = ActorsDef::new("Support");
        actors = actors.actor(ActorDef::new("Root"));
        actors = actors.actor(ActorDef::new("Agent").extends("Root"));
        actors = actors.actor(ActorDef::new("Agent2").extends("Agent"));
        actors = actors.capability(CapabilityDef::new(
            "ReadTicket",
            class_ref("demo", "Ticket"),
        ));
        actors = actors.grant(grant("Root", vec![GrantEntry::permit("ReadTicket")]));
        let schema = schema_of(&fixture_pair(actors));
        assert_eq!(
            schema["Support"]["actions"]["ReadTicket"]["appliesTo"]["principalTypes"],
            serde_json::json!(["Support::Root", "Support::Agent", "Support::Agent2"]),
            "every descendant along the extends chain inherits the grant"
        );
    }

    #[test]
    fn a_delegation_to_an_actor_widens_principal_types() {
        // Delegation bindings are authority too: the `to` actor's principal
        // type must validate for the delegated capability even without a
        // direct grant (the delegator itself gains nothing here — it holds
        // no grant of its own).
        let mut actors = ActorsDef::new("Support");
        actors = actors.actor(ActorDef::new("Customer"));
        actors = actors.actor(ActorDef::new("Triage"));
        actors = actors.capability(CapabilityDef::new(
            "ReadTicket",
            class_ref("demo", "Ticket"),
        ));
        actors = actors.delegation(delegation(
            "Intake",
            "Customer",
            "Triage",
            vec![GrantEntry::permit("ReadTicket")],
        ));
        let schema = schema_of(&fixture_pair(actors));
        assert_eq!(
            schema["Support"]["actions"]["ReadTicket"]["appliesTo"]["principalTypes"],
            serde_json::json!(["Support::Triage"]),
            "the delegation's `to` actor reaches the action"
        );
    }

    #[test]
    fn an_inherited_forbid_still_counts_for_principal_types() {
        // Forbid entries bound an actor to the action as much as permits do
        // (the schema documents who may appear in requests); inheritance
        // propagates that binding down the extends chain. Manager is
        // unconnected in this fixture — no extends, no grant — and stays
        // excluded.
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::forbid("ReadTicket")],
        )]);
        let schema = schema_of(&fixture_pair(actors));
        assert_eq!(
            schema["Support"]["actions"]["ReadTicket"]["appliesTo"]["principalTypes"],
            serde_json::json!(["Support::Customer", "Support::Agent"]),
            "the forbid binding reaches the extends descendant"
        );
    }

    #[test]
    fn capability_without_grants_gets_empty_principal_types() {
        let schema = schema_of(&fixture_pair(fixture_actors(vec![])));
        assert_eq!(
            schema["Support"]["actions"]["RaiseRefund"]["appliesTo"]["principalTypes"],
            serde_json::json!([]),
            "no grant mentions RaiseRefund"
        );
    }

    #[test]
    fn multiple_blocks_emit_one_pair_each_in_ir_order() {
        let (mut actors, model) = fixture_pair(fixture_actors(vec![]));
        let mut second = ActorsDef::new("Billing");
        second = second.actor(ActorDef::new("Clerk"));
        actors.blocks.push(second);
        let files = generate(&actors, &model).expect("generate");
        assert_eq!(
            files.keys().collect::<Vec<_>>(),
            [
                "Billing.cedar",
                "Billing.cedarschema.json",
                "Support.cedar",
                "Support.cedarschema.json"
            ]
        );
    }

    // -- pair API -------------------------------------------------------------

    #[test]
    fn empty_actor_model_yields_an_empty_map() {
        let (_, model) = fixture_pair(fixture_actors(vec![]));
        let empty = ActorModel::new();
        let files = generate(&empty, &model).expect("generate");
        assert!(files.is_empty(), "got: {:?}", files.keys());
    }

    #[test]
    fn blocks_and_classes_live_in_different_packages_of_the_pair() {
        // The payoff surface: the actors block references its capability
        // class through a package-qualified TypeRef, and the class is found
        // in the domain model even though the block itself is in no package
        // of that model at all.
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        let pair = fixture_pair(actors);
        let files = generate(&pair.0, &pair.1).expect("generate");
        assert_eq!(files.keys().len(), 2);
        assert!(files["Support.cedar"].contains("resource is Support::Ticket)"));
    }

    #[test]
    fn generate_for_model_wraps_inline_blocks_and_matches_the_pair_call() {
        // The `gen cedar x.mox` path: a model with inline `actors` blocks
        // produces exactly what the explicit pair call produces.
        let mut actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        actors.capabilities[0].class = class_ref("demo", "Ticket");
        let mut model = Model::new();
        let mut package = Package::new("demo");
        package.classes = vec![class(
            "Ticket",
            vec![attribute("id", PrimitiveType::String)],
        )];
        package.actors = vec![actors.clone()];
        model.packages.push(package);

        let via_model = generate_for_model(&model).expect("generate_for_model");
        let via_pair = generate(&ActorModel::new().block(actors), &model).expect("generate");
        assert_eq!(via_model, via_pair);
        assert_eq!(via_model.keys().len(), 2);
    }

    // -- policy mapping ------------------------------------------------------

    #[test]
    fn banner_and_never_both_comment_lines_are_pinned() {
        let mut actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        actors = actors.never_both(NeverBothDef::new("ReadTicket", "RaiseRefund"));
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.starts_with(
                "// generated by rexlang from actors block `Support` — do not edit\n\
                 // default-deny: anything not permitted is forbidden\n\n"
            ),
            "banner: {cedar}"
        );
        assert!(
            cedar.ends_with("// never_both: ReadTicket, RaiseRefund\n"),
            "trailing comment: {cedar}"
        );
    }

    #[test]
    fn effect_policy_line_shape_is_pinned() {
        let actors = fixture_actors(vec![grant(
            "Agent",
            vec![GrantEntry::forbid("RaiseRefund")],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains(
                "forbid(principal is Support::Agent, action == \
                 Support::Action::\"RaiseRefund\", resource is Support::Ticket);"
            ),
            "policy line: {cedar}"
        );
    }

    #[test]
    fn obligations_become_annotation_lines_above_the_policy() {
        let actors = fixture_actors(vec![grant(
            "Agent",
            vec![GrantEntry::permit("RaiseRefund")
                .obligation("audit")
                .obligation("four-eyes")],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains(
                "@obligation(\"audit,four-eyes\")\npermit(\
                 principal is Support::Agent"
            ),
            "multiple obligations join one annotation value (Cedar rejects \
             duplicate annotation keys): {cedar}"
        );
    }

    #[test]
    fn a_single_obligation_stays_a_bare_annotation() {
        let actors = fixture_actors(vec![grant(
            "Agent",
            vec![GrantEntry::permit("RaiseRefund").obligation("audit")],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains("@obligation(\"audit\")\npermit(principal is Support::Agent"),
            "single obligation: {cedar}"
        );
    }

    #[test]
    fn when_conditions_map_names_and_ops_fully_parenthesized() {
        let actors = fixture_actors(vec![grant(
            "Agent",
            vec![GrantEntry::permit("RaiseRefund")
                .when("(!(internal) && amount <= 1000) || title == \"a \\ b\"")],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains(
                "when { ((!(resource.internal) && (resource.amount <= 1000)) \
                 || (resource.title == \"a \\\\ b\")) };"
            ),
            "condition: {cedar}"
        );
    }

    #[test]
    fn rejected_constructs_name_the_capability_and_the_construct() {
        let rejected: [(&str, &str); 9] = [
            ("let x = 1; x", "`let`"),
            ("if internal { 1 } else { 2 }", "`if`"),
            ("internal?.flag == true", "`?.`"),
            ("internal ?: true", "`?:`"),
            ("internal == null", "`null`"),
            ("internal.filter(x => x)", "collection algebra `filter`"),
            ("score(amount)", "a free-function call"),
            ("x => x", "a lambda"),
            ("[1, 2]", "list literals"),
        ];
        for (source, construct) in rejected {
            let actors = fixture_actors(vec![grant(
                "Agent",
                vec![GrantEntry::permit("RaiseRefund").when(source)],
            )]);
            let pair = fixture_pair(actors);
            let error = generate(&pair.0, &pair.1).expect_err("must be rejected");
            assert_eq!(
                error.to_string(),
                format!(
                    "capability `RaiseRefund`: `when` condition uses {construct}, \
                     which has no Cedar mapping"
                ),
                "source: {source}"
            );
        }
    }

    #[test]
    fn condition_over_non_primitive_attribute_is_an_error() {
        let actors = fixture_actors(vec![grant(
            "Agent",
            vec![GrantEntry::permit("RaiseRefund").when("assignee != null")],
        )]);
        let pair = fixture_pair(actors);
        let error = generate(&pair.0, &pair.1).expect_err("must be rejected");
        assert_eq!(
            error.to_string(),
            "capability `RaiseRefund`: `when` condition references `assignee`, \
             which is not a primitive attribute of `Ticket`"
        );
    }

    #[test]
    fn date_conditions_are_refused_naming_the_capability() {
        // Cedar has no date type: a condition over a date attribute is
        // refused (the attribute is not a primitive attribute)...
        let mut actors = ActorsDef::new("Support");
        actors = actors.actor(ActorDef::new("Agent"));
        actors = actors.capability(CapabilityDef::new(
            "RaiseRefund",
            class_ref("demo", "Ticket"),
        ));
        actors = actors.grant(grant(
            "Agent",
            vec![GrantEntry::permit("RaiseRefund").when("dueDate >= date(\"2026-01-01\")")],
        ));
        let mut model = Model::new();
        let mut package = Package::new("demo");
        let mut ticket = class("Ticket", vec![attribute("amount", PrimitiveType::Int)]);
        ticket.features.push(Feature::new(
            "dueDate",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::Date),
            Multiplicity::REQUIRED,
        ));
        package.classes = vec![ticket];
        model.packages.push(package);
        let error =
            generate(&ActorModel::new().block(actors), &model).expect_err("must be refused");
        assert!(
            error.to_string().contains("capability `RaiseRefund`"),
            "the refusal names the capability: {error}"
        );

        // ...and so is a bare date literal, which the emitter reaches when
        // the rest of the condition maps.
        let actors = fixture_actors(vec![grant(
            "Agent",
            vec![GrantEntry::permit("RaiseRefund")
                .when("amount > 0 || date(\"2026-01-01\") < date(\"2027-01-01\")")],
        )]);
        let pair = fixture_pair(actors);
        let error = generate(&pair.0, &pair.1).expect_err("must be refused");
        assert_eq!(
            error.to_string(),
            "capability `RaiseRefund`: `when` condition uses a date literal, \
             which has no Cedar mapping"
        );
    }

    #[test]
    fn cedar_entries_are_spliced_verbatim_as_their_own_paragraph() {
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![
                GrantEntry::permit("ReadTicket"),
                GrantEntry::permit("").cedar("permit(principal, action, resource);"),
                GrantEntry::forbid("RaiseRefund"),
            ],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains(
                ";\n\npermit(principal, action, resource);\n\nforbid(\
                 principal is Support::Customer"
            ),
            "splice sits between the two policies as its own paragraph: {cedar}"
        );
    }

    #[test]
    fn policy_files_end_with_exactly_one_newline() {
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(cedar.ends_with('\n'));
        assert!(!cedar.ends_with("\n\n"), "exactly one trailing newline");
    }

    #[test]
    fn inherited_authority_emits_a_policy_per_principal_type() {
        // Cedar's `is` is an exact-type match, so an actor whose extends
        // chain carries the grant needs its own policy — otherwise the
        // widened `principalTypes` admit the agent's request at validation
        // and the policy set would still deny it at evaluation (rexlang#132).
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains(
                "permit(principal is Support::Customer, action == \
                 Support::Action::\"ReadTicket\", resource is Support::Ticket);"
            ),
            "{cedar}"
        );
        assert!(
            cedar.contains(
                "permit(principal is Support::Agent, action == \
                 Support::Action::\"ReadTicket\", resource is Support::Ticket);"
            ),
            "the agent inherits the grant and gets its own policy: {cedar}"
        );
        // Manager is unconnected in this fixture — no extends, no grant —
        // so no policy of its own.
        assert!(!cedar.contains("principal is Support::Manager"), "{cedar}");
    }

    // -- delegation evidence --------------------------------------------------

    #[test]
    fn a_block_without_delegations_keeps_its_exact_shape() {
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )]);
        let cedar = cedar_of(&fixture_pair(actors));
        assert_eq!(
            cedar,
            "// generated by rexlang from actors block `Support` — do not edit\n\
             // default-deny: anything not permitted is forbidden\n\
             \n\
             permit(principal is Support::Customer, action == \
             Support::Action::\"ReadTicket\", resource is Support::Ticket);\n\
             \n\
             permit(principal is Support::Agent, action == \
             Support::Action::\"ReadTicket\", resource is Support::Ticket);\n",
            "no delegation evidence may appear when the block has none; the \
             Customer grant also emits the inheriting Agent's policy"
        );
    }

    #[test]
    fn delegation_evidence_comment_is_pinned_byte_for_byte() {
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )])
        .delegation(delegation(
            "CustomerToAgent",
            "Customer",
            "Agent",
            vec![
                GrantEntry::permit("ReadTicket"),
                GrantEntry::permit("RaiseRefund"),
            ],
        ));
        let cedar = cedar_of(&fixture_pair(actors));
        assert_eq!(
            cedar,
            "// generated by rexlang from actors block `Support` — do not edit\n\
             // default-deny: anything not permitted is forbidden\n\
             \n\
             permit(principal is Support::Customer, action == \
             Support::Action::\"ReadTicket\", resource is Support::Ticket);\n\
             \n\
             permit(principal is Support::Agent, action == \
             Support::Action::\"ReadTicket\", resource is Support::Ticket);\n\
             \n\
             // delegation CustomerToAgent: Customer -> Agent (2 capabilities)\n",
            "delegations are evidence comments only; policies stay unchanged"
        );
    }

    #[test]
    fn delegations_emit_one_evidence_line_each_in_ir_order() {
        let actors = fixture_actors(vec![])
            .delegation(delegation(
                "FrontDesk",
                "Customer",
                "Agent",
                vec![GrantEntry::permit("ReadTicket")],
            ))
            .delegation(delegation(
                "Escalation",
                "Manager",
                "Agent",
                vec![GrantEntry::permit("RaiseRefund")],
            ));
        let cedar = cedar_of(&fixture_pair(actors));
        assert_eq!(
            cedar,
            "// generated by rexlang from actors block `Support` — do not edit\n\
             // default-deny: anything not permitted is forbidden\n\
             \n\
             // delegation FrontDesk: Customer -> Agent (1 capabilities)\n\
             \n\
             // delegation Escalation: Manager -> Agent (1 capabilities)\n"
        );
    }

    #[test]
    fn delegation_count_only_counts_permit_entries() {
        let actors = fixture_actors(vec![]).delegation(delegation(
            "Mixed",
            "Customer",
            "Agent",
            vec![
                GrantEntry::forbid("RaiseRefund"),
                GrantEntry::permit("ReadTicket"),
                GrantEntry::permit("RaiseRefund"),
                GrantEntry::forbid("ReadTicket"),
            ],
        ));
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains("// delegation Mixed: Customer -> Agent (2 capabilities)\n"),
            "forbid entries never count toward the total: {cedar}"
        );
    }

    #[test]
    fn zero_permit_delegations_still_emit_the_comment() {
        let actors = fixture_actors(vec![])
            .delegation(delegation(
                "OnlyForbids",
                "Customer",
                "Agent",
                vec![GrantEntry::forbid("RaiseRefund")],
            ))
            .delegation(delegation("NoEntries", "Manager", "Agent", vec![]));
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.contains("// delegation OnlyForbids: Customer -> Agent (0 capabilities)\n"),
            "forbid-only delegation: {cedar}"
        );
        assert!(
            cedar.contains("// delegation NoEntries: Manager -> Agent (0 capabilities)\n"),
            "empty delegation: {cedar}"
        );
    }

    #[test]
    fn delegation_comments_follow_the_never_both_comment() {
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )])
        .never_both(NeverBothDef::new("ReadTicket", "RaiseRefund"))
        .delegation(delegation(
            "SupportHelp",
            "Customer",
            "Agent",
            vec![GrantEntry::permit("ReadTicket")],
        ));
        let cedar = cedar_of(&fixture_pair(actors));
        assert!(
            cedar.ends_with(
                "// never_both: ReadTicket, RaiseRefund\n\
                 \n\
                 // delegation SupportHelp: Customer -> Agent (1 capabilities)\n"
            ),
            "delegation evidence trails the never_both evidence: {cedar}"
        );
    }

    #[test]
    fn delegation_with_purpose_pins_the_purpose_suffix() {
        let actors = fixture_actors(vec![grant(
            "Customer",
            vec![GrantEntry::permit("ReadTicket")],
        )])
        .delegation(
            delegation(
                "CustomerToAgent",
                "Customer",
                "Agent",
                vec![
                    GrantEntry::permit("ReadTicket"),
                    GrantEntry::permit("RaiseRefund"),
                ],
            )
            .purpose("seasonal support surge"),
        );
        let cedar = cedar_of(&fixture_pair(actors));
        assert_eq!(
            cedar,
            "// generated by rexlang from actors block `Support` — do not edit\n\
             // default-deny: anything not permitted is forbidden\n\
             \n\
             permit(principal is Support::Customer, action == \
             Support::Action::\"ReadTicket\", resource is Support::Ticket);\n\
             \n\
             permit(principal is Support::Agent, action == \
             Support::Action::\"ReadTicket\", resource is Support::Ticket);\n\
             \n\
             // delegation CustomerToAgent: Customer -> Agent \
             (2 capabilities) purpose: seasonal support surge\n",
            "a declared purpose appends exactly ` purpose: <P>` to the \
             evidence line; nothing else changes"
        );
    }

    #[test]
    fn delegation_without_purpose_keeps_the_pre_feature_byte_format() {
        let actors = fixture_actors(vec![]).delegation(delegation(
            "Plain",
            "Customer",
            "Agent",
            vec![GrantEntry::permit("ReadTicket")],
        ));
        let cedar = cedar_of(&fixture_pair(actors));
        assert_eq!(
            cedar,
            "// generated by rexlang from actors block `Support` — do not edit\n\
             // default-deny: anything not permitted is forbidden\n\
             \n\
             // delegation Plain: Customer -> Agent (1 capabilities)\n",
            "no purpose: the line ends right after the count, no suffix and \
             no trailing space"
        );
    }

    #[test]
    fn mixed_purpose_delegations_keep_each_line_correct_in_ir_order() {
        let actors = fixture_actors(vec![])
            .delegation(
                delegation(
                    "FrontDesk",
                    "Customer",
                    "Agent",
                    vec![GrantEntry::permit("ReadTicket")],
                )
                .purpose("front desk coverage"),
            )
            .delegation(delegation(
                "Escalation",
                "Manager",
                "Agent",
                vec![GrantEntry::permit("RaiseRefund")],
            ));
        let cedar = cedar_of(&fixture_pair(actors));
        assert_eq!(
            cedar,
            "// generated by rexlang from actors block `Support` — do not edit\n\
             // default-deny: anything not permitted is forbidden\n\
             \n\
             // delegation FrontDesk: Customer -> Agent (1 capabilities) \
             purpose: front desk coverage\n\
             \n\
             // delegation Escalation: Manager -> Agent (1 capabilities)\n",
            "each line carries its own delegation's purpose, in IR order"
        );
    }
