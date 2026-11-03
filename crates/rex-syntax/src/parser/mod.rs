//! A fault-tolerant parser built with chumsky, producing a spanned AST.
//!
//! Recovery strategy:
//! * top-level: when the input at a position cannot start a declaration, a
//!   junk-region parser consumes tokens up to the next top-level declaration
//!   keyword (or end of input) and emits a single error for the region;
//! * features inside a class body: same approach, but the junk region stops at
//!   the body's closing `}` and at tokens that could begin the next feature.
//!
//! Both junk parsers always consume at least one token, which guarantees
//! forward progress (and therefore termination).
//!
//! # The normative grammar
//!
//! This module is the **grammar authority** for the five chumsky surfaces:
//! the EBNF below is derived from the parsers in this file, and the docs
//! (`docs/LANGUAGE.md`, `docs/DDD.md`) link here instead of restating it.
//! When any prose and this EBNF disagree, the parsers in this file win;
//! change both in the same commit. The grammar of the fourth surface,
//! `.ifml`, is normatively defined by its Pest grammar file
//! (`crates/rex-ifml/src/grammar/ifml.pest`), which is machine-checked at
//! build time — it is not duplicated here.
//!
//! The unit tests at the bottom of this file spot-check the EBNF against the
//! lexer and the `.ddd` keyword tables (every real keyword must appear as a
//! terminal, every terminal must be a known keyword or contextual word), so
//! adding or renaming a keyword without updating the grammar fails a test.
//! Structural drift (a changed repetition or optionality) is *not* caught
//! mechanically; keep this block in the parser's change.
//!
//! Notation shared by all three surfaces:
//!
//! * Terminals are quoted (`"class"`); everything else is a metavariable.
//!   `name` is an identifier ([`Token::Ident`], or an escaped keyword,
//!   [`Token::IdentEscaped`]); `qualified_name` is `name ("." segment)*`
//!   where a *segment* may exceptionally be any keyword token — keywords
//!   introduce constructs only in statement position, never inside a dotted
//!   reference. `type_ref` is a `qualified_name`; `string` and `int` are the
//!   string and integer literals.
//! * Real keywords are fixed lexer tokens, escapable with `^` — the list is
//!   the [`Token`] enum. *Contextual* words are ordinary identifiers that are
//!   special only in the grammar position noted: `id`/`readonly`
//!   (feature modifiers), `schema`/`sigil` (only directly after `import`),
//!   `to` (delegation line), `create`/`convert`/`format` (datatype blocks),
//!   the constraint keywords, and every `.ddd` word — the `.ddd` word tables
//!   in `crate::ddd` are that surface's keyword authority and are not
//!   restated here.
//! * Doc comments (`///`, `/** ... */`) attach to the element whose first
//!   line they end on, by position. They are not part of any production.
//! * `raw_body` is a balanced `{ ... }` scan whose contents are deliberately
//!   not parsed; `raw_parens` is the same over `(...)`. Both yield a span the
//!   driver slices (op bodies, `when` conditions, document expressions).
//! * `multiplicity` is `"[" (int (".." (int | "*"))?)? "]"`: `[]` is
//!   unbounded, `[n]` exact, `[n..m]` and `[n..*]` ranges.
//! * Expressions (`when` conditions, `expr` op bodies, `.ddd` document
//!   entries) are captured raw here and parsed/typed later by rex-expr; the
//!   expression grammar is normative in `docs/EXPRESSIONS.md` and is not
//!   duplicated below.
//!
//! ## `.mox` model sources ([`parse`])
//!
//! ```text
//! model            := item*
//! item             := package_decl | annotation_decl | import_schema_decl
//!                   | import_sigil_decl | class_decl | interface_decl
//!                   | enum_decl | datatype_decl | vocabulary_decl
//!                   | actors_block
//! package_decl     := "package" qualified_name
//! annotation_decl  := "annotation" string ("as" name)?
//! import_schema_decl := "import" "schema" string ("as" name)?
//! import_sigil_decl  := "import" "sigil" string
//! class_decl       := "class" name ("extends" type_ref ("," type_ref)*)?
//!                     "{" feature* "}"
//! feature          := modifier* ( containment | reference | container
//!                               | op_decl | derived_decl | attribute )
//! modifier         := "id" | "readonly"
//! containment      := "contains" type_ref multiplicity? name
//!                     ("opposite" name)?
//! reference        := "refers" type_ref multiplicity? name
//!                     ("opposite" name)?
//! container        := "container" type_ref name ("opposite" name)?
//! op_decl          := "op" type_ref name "(" params? ")" op_body?
//! derived_decl     := "derived" type_ref multiplicity? name op_body?
//! attribute        := type_ref multiplicity? name ("=" default)?
//!                     constraint_block?
//! params           := param ("," param)*
//! param            := type_ref name
//! default          := string | int | "true" | "false" | name
//! op_body          := "{" target_body+ "}" | raw_body
//! target_body      := name raw_body
//! constraint_block := "{" constraint_entry* "}"
//! constraint_entry := constraint_keyword (string | int) | "unique"
//! constraint_keyword := "pattern" | "minLength" | "maxLength"
//!                     | "minimum" | "maximum"
//! enum_decl        := "enum" name "{" literal+ "}"
//! literal          := name ("as" string)? ("=" int)?
//! datatype_decl    := "type" name "wraps" ("opaque" | qualified_name)?
//!                     datatype_block?
//! datatype_block   := "{" datatype_entry* "}"
//! datatype_entry   := binding_entry | "format" string
//!                   | "create" target_bodies | "convert" target_bodies
//! binding_entry    := name string
//! interface_decl   := "interface" name "{" binding_entry* "}"
//! vocabulary_decl  := "vocabulary" name "from" string "{" vocab_item* "}"
//! vocab_item       := "version" string | "key" name | "facet" type_ref name
//! ```
//!
//! Grammar-position notes the AST or driver enforces (a violated note is a
//! reported error, not a silent acceptance):
//!
//! * At most one `package` declaration is kept — the first; any later one is
//!   dropped silently. `annotation` declarations are top-level only.
//! * An `import schema` is committed once `"schema"` matched: a missing path
//!   or a missing name after `as` is an error (recovered, so the model
//!   parses on). `import sigil` likewise, and a present `as` clause is an
//!   error — a whole namespace set is imported, never one type.
//! * `op_body`'s bare `raw_body` alternative parses but is rejected by the
//!   driver; a `derived` body must be a single `expr` target body (the
//!   neutral expression language, `docs/EXPRESSIONS.md`).
//! * Constraint keywords each at most once per block; `unique` takes no
//!   value (a literal after it is an error) and is the collection-level
//!   constraint.
//! * In a `datatype_block`, `format`, `create`, and `convert` each appear at
//!   most once, and a binding target literally named `format` is an error —
//!   the unescaped key declares the format (only writable escaped,
//!   `^format`).
//!
//! ## The shared `actors_block`
//!
//! One grammar, two homes: the inline `.mox` declaration (as an `item`
//! above) and the standalone `.actor` file.
//!
//! ```text
//! actors_block    := "actors" name "{" actors_item* "}"
//! actors_item     := actor_decl | capability_decl | purpose_decl
//!                  | obligation_decl | grant_decl | delegation_decl
//!                  | never_both_decl
//! actor_decl      := ("actor" | "agent") name ("extends" name)?
//! capability_decl := "capability" name "on" type_ref
//! purpose_decl    := "purpose" name
//! obligation_decl := "obligation" name
//! grant_decl      := "grant" name "{" grant_entry* "}"
//! grant_entry     := effect_entry | cedar_entry
//! effect_entry    := ("permit" | "forbid") name ("when" raw_parens)?
//!                    obligation*
//! obligation      := "obligation" name
//! cedar_entry     := "cedar" raw_body
//! delegation_decl := "delegation" name "{" "from" name "to" name
//!                     ("purpose" name)? delegation_entry* "}"
//! delegation_entry := effect_entry
//! never_both_decl := "never_both" "{" name "," name ("," name)* "}"
//! ```
//!
//! * The body items may be interleaved in any order; the AST folds them into
//!   per-kind lists.
//! * The delegation shape above is the only *valid* one; the parser accepts
//!   any item order and reports misplaced/duplicate `from`/`to`/`purpose`
//!   lines and `cedar` entries (not allowed inside a delegation) as errors.
//! * `never_both` requires at least two capability names (an error,
//!   recovered).
//! * `when` conditions are `raw_parens` spans: the driver slices the
//!   condition text and type-checks it against the capability's class via
//!   rex-expr.
//!
//! ## `.actor` policy files ([`parse_actors`])
//!
//! ```text
//! actor_file  := actor_file_item*
//! actor_file_item := import_decl | actors_block
//! import_decl := "import" string
//! ```
//!
//! * An empty file is legal; duplicate imports are a driver concern, not a
//!   syntax error. An `import` after an `actors` block is an error
//!   (recovered): imports must precede every block.
//! * Unlike the `.ddd` import (below), an `.actor` import takes no trailing
//!   `;`.
//!
//! ## `.ddd` design sources ([`parse_ddd`])
//!
//! ```text
//! ddd_file          := ddd_file_item*
//! ddd_file_item     := ddd_import | application_decl
//! ddd_import        := "import" string ";"?
//! application_decl  := "application" name "{" application_item* "}"
//! application_item  := base_decl | module_decl
//! base_decl         := "base" qualified_name
//! module_decl       := "module" name "{" module_member* "}"
//! module_member     := service_decl | design_decl | search_decl
//! service_decl      := "service" name "{" service_member* "}"
//! service_member    := service_op | inject_decl
//! service_op        := "protected"? signature capability_clause? guard_clause? ";"
//!                    | "protected"? name "=>" qualified_name
//!                      capability_clause? guard_clause? ";"
//! inject_decl       := "inject" name ";"
//! signature         := type_ref multiplicity? name
//!                      "(" (param ("," param)*)? ")"
//! param             := type_ref multiplicity? name
//! capability_clause := "capability" name ("," name)*
//! design_decl       := "abstract"? stereotype name design_flag*
//!                      repository_decl? machine_decl?
//! stereotype        := "entity" | "value" | "dto"
//! design_flag       := "!"? ("scaffold" | "auditable" | "optimisticLocking"
//!                               | "nonPersistent" | "cache")
//! repository_decl   := "repository" name "{" repository_op* "}"
//! repository_op     := "protected"? (repository_builtin | signature)
//!                      guard_clause? ";"
//! repository_builtin := "findById" | "findAll" | "findByExample"
//!                    | "findByKeys" | "save" | "delete"
//! machine_decl      := "machine" name ":" type_ref "{" machine_member* "}"
//! machine_member    := "initial" name ";" | "terminal" name ";"
//!                    | transition ";"
//! transition        := name "->" name ("via" name)? guard_clause?
//! guard_clause      := "when" raw_parens
//! search_decl       := "search" name "{" search_member* "}"
//! search_member     := "entity" qualified_name
//!                    | "text" "{" search_field* "}"
//!                    | "filters" "{" qualified_name* "}"
//!                    | "sort" "{" qualified_name* "}"
//!                    | "document" "{" document_entry* "}"
//!                    | "ranking" ("bm25" | "tfIdf" | "exact"
//!                                | "custom" string)
//!                    | "analyzer" string
//!                    | "pagination" "{" pagination_member* "}"
//!                    | "capability" name
//! search_field      := qualified_name ("boost" int)? ("analyzer" string)?
//! document_entry    := name "=" raw_expr ";"
//! pagination_member := "limit" int | "max" int | "cursor"
//! ```
//!
//! * Every `.ddd` word above is a **contextual identifier** (the `id`/
//!   `readonly` precedent), escapable with `^`; the tables in `crate::ddd`
//!   are the keyword authority, and the stereotype/flag/builtin/search-member
//!   alternatives follow the table orders (the formatter's canonical order).
//! * Imports must precede the `application` declaration and there is exactly
//!   one application per file (violations are reported errors); the
//!   delegated-operation `qualified_name` is `target.operation`, split at its
//!   last segment (a bare name recovers with an error).
//! * Declared repository operations take no `capability` clause (the wire
//!   artifact carries none, so accepting one would silently drop data); a
//!   delegated op may be named `save` etc. — contextual words.
//! * `design_flag`s and the single-member search lines (`entity`, `ranking`,
//!   `analyzer`, `pagination`) are idempotent on repeat — the first
//!   declaration wins; repeated `text`/`filters`/`sort`/`document` clauses
//!   merge. A flag may be written negated (`!auditable`): the negation is
//!   the opt-out from a flag that defaults on — whether a `!flag` is
//!   *meaningful* (and each flag's default) is the driver's rule 4.
//! * `base` must be the application's first member; a duplicate or late
//!   `base` is an error.
//! * `document_entry`'s `raw_expr` spans balanced `()`/`[]`/`{}` nesting up
//!   to the terminating `;` (string literals are single tokens), and is
//!   stored verbatim; parsing/typing it is the driver's job.
//! * A design may carry at most one `machine` block, after the optional
//!   `repository` block (the formatter's canonical order). `machine`,
//!   `initial`, `terminal`, and `via` are contextual words like the rest.
//!   A transition takes one target — the comma-separated multi-target arrow
//!   is deliberately excluded (an unguarded external choice is not a
//!   deterministic machine; write one edge per transition). The `when` of a
//!   `guard_clause` is the real lexer keyword; the guard itself is a
//!   `raw_parens` span the driver slices and types. The driver validates the
//!   machine (one `initial`, declared transition endpoints, terminal
//!   states without outgoing edges, reachability, duplicate triggers,
//!   guard/op state references, the bound feature and its enum) and derives
//!   the workflow actions (`via` names; bare arrows get a documented
//!   derivation) — see `docs/DDD.md`.
//!
//! ## `.evt` event-contract sources ([`parse_evt`])
//!
//! ```text
//! evt_file          := evt_file_item*
//! evt_file_item     := evt_import | event_decl | channel_decl
//!                    | subscription_decl
//! evt_import        := "import" string
//! event_decl        := "event" name ("version" string)? "{" field* "}"
//! field             := name ":" type_ref ";"
//! channel_decl      := "channel" name "{" publishes_entry* "}"
//! publishes_entry   := "publishes" name ";"
//! subscription_decl := "subscription" name "{" "events" "[" name* "]"
//!                      "consumer" name "}"
//! ```
//!
//! * `evt_import` is the `.actor` import production verbatim (no trailing
//!   `;`), and `type_ref` is the shared `.mox` production. Imports must
//!   precede the first declaration; a later `import` is an error
//!   (recovered, and the misplaced import is dropped), mirroring the
//!   `.actor` rule. Duplicate imports are a driver concern, not a syntax
//!   error.
//! * The three declaration kinds may be **interleaved** in any order; the
//!   AST folds them into per-kind lists (the `actors_block` precedent), and
//!   the formatter canonicalizes to import → event → channel →
//!   subscription order.
//! * `events` and `consumer` are **contextual** words: ordinary identifiers
//!   special only inside a subscription body (the delegation-`to`
//!   precedent; escapable forms `^events`/`^consumer` never match). All of
//!   `event`, `channel`, `publishes`, `subscription` are real keywords,
//!   escapable with `^` like every keyword.
//! * Slice-1 exclusions are deliberate: no `when` filters, no delivery
//!   policy, no webhooks, no CloudEvents attributes; event fields are plain
//!   typed fields (no constraints, facets, `readonly`/`id`, or
//!   multiplicities).
//! * Recovery: an event's `version` clause, a field's `:`/type/`;`, a
//!   `publishes` entry's name/`;`, and a subscription's `events [...]` /
//!   `consumer` halves are each committed once their leading word matched —
//!   missing pieces are reported errors and the declaration still recovers.
//!   Inside the `events [...]` list the contextual `consumer` word ends the
//!   list, so a missing `]` never swallows the consumer line. A body item
//!   that cannot start a field/entry at all fails the whole declaration
//!   into file-level junk (the `actors_block` coarseness).
//!
//! ## `.deploy` deployment sources ([`parse_deploy`])
//!
//! ```text
//! deploy_file       := (deploy_import | application_decl | profile_decl
//!                      | deployment_decl)*
//! deploy_import     := "import" string
//! application_decl  := "application" name "{" application_item* "}"
//! application_item  := component_decl | connects_decl
//! component_decl    := "component" name ":" name component_body?
//! component_body    := "{" component_member* "}"
//! component_member  := requires_clause | design_binding | flow_binding
//!                    | setting_line
//! requires_clause   := "requires" name ("," name)*
//! design_binding    := "design" string
//! flow_binding      := "flow" string
//! setting_line      := name ":" setting_value
//! setting_value     := string | int | name
//! connects_decl     := "connects" name "->" name
//! profile_decl      := "profile" name "{" profile_member* "}"
//! profile_member    := target_line | defaults_block | mappings_block
//!                    | policy_decl
//! target_line       := "target" ":" name
//! defaults_block    := "defaults" "{" path_setting* "}"
//! configure_block   := "configure" "{" path_setting* "}"
//! path_setting      := qualified_name ":" setting_value
//! mappings_block    := "mappings" "{" mapping_entry* "}"
//! mapping_entry     := name "->" qualified_name
//! policy_decl       := ("require" | "prohibit") raw_parens
//! deployment_decl   := "deployment" name "for" name "{" use_line
//!                      configure_block? "}"
//! use_line          := "use" name
//! ```
//!
//! * Every `.deploy` word above is a **contextual identifier** (the `.ddd`
//!   discipline): special only in the grammar position noted, an ordinary
//!   name everywhere else, escapable with `^`. The tables in
//!   `crate::deploy` are the keyword authority shared by the parser and the
//!   formatter. The `->` arrow is a real lexer token ([`Token::Arrow`]).
//! * The component-kind and target *vocabularies* (`api`, `kubernetes`,
//!   `cloudflareWorkers`, ...) are deliberately **not** syntax: both
//!   positions are plain `name`s and the driver validates them against its
//!   closed tables, so the word lists live in the driver, not here. Policy
//!   expressions are [`raw_parens`] spans — the driver slices strictly
//!   inside the parens (the `when`-condition convention) and parses, types,
//!   and evaluates the text with rex-expr.
//! * A binding's string is `"<path>"` or `"<path>#<Module>"`, split at its
//!   last `#`; the path must match one of the file's imports *as written*.
//! * Imports must precede the first declaration; a later `import` is an
//!   error (recovered, and the misplaced import is dropped), mirroring the
//!   `.actor`/`.evt` rule. Duplicate imports are a driver concern.
//! * The three declaration kinds may be **interleaved** in any order; the
//!   AST folds them into per-kind lists, and the formatter canonicalizes to
//!   import → application → profile → deployment order.
//! * Recovery: a component's kind or body, a `requires` clause's comma
//!   list, a setting's `:`/value, a `connects` entry's halves, a profile's
//!   `target` line, a policy's parens, a mapping entry, a deployment's
//!   `for`/`use` halves, and a `configure` block are each committed once
//!   their leading word matched — missing pieces are reported errors and
//!   the declaration still recovers.
//!
//! ## `.uc` use case sources ([`parse_usecases`])
//!
//! ```text
//! uc_file          := (uc_import | usecase_decl)*
//! uc_import        := "import" string ("as" name)?
//! usecase_decl     := "usecase" name "{" usecase_member* "}"
//! usecase_member   := participant_decl | message_decl | succession_stmt
//!                   | realizes_decl
//! participant_decl := "subject" name ":" type_ref participant_body?
//!                   | "actor" name (":" type_ref)? participant_body?
//! participant_body := "{" (occurrence_decl | perform_decl)* "}"
//! occurrence_decl  := "occurrence" name ("." ("source" | "target"))? ";"
//! perform_decl     := "perform" name (":" name)?
//!                     ("requires" "[" name* "]")? ";"
//! message_decl     := "message" name ("of" type_ref)? ("from" endpoint)?
//!                     ("to" endpoint)? ";"
//! endpoint         := name "." name
//! succession_stmt  := "first" node_ref "then" node_ref ";"
//! node_ref         := ("start" | "done") | name ("." name)?
//! realizes_decl    := "realizes" name "." name "." name ";"
//! ```
//!
//! * Every `.uc` word above is a **contextual identifier** (the `.ddd`
//!   discipline): special only in the grammar position noted, an ordinary
//!   name everywhere else, escapable with `^`. The table in
//!   `crate::usecases` is the keyword authority shared by the parser and
//!   the formatter. `actor`, `import`, `as`, and the message `from` are
//!   real lexer keywords (`actor` is the bare policy-actor introducer).
//! * Semicolons follow the `.evt` rule: every statement line
//!   (`occurrence`, `perform`, `message`, `succession`, `realizes`) ends
//!   in `;`, participant lines and block declarations end without one. The
//!   formatter emits the canonical `;` whether or not the source had one.
//! * Usecase members may be **interleaved** in any order; the AST folds
//!   them into per-kind lists (the `actors_block` precedent), with `type_ref`
//!   the shared `.mox` production. A `perform x: op` clause names the
//!   performed operation; the bare `perform x` form names it after the
//!   step. A `subject` requires its `: <type>` clause; a bare `actor` is a
//!   policy actor with no type. A second `subject` line is a reported
//!   error (the first wins); duplicate names of any other kind are a
//!   driver concern, not a syntax error.
//! * Imports must precede the first `usecase` declaration; a later
//!   `import` is an error (recovered, and the misplaced import is
//!   dropped), mirroring the `.actor`/`.evt` rule. Duplicate imports are a
//!   driver concern.
//! * Doc comments (`///` runs) directly above a `usecase` declaration
//!   become its description.
//! * Recovery: an import's alias, a participant's `:`/type, an
//!   occurrence's name/tail/`;`, a perform's `:`/operation/bracketed
//!   list/`;`, a message's name/clauses/`;`, a succession's `then` half
//!   and `;`, and a realizes binding's arity/`;` are each committed once
//!   their leading word matched — missing pieces are reported errors and
//!   the declaration still recovers. Inside a `requires [...]` list the
//!   contextual `occurrence`/`perform` words end the list (the words
//!   introducing the next member), so a missing `]` never swallows the
//!   next line. A member that cannot start anything at all fails the whole
//!   usecase into file-level junk (the `actors_block` coarseness).
//!
//! ## `.cmmn` case management sources ([`parse_cmmn`])
//!
//! ```text
//! cmmn_file        := (cmmn_import | case_decl)*
//! cmmn_import      := "import" string ("as" name)?
//! case_decl        := "case" name "{" case_member* "}"
//! case_member      := roles_block | case_file_block | plan_item_decl
//! roles_block      := "roles" "{" role_decl* "}"
//! role_decl        := "role" name ("as" name)? ";"
//! case_file_block  := "case_file" "{" file_item* "}"
//! file_item        := type_ref multiplicity? name ";"
//! plan_item_decl   := "discretionary" simple_item_decl
//!                   | simple_item_decl | stage_decl
//! simple_item_decl := ("task" | "service" | "milestone") name plan_body?
//! stage_decl       := "stage" name stage_body?
//! plan_body        := "{" plan_member* "}"
//! stage_body       := "{" (plan_member | plan_item_decl)* "}"
//! plan_member      := performer_clause | execute_clause
//!                   | criterion_decl | flag_decl
//! performer_clause := "performer" ":" name ";"
//! execute_clause   := "execute" name "." name "." name ";"
//! criterion_decl   := ("entry" | "exit") "on" on_part ("," on_part)*
//!                     ("when" raw_parens)? ";"
//! on_part          := name ("." name)?
//! flag_decl        := "required" ";" | "repeats" ";" | "auto" ";"
//! ```
//!
//! * Every `.cmmn` word above is a **contextual identifier** (the `.ddd`
//!   discipline): special only in the grammar position noted, an ordinary
//!   name everywhere else, escapable with `^`. The table in `crate::cmmn`
//!   is the keyword authority shared by the parser and the formatter.
//!   `import`, `as`, `on`, and `when` are real lexer keywords (`on`/`when`
//!   are the `.actor` condition keywords).
//! * A case-file item reuses the `.mox` attribute production (type-first
//!   with the shared `multiplicity` annotation): `Borrower borrower;`,
//!   `Borrower[0..1] borrower;`, `FinancialStatements[] financials;`.
//! * An on-part is `name` (a domain event from an imported `.evt`
//!   contract) or `name.tail` — a plan item standard event or a case-file
//!   transition. The **transition vocabulary** (`create`, `enable`,
//!   `start`, `complete`, `terminate`, `fail`, `occur`, `update`,
//!   `delete`, `childUpdate`) is deliberately **not** syntax: both
//!   segments are plain names and the driver validates the tail against
//!   its closed tables (the `.deploy` vocabulary discipline). The `when`
//!   if-part is a [`raw_parens`] span — the driver slices strictly inside
//!   the parens (the `when`-condition convention) and types the text with
//!   rex-expr.
//! * The `discretionary` prefix is grammatical before a simple item
//!   (`task`, `service`, or `milestone`) but not before a `stage`; the
//!   driver rejects it on milestones — discretionary planning is a
//!   task/service concept.
//! * Semicolons follow the `.evt` rule: every statement line (`role`,
//!   case-file item, `performer`, `execute`, criterion, flag) ends in `;`;
//!   block declarations (`case`, `stage`, `task`, `service`, `milestone`,
//!   `roles`, `case_file`) end without one. The formatter emits the
//!   canonical `;` whether or not the source had one.
//! * Imports must precede the first `case` declaration; a later `import`
//!   is an error (recovered, and the misplaced import is dropped),
//!   mirroring the `.actor`/`.evt`/`.uc` rule. Duplicate imports are a
//!   driver concern.
//! * Doc comments (`///` runs) directly above a `case`, a plan item, or a
//!   case-file item become its description.
//! * Recovery: an import's alias, a role's `as`/name/`;`, a case-file
//!   item's type/multiplicity/name/`;`, a plan item's name/body, a
//!   performer's `:`/name/`;`, an execute binding's arity/`;`, a
//!   criterion's `on`/parts/`when`/`;`, and a flag's `;` are each
//!   committed once their leading word matched — missing pieces are
//!   reported errors and the declaration still recovers. A body member
//!   that cannot start anything fails into the enclosing body's junk (the
//!   `actors_block` coarseness); a member that cannot start anything at
//!   all fails the whole case into file-level junk.

