# Case management (.cmmn)

## Overview

A `.cmmn` file declares a **case**: the adaptive, event-and-condition-driven
work coordinated around a domain — the CMMN 1.1 (OMG formal/2016-06-01)
notion of case management, in rexlang idiom. A case is *not* a workflow and
*not* a state machine: its plan declares the work that is **available**
(tasks, service tasks, milestones, grouped into stages) and the **criteria**
under which each item activates, without prescribing a sequence. Sibling
tasks are concurrent by default; ordering exists only where a criterion
imposes it.

The division of responsibility this surface codifies:

- The **domain** (`.mox`) owns the data and its invariants.
- The **domain lifecycle** (`.ddd` machines) owns an aggregate's valid state
  transitions.
- The **case** owns the work: who does what, when, and why.
- The **engine** executes — rexlang authors and validates, the
  [cmmn-rs](https://github.com/yestechgroup/cmmn-rs) engine runs compiled
  definitions (see the [backend](#tooling)).

A lending application is the canonical shape: `VerifyBorrower` and
`CollectFinancials` can run concurrently; `CreditAssessment` activates only
once both complete; a credit-committee milestone marks the goal. The
application's aggregate state stays in the domain; the case coordinates the
work *around* it. **The domain owns its state; the case owns its work.**

```text
import "lending.mox"
import "lending.evt"
import "lending.actor"
import "lending.ddd" as lending

case CommercialLoan {

    roles {
        role relationship_manager as RelationshipManager;
        role credit_analyst as CreditAnalyst;
    }

    case_file {
        /// The borrower under assessment.
        Borrower[0..1] borrower;
        /// Financial statements received during assessment.
        FinancialStatements[] financials;
    }

    stage Intake {
        task VerifyBorrower {
            performer: relationship_manager;
        }
        milestone ApplicationReady {
            entry on VerifyBorrower.complete;
        }
    }

    stage Assessment {
        entry on ApplicationReady.occur;
        task CreditAssessment {
            performer: credit_analyst;
            entry on ApplicationReady.occur;
            exit on AssessmentComplete.occur;
        }
        service ScoreApplication {
            execute lending.CreditAssessmentService.assess;
            entry on financials.create, financials.update when (financials.size() > 0);
            required;
        }
        milestone AssessmentComplete {
            entry on CreditAssessment.complete, ScoreApplication.complete;
        }
    }
}
```

## Anatomy

- **`case`** — the declaration. Names are unique per file.
- **`roles`** — case roles a human task's `performer` names. A role may
  bind to an `.actor` surface actor (`role analyst as Analyst`): validated
  authorization metadata, carried on the artifact, with no CMMN semantics.
- **`case_file`** — typed case-file items, using the `.mox` attribute
  production (`Type[mult]? name;`). Multiplicity defaults to `0..1` (the
  `.mox` feature convention); `[1..1]`, `[0..*]`/`[]`, and `[1..*]` are the
  other engine-supported shapes. Plan items and case-file items share one
  namespace per case, so criterion references stay unambiguous.
- **Plan items** — `task` (human work; optional `performer: <role>`),
  `service` (an `execute <alias>.<Service>.<operation>;` binding, the
  use-case `realizes` rule), `milestone` (a goal; entry criteria only),
  and `stage` (grouping; the only kind that nests plan items).
- **Criteria** — `entry on <part>, <part> when (...);` and `exit on ...;`.
  An on-part is `item.word` — a plan item standard event (`create`,
  `enable`, `start`, `complete`, `terminate`, `fail`, `occur`) or a
  case-file transition (`create`, `update`, `delete`, `addChild`,
  `removeChild`) — or a bare `name` naming an event of an imported `.evt`
  contract. All on-parts must occur for the criterion to satisfy; the
  optional `when (...)` if-part is a boolean rex-expr condition over the
  case file, type-checked at compile time (below). Milestones carry no
  exit criteria.
- **Flags** — `required;` (the containing stage cannot complete without
  the item), `repeats;` (repetition on completion), and `auto;`
  (auto-start — an explicit FALSE manual-activation rule; the CMMN default
  is manual activation, so entry criteria *enable* a task and a human
  starts it). `discretionary task` marks work a knowledge worker plans
  into the case at runtime; discretionary milestones are rejected.

## Artifact wire format

`rexlang ir file.cmmn` emits the standalone
[`rex_ir::cmmn::CmmnModel`](crates/rex-ir/src/cmmn.rs) artifact — the
versioned wire format in the rexlang artifact family
(`CMMN_MODEL_FORMAT_VERSION`, camelCase keys, adjacent tagging, an empty
artifact serializing as exactly `{"formatVersion":1}`). The wire carries
**names as written and `when` conditions verbatim** (the wire rule 9
convention); resolution and expression semantics are driver concerns, never
wire concerns. The plan is a **flat graph**: stages reference children by
name, and a case's root items are the ones no stage lists.

## Grammar

The normative EBNF lives once in the parser module docs
([`crates/rex-syntax/src/parser.rs`](crates/rex-syntax/src/parser.rs),
section "`.cmmn` case management sources"); `rexlang fmt` canonicalizes to
the layout documented on [`format_cmmn`](crates/rex-syntax/src/fmt.rs).
Every `case` word is a contextual identifier (escapable with `^`);
`import`, `as`, `on`, and `when` are real lexer keywords. The transition
vocabularies are deliberately **not** syntax — both segments of an on-part
are plain names and the driver validates the tail against its closed
tables (the `.deploy` vocabulary discipline).

## Resolution & validation

`compile_cmmn_str` (normative contract on
[`crates/rex-driver/src/lib.rs`](crates/rex-driver/src/lib.rs)) resolves a
`.cmmn` file against **four import buckets**: raw `.mox` domains, plus
pre-compiled `EventModel`, `ActorModel`, and `DddModel` pairs the host
provides. Imports match by exact path or lexical resolution relative to the
case file (the `.deploy` rule); only design imports take an alias, because
aliases exist for `execute`.

The numbered rules (quoted here in brief, specified in full on the entry
point): unprovided imports; case-name uniqueness; role uniqueness and
`.actor` binding resolution; case-file item typing through the domain
union (`unknown type`, `ambiguous type`); the shared plan-item/case-file
namespace; performer placement and role resolution; `execute` placement,
necessity, and design resolution; discretionary placement; criterion
resolution (milestone exit criteria, on-part sources, transition
vocabularies, event resolution); and `when` typing.

**`when` conditions** type-check against a *synthetic case universe* (the
`.deploy` settings-universe pattern): the case file as a class whose
features are the case-file items (typed by the imported domains), each plan
item as a class of boolean transition predicates, and the case itself as
the root class. `case_file.financials.size() > 0` and
`VerifyBorrower.complete` are plain feature navigation from `self`; the
condition must be boolean, and the expression rules R1–R9 of
[docs/EXPRESSIONS.md](EXPRESSIONS.md) apply and are cited in errors.

## Tooling

```sh
rexlang fmt file.cmmn            # canonical layout (also --check)
rexlang check tests/conformance/ # directory batch: every surface, independently
rexlang ir file.cmmn -o case.json        # the CmmnModel artifact
rexlang artifact check case.json         # classifies + lint-checks the artifact
```

**The cmmn-rs backend** (`rex-backend-cmmn`) lowers a compiled `CmmnModel`
plus its domain model into the engine's `CaseDefinition` JSON — the case
file items (multiplicity vocabulary, domain-class features as properties),
a synthetic root stage, generated sentries, `User` tasks with
`requiredRole`, service tasks as `Process` compositions keyed by the
`execute` binding (the engine's executor-seam `processRef`), and
discretionary tasks as planning tables. `when` conditions translate
rex-expr → FEEL within a documented subset (`&&`→`and`, `size()`→`count`,
`case_file.x`→`x`, `if/then/else`, …); everything outside the subset is a
**named generate-time error** (the Cedar-backend convention), as are the
two engine gaps the surface accepts today: bare `.evt` event on-parts (the
engine's `OnPart` is plan-item/case-file transitions only) and plan-item
predicates inside `when` conditions (the engine evaluates if-parts against
the case-file scope alone).

Known follow-up: a dev-dependency on `cmmn-rs` (the `cedar-policy`
precedent) to validate the emitted definitions with the real
`deserialize_case_definition` in CI.
