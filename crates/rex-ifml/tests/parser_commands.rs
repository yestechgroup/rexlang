use rex_ifml::*;

#[test]
fn test_command_declaration_with_members_and_subcommands() {
    let input = r#"
command "customers" {
    description: "Manage customers";
    requires: [ManageCustomers];

    argument id: Uuid;

    option status: String;
    option limit: Int = 20;

    prompt confirm: confirm;

    on run -> action("CreateCustomer");

    command "list" {
        option sort: String = "desc";
        output: table;
    }
}
"#;
    let model = parse_ifml(input).unwrap();
    assert_eq!(model.commands.len(), 1);
    let command = &model.commands[0];
    assert_eq!(command.name, "customers");
    assert_eq!(
        command.description.as_deref(),
        Some("Manage customers"),
        "description is extracted from the property bag"
    );
    assert_eq!(command.requires, vec!["ManageCustomers"]);
    assert!(
        !command
            .properties
            .iter()
            .any(|p| p.key == "description" || p.key == "requires"),
        "extracted bag entries don't stay in the bag"
    );
    assert_eq!(command.arguments.len(), 1);
    assert_eq!(command.arguments[0].name, "id");
    assert_eq!(command.arguments[0].type_ref, "Uuid");
    assert_eq!(command.options.len(), 2);
    assert_eq!(command.options[0].name, "status");
    assert!(command.options[0].default.is_none());
    assert_eq!(command.options[1].name, "limit");
    assert_eq!(command.options[1].type_ref, "Int");
    assert_eq!(
        command.options[1].default,
        Some(ValueExpression::Number(20.0.into()))
    );
    assert_eq!(command.prompts.len(), 1);
    assert_eq!(command.prompts[0].name, "confirm");
    assert_eq!(command.prompts[0].kind, PromptKind::Confirm);
    assert_eq!(command.events.len(), 1);
    match &command.events[0].action {
        EventAction::ActionInvocation { name, .. } => {
            assert_eq!(name, "CreateCustomer");
        }
        other => panic!("expected an action invocation, got {other:?}"),
    }
    assert_eq!(command.subcommands.len(), 1);
    let list = &command.subcommands[0];
    assert_eq!(list.name, "list");
    assert_eq!(
        list.options[0].default,
        Some(ValueExpression::String("desc".to_string()))
    );
    assert!(
        list.properties.iter().any(|p| p.key == "output"),
        "unextracted bag entries stay in the bag"
    );
}

#[test]
fn test_argument_outside_command_body_is_a_parse_error() {
    assert!(parse_ifml("argument id: Uuid;").is_err());
    assert!(parse_ifml("option limit: Int = 20;").is_err());
    assert!(parse_ifml("prompt name: text;").is_err());
}
