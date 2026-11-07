//! The `dmn.check` band: `E0200`–`E0299`, the `.dmn` decision-model
//! surface's checker and the `.ddd` decision-link checks (issue
//! rexlang#78 — `crates/rex-driver/src/dmn.rs`). Every check prefixes its
//! code onto the diagnostic message (`E0200 unknown type …`). The band
//! serves a DMN 1.6-style decision-requirements graph: decisions, input
//! data, business knowledge models, and knowledge sources wired by
//! information requirements (inputs and other decisions) and authority
//! requirements (knowledge sources), exposed through decision services,
//! with logic in FEEL expressions and external markdown decision tables
//! the host provides. `E0200`–`E0214` are the `.dmn` surface's codes;
//! `E0250`–`E0251` are the `.ddd` decision-link codes, joined against
//! the decision surface at compile time. Claim a new code by adding a
//! documented constant plus an `Entry` to `ENTRIES`.

use crate::{Entry, Severity};

/// The band's registry id ([`crate::all`] entries reference it).
pub const DMN_BAND: &str = "dmn.check";

/// `unknown type '<type>' on <what> '<name>'`.
pub const E_UNKNOWN_TYPE: &str = "E0200";
/// `unknown input or decision '<required>' required by decision
/// '<requiring>'`.
pub const E_UNKNOWN_INFORMATION_REQUIREMENT: &str = "E0201";
/// `unknown knowledge model '<required>' invoked by decision
/// '<requiring>'`.
pub const E_UNKNOWN_KNOWLEDGE_MODEL: &str = "E0202";
/// `unknown knowledge source '<required>' governed by decision
/// '<requiring>'`.
pub const E_UNKNOWN_KNOWLEDGE_SOURCE: &str = "E0203";
/// `duplicate <kind> '<name>'`.
pub const E_DUPLICATE_NAME: &str = "E0204";
/// `decision cycle: '<a>' -> '<b>' -> ... -> '<a>'`.
pub const E_DECISION_CYCLE: &str = "E0205";
/// `unknown <input/output/decision/knowledge model> '<name>' on decision
/// service '<service>'`.
pub const E_UNKNOWN_SERVICE_ENDPOINT: &str = "E0206";
/// `unknown element '<name>' in view '<view>'`.
pub const E_UNKNOWN_VIEW_ELEMENT: &str = "E0207";
/// `invalid FEEL expression on <what> '<name>': <error>`.
pub const E_INVALID_FEEL: &str = "E0208";
/// `decision table "<path>" was not provided`.
pub const E_TABLE_NOT_PROVIDED: &str = "E0209";
/// `decision table "<path>": <error>`.
pub const E_TABLE_ERROR: &str = "E0210";
/// `decision table "<path>" input clause '<name>' is not a declared input
/// of decision '<decision>'` / `decision table "<path>" is missing input
/// clause '<name>' required by decision '<decision>'`.
pub const E_TABLE_INPUT_MISMATCH: &str = "E0211";
/// `decision table "<path>" output clause '<name>' is not a declared
/// output of decision '<decision>'` / `decision table "<path>" has <n>
/// output clauses but decision '<decision>' declares <m>`.
pub const E_TABLE_OUTPUT_MISMATCH: &str = "E0212";
/// `unknown parameter '<param>' bound by decision '<decision>'` /
/// `decision '<decision>' does not bind parameter '<param>' of knowledge
/// model '<model>'` / `parameter '<param>' bound twice by <what> '<name>'`.
pub const E_BINDING_MISMATCH: &str = "E0213";
/// `duplicate '<word>' metadata`.
pub const E_DUPLICATE_METADATA: &str = "E0214";
/// `unknown decision service '<name>' invoked by operation '<op>' of
/// service '<service>'`.
pub const E_UNKNOWN_INVOKED_SERVICE: &str = "E0250";
/// `unknown decision service '<name>' injected by service '<service>'`.
pub const E_UNKNOWN_INJECTED_SERVICE: &str = "E0251";