use std::ops::{Range, RangeFrom};

use chumsky::input::{ExactSizeInput, ValueInput};
use chumsky::prelude::*;

use crate::ast::*;
use crate::lexer::Token;

/// Parser input: a token slice paired with a custom [`chumsky::Input`]
/// implementation that reports **byte-offset** spans (chumsky's built-in slice
/// input would report token indices instead).
#[derive(Clone, Copy)]
struct Tokens<'src> {
    tokens: &'src [(Token<'src>, Span)],
}

impl<'src> Tokens<'src> {
    fn new(tokens: &'src [(Token<'src>, Span)]) -> Self {
        Tokens { tokens }
    }

    /// Byte offset just past the end of the last token (0 for empty input).
    fn eof_offset(&self) -> usize {
        self.tokens.last().map(|(_, span)| span.end).unwrap_or(0)
    }

    /// Convert a token-index range into a byte span. An empty range maps to an
    /// empty span at the start offset of the token at `start` (or the end of
    /// the input when at EOF).
    fn byte_span(&self, start: usize, end: usize) -> Span {
        let start_byte = match self.tokens.get(start) {
            Some((_, span)) => span.start,
            None => self.eof_offset(),
        };
        let end_byte = if end > start {
            self.tokens
                .get(end - 1)
                .map(|(_, span)| span.end)
                .unwrap_or_else(|| self.eof_offset())
        } else {
            start_byte
        };
        (start_byte..end_byte).into()
    }
}

