# Use case patterns (.uc)

## Overview

A standalone use-case modeling surface implementing the SysML v2 use case
patterns in the rexlang spirit: a use case declares a **subject** (the
classifier whose behavior is specified), the **actors** participating, the
**messages** exchanged between anchored points of their behavior, the
**successions** ordering that behavior, and the domain **service operations**
the use case realizes. Parsed by **rex-syntax** (`parse_usecases`, the
chumsky parser) at compile time, lowered and validated by **rex-driver**
(`compile_usecase_str`) against its imported domains and the pre-compiled
design, actor, and event artifacts its imports resolve to, and emitted as
the versioned `rex_ir::usecases::UseCaseModel` artifact (the rex-ir crate).

One abstraction note up front, because the name invites the wrong reading:
an `occurrence` is a **scenario-local lifetime point** of one participant —
a moment its behavior is punctuated — and is deliberately **not** an
`.evt` event. `.evt` contracts enter a use case only through **payload
binding** (`message m of SomeEvent` → `PayloadRef::Event`); no occurrence
ever declares, publishes, or subscribes to anything.

**No in-tree backend exists yet**: downstream consumers ingest the artifact
exactly like the `DddModel`, `EventModel`, and `IfmlModel` artifacts.

The normative grammar lives exactly once — the *`.uc` use case sources*
section of the parser module docs (`crates/rex-syntax/src/parser.rs`); the
normative compile contract — entry points, resolution semantics, and
validation rules — is documented on `rex_driver::compile_usecase_str`
(`crates/rex-driver/src/lib.rs`). This page is the language reference; when
prose and either authority disagree, they win.

---

## File layout and imports

A `.uc` file is `import` declarations followed by `usecase` declarations:

```
import "delivery.mox"
import "handover.ddd" as handover
import "handover.actor"
import "handover.evt"

usecase HandOverParcel1 {
    subject Drone: DeliveryDrone
    actor Courier
}
```

- **Imports come first**: an `import` after the first `usecase` is a
  reported, recovered syntax error (the misplaced import is dropped),
  mirroring the `.actor`/`.evt` rule. Duplicate imports are a driver
  concern, not a syntax error.
- **Four buckets**: an import resolves against raw `.mox` domain texts,
  pre-compiled `.ddd` designs, `.actor` policies, or `.evt` contracts —
  whichever the host provides, matched by exact path or lexical resolution
  relative to the `.uc` file's directory (the `.deploy` rule). An import no
  provided entry names is the error ``imported file "<path>" was not
  provided``.
- **Aliases are design-only**: `import "<path>" as <name>` exists so
  `realizes` bindings can name a design (`handover.HandoverService
  .releaseParcel`); an `as` clause on any other surface is an error. A
  design import needs its alias only when a `realizes` binding references
  it.
- Imported `.mox` domains lower through the ordinary domain pipeline
  against the **union namespace of all imports** (the same rule multi-file
  `.mox` compiles follow); participant and payload types resolve there.

---

## Use case anatomy

`usecase <name> { ... }` — one use case pattern. A `///` doc comment
directly above the declaration becomes its **description** (the objective
of the pattern). Members may be written in any order; the AST folds them
into per-kind lists and the formatter canonicalizes to imports, then
`usecase` declarations whose bodies read: the `subject` line, the `actor`
lines, messages, successions, `realizes` bindings — source order kept
within each group. Statement lines (`occurrence`, `perform`, `message`,
`first ... then ...`, `realizes`) end in `;`; participant lines and the
`usecase` block itself do not, and the formatter emits the canonical `;`
whether or not the source had one.

Names: use case names are unique per file. Within one use case,
**participants, steps, and messages share one namespace** (so every
reference kind stays unambiguous); occurrence names are unique per
participant.

---

## Participants

Two forms:

- **`subject <name>: <type>`** — the use case's initiating classifier, the
  class whose behavior is specified. Exactly one; a second `subject` line
  is a reported syntax error (the first wins). The type resolves in the
  domain union namespace (any declared kind — class, enum, datatype,
  interface, vocabulary) and is stored **resolved** (qualified) in the
  artifact. Only the subject performs steps.
