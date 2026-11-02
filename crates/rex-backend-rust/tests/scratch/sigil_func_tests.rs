// Behavioral assertions for sigil-lowered package functions: the emitted
// stubs compile with the multiplicity conventions in their signatures, and
// calling one panics as `todo!()` — the verbatim `rosetta` body text never
// reaches generated code (wire contract rule 15).
#[cfg(test)]
mod sigil_func_assertions {
    #[test]
    fn stub_signatures_follow_the_multiplicity_conventions() {
        // Compile-time proof of the emitted signatures: a `0..*` parameter
        // arrives as `Vec<..>`, a required `(1..1)` one as a plain value.
        let any_active: fn(Vec<String>) -> bool = crate::any_active;
        let calc_rate: fn(f64, Vec<String>) -> f64 = crate::calc_rate;
        let _ = (any_active, calc_rate);
    }

    #[test]
    #[should_panic(expected = "not yet implemented")]
    fn calling_a_stub_panics_as_todo() {
        crate::any_active(vec!["TR1".to_string(), "TR2".to_string()]);
    }

    #[test]
    fn the_expr_body_op_reaches_the_stub_through_a_bare_call() {
        // Compile-time proof of the lowered `expr`-body method: its body is
        // the free stub call, with the class-typed parameter as a typed id.
        let double_rate: fn(&crate::Desk, &crate::Resource, crate::UnderlierId) -> f64 =
            crate::Desk::double_rate;
        let _ = double_rate;
    }

    #[test]
    #[should_panic(expected = "not yet implemented")]
    fn calling_the_expr_body_op_panics_as_todo() {
        // The stub contract, now reachable through an `expr` body: the Desk
        // method lowers the bare `calcRate(...)` call to `calc_rate`, whose
        // body panics as `todo!()`.
        let mut res = crate::Resource::default();
        let underlier = res.new_underlier();
        let desk = res.new_desk();
        res.desk(desk).unwrap().double_rate(&res, underlier);
    }
}
