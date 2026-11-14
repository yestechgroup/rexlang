//! `.mox` domain emission: classes, enums, datatypes, features, operations,
//! and the object-graph edges (`extends`, `contains`/`refers`, `opposite`,
//! `typeOf`). Inline `actors` blocks emit through [`super::actors`], the same
//! emitter the standalone `.actor` surface uses.

use rex_ir::graph::{kinds, GraphEdge};
use rex_ir::{FeatureKind, Model};

use super::{ids, Builder, FileRef};

/// Emits the domain's nodes: one `file` node, then every declaration.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &Model) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    for package in &model.packages {
        for interface in &package.interfaces {
            let name = ids::qualify(&[&package.name, &interface.name]);
            builder.add_named(kinds::CLASS, &file.display, &file.application, &name);
        }
        for class in &package.classes {
            let name = ids::qualify(&[&package.name, &class.name]);
            builder.add_named(kinds::CLASS, &file.display, &file.application, &name);
            for feature in &class.features {
                let feature_name = ids::qualify(&[&name, &feature.name]);
                builder.add_named(
                    kinds::FEATURE,
                    &file.display,
                    &file.application,
                    &feature_name,
                );
            }
            for operation in &class.operations {
                let operation_name = ids::qualify(&[&name, &operation.name]);
                builder.add_named(
                    kinds::OPERATION,
                    &file.display,
                    &file.application,
                    &operation_name,
                );
            }
        }
        for enumeration in &package.enums {
            let name = ids::qualify(&[&package.name, &enumeration.name]);
            builder.add_named(kinds::ENUM, &file.display, &file.application, &name);
        }
        for datatype in &package.datatypes {
            let name = ids::qualify(&[&package.name, &datatype.name]);
            builder.add_named(kinds::DATATYPE, &file.display, &file.application, &name);
        }
        for block in &package.actors {
            super::actors::block_nodes(builder, file, block);
        }
    }
}

/// Emits the domain's edges, resolving every reference through the node
/// table (a class, enum, or datatype node emitted in [`nodes`], in this or
/// another file of the application).
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &Model) {
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        for package in &model.packages {
            for class in &package.classes {
                let class_name = ids::qualify(&[&package.name, &class.name]);
                let Some(class_symbol) = symbols.one(kinds::CLASS, &class_name, application) else {
                    continue;
                };
                let class_id = class_symbol.id.clone();
                for parent in &class.extends {
                    if let Some(target) = symbols.type_target(parent, application) {
                        edges.push(GraphEdge::new(
                            class_id.clone(),
                            target.id.clone(),
                            kinds::EXTENDS,
                        ));
                    }
                }
                for feature in &class.features {
                    let feature_name = ids::qualify(&[&class_name, &feature.name]);
                    let feature_id = ids::node(kinds::FEATURE, &file.display, &feature_name);
                    edges.push(GraphEdge::new(
                        feature_id.clone(),
                        class_id.clone(),
                        kinds::MEMBER_OF,
                    ));
                    // The object graph: containment and reference ends only.
                    // A `container` back-pointer is the containment's other
                    // end (the artifact carries both), so re-emitting it
                    // would draw the same relation backwards.
                    let object_kind = match feature.kind {
                        FeatureKind::Containment => Some(kinds::CONTAINS),
                        FeatureKind::CrossReference => Some(kinds::REFERS),
                        FeatureKind::Attribute | FeatureKind::Container => None,
                    };
                    if let Some(kind) = object_kind {
                        if let Some(target) = symbols.type_target(&feature.type_, application) {
                            edges.push(
                                GraphEdge::new(class_id.clone(), target.id.clone(), kind)
                                    .attribute("feature", &feature.name),
                            );
                        }
                    }
                    if let Some(target) = symbols.type_target(&feature.type_, application) {
                        edges.push(
                            GraphEdge::new(feature_id.clone(), target.id.clone(), kinds::TYPE_OF)
                                .attribute("field", &feature.name),
                        );
                    }
                    if let Some(opposite) = &feature.opposite {
                        let opposite_name =
                            ids::qualify(&[&package.name, &opposite.class, &opposite.feature]);
                        if let Some(target) =
                            symbols.one(kinds::FEATURE, &opposite_name, application)
                        {
                            edges.push(
                                GraphEdge::new(
                                    feature_id.clone(),
                                    target.id.clone(),
                                    kinds::OPPOSITE,
                                )
                                .attribute("field", &feature.name),
                            );
                        }
                    }
                }
                for operation in &class.operations {
                    let operation_name = ids::qualify(&[&class_name, &operation.name]);
                    let operation_id = ids::node(kinds::OPERATION, &file.display, &operation_name);
                    edges.push(GraphEdge::new(
                        operation_id.clone(),
                        class_id.clone(),
                        kinds::MEMBER_OF,
                    ));
                    if let Some(target) = symbols.type_target(&operation.return_type, application) {
                        edges.push(
                            GraphEdge::new(operation_id.clone(), target.id.clone(), kinds::TYPE_OF)
                                .attribute("role", "return"),
                        );
                    }
                    for param in &operation.params {
                        if let Some(target) = symbols.type_target(&param.type_, application) {
                            edges.push(
                                GraphEdge::new(
                                    operation_id.clone(),
                                    target.id.clone(),
                                    kinds::TYPE_OF,
                                )
                                .attribute("role", "param")
                                .attribute("field", &param.name),
                            );
                        }
                    }
                }
            }
            for block in &package.actors {
                super::actors::block_edges(&mut edges, symbols, &file.display, application, block);
            }
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}