impl<'src> Input<'src> for Tokens<'src> {
    type Cursor = usize;
    type Span = Span;
    type Token = Token<'src>;
    type MaybeToken = Token<'src>;
    type Cache = Self;

    fn begin(self) -> (Self::Cursor, Self::Cache) {
        (0, self)
    }

    fn cursor_location(cursor: &Self::Cursor) -> usize {
        *cursor
    }

    unsafe fn next_maybe(
        this: &mut Self::Cache,
        cursor: &mut Self::Cursor,
    ) -> Option<Self::MaybeToken> {
        let (token, _) = this.tokens.get(*cursor)?;
        *cursor += 1;
        Some(token.clone())
    }

    unsafe fn span(this: &mut Self::Cache, range: Range<&Self::Cursor>) -> Self::Span {
        this.byte_span(*range.start, *range.end)
    }
}

impl<'src> ValueInput<'src> for Tokens<'src> {
    unsafe fn next(this: &mut Self::Cache, cursor: &mut Self::Cursor) -> Option<Self::Token> {
        Self::next_maybe(this, cursor)
    }
}

impl<'src> ExactSizeInput<'src> for Tokens<'src> {
    unsafe fn span_from(this: &mut Self::Cache, range: RangeFrom<&Self::Cursor>) -> Self::Span {
        let start_byte = match this.tokens.get(*range.start) {
            Some((_, span)) => span.start,
            None => this.eof_offset(),
        };
        (start_byte..this.eof_offset()).into()
    }
}

