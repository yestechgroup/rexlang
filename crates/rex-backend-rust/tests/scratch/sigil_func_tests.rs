// Behavioral assertions for sigil-lowered package functions: a function
// with a checked `rune` body (RUNE-BODIES stage 2) compiles to a real
// straight-line body and executes — guards assert, steps compute; a
// function without one still compiles with the multiplicity conventions in
// its signature and panics as `todo!()` when called. The verbatim
// `rosetta` body text never reaches generated code (wire contract rule 15).
#[cfg(test)]
mod sigil_func_assertions {
    #[test]
    fn signatures_follow_the_multiplicity_conventions() {
        // Compile-time proof of the emitted signatures: a `0..*` parameter
        // arrives as `Vec<..>`, a required `(1..1)` one as a plain value,
        // and a `(0..*)` output lifts the return to `Vec<..>`.
        let any_active: fn(Vec<String>) -> bool = crate::any_active;
        let calc_rate: fn(f64, Vec<String>) -> f64 = crate::calc_rate;
        let trade_ids: fn(String) -> Vec<String> = crate::trade_ids;
        let _ = (any_active, calc_rate, trade_ids);
    }

    #[test]
    fn a_rune_body_computes_and_returns_the_output() {
        // `calcRate` lowers its checked body: the guard passes, the step
        // assigns `rate: notional`, the function returns the output local.
        assert_eq!(crate::calc_rate(1000.0, vec![]), 1000.0);
        assert_eq!(crate::calc_rate(2.5, vec!["leg-1".to_string()]), 2.5);
    }

    #[test]
    #[should_panic(expected = "guard 'PositiveNotional' failed")]
    fn a_false_guard_panics_naming_the_guard() {
        crate::calc_rate(-1.0, vec![]);
    }

    #[test]
    #[should_panic(expected = "not yet implemented")]
    fn calling_a_stub_panics_as_todo() {
        // `anyActive` is a library function: no `rune` body exists, the
        // stub remains.
        crate::any_active(vec!["TR1".to_string(), "TR2".to_string()]);
    }

    #[test]
    fn a_body_less_function_stays_bind_only() {
        // `tradeIds` carries no statements (no `rune` body): its stub is
        // never called here — Wave 1's bind-only proof stands.
        let trade_ids: fn(String) -> Vec<String> = crate::trade_ids;
        let _ = trade_ids;
    }

    #[test]
    fn the_expr_body_op_reaches_the_function_through_a_bare_call() {
        // Compile-time proof of the lowered `expr`-body method: its body is
        // the free function call, with the class-typed parameter as a typed
        // id.
        let double_rate: fn(&crate::Desk, &crate::Resource, crate::UnderlierId) -> f64 =
            crate::Desk::double_rate;
        let _ = double_rate;
    }

    #[test]
    fn calling_the_expr_body_op_computes_through_the_function() {
        // The op's `expr` body calls `calcRate(2.0, ["leg-1"])`, whose real
        // body returns the notional — the call chain computes end to end.
        let mut res = crate::Resource::default();
        let underlier = res.new_underlier();
        let desk = res.new_desk();
        let rate = res.desk(desk).unwrap().double_rate(&res, underlier);
        assert_eq!(rate, 2.0);
    }
}
