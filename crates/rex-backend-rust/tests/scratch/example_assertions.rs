// Behavioral assertions for the five canonical examples, appended to the
// generated scratch crate's lib.rs. Each submodule targets one `pub mod
// <example>` module above and builds its resources through generated
// mutators only, so opposite pairs stay consistent by construction.
//
// Canonical-instance agreement: for `library`, `ecommerce`, `org`, and
// `support` the mutator-built resource's `to_instance_json()` must
// byte-match `examples/instances/<name>.instance.json` (checked when the
// harness sets `REX_EXAMPLES_DIR`; rewritten with `REX_UPDATE_FIXTURES=1`,
// the same convention as the workspace goldens). `iot` and `shapes` are
// schema-validated in the `rex-backend-jsonschema` harness instead:
// `Sensor.sensorId` is `readonly` (no generated mutator can set it), and
// `shapes` inheritance is not materialized in Rust codegen, so their Rust
// serializations deliberately do not pin the full documents.
#[cfg(test)]
mod example_assertions {
    /// Byte-compares the canonical instance JSON against the committed
    /// example file when the harness environment is present.
    fn assert_instance_json(name: &str, json: &str) {
        let Ok(dir) = std::env::var("REX_EXAMPLES_DIR") else {
            return;
        };
        let path = std::path::Path::new(&dir)
            .join("instances")
            .join(format!("{name}.instance.json"));
        if std::env::var_os("REX_UPDATE_FIXTURES").is_some() {
            std::fs::create_dir_all(path.parent().unwrap()).expect("create instances dir");
            std::fs::write(&path, json).expect("write instance fixture");
            eprintln!("updated {}", path.display());
            return;
        }
        let expected = std::fs::read_to_string(&path).unwrap_or_else(|error| {
            panic!("missing canonical instance {}: {error}", path.display())
        });
        assert_eq!(
            json, expected,
            "canonical instance JSON for {name} drifted from {path:?}"
        );
    }

    // ------------------------------------------------------------------
    // library — the flagship: Tier-2 expr bodies for `getBook` and the
    // derived `citation`.
    // ------------------------------------------------------------------
    mod library_example {
        use crate::library::*;

        /// The resource serialized in `examples/instances/library.instance.json`.
        fn conformance_resource() -> Resource {
            let mut res = Resource::default();
            let lib = res.new_library();
            let dune = res.new_book();
            let hobbit = res.new_book();
            let frank = res.new_writer();
            let tolkien = res.new_writer();

            res.library_add_books(lib, dune);
            res.library_add_books(lib, hobbit);

            {
                let book = res.book_mut(dune).unwrap();
                book.set_title("Dune".to_string());
                book.set_pages(412);
                book.set_copyright(Date("1965-08-01".to_string()));
                book.set_category(BookCategory::ScienceFiction);
            }
            res.book_add_authors(dune, frank);

            {
                let book = res.book_mut(hobbit).unwrap();
                book.set_title("The Hobbit".to_string());
                book.set_pages(310);
                book.set_copyright(Date("1937-09-21".to_string()));
                book.set_category(BookCategory::Mystery);
            }
            res.book_add_authors(hobbit, frank);
            res.book_add_authors(hobbit, tolkien);

            res.writer_mut(frank)
                .unwrap()
                .set_name("Frank Herbert".to_string());
            res.writer_mut(tolkien)
                .unwrap()
                .set_name("J.R.R. Tolkien".to_string());
            res
        }

        #[test]
        fn get_book_finds_dune_and_returns_none_for_a_missing_title() {
            let res = conformance_resource();
            let lib = res.libraries.iter().next().map(|(key, _)| key).unwrap();
            let library = res.library(lib).unwrap();

            let found = library.get_book(&res, "Dune".to_string());
            assert_eq!(
                found.map(|id| res.book(id).unwrap().title.clone()),
                Some("Dune".to_string()),
                "getBook must resolve the contained book by title"
            );
            assert_eq!(
                library.get_book(&res, "no such shelf".to_string()),
                None,
                "a missing title must yield the absent state"
            );
        }

        #[test]
        fn derived_citation_depends_on_the_page_count() {
            // `derived String citation { expr { if pages > 400 { title }
            // else { "short read" } } }` — both branches exercised.
            let res = conformance_resource();
            let dune = res
                .books
                .iter()
                .find(|(_, book)| book.title == "Dune")
                .map(|(key, _)| key)
                .unwrap();
            let hobbit = res
                .books
                .iter()
                .find(|(_, book)| book.title == "The Hobbit")
                .map(|(key, _)| key)
                .unwrap();
            assert_eq!(
                res.book(dune).unwrap().citation(&res),
                Some("Dune".to_string()),
                "a long book cites its title"
            );
            assert_eq!(
                res.book(hobbit).unwrap().citation(&res),
                Some("short read".to_string()),
                "a short book gets the fallback citation"
            );
        }

