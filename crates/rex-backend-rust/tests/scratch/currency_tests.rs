// Behavioral tests appended to the generated vocabulary scratch crate's
// lib.rs. Compiled together with the generated `models.rs` content for the
// currency model above; `use crate::*` reaches the generated types.
#[cfg(test)]
mod currency_scratch_tests {
    use crate::*;
    use std::convert::TryFrom;

    /// The conformance scenario: one Account { owner "Alice", currency USD }.
    fn conformance_resource() -> Resource {
        let mut res = Resource::default();
        let account = res.new_account();
        {
            let account = res.account_mut(account).unwrap();
            account.set_owner("Alice".to_string());
            account.set_currency(Currency::USD);
        }
        res
    }

    #[test]
    fn vocabulary_accessors_lookup_display_and_default() {
        assert_eq!(Currency::USD.key(), "USD");
        assert_eq!(Currency::USD.symbol(), "$");
        assert_eq!(Currency::EUR.symbol(), "€");
        assert_eq!(Currency::JPY.minor_units(), 0);
        assert_eq!(Currency::GBP.symbol(), "£");
        assert_eq!(Currency::GBP.minor_units(), 2);
        assert_eq!(Currency::CHF.symbol(), "CHF");
        assert_eq!(Currency::CHF.minor_units(), 2);
        assert_eq!(Currency::USD.to_string(), "USD");
        assert_eq!(Currency::default(), Currency::USD, "Default is the first entry");
        assert_eq!(Currency::try_from("USD"), Ok(Currency::USD));
        assert_eq!(Currency::try_from("JPY"), Ok(Currency::JPY));
        let error = Currency::try_from("XYZ").expect_err("unknown key");
        assert!(
            error.to_string().contains("Currency"),
            "error must name the vocabulary: {error}"
        );
    }

    #[test]
    fn account_defaults_and_setter() {
        let mut res = Resource::default();
        let account = res.new_account();
        let account = res.account(account).unwrap();
        assert_eq!(account.owner, "");
        assert_eq!(account.currency, Currency::USD, "first entry is the default");
    }

    #[test]
    fn canonical_instance_round_trip_by_key() {
        let res = conformance_resource();
        let json = res.to_instance_json();

        // Vocabulary attributes serialize as the entry key string.
        assert!(json.contains("\"currency\": \"USD\""), "json:\n{json}");
        assert!(json.contains("\"owner\": \"Alice\""));

        // Round-trip identity: load(save(x)) == x and save(load(save(x))) == save(x).
        let loaded = Resource::from_instance_json(&json).unwrap();
        assert_eq!(loaded, res);
        assert_eq!(loaded.to_instance_json(), json);

        // The loaded account resolves back to the same variant.
        let account = loaded.accounts.iter().next().map(|(_, a)| a).unwrap();
        assert_eq!(account.currency, Currency::USD);
        assert_eq!(account.currency.symbol(), "$");

        // Hand the instance to the conformance harness when asked to.
        if let Ok(path) = std::env::var("REX_CURRENCY_INSTANCE_OUT") {
            std::fs::write(path, &json).expect("write currency instance JSON");
        }
    }

    #[test]
    fn load_rejects_unknown_vocabulary_keys() {
        let res = conformance_resource();
        let json = res.to_instance_json();

        let tampered = json.replace("\"currency\": \"USD\"", "\"currency\": \"XYZ\"");
        assert!(
            Resource::from_instance_json(&tampered).is_err(),
            "unknown vocabulary keys must be InvalidInstance:\n{tampered}"
        );
    }
}