type MoxError<'src> = Rich<'src, Token<'src>, Span>;
type MoxExtra<'src> = extra::Err<MoxError<'src>>;

/// A parse error with a span and a rendered message.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message} at byte offsets {span}")]
pub struct ParseError {
    /// Human-readable description of the error.
    pub message: String,
    /// Byte range the error refers to.
    pub span: Span,
}

/// The outcome of parsing: as much of the AST as could be recovered, plus all
/// encountered errors. Parsing never panics and always produces a result.
#[derive(Debug, Clone, PartialEq)]
pub struct ParseResult {
    /// The recovered model, or `None` if no output could be produced at all.
    pub ast: Option<Model>,
    /// All errors encountered during lexing and parsing.
    pub errors: Vec<ParseError>,
}

fn kw<'src>(token: Token<'src>) -> impl Parser<'src, Tokens<'src>, Span, MoxExtra<'src>> + Clone {
    select! { t = e if t == token => e.span() }
}

// Shared combinators and per-surface grammars:
mod actor;
mod cmmn;
mod ddd;
mod deploy;
mod docs;
mod evt;
mod imports;
mod mox;
mod shared;
mod uc;

use shared::*;

pub use actor::parse_actors;
pub use actor::ActorsParseResult;
pub use cmmn::parse_cmmn;
pub use cmmn::CmmnParseResult;
pub use ddd::parse_ddd;
pub use ddd::DddParseResult;
pub use deploy::parse_deploy;
pub use deploy::DeployParseResult;
pub use evt::parse_evt;
pub use evt::EvtParseResult;
pub use mox::parse;
pub use uc::parse_usecases;
pub use uc::UcParseResult;