        #[test]
        fn canonical_instance_matches_the_example_file_and_round_trips() {
            let res = conformance_resource();
            let json = res.to_instance_json();

            let loaded = Resource::from_instance_json(&json).expect("load canonical instance");
            assert_eq!(loaded, res, "load(save(x)) != x");
            assert_eq!(loaded.to_instance_json(), json, "re-saved bytes drifted");

            super::assert_instance_json("library", &json);
        }
    }

    // ------------------------------------------------------------------
    // ecommerce — vocabulary-typed attribute, readonly id, containment
    // lines, derived int total over a map/sum pipeline.
    // ------------------------------------------------------------------
    mod ecommerce_example {
        use crate::ecommerce::*;

        /// One customer, one order with three lines: 2×1000 + 1×2500 + 3×500.
        fn order_with_three_lines() -> (Resource, OrderId) {
            let mut res = Resource::default();
            let alice = res.new_customer();
            res.customer_mut(alice)
                .unwrap()
                .set_name("Alice".to_string());

            let order = res.new_order();
            {
                let slot = res.order_mut(order).unwrap();
                // `id readonly String orderNo` has NO generated setter (the
                // compile-time proof of readonly), so initialization is the
                // one direct field write in this suite; every other value
                // goes through generated mutators.
                slot.order_no = "SO-0001".to_string();
                slot.set_currency(Currency::EUR);
            }
            res.order_set_customer(order, Some(alice));

            for (sku, quantity, unit_price) in
                [("BOOK-DUNE", 2, 1000), ("BOOK-HOBBIT", 1, 2500), ("PEN", 3, 500)]
            {
                let line = res.new_order_line();
                {
                    let slot = res.order_line_mut(line).unwrap();
                    slot.set_sku(sku.to_string());
                    slot.set_quantity(quantity);
                    slot.set_unit_price(unit_price);
                }
                res.order_add_lines(order, line);
            }
            res.customer_add_orders(alice, order);
            (res, order)
        }

        #[test]
        fn order_total_sums_quantity_times_unit_price() {
            let (res, order) = order_with_three_lines();
            assert_eq!(
                res.order(order).unwrap().total(&res),
                Some(2 * 1000 + 1 * 2500 + 3 * 500),
                "total must sum quantity * unitPrice over all lines"
            );

            // An order without lines totals to the algebra's empty sum.
            let mut empty = Resource::default();
            let bare = empty.new_order();
            assert_eq!(empty.order(bare).unwrap().total(&empty), Some(0));
        }

        #[test]
        fn order_no_is_readonly_stored_but_never_resettable() {
            // `id readonly String orderNo` keeps the stored field (read
            // here through the resource), but the generated code has NO
            // `set_order_no` mutator — any call would fail to compile this
            // crate, which is the compile-time proof of absence. The schema
            // side pins the `readOnly` keyword (examples_schemas.rs).
            let (res, order) = order_with_three_lines();
            assert_eq!(res.order(order).unwrap().order_no, "SO-0001");
        }

        #[test]
        fn currency_attribute_round_trips_by_key_with_facet_accessors() {
            let (res, order) = order_with_three_lines();
            let currency = res.order(order).unwrap().currency;
            assert_eq!(currency.key(), "EUR");
            assert_eq!(currency.symbol(), "€");
            assert_eq!(currency.minor_units(), 2);
            assert_eq!(Currency::try_from("EUR"), Ok(Currency::EUR));
            assert!(Currency::try_from("XYZ").is_err(), "closed key set");
        }

        #[test]
        fn canonical_instance_matches_the_example_file_and_round_trips() {
            let (res, _order) = order_with_three_lines();
            let json = res.to_instance_json();

            let loaded = Resource::from_instance_json(&json).expect("load canonical instance");
            assert_eq!(loaded, res, "load(save(x)) != x");
            assert_eq!(loaded.to_instance_json(), json, "re-saved bytes drifted");

            super::assert_instance_json("ecommerce", &json);
        }
    }

