//! `rexlang` CLI tests: gen_tools.

use std::path::Path;

use crate::support::*;

// --- `gen tools` ----------------------------------------------------------------

#[test]
fn gen_tools_actor_file_prints_every_agent_manifest() {
    let actor =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance/models/actors.actor");
    let output = rexlang()
        .args(["gen", "tools", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang gen tools");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        r#"{
  "agents": [
    {
      "agent": "Helper",
      "tools": [
        "EscalateTicket",
        "ApproveRefund"
      ],
      "delegations": [
        {
          "name": "EscalateHelp",
          "entries": [
            {
              "capability": "EscalateTicket"
            }
          ]
        }
      ]
    },
    {
      "agent": "Agent",
      "tools": [
        "ReadTicket",
        "RaiseRefund"
      ],
      "delegations": [
        {
          "name": "SupportHelp",
          "purpose": "RefundTriage",
          "entries": [
            {
              "capability": "ReadTicket",
              "obligations": [
                "audit"
              ]
            }
          ]
        },
        {
          "name": "RefundIntake",
          "purpose": "RefundTriage",
          "entries": [
            {
              "capability": "ReadTicket",
              "when": "!internal",
              "obligations": [
                "ack"
              ]
            }
          ]
        }
      ]
    },
    {
      "agent": "Manager",
      "tools": [
        "ReadTicket",
        "ResolveTicket",
        "RaiseRefund"
      ],
      "delegations": []
    },
    {
      "agent": "Auditor",
      "tools": [
        "EscalateTicket",
        "ApproveRefund"
      ],
      "delegations": []
    }
  ]
}
"#,
        "the manifest document is the full agent surface of the policy set"
    );
}

#[test]
fn gen_tools_mox_prints_manifests_for_inline_actors() {
    let path = write_source(
        "tools.mox",
        r#"package demo

class Ticket {
    boolean internal
}

actors Support {
    actor Customer
    agent Agent extends Customer
    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    delegation Intake {
        from Customer
        to Agent
        permit ReadTicket
    }
}
"#,
    );
    let output = rexlang()
        .args(["gen", "tools", path.to_str().unwrap()])
        .output()
        .expect("run rexlang gen tools");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        r#"{
  "agents": [
    {
      "agent": "Agent",
      "tools": [
        "ReadTicket"
      ],
      "delegations": [
        {
          "name": "Intake",
          "entries": [
            {
              "capability": "ReadTicket"
            }
          ]
        }
      ]
    }
  ]
}
"#
    );
}

#[test]
fn gen_tools_fails_on_unresolvable_import_with_diagnostics() {
    let actor = write_source(
        "tools-dangling.actor",
        "import \"missing-domain.mox\"\n\nactors E {\n    actor A\n}\n",
    );
    let output = rexlang()
        .args(["gen", "tools", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang gen tools");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error:"),
        "clean clap-style error, no panic: {stderr}"
    );
    assert!(
        stderr.contains("missing-domain.mox"),
        "the import path must be named: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "must not panic: {stderr}");
}
