//! `rexlang` CLI tests: ddd_decisions.

use crate::support::*;

// --- `.ddd` decision links (rexlang#78 rule 17) --------------------------------

const JOIN_DDD: &str = r#"
import "decisions.dmn";

application Desk {
    module desk {
        service TicketService {
            inject decision AssessRefund;
            boolean raiseRefund(String ticketNo, int cents) invokes AssessRefund;
        }
    }
}
"#;

const JOIN_DMN: &str = r#"
decision_model Decisions {
    decisions {
        decision refund {
            output boolean;
        }
    }

    services {
        service AssessRefund {
            outputs { refund }
            encapsulates { refund }
        }
    }
}
"#;

#[test]
fn check_resolves_ddd_decision_links_against_a_sibling_dmn_artifact() {
    let dir = scratch_dir();
    std::fs::write(dir.join("desk.ddd"), JOIN_DDD).expect("write design");
    std::fs::write(dir.join("decisions.dmn"), JOIN_DMN).expect("write decisions");
    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn check_fails_when_an_invoked_decision_service_is_unknown() {
    let dir = scratch_dir();
    std::fs::write(
        dir.join("desk.ddd"),
        JOIN_DDD.replace("AssessRefund", "AssessNothing"),
    )
    .expect("write design");
    std::fs::write(dir.join("decisions.dmn"), JOIN_DMN).expect("write decisions");
    let output = rexlang()
        .args(["check", dir.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("E0250"), "stderr: {stderr:?}");
    assert!(stderr.contains("AssessNothing"), "stderr: {stderr:?}");
}