    // ------------------------------------------------------------------
    // org — three-level hierarchy (Department → Team → Employee) built
    // through container/refers mutators, navigation on both ends of the
    // lead (manager) / members (reports) relation, and two expr operations
    // over the members collection.
    //
    // The manager/reports relation spans Team↔Employee rather than
    // Employee↔Employee: same-class opposite pairs would collide in the
    // generated mutator signatures (both parameters are named after the
    // single class), so the model factors the manager role out into Team.
    // ------------------------------------------------------------------
    mod org_example {
        use crate::org::*;

        /// Engineering ⊃ { staff: [Ada], teams: [Platform (lead Ada,
        /// members Grace and Alan)] } — every object has exactly one
        /// container, as the canonical format requires.
        #[allow(clippy::type_complexity)]
        fn org_resource() -> (Resource, DepartmentId, EmployeeId, EmployeeId, EmployeeId, TeamId) {
            let mut res = Resource::default();
            let engineering = res.new_department();
            res.department_mut(engineering)
                .unwrap()
                .set_name("Engineering".to_string());

            let ada = res.new_employee();
            {
                let slot = res.employee_mut(ada).unwrap();
                slot.set_name("Ada".to_string());
                slot.set_salary(200_000);
            }
            let grace = res.new_employee();
            {
                let slot = res.employee_mut(grace).unwrap();
                slot.set_name("Grace".to_string());
                slot.set_salary(150_000);
            }
            let alan = res.new_employee();
            {
                let slot = res.employee_mut(alan).unwrap();
                slot.set_name("Alan".to_string());
                slot.set_salary(90_000);
            }

            let platform = res.new_team();
            res.team_mut(platform)
                .unwrap()
                .set_name("Platform".to_string());

            res.department_add_staff(engineering, ada);
            res.department_add_teams(engineering, platform);
            res.team_add_members(platform, grace);
            res.team_add_members(platform, alan);
            res.team_set_lead(platform, Some(ada));
            (
                res,
                engineering,
                ada,
                grace,
                alan,
                platform,
            )
        }

        #[test]
        fn hierarchy_navigates_lead_and_members_both_ways() {
            let (res, engineering, ada, grace, alan, platform) = org_resource();

            // Down (manager → reports): the team lists its members and
            // resolves its lead through the resource.
            let member_names: Vec<String> = res.team(platform).unwrap().members(&res).iter().map(|member| member.name.clone()).collect();
            assert_eq!(
                member_names,
                vec!["Grace".to_string(), "Alan".to_string()]
            );
            assert_eq!(res.team(platform).unwrap().lead, Some(ada));
            assert_eq!(
                res.team(platform)
                    .unwrap()
                    .lead(&res)
                    .map(|lead| lead.name.clone()),
                Some("Ada".to_string())
            );

            // …and up (report → manager): the member's back-pointer and the
            // lead's reverse link.
            assert_eq!(res.employee(grace).unwrap().member_of, Some(platform));
            assert_eq!(res.employee(alan).unwrap().member_of, Some(platform));
            assert_eq!(res.employee(ada).unwrap().member_of, None);
            assert_eq!(
                res.employee(alan)
                    .unwrap()
                    .member_of(&res)
                    .map(|team| team.name.clone()),
                Some("Platform".to_string())
            );
            assert_eq!(res.employee(ada).unwrap().leads_teams, vec![platform]);
            assert!(res.employee(grace).unwrap().leads_teams.is_empty());

            // Containment: the department holds the lead as staff, owns the
            // team, and every object has exactly one container.
            let staff = res.department(engineering).unwrap().staff(&res);
            assert_eq!(staff.len(), 1);
            assert_eq!(staff[0].name, "Ada");
            assert_eq!(
                res.department(engineering).unwrap().teams(&res).len(),
                1
            );
            assert_eq!(res.employee(ada).unwrap().department, Some(engineering));
            assert_eq!(res.employee(grace).unwrap().department, None);
        }

        #[test]
        fn any_member_earns_over_threshold() {
            let (res, _engineering, _ada, _grace, _alan, platform) = org_resource();
            let team = res.team(platform).unwrap();
            assert!(team.any_member_earns_over(&res, 100_000));
            assert!(!team.any_member_earns_over(&res, 150_000));
        }

        #[test]
        fn payroll_sums_member_salaries() {
            let (res, _engineering, _ada, _grace, _alan, platform) = org_resource();
            assert_eq!(res.team(platform).unwrap().payroll(&res), 150_000 + 90_000);
        }

        #[test]
        fn canonical_instance_matches_the_example_file_and_round_trips() {
            let (res, _engineering, _ada, _grace, _alan, _platform) = org_resource();
            let json = res.to_instance_json();

            let loaded = Resource::from_instance_json(&json).expect("load canonical instance");
            assert_eq!(loaded, res, "load(save(x)) != x");
            assert_eq!(loaded.to_instance_json(), json, "re-saved bytes drifted");

            super::assert_instance_json("org", &json);
        }
    }

