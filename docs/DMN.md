# DMN decision model DSL (.dmn)

## Overview

A standalone decision-modeling surface implementing the DMN decision
architecture in the rexlang spirit: a `.dmn` file declares one **decision
model** whose central semantic structure is the **Decision Requirements
Graph** — the **input data** (the `inputs` block), the **decisions**, the
**business knowledge models** (reusable functions, the `model` declarations),
the **knowledge sources** (the authorities behind decisions), the **decision
services** (invocation contracts), and the **three requirement kinds**
(information, knowledge, authority) that wire the graph. Logic — a decision
table, a FEEL expression, an invocation of a business knowledge model — is
an implementation detail **attached to a node**, never the node itself: a
decision with no `logic` clause at all is a valid, first-class declaration
(the architecture-first reading of DMN, which `minimal.dmn` pins). Parsed by
**rex-syntax** (`parse_dmn`, the chumsky parser) at compile time, lowered
and validated by **rex-driver** (`compile_dmn_str`) against the imported
`.mox` domains, and emitted as the versioned `rex_ir::dmn::DmnModel`
artifact (the rex-ir crate).

**No in-tree backend exists yet**: downstream consumers ingest the artifact
exactly like the `DddModel`, `EventModel`, and `UseCaseModel` artifacts.

The normative grammar lives exactly once — the *`.dmn` decision-model
sources* section of the parser module docs
(`crates/rex-syntax/src/parser/mod.rs`); the normative compile contract —
entry points, resolution semantics, and validation rules — is documented on
`rex_driver::compile_dmn_str` (`crates/rex-driver/src/lib.rs`). This page is
the language reference; when prose and either authority disagree, they win.

---

## File layout and imports

A `.dmn` file is `import` declarations followed by exactly one
`decision_model`. The fence below compiles from this document (the docs
fences are compiled by CI); it needs no imports to be valid:

```dmn
decision_model Approval {
    namespace nz.example.approval;

    inputs {
        amount: int;
        applicant: string;
    }

    decisions {
        /// The desk-level auto-approval gate.
        decision auto_approve {
            output boolean;
            logic expression "amount <= 5000"
        }
    }
}
```

- **One model per file**: the `decision_model` is the file's single
  top-level declaration; the file name is the artifact's identity.
