// Delivery handover use cases.
import "delivery.mox"
import "handover.ddd" as handover
import "handover.actor"
import "handover.evt"

/// Iteration 1: a subject and one bare policy actor, with the objective
/// doc — nothing else.
usecase HandOverParcel1 {
    subject Drone: DeliveryDrone
    actor Courier
}

/// Iteration 2a: named occurrences and a fully anchored message.
usecase HandOverParcel2a {
    subject Drone: DeliveryDrone {
        occurrence recipientVerified;
    }
    actor Recipient: Recipient {
        occurrence promptAcknowledged;
    }
    message promptHandover of HandoverPrompt from Drone.recipientVerified
        to Recipient.promptAcknowledged;
    first start then promptHandover.source;
    first promptHandover.source then promptHandover;
    first promptHandover then done;
}

/// Iteration 2b: message-owned occurrences and an end-less message.
usecase HandOverParcel2b {
    subject Drone: DeliveryDrone {
        occurrence releaseParcel.source;
        occurrence releaseParcel.target;
    }
    message releaseParcel of ReleaseRequest;
}

/// Iteration 3: subject-performed steps anchored to operations, actor
/// occurrences, an event payload, and a realized service operation.
usecase HandOverParcel3 {
    subject Drone: DeliveryDrone {
        perform verify: verifyRecipient;
        perform release: releaseParcel requires [ ReleaseParcel ];
    }
    actor Courier {
        occurrence handoverRequested;
        occurrence parcelReleased;
    }
    actor RecipientAgent {
        occurrence promptAcknowledged;
    }
    message requestRelease of ReleaseRequest from Courier.handoverRequested to Drone.verify;
    message promptHandover of PromptIssued from Drone.verify to RecipientAgent.promptAcknowledged;
    message confirmRelease of HandoverPrompt from Drone.release to Courier.parcelReleased;
    first start then requestRelease.source;
    first requestRelease.source then requestRelease;
    first requestRelease then verify;
    first verify then promptHandover.source;
    first promptHandover.source then promptHandover;
    first promptHandover then release;
    first release then confirmRelease.source;
    first confirmRelease.source then confirmRelease;
    first confirmRelease then done;
    realizes handover.HandoverService.releaseParcel;
}

/// The courier watches a handover without participating in it.
usecase MonitorHandover {
    actor Courier
}