/// Keeps the normative grammar in this module's docs in sync with the code.
///
/// The grammar authority lives as EBNF blocks in the `//!` docs above (see
/// "The normative grammar"). Full grammar-from-docs validation is out of
/// scope; these tests are the cheap, honest tripwire: every *keyword
/// terminal* of the EBNF must be a word the lexer/parser actually knows, and
/// every keyword the lexer knows must appear in the EBNF — so adding,
/// renaming, or removing a keyword without updating the grammar fails here.
/// Structural drift (repetition/optionality changes) is not caught
/// mechanically.
#[cfg(test)]
mod grammar_doc_tests {
    use crate::cmmn;
    use crate::ddd;
    use crate::deploy;
    use crate::lexer::lex;
    use crate::usecases;

    /// This file's source; the leading `//!` lines are the module docs that
    /// carry the grammar blocks.
    const PARSER_SOURCE: &str = include_str!("mod.rs");

    /// The module docs (the leading `//!` line run), comment markers stripped.
    fn module_docs() -> String {
        let mut docs = String::new();
        for line in PARSER_SOURCE.lines() {
            let Some(rest) = line.strip_prefix("//!") else {
                break;
            };
            docs.push_str(rest.strip_prefix(' ').unwrap_or(rest));
            docs.push('\n');
        }
        docs
    }

