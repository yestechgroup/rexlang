//! Span index for `.evt` event-contract sources (issue rexlang#245).
//!
//! Definitions are the graph's event-contract nodes — events, channels, and
//! subscriptions — named `<Model>.<Name>` where the model name is the file
//! stem (the event model carries no name of its own, the graph builder's
//! rule). Because that prefix is path-derived, the builder takes the
//! document's path; the source alone cannot know it.
//!
//! References: a subscription's `consumer` names a deploy component (the
//! union pass's X0007 join), `events`/`publishes` names name events, and
//! payload field types name domain classes, enums, or datatypes.

use rex_ir::graph::kinds;

use super::surface::{ReferenceContext, SurfaceIndex};
use crate::graph::ids;

/// Builds the span index for one `.evt` source. `file` is the document's
/// display path — its stem is the graph's `<Model>` prefix. A source that
/// does not parse yields an empty index; this never panics.
pub fn evt_index(source: &str, file: &str) -> SurfaceIndex {
    let mut index = SurfaceIndex::default();
    let Some(ast) = rex_syntax::parse_evt(source).ast else {
        return index;
    };
    let model_name = ids::file_stem(file);

    for event in &ast.events {
        index.push_definition(
            kinds::EVENT,
            ids::event_name(&model_name, &event.name.text),
            event.name.span,
            event.span,
        );
        for field in &event.fields {
            index.push_reference(
                &[kinds::CLASS, kinds::ENUM, kinds::DATATYPE],
                field.ty.name.full_name(),
                field.ty.span,
                ReferenceContext::FieldType,
            );
        }
    }
    for channel in &ast.channels {
        index.push_definition(
            kinds::CHANNEL,
            ids::channel_name(&model_name, &channel.name.text),
            channel.name.span,
            channel.span,
        );
        for publication in &channel.publishes {
            index.push_reference(
                &[kinds::EVENT],
                publication.event.text.clone(),
                publication.event.span,
                ReferenceContext::Publishes,
            );
        }
    }
    for subscription in &ast.subscriptions {
        index.push_definition(
            kinds::SUBSCRIPTION,
            ids::subscription_name(&model_name, &subscription.name.text),
            subscription.name.span,
            subscription.span,
        );
        for event in &subscription.events {
            index.push_reference(
                &[kinds::EVENT],
                event.text.clone(),
                event.span,
                ReferenceContext::Subscribes,
            );
        }
        // `consumer` is recovered as an empty name when the source omitted it
        // (a reported syntax error); `push_reference` drops empty names.
        index.push_reference(
            &[kinds::DEPLOY_COMPONENT],
            subscription.consumer.text.clone(),
            subscription.consumer.span,
            ReferenceContext::Consumer,
        );
    }
    index.sort();
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::test_support;

    const SUPPORT: &str = "examples/support.evt";
    const SUPPORT_DOMAIN: &str = "examples/support.mox";

    fn support_index() -> (String, SurfaceIndex) {
        let source = test_support::fixture(SUPPORT);
        let index = evt_index(&source, SUPPORT);
        (source, index)
    }

    #[test]
    fn evt_definitions_are_the_graph_nodes() {
        let (_, index) = support_index();
        test_support::assert_definitions_match_graph(
            &index,
            &test_support::evt_graph(SUPPORT, SUPPORT_DOMAIN),
            SUPPORT,
        );
    }

    #[test]
    fn subscription_consumer_is_a_component_reference() {
        let source = r#"
subscription refundDesk {
    events [ RefundRequested ]
    consumer refund_approval_service
}
"#;
        let index = evt_index(source, "support.evt");
        let consumer = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::Consumer)
            .expect("the subscription's consumer");
        assert_eq!(consumer.target_kinds, &[kinds::DEPLOY_COMPONENT]);
        assert_eq!(consumer.name, "refund_approval_service");
        assert_eq!(
            &source[consumer.span.start..consumer.span.end],
            "refund_approval_service"
        );
        assert_eq!(
            index
                .references_to(
                    kinds::DEPLOY_COMPONENT,
                    "SupportDesk.refund_approval_service"
                )
                .len(),
            1
        );

        // The support fixture has one consumer per subscription.
        let (_, support) = support_index();
        let consumers: Vec<_> = support
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Consumer)
            .collect();
        assert_eq!(consumers.len(), 2);
    }

    #[test]
    fn channel_publications_are_event_references() {
        let (source, index) = support_index();
        let published: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Publishes)
            .collect();
        assert_eq!(
            published.len(),
            4,
            "two channels publishing two events each"
        );
        assert_eq!(published[0].name, "TicketRaised");
        assert_eq!(published[0].target_kinds, &[kinds::EVENT]);
        assert_eq!(
            &source[published[0].span.start..published[0].span.end],
            "TicketRaised"
        );
        assert_eq!(
            index
                .references_to(kinds::EVENT, "support.TicketRaised")
                .len(),
            2,
            "one publishes and one subscription mention"
        );
    }

    #[test]
    fn field_types_are_type_references() {
        let source = r#"
event TicketRaised version "1.0.0" {
    ticketNo: String;
    state: TicketState;
    raised: DateTime;
}
"#;
        let index = evt_index(source, "support.evt");
        let types: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::FieldType)
            .collect();
        assert_eq!(types.len(), 3);
        assert!(types.iter().all(
            |reference| reference.target_kinds == [kinds::CLASS, kinds::ENUM, kinds::DATATYPE]
        ));
        let state = types
            .iter()
            .find(|reference| reference.name == "TicketState")
            .expect("the enum-typed field");
        assert_eq!(&source[state.span.start..state.span.end], "TicketState");
        assert_eq!(
            index
                .references_to(kinds::ENUM, "nz.example.support.TicketState")
                .len(),
            1
        );
    }

    #[test]
    fn broken_source_yields_an_empty_index_not_a_panic() {
        assert!(evt_index("%%% not an event contract %%%", "x.evt").is_empty());
        assert!(evt_index("", "x.evt").is_empty());
        // A subscription missing its consumer recovers an empty name (a
        // reported syntax error); no nameless reference is indexed.
        let index = evt_index(
            "event E { a: int; }\nsubscription s { events [ E ] }\n",
            "x.evt",
        );
        assert!(index.definitions_of(kinds::SUBSCRIPTION, "x.s").len() == 1);
        assert!(!index
            .references
            .iter()
            .any(|reference| reference.context == ReferenceContext::Consumer));
    }
}