- **`actor <name>`** or **`actor <name>: <type>`** — a participating
  actor. The bare form is a **policy principal**: its name must match an
  actor declared in a provided `.actor` model (every declared actor name
  matches — `extends` chain members are declared actors themselves); with
  no `.actor` models provided, always an error. The typed form is a domain
  classifier instance (iteration 2a's `actor Recipient: Recipient`) and
  contributes nothing to policy coverage (below).

A bare `actor Courier` is how the `.actor` policy dimension enters the
scenario: the artifact stores such a participant with no type name, and
`requires` coverage and `never_both` checks run against exactly these
bare participants.

---

## Occurrences

Declared inside a participant's body — occurrences are allowed on any
participant, subject or actor:

- **Named** (the pattern article's 2a style): `occurrence recipientVerified;`
  — a scenario-local lifetime point the participant witnesses.
- **Message-owned** (2b): `occurrence releaseParcel.source;` /
  `occurrence releaseParcel.target;` — this participant owns one end of the
  declared message `releaseParcel`. The message must be declared in the
  same use case.

Both are lifetime points and nothing more: no payload, no channel, no
subscription — `.evt` files are used for payload binding only (see
Messages). Occurrence names are unique per participant; a message name
occurring as `m.source` on two participants binds that end twice (an
error, see Messages).

---

## Messages and endpoint anchoring

```
message promptHandover of HandoverPrompt from Drone.recipientVerified
    to Recipient.promptAcknowledged;
```

`message <name> (of <type>)? (from <endpoint>)? (to <endpoint>)?;` — one
named communication. An **endpoint** is `participant.anchor`, where the
anchor names an occurrence or a step declared in that participant's body
(occurrences win when both exist, and the artifact records which kind:
`Anchor::Occurrence` or `Anchor::Step`).

The semantic rule the whole surface hangs on: **a message is sent at the
end of its source anchor and received at the start of its target anchor**
— the sending step/occurrence completes before the message leaves; the
receiving step/occurrence begins after it arrives.

- **One binding per end**: for each message and each side, the inline
  endpoint **xor** exactly one participant declaring `occurrence
  m.<side>` — zero bindings is the `unbound <side> end on message` error,
  more than one is `bound more than once`.
- **Bare participant ends are compile errors** — the lifetime-event
  semantics of the pattern article's own warning. The grammar has no
  anchor-less participant reference (`from Courier` does not parse; an
  endpoint is always `participant.anchor`), and an end with no inline
  endpoint and no message-owned occurrence anywhere is the unbound-end
  error: a message end is itself a temporal point, so it must attach to
  one.
- **Payloads** (`of X`): `X` resolves in the domain union first (declared
  kinds; primitives are not payload types) → `PayloadRef::Domain` carrying
  the resolved `TypeRef`; otherwise by **event name** across the provided
  `.evt` contracts → `PayloadRef::Event`. Found in several contracts is an
  ambiguity error; found nowhere is `unknown type 'X'`.

---

## Performed steps and `requires`

```
perform verify: verifyRecipient;
perform release: releaseParcel requires [ ReleaseParcel ];
```

`perform <name> (: <operation>)? (requires [<capability>*])?;` — a step
the **subject** performs (a step on an actor is the
`only the subject performs steps` error; the bare `perform x` form names
the operation after the step itself). Each step's operation must name an
`Operation` on the subject's resolved class, walking `extends` (cycles are
guarded; an `extends` target naming no class of the union errors;
interface targets end the walk — they carry no operations).

The sigil limitation, precisely: sigil v1 skips Rune `Function`
declarations, so sigil-lowered classes carry attributes only — they have
no operations, and therefore a **sigil-typed subject cannot take steps**
(operations never resolve through them) until Function lowering lands.

`requires [...]` names capabilities declared in an imported `.actor`
file; every name must exist across the provided actor models and be
covered by a participating actor (the structural contract below). Inside
the list the contextual `occurrence`/`perform` words end it, so a missing
`]` never swallows the next member line.

---

## Successions

```
first start then requestRelease.source;
first requestRelease.source then requestRelease;
first requestRelease then verify;
```

`first <node> then <node>;` — an ordering edge: `first` completes before
`then` begins. `start` and `done` are the use case's implicit initial and
final nodes; everything else is one of:

- **bare `x`** — a step name, else a message name (**normalizing to its
  target end**: `first requestRelease then verify` means the message's
  receive completes before `verify` begins — the shorthand of the
  message-then-succession idiom), else an occurrence name (must be unique
  across participants; ambiguity is an error telling you to qualify).
- **`m.source` / `m.target`** — one end of declared message `m`.
- **`p.x`** — an occurrence or step `x` of participant `p`.

Graph checks run only when a use case declares at least one succession:
the graph must be acyclic, **every step and message end must be reachable
from `start`**, and `done` must be reachable (unreachable nodes are
listed, first few first). A use case with no successions makes no
ordering promises and skips the checks — iteration 1's subject-and-actors
pattern compiles without any.

---

## `realizes`

```
realizes handover.HandoverService.releaseParcel;
```

Names a domain service operation that realizes the use case:
`alias.Service.operation`. The alias must name an aliased `.ddd` design
import; the service must exist in that design (unique by name across its
modules), and the operation must exist on that service — each failure
names what missed. The binding is kept as authored
(`handover.HandoverService.releaseParcel`) in the artifact's `realizes`
list, driver-resolved and validated but not rewritten.

---

## Artifact wire format

`UseCaseModel` is a standalone, versioned wire artifact following the
rex-ir [wire format contract](BACKENDS.md) (see also the
`rex_ir::usecases` module docs):

- Versioned via **`USE_CASE_MODEL_FORMAT_VERSION`** (= 1), the
  `ActorModel` precedent. `UseCaseModel::from_json` rejects absent/wrong
  versions with `IrError::UnsupportedFormatVersion` (an absent version
  reports as 0).
- Struct fields serialize as `camelCase`; `Occurrence`, `PayloadRef`,
  `Anchor`, and `NodeRef` are **adjacently tagged** like `TypeRef`
  (`{"type": "<variant>", "value": ...}`; unit variants carry no
  `"value"` — `{"type": "start"}`).
- A domain-typed payload reuses the domain IR's `TypeRef` under the
  adjacent tags — no parallel wire-level type shape. Event payloads are
  bare event name strings.
- Names, successions, and `realizes` bindings are kept **as authored**;
  only participant types are stored resolved.
- Empty = minimal: every collection field is `#[serde(default)]` and
  omitted when empty — an empty artifact serializes as exactly
  `{"formatVersion":1}`, byte-identical no matter what is added later.
  Unknown fields are ignored on deserialize.

Semantics consumers can rely on (from the `rex_ir::usecases` module docs):
the message timing rule above; an `Anchor::MessageEnd` denotes a
message-owned end *of the containing message*; a participant with no
`typeName` is a bare policy principal, not a domain class instance.

Excerpt of the committed golden artifact
(`tests/conformance/usecases/delivery.uc.json`) — iteration 2b, both ends
message-owned:

```json
{
  "name": "HandOverParcel2b",
  "subject": {
    "name": "Drone",
    "typeName": "nz.example.delivery.DeliveryDrone",
    "occurrences": [
      { "type": "messageEnd", "value": { "message": "releaseParcel", "end": "source" } },
      { "type": "messageEnd", "value": { "message": "releaseParcel", "end": "target" } }
    ]
  },
  "messages": [
    {
      "name": "releaseParcel",
      "payload": { "type": "domain", "value": { "type": "class",
                  "value": { "package": "nz.example.delivery", "name": "ReleaseRequest" } } },
      "source": { "participant": "Drone", "anchor": { "type": "messageEnd", "value": "source" } },
      "target": { "participant": "Drone", "anchor": { "type": "messageEnd", "value": "target" } }
    }
  ]
}
```

---

## Resolution & validation

Any error-severity diagnostic anywhere (the `.uc` file or an imported
domain) drops the artifact; warnings do not. The authoritative, numbered
contract — one rule per concern, with the exact diagnostic wording — is
the *Validation rules* section of `compile_usecase_str`
(`crates/rex-driver/src/lib.rs`); the same thirteen in brief:

1. **Imports** — every import names a provided entry in one of the four
   buckets; aliases are design-only.
2. **Participant typing** — typed participants resolve in the union
   namespace; bare actors must name an actor of a provided `.actor`
   model (always an error with none provided).
3. **Steps** — each operation names an operation on the subject's
   resolved class, walking `extends`.
4. **Placement** — steps are subject-only; occurrences on any
   participant.
5. **Message end binding** — inline endpoint xor exactly one
   `occurrence m.<side>` declaration per end; inline endpoints must name
   a declared participant and one of its occurrences or steps.
6. **Payloads** — domain union first, then event contracts by name;
   ambiguity and unknown names error.
7. **Endpoint lowering** — `Anchor::Occurrence` / `Anchor::Step` per
   what the anchor matched; a 2b-bound end anchors at
   `Anchor::MessageEnd(side)`.
8. **Successions** — refs resolve within the scenario node set; with at
   least one succession: acyclic, every step and message end reachable
   from `start`, `done` reachable.
9. **Uniqueness** — one per-use-case namespace for participants, steps,
   and messages; occurrences unique per participant; use case names
   unique per file.
10. **`requires` coverage** — every required capability exists across
    the provided actor models and is held by a participating bare actor
    (the structural contract below).
11. **`never_both`** — two required capabilities of one use case must
    not form a pair of any provided `never_both` constraint.
12. **`realizes`** — alias, service, and operation must each exist as
    named.
13. **Order** — diagnostics are deterministic: parse diagnostics first,
    then import errors, then each domain's own diagnostics (import
    order), then per use case: uniqueness, participants, messages,
    successions, `requires` coverage, `never_both`, `realizes`.