    /// The fenced ```text blocks of the docs: the EBNF grammar blocks.
    fn grammar_blocks() -> Vec<String> {
        let mut blocks = Vec::new();
        let mut current: Option<Vec<String>> = None;
        for line in module_docs().lines() {
            match (current.as_mut(), line.trim()) {
                (None, "```text") => current = Some(Vec::new()),
                (Some(block), "```") => {
                    blocks.push(block.join("\n"));
                    current = None;
                }
                (Some(block), line) => block.push(line.to_string()),
                (None, _) => {}
            }
        }
        assert!(current.is_none(), "unterminated grammar block");
        blocks
    }

    /// The quoted EBNF terminals of all grammar blocks that are shaped like
    /// words (punctuation terminals such as `"{"` are skipped).
    fn grammar_words() -> Vec<String> {
        let mut words = Vec::new();
        for block in grammar_blocks() {
            for terminal in block.split('"').skip(1).step_by(2) {
                if !terminal.is_empty()
                    && terminal
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_')
                {
                    words.push(terminal.to_string());
                }
            }
        }
        words
    }

    /// Contextual words the grammar quotes that are *not* lexer keywords and
    /// not covered by a table below: feature modifiers, the import sigils,
    /// the delegation `to`, the datatype block words, `unique`, the `.evt`
    /// subscription words, and the parser-only `ranking` values
    /// (`crate::ddd` documents them as parser-only).
    const CONTEXTUAL_WORDS: [&str; 17] = [
        "id", "readonly", "schema", "sigil", "to", "create", "convert", "format", "unique", "bm25",
        "tfIdf", "exact", "custom", "entity", "value", "events", "consumer",
    ];