    // ------------------------------------------------------------------
    // iot — Tier-1 datatype bodies (create/convert), labeled enum, one-way
    // navigation, derived display name.
    // ------------------------------------------------------------------
    mod iot_example {
        use crate::iot::*;

        fn greenhouse() -> (Resource, DeviceId, SensorId, SensorId) {
            let mut res = Resource::default();
            let device = res.new_device();
            {
                let slot = res.device_mut(device).unwrap();
                slot.set_name("Greenhouse Hub".to_string());
                slot.set_state(DeviceState::Online);
            }
            let temperature = res.new_sensor();
            let humidity = res.new_sensor();
            res.device_add_sensors(device, temperature);
            res.device_add_sensors(device, humidity);
            (res, device, temperature, humidity)
        }

        #[test]
        fn mac_create_convert_round_trip() {
            let mac = Mac::create("a1:b2:c3:d4:e5:f5".to_string());
            assert_eq!(mac, Mac("a1:b2:c3:d4:e5:f5".to_string()));
            assert_eq!(mac.convert(), "a1:b2:c3:d4:e5:f5".to_string());
        }

        #[test]
        fn device_state_enum_label_and_value() {
            assert_eq!(DeviceState::Online.name(), "Online");
            assert_eq!(DeviceState::Online.label(), "on");
            assert_eq!(DeviceState::Online.value(), 0);
            assert_eq!(DeviceState::Offline.name(), "Offline");
            assert_eq!(DeviceState::Offline.label(), "off");
            assert_eq!(DeviceState::Offline.value(), 1);
            assert_eq!(DeviceState::default(), DeviceState::Online);
            assert_eq!(DeviceState::try_from(1), Ok(DeviceState::Offline));
            assert!(DeviceState::try_from(7).is_err());
        }

        #[test]
        fn device_sensor_containment_and_derived_display_name() {
            let (res, device, temperature, humidity) = greenhouse();

            let sensors = res.device(device).unwrap().sensors(&res);
            assert_eq!(sensors.len(), 2);

            // `readonly id String sensorId`: stored and readable, but the
            // generated code has NO setter (a call would not compile); the
            // canonical-JSON loader fills it (see the iot instance file).
            assert_eq!(res.sensor(temperature).unwrap().sensor_id, "");
            assert_eq!(res.sensor(temperature).unwrap().device, Some(device));
            assert_eq!(
                res.sensor(humidity)
                    .unwrap()
                    .device(&res)
                    .map(|owner| owner.name.clone()),
                Some("Greenhouse Hub".to_string())
            );

            assert_eq!(res.device(device).unwrap().state, DeviceState::Online);
            // `derived String displayName { expr { if sensors.size() > 0
            // { name } else { "unprovisioned device" } } }` — both branches.
            assert_eq!(
                res.device(device).unwrap().display_name(&res),
                Some("Greenhouse Hub".to_string()),
                "a device with sensors displays its name"
            );
            let mut empty = Resource::default();
            let bare = empty.new_device();
            assert_eq!(
                empty.device(bare).unwrap().display_name(&empty),
                Some("unprovisioned device".to_string()),
                "a sensorless device gets the fallback name"
            );
        }
    }

    // ------------------------------------------------------------------
    // shapes — `extends` is declared but not materialized in Rust codegen:
    // each struct carries its own features only.
    // ------------------------------------------------------------------
    mod shapes_example {
        use crate::shapes::*;

        #[test]
        fn dog_and_cat_construct_with_own_features_only() {
            let mut res = Resource::default();

            let dog = res.new_dog();
            assert_eq!(res.dog(dog).unwrap().breed, "");
            res.dog_mut(dog).unwrap().set_breed("Labrador".to_string());
            assert_eq!(res.dog(dog).unwrap().breed, "Labrador");

            let cat = res.new_cat();
            assert!(!res.cat(cat).unwrap().indoor);
            res.cat_mut(cat).unwrap().set_indoor(true);
            assert!(res.cat(cat).unwrap().indoor);

            let animal = res.new_animal();
            assert_eq!(res.animal(animal).unwrap().name, "");
            res.animal_mut(animal)
                .unwrap()
                .set_name("Generic".to_string());

            // Plain construction needs no resource.
            let puppy = Dog::default();
            assert_eq!(puppy.breed, "");
        }
    }

