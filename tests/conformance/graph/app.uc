// The use-case half of the fixture: a scenario anchored to the design's
// refund operation and gated by the policy's `RaiseRefund` capability (the
// actor block resolves bare capability names union-wide).
import "app.mox"
import "app.ddd" as app
import "app.actor"
import "app.evt"

usecase RaiseRefund {
    subject Ticket: Ticket {
        perform raise: raiseRefund requires [ RaiseRefund ];
    }
    actor Agent {
        occurrence refundRequested;
    }
    message requestRefund of RefundRequested from Agent.refundRequested to Ticket.raise;
    first start then requestRefund.source;
    first requestRefund.source then requestRefund;
    first requestRefund then raise;
    first raise then done;
}

usecase TrackTickets {
    subject Ticket: Ticket
    actor Customer
}
