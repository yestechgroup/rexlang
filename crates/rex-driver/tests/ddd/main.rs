//! Integration tests for the `ddd` surface, one module per
//! validation group; fixtures live in `support`.

mod support;


mod happy_path;
mod repository_breadth;
mod resolution_semantics;
mod rule_10_capabilities;
mod rule_11_searches;
mod rule_12_value_contract;
mod rule_13_aggregate_references;
mod rule_14_scaffold;
mod rule_15_dto_ops;
mod rule_1_base;
mod rule_2_uniqueness;
mod rule_3_stereotype_targets;
mod rule_4_flag_compatibility;
mod rule_5_repository_placement;
mod rule_6_aggregate_boundary;
mod rule_7_signature_typecheck;
mod rule_8_delegations;
mod rule_9_inject;
mod syntax_errors;