Every diagnostic is tagged with its file's path; on any error the
result's `model` is `None` and `domains_model` still carries the imported
domains when they lowered.

---

## Cross-surface integration contracts

### `.actor` — the policy dimension

Bare `actor X` participants are policy principals resolved by name
against the provided `ActorModel`s (rule 2). The `requires` grant-
coverage check (rule 10) is **structural** — use cases carry no
instances, so no `when` condition is ever evaluated:

- Capability `C` is covered when some **bare policy-actor participant**
  holds it: a `grant <p>` permit entry for `C`, **or** a delegation
  **to** `p` with a permit entry for `C` — one hop, structural.
- The delegation's `from` chain is deliberately **not chased
  transitively**: an `extends` lineage that would satisfy the `.actor`
  surface's own rules never satisfies use case coverage.
- `when`-guarded permits **count structurally** (there are no instances
  to evaluate conditions against).
- When no participating actor holds a permit but the capability only
  ever appears as an **unconditional `forbid`** on them — or every
  permit is paired with one — the error is
  `capability ... only forbidden for participating actors`; with no
  connection at all it is `capability ... is not held by any
  participating actor`.
- **Domain-typed participants contribute nothing** — no class↔
  policy-actor identity exists, so `actor Recipient: Recipient` never
  covers a capability.
