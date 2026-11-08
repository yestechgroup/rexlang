//! `ddd` tests: rule 8 delegations.

use crate::support::*;

// --- rule 8: delegations -----------------------------------------------------

#[test]
fn delegation_target_must_be_a_service_or_repository() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            lend => Money.findById;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown delegation target 'Money'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("delegate to an injected dependency"),
        "help names the fix: {:?}",
        diagnostic.help
    );
}

#[test]
fn delegation_operation_must_exist_on_the_target() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            lend => LibraryRepository.renew;\n",
        "        }\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(
        &compilation,
        "repository 'LibraryRepository' has no operation 'renew'",
    );
}

#[test]
fn service_to_service_delegation_across_modules_is_allowed() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module media {\n",
        "        service MediaService {\n",
        "            registerBorrower => PersonService.register;\n",
        "        }\n",
        "    }\n",
        "\n",
        "    module person {\n",
        "        service PersonService {\n",
        "            boolean register();\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
}

#[test]
fn service_to_repository_delegation_across_modules_is_rejected() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module media {\n",
        "        service MediaService {\n",
        "            lend => LibraryRepository.findById;\n",
        "        }\n",
        "    }\n",
        "\n",
        "    module person {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "interaction between a Service in one Module and a Repository in another Module \
         is not allowed; go via a Service",
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("repository 'LibraryRepository' lives in module 'person'"),
        "help names both modules: {:?}",
        diagnostic.help
    );
}