pub(crate) const ENTRIES: &[Entry] = &[
    Entry {
        code: E_UNKNOWN_TYPE,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown type",
        meaning: "`unknown type '<type>' on <what> '<name>'` — the element's declared type (`<what>` is the element kind: input data, decision, knowledge-model parameter, …) names neither a FEEL builtin (`string`, `number`, `boolean`, `date`, …) nor a type the imported domain union declares.",
        fix: "Use a builtin type, import the domain that declares the type, or fix the spelling.",
    },
    Entry {
        code: E_UNKNOWN_INFORMATION_REQUIREMENT,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown information requirement",
        meaning: "`unknown input or decision '<required>' required by decision '<requiring>'` — the decision's information requirement names neither a declared input-data element nor another decision in the decision-requirements graph, so the decision has no source for the input it consumes.",
        fix: "Declare the input data, point the requirement at a declared decision, or fix the spelling.",
    },
    Entry {
        code: E_UNKNOWN_KNOWLEDGE_MODEL,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown knowledge model",
        meaning: "`unknown knowledge model '<required>' invoked by decision '<requiring>'` — the decision's knowledge requirement names no declared business knowledge model, so the decision has no boxed logic to apply.",
        fix: "Declare the knowledge model, or fix the spelling.",
    },
    Entry {
        code: E_UNKNOWN_KNOWLEDGE_SOURCE,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown knowledge source",
        meaning: "`unknown knowledge source '<required>' governed by decision '<requiring>'` — the decision's authority requirement names no declared knowledge source; DMN models the authority behind a decision (a person, policy, or system) separately from the logic that computes it.",
        fix: "Declare the knowledge source, or fix the spelling.",
    },
    Entry {
        code: E_DUPLICATE_NAME,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Duplicate element name",
        meaning: "`duplicate <kind> '<name>'` — two elements of the same kind (decision, input data, knowledge model, knowledge source, decision service) share one name; requirements resolve by name, so every reference to it is ambiguous.",
        fix: "Rename one of the elements and update the requirements that reference it.",
    },
    Entry {
        code: E_DECISION_CYCLE,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Decision cycle",
        meaning: "`decision cycle: '<a>' -> '<b>' -> ... -> '<a>'` — the information requirements form a cycle, so no decision in it can be evaluated first; the decision-requirements graph must stay acyclic (the message walks the whole loop).",
        fix: "Break the cycle — remove or redirect one requirement so the dependencies form a DAG.",
    },
    Entry {
        code: E_UNKNOWN_SERVICE_ENDPOINT,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown decision-service endpoint",
        meaning: "`unknown <input/output/decision/knowledge model> '<name>' on decision service '<service>'` — the decision service's endpoint list names an element that is not declared; a service exposes decisions (and their inputs, outputs, and knowledge models) as one invocable unit.",
        fix: "Point the endpoint at an element the model declares, or fix the spelling.",
    },
    Entry {
        code: E_UNKNOWN_VIEW_ELEMENT,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown view element",
        meaning: "`unknown element '<name>' in view '<view>'` — the view's element list names no declared element; a view is a decision service's projection of the graph, so it can only show what exists.",
        fix: "Point the view at a declared element, or fix the spelling.",
    },
    Entry {
        code: E_INVALID_FEEL,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Invalid FEEL expression",
        meaning: "`invalid FEEL expression on <what> '<name>': <error>` — the element's FEEL expression (FEEL is DMN's expression language for decision logic and table cells) failed to parse or type; the message carries the FEEL error verbatim.",
        fix: "Fix the reported FEEL error in the expression.",
    },
    Entry {
        code: E_TABLE_NOT_PROVIDED,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Decision table not provided",
        meaning: "`decision table \"<path>\" was not provided` — the model references an external markdown decision table the host bundle carries no content for (the driver stays filesystem-free; tables ride in the imports bundle, like schema imports).",
        fix: "Provide the table's markdown under its referenced path, or fix the path's spelling.",
    },
    Entry {
        code: E_TABLE_ERROR,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Decision table error",
        meaning: "`decision table \"<path>\": <error>` — the external markdown decision table was provided but could not be ingested (malformed table markdown, a hit policy the dialect does not support, …); the message carries the ingest error verbatim.",
        fix: "Fix the reported table error in the referenced markdown file.",
    },
    Entry {
        code: E_TABLE_INPUT_MISMATCH,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Decision table input mismatch",
        meaning: "`decision table \"<path>\" input clause '<name>' is not a declared input of decision '<decision>'`, or `decision table \"<path>\" is missing input clause '<name>' required by decision '<decision>'` — the table's input clauses and the decision's declared inputs must match one-for-one; an extra clause reads data no decision provides, a missing one leaves an input unfed.",
        fix: "Align the table's input clauses with the decision's declared inputs — rename or drop the extra clause, add the missing one.",
    },
    Entry {
        code: E_TABLE_OUTPUT_MISMATCH,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Decision table output mismatch",
        meaning: "`decision table \"<path>\" output clause '<name>' is not a declared output of decision '<decision>'`, or `decision table \"<path>\" has <n> output clauses but decision '<decision>' declares <m>` — the table's output clauses and the decision's declared outputs must match one-for-one.",
        fix: "Align the table's output clauses with the decision's declared outputs — rename or drop the extra clause, add the missing one.",
    },
    Entry {
        code: E_BINDING_MISMATCH,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Knowledge-model binding mismatch",
        meaning: "`unknown parameter '<param>' bound by decision '<decision>'`, `decision '<decision>' does not bind parameter '<param>' of knowledge model '<model>'`, or `parameter '<param>' bound twice by <what> '<name>'` — an invocation (a decision's or a knowledge model's) must bind every parameter of the invoked knowledge model, and no others, exactly once, or the invocation cannot evaluate.",
        fix: "Bind exactly the knowledge model's declared parameters — add the missing binding or drop the unknown one.",
    },
    Entry {
        code: E_DUPLICATE_METADATA,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Duplicate metadata",
        meaning: "`duplicate '<word>' metadata` — the same metadata word (description, owner, …) is declared twice on one element, so the element carries two values for one attribute.",
        fix: "Keep one occurrence of the metadata word and merge the values.",
    },
    Entry {
        code: E_UNKNOWN_INVOKED_SERVICE,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown invoked decision service",
        meaning: "`unknown decision service '<name>' invoked by operation '<op>' of service '<service>'` — the `.ddd` application service's operation invokes a decision service no provided `.dmn` model declares; the decision-link check joins the design against the decision surface.",
        fix: "Declare the decision service in a `.dmn` model, or fix the spelling.",
    },
    Entry {
        code: E_UNKNOWN_INJECTED_SERVICE,
        band: DMN_BAND,
        severity: Severity::Error,
        title: "Unknown injected decision service",
        meaning: "`unknown decision service '<name>' injected by service '<service>'` — the `.ddd` application service injects a decision service no provided `.dmn` model declares; the decision-link check joins the design against the decision surface.",
        fix: "Declare the decision service in a `.dmn` model, or fix the spelling.",
    },
];