    // ------------------------------------------------------------------
    // support — the actors-model showcase: refers↔refers pair between two
    // root classes, readonly identity features, labeled enum state, and a
    // derived boolean. The `actors Support { ... }` block itself has no
    // Rust-codegen surface; its Cedar, schema, and policy-set guarantees
    // are pinned by the rex-backend-cedar examples harness.
    // ------------------------------------------------------------------
    mod support_example {
        use crate::support::*;

        /// Two users and two tickets wired through the assignee /
        /// assignedTickets opposite pair. As in `ecommerce`, the `id
        /// readonly` features have no generated setters, so their one-time
        /// initialization is the only direct field write here.
        fn support_resource() -> (Resource, UserId, UserId, TicketId, TicketId) {
            let mut res = Resource::default();

            let ada = res.new_user();
            {
                let slot = res.user_mut(ada).unwrap();
                slot.user_id = "USR-001".to_string();
                slot.set_name("Ada".to_string());
                slot.set_active(true);
            }
            let grace = res.new_user();
            {
                let slot = res.user_mut(grace).unwrap();
                slot.user_id = "USR-002".to_string();
                slot.set_name("Grace".to_string());
                slot.set_active(false);
            }

            let password_reset = res.new_ticket();
            {
                let slot = res.ticket_mut(password_reset).unwrap();
                slot.ticket_no = "TCK-0001".to_string();
                slot.set_title("Password reset".to_string());
                slot.set_internal(false);
                slot.set_refund_cents(0);
                slot.set_state(TicketState::Open);
            }
            let data_export = res.new_ticket();
            {
                let slot = res.ticket_mut(data_export).unwrap();
                slot.ticket_no = "TCK-0002".to_string();
                slot.set_title("Data export".to_string());
                slot.set_internal(true);
                slot.set_refund_cents(2500);
                slot.set_state(TicketState::InProgress);
            }

            res.ticket_set_assignee(password_reset, Some(ada));
            res.ticket_set_assignee(data_export, Some(grace));
            res.user_add_assigned_tickets(ada, password_reset);
            res.user_add_assigned_tickets(grace, data_export);
            (res, ada, grace, password_reset, data_export)
        }

        #[test]
        fn assignee_relation_navigates_both_ways() {
            let (res, ada, grace, password_reset, data_export) = support_resource();

            // Ticket → assignee.
            assert_eq!(res.ticket(password_reset).unwrap().assignee, Some(ada));
            assert_eq!(
                res.ticket(password_reset)
                    .unwrap()
                    .assignee(&res)
                    .map(|user| user.name.clone()),
                Some("Ada".to_string())
            );

            // …and user → assigned tickets.
            let assigned: Vec<String> = res
                .user(grace)
                .unwrap()
                .assigned_tickets(&res)
                .iter()
                .map(|ticket| ticket.title.clone())
                .collect();
            assert_eq!(assigned, vec!["Data export".to_string()]);
            assert_eq!(
                res.ticket(data_export).unwrap().assignee,
                Some(grace),
                "the opposite side of the pair stays consistent by construction"
            );
        }

        #[test]
        fn ticket_state_enum_round_trips_by_value_and_label() {
            assert_eq!(TicketState::Open.name(), "Open");
            assert_eq!(TicketState::Open.label(), "open");
            assert_eq!(TicketState::Open.value(), 0);
            assert_eq!(TicketState::InProgress.label(), "in_progress");
            assert_eq!(TicketState::Resolved.value(), 2);
            assert_eq!(TicketState::try_from(1), Ok(TicketState::InProgress));
            assert!(TicketState::try_from(9).is_err());
        }

        #[test]
        fn derived_needs_refund_follows_the_refund_cents() {
            let (res, _ada, _grace, password_reset, data_export) = support_resource();
            assert_eq!(
                res.ticket(password_reset).unwrap().needs_refund(&res),
                Some(false),
                "a ticket without a refund does not need one"
            );
            assert_eq!(
                res.ticket(data_export).unwrap().needs_refund(&res),
                Some(true),
                "a ticket with a refund balance needs a refund"
            );
        }

        #[test]
        fn canonical_instance_matches_the_example_file_and_round_trips() {
            let (res, _ada, _grace, _password_reset, _data_export) = support_resource();
            let json = res.to_instance_json();

            let loaded = Resource::from_instance_json(&json).expect("load canonical instance");
            assert_eq!(loaded, res, "load(save(x)) != x");
            assert_eq!(loaded.to_instance_json(), json, "re-saved bytes drifted");

            super::assert_instance_json("support", &json);
        }
    }
}
