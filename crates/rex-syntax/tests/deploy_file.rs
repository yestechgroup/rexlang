//! Integration tests for `.deploy` files: standalone deployment sources
//! made of `import` declarations followed by `application`, `profile`, and
//! `deployment` declarations (which may be interleaved; imports must come
//! first). The import production is reused from the `.mox`/`.actor`
//! surfaces verbatim; component kinds and target names are plain names
//! here — their vocabularies are driver concerns.

use rex_syntax::ast::*;
use rex_syntax::{format_deploy, parse_deploy, Span};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

// --- parsing -----------------------------------------------------------------

#[test]
fn imports_and_declarations_parse_with_whole_decl_spans() {
    let source = concat!(
        "import \"library.ddd\"\n",
        "application Svc {\n",
        "    component api: api {}\n",
        "}\n",
    );
    let result = parse_deploy(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");

    assert_eq!(file.imports.len(), 1);
    assert_eq!(file.imports[0].path, "library.ddd");
    assert_eq!(
        span_text(source, file.imports[0].span),
        "import \"library.ddd\""
    );

    assert_eq!(file.applications.len(), 1);
    let application = &file.applications[0];
    assert_eq!(application.name.text, "Svc");
    assert!(span_text(source, application.span).starts_with("application Svc"));
    assert_eq!(application.components.len(), 1);
    let component = &application.components[0];
    assert_eq!(component.name.text, "api");
    assert_eq!(component.kind.text, "api");
    assert_eq!(span_text(source, component.span), "component api: api {}");
    assert!(file.profiles.is_empty());
    assert!(file.deployments.is_empty());
}

#[test]
fn component_members_parse() {
    let source = concat!(
        "application Svc {\n",
        "    component api: api {\n",
        "        runtime: container\n",
        "        replicas: 3\n",
        "        entrypoint: \"src/web.ts\"\n",
        "        requires longRunningProcess, persistentFilesystem\n",
        "        design \"library.ddd#media\"\n",
        "        flow \"app.ifml\"\n",
        "    }\n",
        "}\n",
    );
    let result = parse_deploy(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let component = &result.ast.unwrap().applications[0].components[0];

    assert_eq!(component.settings.len(), 3);
    assert_eq!(component.settings[0].path.full_name(), "runtime");
    match &component.settings[0].value {
        DeploySettingValue::Word(word) => assert_eq!(word.text, "container"),
        other => panic!("expected a word value, found {other:?}"),
    }
    assert_eq!(component.settings[1].value, DeploySettingValue::Int(3));
    assert_eq!(
        component.settings[2].value,
        DeploySettingValue::Str("src/web.ts".to_string())
    );
    assert_eq!(
        component
            .requires
            .iter()
            .map(|capability| capability.text.as_str())
            .collect::<Vec<_>>(),
        ["longRunningProcess", "persistentFilesystem"]
    );
    assert_eq!(component.designs.len(), 1);
    assert_eq!(component.designs[0].reference, "library.ddd#media");
    assert!(component.designs[0].is_design);
    assert_eq!(component.flows.len(), 1);
    assert_eq!(component.flows[0].reference, "app.ifml");
    assert!(!component.flows[0].is_design);
}

#[test]
fn connects_and_keyword_words_as_values_parse() {
    let source = concat!(
        "application Svc {\n",
        "    component web: frontend {\n",
        "        runtime: container\n",
        "    }\n",
        "    component api: api {}\n",
        "    connects web -> api\n",
        "}\n",
    );
    let result = parse_deploy(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let application = &result.ast.unwrap().applications[0];
    assert_eq!(application.connections.len(), 1);
    assert_eq!(application.connections[0].from.text, "web");
    assert_eq!(application.connections[0].to.text, "api");
    assert_eq!(
        span_text(source, application.connections[0].span),
        "connects web -> api"
    );
}

#[test]
fn profiles_parse_targets_defaults_policies_and_mappings() {
    let source = concat!(
        "profile prod {\n",
        "    target: kubernetes\n",
        "    defaults {\n",
        "        api.runtime: container\n",
        "        api.replicas: 3\n",
        "    }\n",
        "    mappings {\n",
        "        api -> kubernetes.deployment\n",
        "        database -> external.postgres\n",
        "    }\n",
        "    require (api.replicas >= 3)\n",
        "    prohibit (db.storage == \"local\")\n",
        "}\n",
    );
    let result = parse_deploy(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let profile = &result.ast.unwrap().profiles[0];
    assert_eq!(profile.name.text, "prod");
    assert_eq!(profile.target.as_ref().expect("target").text, "kubernetes");
    assert_eq!(profile.defaults.len(), 2);
    assert_eq!(profile.defaults[0].path.full_name(), "api.runtime");
    assert_eq!(profile.defaults[1].path.full_name(), "api.replicas");
    assert_eq!(profile.mappings.len(), 2);
    assert_eq!(profile.mappings[0].kind.text, "api");
    assert_eq!(
        profile.mappings[0].resource.full_name(),
        "kubernetes.deployment"
    );
    assert_eq!(profile.policies.len(), 2);
    assert!(!profile.policies[0].prohibit);
    assert!(profile.policies[1].prohibit);
    assert_eq!(
        span_text(source, profile.policies[0].expr),
        "(api.replicas >= 3)"
    );
    assert_eq!(
        span_text(source, profile.policies[1].expr),
        "(db.storage == \"local\")"
    );
}

#[test]
fn deployments_parse_for_use_and_configure() {
    let source = concat!(
        "deployment production for Svc {\n",
        "    use prod\n",
        "    configure {\n",
        "        api.replicas: 4\n",
        "    }\n",
        "}\n",
    );
    let result = parse_deploy(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let deployment = &result.ast.unwrap().deployments[0];
    assert_eq!(deployment.name.text, "production");
    assert_eq!(deployment.application.text, "Svc");
    assert_eq!(deployment.profile.as_ref().expect("profile").text, "prod");
    assert_eq!(deployment.configure.len(), 1);
    assert_eq!(deployment.configure[0].path.full_name(), "api.replicas");
}

#[test]
fn interleaved_declarations_fold_into_per_kind_lists() {
    let source = concat!(
        "deployment dev for Svc { use local }\n",
        "application Svc { component api: api {} }\n",
        "profile local { target: standalone }\n",
    );
    let result = parse_deploy(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.unwrap();
    assert_eq!(file.applications.len(), 1);
    assert_eq!(file.profiles.len(), 1);
    assert_eq!(file.deployments.len(), 1);
}

#[test]
fn a_late_import_is_an_error_but_the_file_recovers() {
    let source = concat!(
        "application Svc { component api: api {} }\n",
        "import \"library.ddd\"\n",
    );
    let result = parse_deploy(source);
    assert_eq!(result.errors.len(), 1);
    assert!(
        result.errors[0].message.contains("`import` after"),
        "unexpected error: {:?}",
        result.errors[0]
    );
    let file = result.ast.unwrap();
    assert!(file.imports.is_empty(), "the late import is dropped");
    assert_eq!(file.applications.len(), 1);
}

// --- recovery ------------------------------------------------------------------

#[test]
fn missing_pieces_recover_with_errors() {
    // A component without a kind.
    let result = parse_deploy("application Svc { component api: {} }");
    assert!(result
        .errors
        .iter()
        .any(|error| error.message.contains("kind")));
    // A component body with a setting missing its value.
    let result = parse_deploy("application Svc { component api: api { replicas: } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("setting value")),
        "unexpected errors: {:?}",
        result.errors
    );
    // A connects entry missing its arrow.
    let result = parse_deploy("application Svc { connects web api }");
    assert!(result
        .errors
        .iter()
        .any(|error| error.message.contains("->")));
    // A profile without a target.
    let result = parse_deploy("profile prod { }");
    assert!(
        result.errors.is_empty(),
        "a missing target is a driver error"
    );
    // A policy without parens.
    let result = parse_deploy("profile prod { require api.replicas >= 3 }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("parenthesized condition")),
        "unexpected errors: {:?}",
        result.errors
    );
    // A deployment without `for`.
    let result = parse_deploy("deployment prod { use p }");
    assert!(result
        .errors
        .iter()
        .any(|error| error.message.contains("`for`")));
    // A deployment without `use`.
    let result = parse_deploy("deployment prod for Svc { }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("`use <profile>`")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn contextual_words_remain_usable_as_names() {
    // `use`, `for`, `target`, and `profile` are contextual: escaped forms
    // are never mistaken for the grammar words, and the words themselves
    // remain usable where the grammar expects a name.
    let source = concat!(
        "application Svc {\n",
        "    component target: api {\n",
        "        profile: use\n",
        "    }\n",
        "}\n",
    );
    let result = parse_deploy(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let component = &result.ast.unwrap().applications[0].components[0];
    assert_eq!(component.name.text, "target");
    assert_eq!(component.settings[0].path.full_name(), "profile");
}

#[test]
fn nasty_inputs_never_panic() {
    let nasty = [
        "",
        "application",
        "application Svc",
        "application Svc {",
        "application Svc { component",
        "application Svc { component api",
        "application Svc { component api:",
        "application Svc { component api: api {",
        "application Svc { component api: api { requires }",
        "application Svc { component api: api { design } }",
        "application Svc { connects }",
        "application Svc { connects a }",
        "application Svc { connects a -> }",
        "profile",
        "profile p",
        "profile p {",
        "profile p { target",
        "profile p { target: }",
        "profile p { require }",
        "profile p { require ( }",
        "profile p { mappings { api } }",
        "profile p { defaults { api } }",
        "deployment",
        "deployment prod",
        "deployment prod for",
        "deployment prod for Svc",
        "deployment prod for Svc {",
        "deployment prod for Svc { use }",
        "import \"a.ddd\" application",
        "@@@ ### :",
        "^application ^Svc { ^component ^api: ^api { ^requires x } }",
        "application Svc { component api: api { ^requires: ^use } }",
    ];
    for input in nasty {
        let result = parse_deploy(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
        let _ = format_deploy(input); // must not panic either
    }
}

// --- formatting ----------------------------------------------------------------

fn fmt_deploy(source: &str) -> String {
    format_deploy(source).expect("format succeeds")
}

#[test]
fn empty_deploy_file_formats_to_empty_output() {
    assert_eq!(fmt_deploy(""), "");
    assert_eq!(fmt_deploy("  \n\n "), "");
}

#[test]
fn canonical_layout_sections_and_members() {
    let messy = concat!(
        "deployment  dev for Svc { use local configure { api.replicas : 4 } }\n",
        "profile  local { target : standalone defaults { api.replicas: 1 } mappings { api -> k8s.pod } require ( api.replicas>=1 ) }\n",
        "application  Svc { component  api : api { runtime : container replicas : 3 requires longRunningProcess ,  statefulProcess design \"a.ddd#M\" flow \"b.ifml\" }\n",
        "connects web->api }\n",
        "import   \"library.ddd\"\n",
    );
    let canonical = concat!(
        "import \"library.ddd\"\n",
        "\n",
        "application Svc {\n",
        "    component api: api {\n",
        "        runtime: container\n",
        "        replicas: 3\n",
        "        requires longRunningProcess, statefulProcess\n",
        "        design \"a.ddd#M\"\n",
        "        flow \"b.ifml\"\n",
        "    }\n",
        "    connects web -> api\n",
        "}\n",
        "\n",
        "profile local {\n",
        "    target: standalone\n",
        "    defaults {\n",
        "        api.replicas: 1\n",
        "    }\n",
        "    mappings {\n",
        "        api -> k8s.pod\n",
        "    }\n",
        "    require (api.replicas>=1)\n",
        "}\n",
        "\n",
        "deployment dev for Svc {\n",
        "    use local\n",
        "    configure {\n",
        "        api.replicas: 4\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_deploy(messy), canonical);
    // Re-formatting the output is a fixpoint.
    assert_eq!(fmt_deploy(canonical), canonical);
}

#[test]
fn empty_bodies_stay_inline() {
    let canonical = "application Svc {\n    component api: api {}\n}\n";
    assert_eq!(
        fmt_deploy("application Svc { component api: api {} }"),
        canonical
    );
    assert_eq!(fmt_deploy(canonical), canonical);
}

#[test]
fn policy_conditions_are_verbatim_and_a_fixpoint() {
    let source = "profile p {\n    target: standalone\n    require ( api.replicas   >=  1 )\n}\n";
    let canonical = "profile p {\n    target: standalone\n    require (api.replicas   >=  1)\n}\n";
    assert_eq!(fmt_deploy(source), canonical);
    assert_eq!(fmt_deploy(canonical), canonical);
}

#[test]
fn comments_survive_formatting() {
    let source = concat!(
        "// A header comment.\n",
        "import \"a.ddd\" // trailing\n",
        "\n",
        "// About the application.\n",
        "application Svc {\n",
        "    // About the component.\n",
        "    component api: api {\n",
        "        replicas: 2 // inline\n",
        "    }\n",
        "}\n",
    );
    let formatted = fmt_deploy(source);
    assert_eq!(formatted, source);
}