- **Imports come first**, one per line, with the canonical `;`. An import
  after the `decision_model` is a reported, recovered syntax error, the
  `.evt` rule. `.dmn` files import **`.mox` domains and `.evt` event
  contracts** — never `.ddd` or other `.dmn` files (the architectural
  rule, see [Cross-surface integration](#cross-surface-integration)).
- **Resolution**: each import names a provided `.mox` domain, matched by
  exact path or lexical resolution relative to the `.dmn` file's
  directory (the `.deploy` rule), or a provided `.evt` contract, matched
  by exact path — the import string **as written**, the table-path rule.
  An import no provided entry names is the error
  ``imported file "<path>" was not provided``. With no imports, the model
  compiles against the empty domain union — every type must then be a
  primitive (`minimal.dmn` pins that shape).
- Imported domains lower through the ordinary domain pipeline against the
  union namespace; input, output, and parameter types resolve there.
  Imported contracts join the compile's **events bucket** — the declared
  events the decisions' `emits` entries resolve against (rule 14 below).

---

## Grammar

The normative EBNF for the `.dmn` surface lives exactly once, in the parser
module docs under [*`.dmn` decision-model
sources*](https://docs.rs/rex-syntax/latest/rex_syntax/parser/index.html)
(`crates/rex-syntax/src/parser/mod.rs`) — the single authority for
`dmn_import`, `decision_model_decl`, `model_member`, and every nested
production; this page keeps only the language-level rules.

Language-level rules:

- **Members in canonical order**: the `decision_model` body reads metadata
  (`namespace`, `id`, `version`, `effective`, `jurisdiction` — each at most
  once, each a singleton regardless of source order) then `inputs`, then
  `knowledge` (sources before models), then `decisions`, then
  `requirements` (information, knowledge, authority groups in that order),
  then `services`, then `views`. Members may be written in any source
  order; the AST groups them per kind and the formatter canonicalizes.
  A decision body reads `output`/`outputs`, then `logic`, then `emits`
  — the same any-source-order, canonical-emission discipline.
- **Statement lines end in `;`** — metadata entries, typed entries
  (`name: Type;`), requirement lines, `description` and `direction`
  clauses, and invoke bindings. **`logic` clauses carry no `;`**: a table
  or expression clause ends at its closing string, an invocation at its
  closing brace, and the formatter emits exactly that whether or not the
  source had one.
- **Names are unique across all sections**: inputs, knowledge sources,
  knowledge models, decisions, services, and views share one namespace
  per model (rule 2 below), so every reference stays unambiguous.
- **Types** are the `.dmn` primitive set — `string`, `int`, `boolean`,
  `date`, spelled lowercase — or an imported domain type by its **bare
  class/enum name** (rule 4 below).
- **Doc comments**: a `///` line directly above a `decision`, `model`,
  `source`, `service`, or `view` header becomes that element's
  description, carried in the artifact. Plain `//` comments are preserved
  verbatim by `rexlang fmt`.
- **Name lists** (`inputs`/`outputs`/`encapsulates` of a service, `show` of
  a view) are brace lists of bare names — inline when they fit comfortably,
  one name per line past the column budget.

---

## Inputs

```
inputs {
    age: int;
    other_super_fund: SuperFundType;
}
```

The graph's **input data**: named, typed values decisions consume. Names
are snake_case elements of the model's single namespace; types resolve per
rule 4 and lower to the domain IR's `TypeRef` (qualified) in the artifact.
Inputs are the only nodes nothing is required of: a declared input no
decision requires is legal (a DRG may carry data beyond one evaluation's
reach), while the reverse — a requirement naming an undeclared input — is
rule 5's error.

---

## Knowledge: sources and business knowledge models

The `knowledge` block holds the two knowledge kinds, sources first.

**Knowledge sources** are the authorities decisions answer to:

```
source KiwiSaverAct {
    name "KiwiSaver Act 2006";
    reference "https://www.legislation.govt.nz/act/kiwisaver2006";
}
```

`name` and `reference` are verbatim labels carried into the artifact, never
interpreted — no fetch, no format check (the `.evt` governance-label
discipline). Sources enter the graph only through **authority
requirements** (`governed_by`); an undeclared source is rule 7's error.

**Business knowledge models** are the reusable functions decisions invoke:

```
/// Employer superannuation contribution tax: the tax on the
/// employer's contribution at the given rate.
model calculate_esct {
    parameters {
        employer_contribution: int;
        rate: int;
    }
    output int;
    logic expression "employer_contribution * rate / 100"
}
```

A BKM is `parameters` (typed, like inputs), one `output` type, and one
`logic` clause — the same three logic kinds a decision takes. It has no
requirements of its own and never appears in a decision's outputs: it is
*applied* through a decision's `logic invoke` (below), which is the only
edge into it (rule 11; an `invokes` naming no declared model is rule 6's
error).

---

## Decisions and output contracts

```
/// The pay-run contribution picture.
decision calculate_contributions {
    outputs {
        employee_deduction_required: EmployeeDeductionRequired;
        employer_cec_amount: int;
        esct_amount: int;
        compliance_status: ComplianceStatus;
    }
    logic table "./tables/contributions.md"
}
```

A decision declares an **output contract** and (optionally) logic and
outcome events:

- **`output <Type>;`** — the single-output form. The output has no name of
  its own; consumers read the decision's value under the decision's name,
  and an external decision table labels its one output column freely.
- **`outputs { name: Type; ... }`** — the multi-output form. Names are
  FEEL names: an external decision table's output columns must match them
  character-for-character, and an `invoke` binding may name them (below).
- **`emits { <entry>; ... }`** — the outcome events the decision
  publishes (the [DMN → EVT edge](#dmn--evt-the-emits-block), below).
- **No output at all is a parse error** — a decision that produces nothing
  is not a decision; **no `logic` is deliberate**: a logic-less decision
  declares a node whose implementation is deferred, the architecture-first
  contract `minimal.dmn` pins (`decision draft { output boolean; }`).

The `logic` clause is one of the three kinds below; a decision may carry
it at most once.

---

## Logic kinds

### `logic table "<path>"`

The decision's rules live in an **external markdown decision table**; the
path resolves relative to the `.dmn` file's directory. The `.dmn` declares
the contract (which inputs the table may test, which outputs it must
produce), the markdown file implements the rules — the split keeps logic
reviewable in isolation and lets the same table serve documentation and
execution. Compile-time checks are rule 13 (provided, parseable, signature
exact); the format itself is the next section.

### `logic expression "<FEEL>"`

The decision's value is the FEEL expression text, stored **verbatim**
(interior spacing is never normalized, so `rexlang fmt` round-trips it
byte-identically). The driver syntax-checks the text (rule 12) and stops
there — no deep typing at compile time (see Deviations). Conditional
(`if`) expressions are **not** gated: the rev-pinned dsntk parser
mis-rejects several valid conditional forms, and the gate never
false-positives on legitimate FEEL. The intended discipline is that an
expression reads the decision's **own** information requirements — its
required inputs, or required decisions by name (whose named outputs may be
read individually) — but name-level checking of expression text is a
documented deferral, like the conditional gate.

### `logic invoke <model> { <param>: "<FEEL>"; ... }`

The decision applies a business knowledge model; each binding maps one of
the model's `parameters` to a FEEL expression string. Bindings must cover
every parameter and bind no unknown names (rule 11) — a binding name
resolves to a declared input, a required decision, or a named output of a
required decision, exactly the information-requirement universe. Invoking
an undeclared model is rule 6's error.

```
decision esct_calculation {
    output int;
    logic invoke calculate_esct {
        employer_contribution: "employer_cec_amount";
        rate: "esct_rate";
    }
}
```

---

## DMN → EVT: the `emits` block

A decision may map its outcome to **declared events** — the `.evt`
contracts (`docs/EVENTS.md`). Consumers name events, never their
payloads:

```
decision calculate_contributions {
    outputs { ... }
    logic table "./tables/contributions.md"
    emits {
        ContributionRecorded when "compliance_status = \"compliant\"";
        VoluntaryContribution;
    }
}
```

An entry is `EventName;` or `EventName when "<FEEL>";` — the `when`
condition is optional, its FEEL text a quoted string, and every entry
carries the canonical `;`. Names are **bare** (single-segment) and
resolve **union-wide** against the declared events of every `.evt`
contract the file imports (rule 14): the model names what its outcome
publishes, the contract owns the payload — no field wiring crosses the
edge, exactly the `.ddd` service-operation `emits` shape.

The FEEL gate applies to a `when` text like any `logic expression` (rule
12, the same conditional deferral). The **loosely-coupled fallback**: a
`.dmn` whose host provides no event contracts — an empty events bucket
and no `.evt` imports — lowers its `emits` entries **as authored,
unchecked**; the names become the record a consumer fills in by providing
the contracts. A `.dmn` that declares an `import "<path>.evt"` line is
held to it: the import must resolve (rule 1) and the names must resolve
against the provided contracts (rule 14).

---

## Requirements: the three groups

The `requirements` block wires the graph. Each group holds `Name <connector>
Name;` lines; the three connectors are the three requirement kinds of the
DMN specification:

- **`information`** — `decision requires X;`: the decision needs X's data
  or X's result before it can evaluate. The left side must be a decision;
  the right side resolves to a declared **input** or **decision** (rule 5).
  Information requirements are what make the graph a graph — and what the
  acyclicity check (rule 8) runs over. They also fix a table logic's input
  signature (rule 13): a `logic table` decision's information-required
  inputs are exactly the table's input columns.
- **`knowledge`** — `decision invokes model;`: the decision applies a
  business knowledge model. The right side must be a declared `model`
  (rule 6), and the decision's `logic invoke` must exist and bind (rule
  11) — the requirement states the dependency, the logic states the call.
- **`authority`** — `decision governed_by source;`: the knowledge source
  governs the decision. The right side must be a declared `source`
  (rule 7).

Groups may repeat or interleave in source order; the formatter
canonicalizes to information, knowledge, authority.

---

## Decision services

```
/// The payroll-facing contract: assess one employee's contributions
/// for a pay run.
service AssessContributions {
    inputs { ... }
    outputs { calculate_contributions esct_calculation }
    encapsulates { ... }
}
```

A **decision service** is the invocation contract a caller binds to — the
DDD-facing seam (below): `inputs` names the model's inputs the caller
supplies, `outputs` names the decisions whose results the caller receives,
and `encapsulates` names everything inside the service boundary (decisions,
models, sources). Every name in all three lists must resolve (rule 9);
services are PascalCase by convention, elements snake_case. A service is a
naming and invocation boundary only — no logic, no requirements of its own.

---

## Views

```
view overview {
    description "The KiwiSaver decision requirements graph: eligibility, contributions, and ESCT.";
    show { ... }
    direction left_to_right;
}
```

A **view** declares presentation intent for the graph: `description` is
prose, `show` names the elements the view displays (decisions, models,
sources — any declared element, rule 10), `direction` is an opaque
layout label (`left_to_right` / `top_to_bottom` are the conventions;
the word is carried verbatim). Views are **not** a DRD/DI diagram
format — no coordinates, no edges beyond the requirements the graph
already declares (see Deviations).

---

## Artifact wire format

`DmnModel` is a standalone, versioned wire artifact following the rex-ir
[wire format contract](BACKENDS.md) (see also the `rex_ir::dmn` module
docs):

- Versioned via **`DMN_MODEL_FORMAT_VERSION`** (= 1), the `ActorModel`
  precedent. `DmnModel::from_json` rejects absent/wrong versions with
  `IrError::UnsupportedFormatVersion` (an absent version reports as 0).
- Struct fields serialize as `camelCase`; type references reuse the domain
  IR's `TypeRef` under **adjacent tagging**
  (`{"type": "<variant>", "value": ...}`), no parallel wire-level type
  shape — the `EventModel` precedent.
- Names and reference strings are kept **as authored**: requirement lines,
  invoke bindings, `emits` entries, service/view lists, table paths, and
  source labels are plain strings; resolving them is a driver concern,
  never a wire concern.
- A logic-less decision serializes with no `logic` key at all, and every
  collection field is `#[serde(default)]` and omitted when empty — a
  model reduced to metadata serializes as exactly `{"formatVersion":1}`,
  byte-identical no matter what is added later. Unknown fields are ignored
  on deserialize.

Excerpt of the committed golden artifact
(`tests/conformance/dmn/minimal.dmn.json`) — the metadata, one input, and
the logic-less decision (key order follows the emitter's canonical order):

```json
{
  "formatVersion": 1,
  "name": "Minimal",
  "namespace": "nz.example.minimal",
  "inputs": [
    { "name": "flag", "type": { "type": "primitive", "value": "boolean" } }
  ],
  "decisions": [
    { "name": "draft",
      "outputs": [ { "type": { "type": "primitive", "value": "boolean" } } ] }
  ]
}
```

---

## External decision tables

A `logic table "<path>"` points at a markdown file in the **dsntk decision
table shape**: optional `>` header lines, then exactly **one** table. The
first header cell is the **hit policy** (`U`, `A`, `P`, `F`, `C`, `C+`,
`C#`, `C<`, `C>` — carried verbatim; semantics are the table engine's
concern, not the driver's); the second row marks each column's clause —
`` `i` `` input, `` `o` `` output, `` `ann` `` annotation. Cell values are
FEEL unary tests: strings quoted (`"Compliant"`), ranges (`<10`, `>=10`,
`[1..5]`), `-` the wildcard. **Annotation columns are supporting prose,
never outputs** — the classic ingest defect this contract exists to
prevent is an annotation column typed as a result.

```markdown
> # Contribution eligibility
> Whether the employee's deduction is required (KiwiSaver Act 2006, s 5A).

| U | age  | member | Eligibility |
|:-:|:----:|:------:|:-----------:|
|   | `i`  |  `i`   |     `o`     |
| 1 | <18  |   -    |    "No"     |
| 2 | >=18 | false  |    "No"     |
| 3 | >=18 | true   |    "Yes"    |
```

The division of labor is the contract's point: the **`.dmn` declares the
signature**, the markdown implements the rules, and the compile-time check
(rule 13) holds the two together — the table's input columns must be
exactly the decision's information-required inputs and its output columns
exactly the declared outputs, each name matching character-for-character
(column names are FEEL names). A single-output decision names no output,
so its one output column's header is a free label. Tables are plain data
files: `rexlang fmt` formats the `.dmn`, never the markdown.

---

## Resolution & validation

Any error-severity diagnostic anywhere (the `.dmn` file, an imported
domain, a referenced table) drops the artifact; warnings do not. The
authoritative, numbered contract is the *Validation rules* section of
`compile_dmn_str` (`crates/rex-driver/src/lib.rs`); the same fourteen in
brief:

1. **Imports** — every import names a provided `.mox` domain (matched by
   exact path or relative resolution) or a provided `.evt` contract
   (matched by exact path); missing names are the error
   ``imported file "<path>" was not provided``.
2. **Uniqueness** — element names are unique across all sections (inputs,
   knowledge sources, knowledge models, decisions, services, views): one
   namespace per model, so every reference kind stays unambiguous (E0204).
3. **Metadata singletons** — `namespace`, `id`, `version`, `effective`,
   and `jurisdiction` each appear at most once (E0214); all five are
   optional, and none is interpreted beyond the string carried into the
   artifact.
4. **Typing** — every input, output, and parameter type resolves to a
   `.dmn` primitive or an imported-domain type by bare name; unknown or
   ambiguous names error (E0200).
5. **Information requirements** — the requiring side is a decision, and
   every required name resolves to a declared input or decision (E0201).
6. **Knowledge requirements** — every `invokes` target is a declared
   knowledge model (E0202).
7. **Authority requirements** — every `governed_by` target is a declared
   knowledge source (E0203).
8. **Acyclicity** — the decision information graph is acyclic: a decision
   never requires itself, directly or through the chain (E0205).
9. **Decision services** — every `inputs`, `outputs`, and `encapsulates`
   name resolves against the declared elements (E0206).
10. **Views** — every `show` element resolves against the declared
    elements (E0207).
11. **Invocations** — a `logic invoke` names a declared model (else
    E0202), covers all of its parameters, and binds no unknown names
    (E0213): a binding name resolves to a declared input, a required
    decision, or a named output of a required decision.
12. **Expressions** — `logic expression` texts, invoke binding
    expressions, and `emits` `when` texts are syntactically valid FEEL
    (E0208); stored verbatim even when checking fails, the `.cmmn` `when`
    precedent. Conditional (`if`) forms are not gated: the rev-pinned
    dsntk parser mis-rejects several valid conditional forms, and the gate
    never false-positives on legitimate FEEL.
13. **Decision tables** — every referenced markdown table is provided
    (E0209), parses in the dsntk shape (E0210), and its clause signature
    matches the decision's declared contract exactly: input columns equal
    the decision's information-required inputs (E0211), output columns
    equal its declared outputs (E0212), names character-for-character.
14. **Emitted events** — every `emits` entry of every decision names a
    declared event of the provided `.evt` contracts, pooled union-wide
    across all of them (E0215, with a did-you-mean help line naming the
    declared events). An **empty** events bucket with no `.evt` imports is
    the blessed loosely-coupled behavior: the entries lower as authored,
    unchecked.

### Diagnostic codes

Every `.dmn` diagnostic carries a stable code in the **`dmn.check` band**:
`E0200`-`E0215` are the surface codes (typing through table signature and
emitted events, rule by rule as above), and `E0250`/`E0251` are the
**`.ddd` decision-link codes** fired on the design side of the
integration below. Run `rexlang explain E0205` for one entry,
`rexlang explain --list` for the ledger; the registry lives in
[docs/DIAGNOSTICS.md](DIAGNOSTICS.md).

---

## Cross-surface integration

### `.mox` — the type universe

DMN depends on **`.mox` domains and `.evt` event contracts** (never a
design or policy artifact): inputs, outputs, and parameters resolve
against the union namespace of the imported domains (rule 4), lowered
through the ordinary domain pipeline. This is the architectural rule of
the surface: a decision model consumes domain types and declared events,
never a design or policy artifact.

### `.evt` — the `emits` edge

The arrow points from decisions to contracts. A decision's `emits` block
names the declared events its outcome publishes (the [DMN → EVT
section](#dmn--evt-the-emits-block) above); the `.evt` contracts own the
payloads. The compile joins the two when the host provides the compiled
contracts (rule 14, E0215) and stays loosely coupled otherwise — an empty
events bucket lowers the names as authored. The edge is deliberately
one-way at the contract level too: an `.evt` file never references a
`.dmn` model, so decisions can start emitting without touching the
contracts, and contracts can evolve without touching the decisions.

### `.ddd` — the decision link

The arrow points one way. A `.ddd` service operation may **link to a
decision service**: the operation declares which decision service it
applies (`invokes` on the operation, `inject decision` in the module
design — see `docs/DDD.md` for the design-side contract), and the two
decision-link codes **E0250/E0251** diagnose a link that names no declared
decision service or disagrees with the operation's signature. The `.dmn`
side of the link is inert by construction: a decision model cannot import
a design, so a design can always be added, replaced, or removed without
touching the decisions it calls.

---

## CLI usage

```
rexlang check kiwisaver.dmn                  # validate + imported domains + tables
rexlang ir kiwisaver.dmn -o kiwisaver.dmn.json  # emit the DmnModel artifact
rexlang fmt --check kiwisaver.dmn            # canonical formatting gate
```

Try it on the canonical fixtures from the repository root:

```
cargo run -p rex-cli -- check tests/conformance/dmn/kiwisaver.dmn
cargo run -p rex-cli -- ir tests/conformance/dmn/minimal.dmn -o minimal.dmn.json
```

- Import paths and table paths resolve **relative to the `.dmn` file's
  directory**; the CLI reads each imported `.mox` from disk, compiles each
  imported `.evt` through the ordinary event pipeline (the contract's own
  imports resolve next to it), and the referenced markdown tables with
  them. A missing import file or table is a clean error naming the
  resolved path.
- Diagnostics come from every file involved (the `.dmn` file, its imported
  domains, its tables) and render grouped per file, the way `.evt` compiles
  render them.
- Directory batch mode (`rexlang check <dir>`, `rexlang build <dir>`) picks
  `.dmn` files up like every other surface: each model compiles
  independently, imports resolved relative to it, and `build` emits one
  `<input path>.json` artifact per file under the out directory.
- `rexlang fmt` formats `.dmn` files with the canonical layout: imports
  hoisted into a tight first section, then the `decision_model` with
  members in the canonical order above, canonical `;` on every statement
  line, `logic` clauses without one, name lists inline when they fit
  comfortably and one name per line past the column budget, and exactly
  one blank line between sibling blocks and declarations. `kiwisaver.dmn`
  is the layout reference. Stdin is treated as `.mox`.

---

## Deviations from OMG DMN 1.6

The surface follows the DMN decision architecture (DRG, the three
requirement kinds, hit policies) and deviates deliberately where rexlang's
model demands it:

1. **A text surface, not XML** — one `decision_model` per file, no DMN XML
   schema, no model interchange between files.
2. **FEEL is stored, not executed** — expression texts are strings,
   syntax-checked at compile time and never deeply typed; the type
   discipline rexlang applies to its own expression surfaces (`when`
   guards, policies) is not imposed on FEEL.
3. **Decision tables are external markdown** — the dsntk shape, one table
   per file, not boxed expressions inside the model; the `.dmn` carries
   the signature, the markdown the rules.
4. **Types come from `.mox`, not item definitions** — no FEEL
   `itemDefinition` machinery; the domain model is the type universe.
5. **No DRD/DI** — no diagram interchange; `views` declare presentation
   intent (what to show, which direction), not graphics.
6. **No evaluation engine in-tree** — the artifact carries the
   architecture; execution is a backend/consumer concern (the table
   engine, the FEEL evaluator, are downstream choices — dsntk
   compatibility is the table format's north star, not a dependency).
7. **`emits` names events, never payloads** — a decision maps its outcome
   to declared `.evt` events by bare name (the DMN → EVT edge); no
   payload wiring, no event-versioning or delivery contract crosses the
   edge, and a host that provides no contracts gets the loosely-coupled
   record (the names lower as authored, unchecked). The `.ddd` operation
   `emits` clause carries the same discipline.

---

## Conformance

`tests/conformance/dmn/` holds the canonical fixture set: `kiwisaver.mox`
(the NZ-payroll domain), `kiwisaver.dmn` (the flagship model — all five
metadata entries, nine inputs mixing primitives and domain types, two
sources, one invoked BKM, three decisions covering all three logic kinds,
an `emits` block on the table decision, all three requirement groups, a
decision service, and a view; it is the formatter's layout reference),
`kiwisaver.evt` (the contract declaring the emitted `ContributionRecorded`
and `VoluntaryContribution` events over the same domain), the two
referenced tables `tables/eligibility.md` and `tables/contributions.md`
(the latter exercising the four-output signature and the annotation
column that is not an output), and `minimal.dmn` (the architecture-first
minimum: no imports, one input, one logic-less decision).

The `examples/` side ships `support.dmn` (the support-desk refund
decisions over `support.mox`: a domain-typed `Ticket`/`User` input pair,
an expression decision, a table decision with its
`examples/tables/refund-authority.md`, an invoked BKM composing the two,
the `AssessRefund` service the design links to, and an `emits` block on
`RefundAssessment` naming the `RefundApproved` event declared in
`examples/support.evt`) and `lending.dmn`
(the architecture-first lending model with no imports: five decisions,
two BKMs, three sources, two views, and `examples/tables/lvr.md`).

Regenerate the goldens with
`REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test dmn_golden` (the
fixture artifacts `kiwisaver.dmn.json` and `minimal.dmn.json`) and
`REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test examples_artifacts`
(the example artifacts `examples/artifacts/support.dmn.json` and
`examples/artifacts/lending.dmn.json`).
