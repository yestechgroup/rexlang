//! Single source of truth for the `.dmn` decision-model DSL's contextual
//! keywords.
//!
//! Every `.dmn` contextual keyword is an ordinary identifier that is
//! special only in a particular grammar position (the `.ddd` discipline —
//! the `id`/`readonly` precedent), so two consumers must agree on the same
//! word list: the parser (which matches them positionally) and the
//! formatter (which recognizes them to dispatch its scans and to reorder a
//! decision model body). This module is the one table both consume. All
//! words are escapable with `^`.
//!
//! Two more words of the grammar are *not* listed here because they are
//! real lexer keywords: `import` and `version` — the `Token::Import` and
//! `Token::Version` variants in `crate::lexer` (`version` is the `.mox`
//! vocabulary-`version` keyword). Both are matched positionally by their
//! keyword tokens; the escaped spellings `^import`/`^version` stay usable
//! as ordinary names.

/// The `decision_model <name> { ... }` keyword.
pub(crate) const DECISION_MODEL: &str = "decision_model";
/// The `namespace <qname>;` metadata line of a decision model body.
pub(crate) const NAMESPACE: &str = "namespace";
/// The `id "<value>";` metadata line of a decision model body. Also a
/// contextual `.mox` feature modifier — the word is special only in the
/// position each surface's parser gives it.
pub(crate) const ID: &str = "id";
/// The `effective "<value>";` metadata line of a decision model body.
pub(crate) const EFFECTIVE: &str = "effective";
/// The `jurisdiction "<value>";` metadata line of a decision model body.
pub(crate) const JURISDICTION: &str = "jurisdiction";
/// The `inputs { ... }` block of a decision model body and the
/// `inputs { <name>* }` list of a service body.
pub(crate) const INPUTS: &str = "inputs";
/// The `knowledge { ... }` block of a decision model body and the
/// `knowledge { <line>* }` requirement group.
pub(crate) const KNOWLEDGE: &str = "knowledge";
/// The `source <name> { ... }` keyword of a knowledge block.
pub(crate) const SOURCE: &str = "source";
/// The `model <name> { ... }` keyword of a knowledge block (a business
/// knowledge model — parameters, output, logic).
pub(crate) const MODEL: &str = "model";
/// The `parameters { ... }` block of a model (BKM) body.
pub(crate) const PARAMETERS: &str = "parameters";
/// The `output <type>;` line of a model (BKM) body and of a decision body
/// (the anonymous single-output form).
pub(crate) const OUTPUT: &str = "output";
/// The `outputs { ... }` block of a decision body (named outputs).
pub(crate) const OUTPUTS: &str = "outputs";
/// The `decisions { ... }` block of a decision model body.
pub(crate) const DECISIONS: &str = "decisions";
/// The `decision <name> { ... }` keyword of a decisions block.
pub(crate) const DECISION: &str = "decision";
/// The `logic <kind>` clause of a model (BKM) body and of a decision body.
pub(crate) const LOGIC: &str = "logic";
/// The `logic table "<source>"` kind.
pub(crate) const TABLE: &str = "table";
/// The `logic expression "<text>"` kind.
pub(crate) const EXPRESSION: &str = "expression";
/// The `logic invoke <model> { ... }` kind (a business knowledge model
/// invocation with its argument bindings).
pub(crate) const INVOKE: &str = "invoke";
/// The `emits { ... }` block of a decision body: the declared events the
/// decision's outcome publishes (the DMN → EVT edge).
pub(crate) const EMITS: &str = "emits";
/// The `requirements { ... }` block of a decision model body.
pub(crate) const REQUIREMENTS: &str = "requirements";
/// The `information { <line>* }` requirement group.
pub(crate) const INFORMATION: &str = "information";
/// The `requires <name>` connective of a requirement line.
pub(crate) const REQUIRES: &str = "requires";
/// The `invokes <name>` connective of a requirement line.
pub(crate) const INVOKES: &str = "invokes";
/// The `authority { <line>* }` requirement group.
pub(crate) const AUTHORITY: &str = "authority";
/// The `governed_by <name>` connective of a requirement line.
pub(crate) const GOVERNED_BY: &str = "governed_by";
/// The `services { ... }` block of a decision model body.
pub(crate) const SERVICES: &str = "services";
/// The `service <name> { ... }` keyword of a services block. Also a
/// contextual `.ddd` word — the surfaces share the word, never the
/// grammar position.
pub(crate) const SERVICE: &str = "service";
/// The `encapsulates { <name>* }` list of a service body.
pub(crate) const ENCAPSULATES: &str = "encapsulates";
/// The `views { ... }` block of a decision model body.
pub(crate) const VIEWS: &str = "views";
/// The `view <name> { ... }` keyword of a views block.
pub(crate) const VIEW: &str = "view";
/// The `show { <name>* }` list of a view body.
pub(crate) const SHOW: &str = "show";
/// The `direction <name>;` line of a view body.
pub(crate) const DIRECTION: &str = "direction";
/// The `description "<text>";` line of a service or view body (decisions
/// and knowledge sources take their description from doc comments).
pub(crate) const DESCRIPTION: &str = "description";
/// The `reference "<value>";` property of a knowledge source.
pub(crate) const REFERENCE: &str = "reference";
/// The `name "<value>";` display-name property of a knowledge source.
pub(crate) const NAME: &str = "name";
