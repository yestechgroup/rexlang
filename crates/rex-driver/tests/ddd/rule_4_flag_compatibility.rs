//! `ddd` tests: rule 4 flag compatibility.

use crate::support::*;

// --- rule 4: flag/stereotype compatibility -----------------------------------

#[test]
fn entity_only_flags_are_rejected_on_value_designs() {
    for flag in ["scaffold", "auditable", "optimisticLocking"] {
        let source = format!("application A {{ module m {{ value Money {flag} }} }}");
        let compilation = compile_domain(&source);
        let diagnostic = single(
            &compilation,
            &format!("flag '{flag}' requires an entity design; 'Money' is designed as a value"),
        );
        assert_eq!(diagnostic.span, Some(span_of(&source, flag, 0)));
    }
}

#[test]
fn non_persistent_is_rejected_on_entity_designs() {
    let source = "application A { module m { entity Book nonPersistent } }";
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "flag 'nonPersistent' requires a value or dto design; 'Book' is designed as an entity",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "nonPersistent", 0)));
}

#[test]
fn cache_is_allowed_on_any_design() {
    for design in ["value Money cache", "dto Money cache", "entity Book cache"] {
        let source = format!("import \"library.mox\"\napplication A {{ module m {{ {design} }} }}");
        let compilation = compile_domain(&source);
        assert_clean(&compilation);
    }
}

#[test]
fn entity_flags_default_on_and_negation_opts_out() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        entity Book\n",
        "        entity Library !auditable !optimisticLocking\n",
        "        value Money\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let designs = &compilation.model.expect("artifact").modules[0].designs;
    let book = designs.iter().find(|d| d.class == "Book").unwrap();
    assert!(book.flags.auditable, "auditable defaults on for entities");
    assert!(
        book.flags.optimistic_locking,
        "optimisticLocking defaults on for entities"
    );
    let library = designs.iter().find(|d| d.class == "Library").unwrap();
    assert!(!library.flags.auditable, "!auditable opts out");
    assert!(
        !library.flags.optimistic_locking,
        "!optimisticLocking opts out"
    );
    let money = designs.iter().find(|d| d.class == "Money").unwrap();
    assert!(
        !money.flags.auditable && !money.flags.optimistic_locking,
        "value designs carry no entity flag defaults"
    );
}

#[test]
fn negating_default_off_flags_is_rejected() {
    for (design, flag) in [
        ("entity Book !scaffold", "scaffold"),
        ("entity Book !cache", "cache"),
        ("value Money !nonPersistent", "nonPersistent"),
    ] {
        let source = format!("import \"library.mox\"\napplication A {{ module m {{ {design} }} }}");
        let compilation = compile_domain(&source);
        let diagnostic = single(
            &compilation,
            &format!("'!{flag}' is meaningless; '{flag}' defaults to off"),
        );
        assert_eq!(
            diagnostic.span,
            Some(span_of(&source, &format!("!{flag}"), 0))
        );
    }
}

#[test]
fn negated_entity_flags_require_an_entity_design() {
    let source = "import \"library.mox\"\napplication A { module m { value Money !auditable } }";
    let compilation = compile_domain(source);
    single(
        &compilation,
        "flag 'auditable' requires an entity design; 'Money' is designed as a value",
    );
    assert_eq!(
        single(&compilation, "flag 'auditable'").span,
        Some(span_of(source, "!auditable", 0))
    );
}