- **`never_both`** (rule 11): if two capabilities one use case requires
  form a pair of any provided `never_both` constraint, the use case is
  refused — the scenario would require what the policy forbids
  combining.

The conformance fixture exercises the delegation hop: `HandOverParcel3`'s
`release` step requires `ReleaseParcel`, held by `Courier` directly and
delegated to `RecipientAgent` — either participant would cover it.

### `.ddd` — `realizes`

`realizes alias.Service.operation` cross-checks against the aliased
design import's pre-compiled `DddModel` (rule 12); the design pipeline
itself is unchanged (`docs/DDD.md`).

### `.evt` — payload binding only

An imported `.evt` contract contributes exactly one thing: **event names
for message payloads** (`of PromptIssued` → `PayloadRef::Event`). Use
case occurrences are scenario-local lifetime points, not events — no
occurrence, message, or use case ever appears in an `EventModel`, and no
channel or subscription semantics exist on this surface.

### Sigil (`.rosetta`)

Sigil-lowered classes participate in the union namespace like any other
domain type (typed subjects and payload types resolve through them), but
sigil v1 skips Rune `Function` declarations — sigil classes carry no
operations, so sigil-typed subjects cannot take steps (rule 3).

---

## CLI usage

```
rexlang check delivery.uc                  # validate + imported domains/artifacts
rexlang ir delivery.uc -o delivery.uc.json # emit the UseCaseModel artifact
rexlang fmt --check delivery.uc            # canonical formatting gate
```

