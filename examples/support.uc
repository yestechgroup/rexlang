// Support-desk use cases over the full stack: the domain (support.mox)
// types the participants and payloads, the design (support.ddd, aliased
// for `realizes`) supplies the realizing service operations, the policy
// (support.actor) names the bare actor principals and gates the required
// capabilities, and the event contracts (support.evt) supply message
// payloads. RaiseRefund and ApproveRefund are a never_both pair, so the
// two refund scenarios are deliberately separate use cases.
import "support.mox"
import "support.ddd" as support
import "support.actor"
import "support.evt"

/// Iteration 1: a subject and one bare policy actor — nothing else.
usecase TrackRefund {
    subject Ticket: Ticket
    actor Agent
}

/// Iteration 2a: named occurrences and a fully anchored message.
usecase ClarifyTicket {
    subject Ticket: Ticket {
        occurrence clarificationRequested;
    }
    actor Reporter: User {
        occurrence clarificationSent;
    }
    message requestInfo of Ticket from Ticket.clarificationRequested to Reporter.clarificationSent;
    first start then requestInfo.source;
    first requestInfo.source then requestInfo;
    first requestInfo then done;
}

/// Iteration 2b: message-owned ends and an end-less message.
usecase EscalateInternal {
    subject Ticket: Ticket {
        occurrence escalate.source;
        occurrence escalate.target;
    }
    message escalate of Ticket;
}

/// Iteration 3: subject-performed steps anchored to operations, actor
/// occurrences, an event payload and a domain payload, and a realized
/// service operation.
usecase RaiseRefund {
    subject Ticket: Ticket {
        perform raise: raiseRefund requires [ RaiseRefund ];
    }
    actor Agent {
        occurrence refundRequested;
        occurrence refundEscalated;
    }
    message requestRefund of RefundRequested from Agent.refundRequested to Ticket.raise;
    message escalate of RefundDecision from Ticket.raise to Agent.refundEscalated;
    first start then requestRefund.source;
    first requestRefund.source then requestRefund;
    first requestRefund then raise;
    first raise then escalate.source;
    first escalate.source then escalate;
    first escalate then done;
    realizes support.TicketService.raiseRefund;
}

/// The finance approval is its own use case: RaiseRefund and
/// ApproveRefund form a never_both pair, so one scenario never
/// requires both.
usecase ApproveRefund {
    subject Ticket: Ticket {
        perform approve: approveRefund requires [ ApproveRefund ];
    }
    actor Finance
    first start then approve;
    first approve then done;
}

/// The manager watches the queue without participating in a scenario.
usecase MonitorQueue {
    actor Manager
}
