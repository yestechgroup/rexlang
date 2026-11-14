//! `.evt` event-contract emission: events, channels, and subscriptions, with
//! the publication/subscription edges and the payload `typeOf` edges.
//!
//! The event model carries no name of its own, so the surface-qualified name
//! prefix is the file stem (`support.evt` → `support.TicketRaised`), the
//! spelling the CLI's diagnostics use.

use rex_ir::events::EventModel;
use rex_ir::graph::{kinds, GraphEdge};

use super::{ids, Builder, FileRef};

/// Emits the event contract's nodes.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &EventModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    let model_name = ids::file_stem(&file.display);
    for event in &model.events {
        let name = ids::event_name(&model_name, &event.name);
        builder.add_named(kinds::EVENT, &file.display, &file.application, &name);
    }
    for channel in &model.channels {
        let name = ids::channel_name(&model_name, &channel.name);
        builder.add_named(kinds::CHANNEL, &file.display, &file.application, &name);
    }
    for subscription in &model.subscriptions {
        let name = ids::subscription_name(&model_name, &subscription.name);
        builder.add_named(kinds::SUBSCRIPTION, &file.display, &file.application, &name);
    }
}

/// Emits the event contract's edges.
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &EventModel) {
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        let display = file.display.as_str();
        let model_name = ids::file_stem(display);

        for event in &model.events {
            let event_name = ids::event_name(&model_name, &event.name);
            let event_id = ids::node(kinds::EVENT, display, &event_name);
            for field in &event.fields {
                if let Some(target) = symbols.type_target(&field.ty, application) {
                    edges.push(
                        GraphEdge::new(event_id.clone(), target.id.clone(), kinds::TYPE_OF)
                            .attribute("field", &field.name),
                    );
                }
            }
        }

        for channel in &model.channels {
            let channel_name = ids::channel_name(&model_name, &channel.name);
            let channel_id = ids::node(kinds::CHANNEL, display, &channel_name);
            for publication in &channel.publishes {
                let event_name = ids::event_name(&model_name, &publication.event);
                if let Some(target) = symbols.one(kinds::EVENT, &event_name, application) {
                    edges.push(GraphEdge::new(
                        channel_id.clone(),
                        target.id.clone(),
                        kinds::PUBLISHES,
                    ));
                }
            }
        }

        for subscription in &model.subscriptions {
            let subscription_name = ids::subscription_name(&model_name, &subscription.name);
            let subscription_id = ids::node(kinds::SUBSCRIPTION, display, &subscription_name);
            for event in &subscription.events {
                let event_name = ids::event_name(&model_name, event);
                if let Some(target) = symbols.one(kinds::EVENT, &event_name, application) {
                    edges.push(GraphEdge::new(
                        subscription_id.clone(),
                        target.id.clone(),
                        kinds::SUBSCRIBES,
                    ));
                }
            }
            // The consumer names a `.deploy` component by exact name across
            // the application's deploy applications (the union pass's X0007
            // join, made an edge).
            if !subscription.consumer.is_empty() {
                super::pooled(
                    &mut edges,
                    &subscription_id,
                    kinds::DELIVERS,
                    symbols.by_last_segment(
                        kinds::DEPLOY_COMPONENT,
                        &subscription.consumer,
                        application,
                    ),
                );
            }
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}