Try it on the canonical fixture from the repository root:

```
cargo run -p rex-cli -- check tests/conformance/usecases/delivery.uc
cargo run -p rex-cli -- ir tests/conformance/usecases/delivery.uc -o delivery.uc.json
```

- Import paths resolve **relative to the `.uc` file's directory**; each
  imported `.mox` is read from disk and compiled through the ordinary
  domain pipeline, and imported `.ddd`/`.actor`/`.evt` files are compiled
  to their artifacts first — the `.uc` compile consumes those artifacts,
  the `.deploy` pattern. A missing import file is a clean error naming
  the resolved path.
- Diagnostics come from every file involved (the `.uc` file, its imported
  domains) and render grouped per file, the way `.evt` compiles render
  them.
- `rexlang fmt` formats `.uc` files with the canonical layout: imports
  hoisted into a tight first section, then `usecase` declarations with
  exactly one blank line between them, bodies regrouped subject → actors
  → messages → successions → realizes, canonical `;` on every statement
  line, and a message's `from`/`to` clauses broken onto continuation
  lines only past the column budget. Stdin is treated as `.mox`.

---

## Deviations from SysML v2

The surface follows the MBSE4U use-case-pattern progression (subject and
actors → anchored messages → message-owned ends → performed steps with
successions) and deviates deliberately where rexlang's model demands it:

1. **Occurrences are scenario-local lifetime points** — a named moment a
   participant witnesses, or a message end it owns. None of SysML's
   general occurrence machinery (no classification, no timing beyond
   successions), and deliberately not `.evt` events.
2. **`start`/`done` are implicit** — every use case has the two nodes
   without declaring them; no explicit initial/final parts.
3. **The objective is the doc comment** — no separate objective part.
4. **Steps are subject-only** — actors witness occurrences; they never
   perform.
5. **One flat scenario node set** — succession references resolve by
   name within the use case (with participant qualification for
   occurrences), no part nesting.
6. **`realizes` targets `.ddd` service operations** (`alias.Service
   .operation`), not SysML classification relationships.
7. **Policy checks are structural** — `requires` coverage and
   `never_both` are compile-time checks over `.actor` artifacts;
   `when` conditions are never evaluated.
8. **Sigil limitation** — Rune `Function` declarations are skipped in
   sigil v1, so sigil-typed subjects cannot take steps.
9. **No use case generalization, `include`, or `extend`** — out of scope
   for slice 1.

---

## Conformance

`tests/conformance/usecases/` holds the canonical fixture set:
`delivery.mox` (the domain), `handover.ddd` / `handover.actor` /
`handover.evt` (the imported design, policy, and event contract), and
`delivery.uc` — the worked example whose four iterations (1: subject +
bare policy actor; 2a: named occurrences + fully anchored message; 2b:
message-owned ends; 3: steps, `requires`, event payload, `realizes`)
progress through the pattern article's stages, plus the
actors-only `MonitorHandover`. `delivery.uc` is formatter-canonical and
covered by the CI `fmt --check` gate over `tests/conformance/usecases/`.

The driver golden test (`crates/rex-driver/tests/usecase_golden.rs`)
byte-compares the compiled artifact, round-trips it through `from_json`,
pins the formatter fixpoint of `delivery.uc`, and pins the negative
diagnostics; the wire shape is pinned by
`crates/rex-ir/tests/usecase_model.rs`; the formatter golden
`tests/conformance/fmt/usecases.fmt.uc` is byte-compared by
`crates/rex-syntax/tests/fmt.rs` against the deliberately **non-canonical**
source `tests/conformance/models/usecases.uc` — don't cite that file as a
canonical model or "fix" its spacing (the `.evt`/`.deploy` pattern).
Regenerate goldens with `REX_UPDATE_FIXTURES=1 cargo test --workspace`.