    /// Every keyword of the lexer. Each entry is checked against the lexer
    /// itself, so this list and `Token` cannot drift apart silently.
    const KEYWORDS: [&str; 42] = [
        "package",
        "annotation",
        "as",
        "class",
        "extends",
        "interface",
        "enum",
        "type",
        "wraps",
        "opaque",
        "contains",
        "refers",
        "container",
        "opposite",
        "op",
        "derived",
        "true",
        "false",
        "vocabulary",
        "from",
        "version",
        "key",
        "facet",
        "actors",
        "actor",
        "agent",
        "capability",
        "grant",
        "permit",
        "forbid",
        "when",
        "obligation",
        "on",
        "never_both",
        "delegation",
        "purpose",
        "cedar",
        "event",
        "channel",
        "publishes",
        "subscription",
        "import",
    ];

    /// Every word the grammar is allowed to quote as a terminal.
    fn known_words() -> Vec<&'static str> {
        let mut words: Vec<&'static str> = KEYWORDS.to_vec();
        words.extend(CONTEXTUAL_WORDS);
        words.extend(super::VALUED_CONSTRAINT_KEYWORDS);
        words.extend([
            ddd::APPLICATION,
            ddd::BASE,
            ddd::MODULE,
            ddd::SERVICE,
            ddd::SEARCH,
            ddd::ABSTRACT,
            ddd::REPOSITORY,
            ddd::INJECT,
            ddd::PROTECTED,
            ddd::MACHINE,
            ddd::INITIAL,
            ddd::TERMINAL,
            ddd::VIA,
            ddd::BOOST,
            ddd::ANALYZER,
            ddd::LIMIT,
            ddd::MAX,
            ddd::CURSOR,
        ]);
        for (keyword, _) in ddd::STEREOTYPES {
            words.push(keyword);
        }
        for (keyword, _) in ddd::DESIGN_FLAGS {
            words.push(keyword);
        }
        for (keyword, _) in ddd::REPOSITORY_BUILTINS {
            words.push(keyword);
        }
        for (kind, keyword) in ddd::SEARCH_MEMBERS {
            let _ = kind;
            if let Some(keyword) = keyword {
                words.push(keyword);
            }
        }
        for word in [
            deploy::APPLICATION,
            deploy::COMPONENT,
            deploy::CONNECTS,
            deploy::PROFILE,
            deploy::DEPLOYMENT,
            deploy::TARGET,
            deploy::DEFAULTS,
            deploy::MAPPINGS,
            deploy::REQUIRE,
            deploy::PROHIBIT,
            deploy::REQUIRES,
            deploy::DESIGN,
            deploy::FLOW,
            deploy::USE,
            deploy::CONFIGURE,
            deploy::FOR,
        ] {
            words.push(word);
        }
        for word in [
            usecases::USECASE,
            usecases::SUBJECT,
            usecases::OCCURRENCE,
            usecases::PERFORM,
            usecases::MESSAGE,
            usecases::FIRST,
            usecases::THEN,
            usecases::OF,
            usecases::TO,
            usecases::START,
            usecases::DONE,
            usecases::REQUIRES,
            usecases::REALIZES,
            usecases::SOURCE,
            usecases::TARGET,
        ] {
            words.push(word);
        }
        for word in [
            cmmn::CASE,
            cmmn::ROLES,
            cmmn::ROLE,
            cmmn::CASE_FILE,
            cmmn::ENTRY,
            cmmn::EXIT,
            cmmn::PERFORMER,
            cmmn::EXECUTE,
            cmmn::REQUIRED,
            cmmn::REPEATS,
            cmmn::AUTO,
            cmmn::DISCRETIONARY,
            cmmn::STAGE,
            cmmn::TASK,
            cmmn::SERVICE,
            cmmn::MILESTONE,
        ] {
            words.push(word);
        }
        words
    }

    /// The grammar blocks exist and every word terminal is a known keyword or
    /// contextual word — quoting a word the parser would not recognize fails.
    #[test]
    fn grammar_terminals_are_known_words() {
        let blocks = grammar_blocks();
        assert!(
            blocks.len() >= 5,
            "expected the .mox, actors, .actor, .ddd, and .evt grammar blocks, found {}",
            blocks.len()
        );
        let known = known_words();
        for word in grammar_words() {
            let word = word.as_str();
            assert!(
                known.contains(&word),
                "the grammar quotes `{word}`, which is neither a lexer keyword \
                 nor a known contextual word — update the grammar or the word \
                 lists in this test"
            );
        }
    }

    /// Every lexer keyword appears in the grammar — adding or renaming a
    /// keyword in [`Token`] without updating the grammar fails.
    #[test]
    fn every_lexed_keyword_appears_in_the_grammar() {
        let words = grammar_words();
        for keyword in KEYWORDS {
            // The list itself must stay honest: each entry really is a
            // keyword token of the lexer.
            let tokens = lex(keyword).unwrap();
            assert_eq!(
                tokens.len(),
                1,
                "`{keyword}` must be exactly one token to be a keyword"
            );
            assert_eq!(
                tokens[0].0.keyword(),
                Some(keyword),
                "`{keyword}` is listed as a keyword but lexes otherwise — \
                 update KEYWORDS in this test"
            );
            assert!(
                words.iter().any(|word| word == keyword),
                "the lexer keyword `{keyword}` is missing from the normative \
                 grammar blocks in this module's docs"
            );
        }
    }

    /// Every `.ddd` table keyword appears in the grammar — extending a
    /// `crate::ddd` table (the keyword authority #36 single-sourced) without
    /// updating the grammar fails.
    #[test]
    fn every_ddd_table_keyword_appears_in_the_grammar() {
        let words = grammar_words();
        let check = |keyword: &'static str| {
            assert!(
                words.iter().any(|word| word == keyword),
                "the .ddd keyword `{keyword}` is missing from the normative \
                 grammar blocks in this module's docs"
            );
        };
        for (keyword, _) in ddd::STEREOTYPES {
            check(keyword);
        }
        for (keyword, _) in ddd::DESIGN_FLAGS {
            check(keyword);
        }
        for (keyword, _) in ddd::REPOSITORY_BUILTINS {
            check(keyword);
        }
        for (_, keyword) in ddd::SEARCH_MEMBERS {
            if let Some(keyword) = keyword {
                check(keyword);
            }
        }
    }
}
