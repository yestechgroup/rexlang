# DDD design DSL (.ddd)

## Overview

A Sculptor-style domain-driven-design layer over `.mox` domain models. The
domain model defines the data (entities, features, constraints); the `.ddd`
design defines the application architecture on top of it: the application,
its modules, the application services with their use-case operations, and
per-class design decisions (stereotype, flags, repository, state machine,
delegation targets). Parsed by **rex-syntax** (`parse_ddd`, the chumsky parser) at
compile time, lowered and validated by **rex-driver**
(`compile_ddd_str`) against the imported domains, and emitted as the
versioned `rex_ir::ddd::DddModel` artifact (the rex-ir crate).

The `.ddd` DSL lives alongside the `.mox` domain model as a *complementary*
input: a design never declares classes or features — it references the
classes the imported domains declare and records what the application makes
of them. Downstream consumers ingest the artifact exactly like the
`ActorModel` policy artifact; there is no in-tree backend yet.

The normative contract — entry points, resolution semantics, the validation
rules, the aggregate-derivation rule, and the capabilities
contract — is documented on `rex_driver::compile_ddd_str`; this page is the
language reference.

---

## Artifact wire format

`DddModel` is a standalone, versioned wire artifact following the rex-ir
[wire format contract](BACKENDS.md) (see also the `rex_ir::ddd` module
docs):

- Versioned independently of the domain-model marker via
  **`DDD_MODEL_FORMAT_VERSION`** (= 1), the `ActorModel` precedent.
  `DddModel::from_json` rejects absent/wrong versions with
  `IrError::UnsupportedFormatVersion` (an absent version reports as 0).
- All struct field names serialize as `camelCase` (`formatVersion`,
  `returnType`, `optimisticLocking`, ...). Unit-only enums serialize as bare
  strings: stereotypes lowercase (`"entity" | "value" | "dto"`), built-in
  repository operations camelCase (`"findById"`, `"findAll"`,
  `"findByExample"`, `"findByKeys"`, `"save"`, `"delete"`).
- Signatures reuse the domain IR's `TypeRef` (adjacent tagging:
  `{"type": "class", "value": {...}}`) and `OperationParam`
  (`{"name": ..., "type": ...}`) — there are no parallel wire-level
  signature types.
- Unknown fields are ignored on deserialize; every field added after v1
  must keep the `#[serde(default)]` + skip discipline.
- A repository operation carries **exactly one** of a built-in op
  (`"builtin"`) or a declared signature (`returnType`/`params`).

Minimal artifact: an empty model serializes as exactly `{"formatVersion":1}`.

```rust
let compilation = rex_driver::compile_ddd_str("library.ddd", source, &domains, &rex_driver::DomainImports::default());
let json = compilation.model.unwrap().to_json_pretty()?; // versioned artifact
let model = rex_ir::ddd::DddModel::from_json(&json)?;    // version-gated reload
```

---

## Philosophy

1. **Sculptor semantic identity** — every Sculptor design concept has a
   direct DSL equivalent (see the concept-mapping table below). No lossy
   mapping.
2. **Validated against the real domain** — design class references and
   operation signatures type-check against the imported `.mox` models; a
   typo'd class is a compile error, not a silent generation mismatch.
3. **Resolution is validation-only** — the artifact keeps every name
   exactly as authored; consumers resolve against the domain model.
4. **Loosely coupled dimensions** — capability references tie the design
   dimension to the actor-policy dimension by name only, never by
   construction.

---

## Grammar

The normative EBNF for the `.ddd` surface lives exactly once, in the parser
module docs under [*`.ddd` design
sources*](https://docs.rs/rex-syntax/latest/rex_syntax/parser/index.html)
(`crates/rex-syntax/src/parser.rs`) — the single authority for `ddd_file`,
`application_decl`, `module_decl`, `service_decl`, `design_decl`,
`repository_decl`, `machine_decl`, and `search_decl`; this page keeps only
the language-level rules. The `.ddd` keyword tables in
`crates/rex-syntax/src/ddd.rs` are the keyword authority the parser and the
formatter share.

Lexical and structural rules:

- Every `.ddd` word (`application`, `base`, `module`, `service`, `inject`,
  `entity`, `value`, `dto`, `abstract`, the flags, `repository`, and the
  built-in operation names) is a **contextual identifier**, not a keyword:
  it is special only in the grammar position it is parsed in, remains
  usable as a class or operation name elsewhere, and can be escaped with
  `^` (the `id`/`readonly` precedent).
- `import` declarations must precede the `application` declaration; there
  is exactly one application per file, and `base` must be its first member
  when present.
- A `///` doc comment run directly above a `service` or `search` declaration
  becomes its description in the artifact. Contiguous `///` lines join with
  newlines — the same semantic as the `.mox` doc collection (`docs/LANGUAGE.md`).
- Delegated operations (`name => target.operation`) copy the target's
  signature conceptually and declare none of their own; a delegated op may
  be named `save` (contextual keywords). Declared service operations may
  carry a `capability a, b` clause; declared repository operations take
  none (the wire artifact's repository operations carry no capabilities, so
  accepting one would silently drop data).
- A service or repository operation may carry a leading `protected`
  modifier (Sculptor's visibility): the operation stays off the public
  interface. `protected` is contextual — in leading position it is the
  modifier, anywhere else an ordinary name (`^protected` escapes).
- The repository built-ins are `findById`, `findAll`, `findByExample`,
  `findByKeys`, `save`, and `delete` — the consumer knows their signatures.
- Multiplicity annotations on return types and parameters (`String[]`,
  `Book[0,3]`) lower into the artifact's cardinality slots
  (`returnMultiplicity` / parameter `multiplicity`).
- A `search` projection's members may appear in any order (the formatter
  canonicalizes them); repeated `text`/`filters`/`sort`/`document` clauses
  merge, and repeats of the single members (`entity`, `ranking`,
  `analyzer`, `pagination`) are idempotent — the first declaration wins.
- A document entry's expression is captured **raw** (tokens up to the `;`,
  nested brackets and string literals respected) and stored verbatim in the
  artifact; parsing and type-checking it is the driver's job (rule 11). The
  expression language is the Tier-2 neutral expression language with the
  entity's features as `self` — string concatenation via `+` is rule R9 of
  `docs/EXPRESSIONS.md`.

---

## Resolution semantics

Names resolve with the `.mox` cross-package rules against the union
namespace of the imported domains (the same rule multi-file `.mox` compiles
follow). A bare single-segment name must be unique across all domain
packages; when several packages declare it the error is
``ambiguous type `X`; qualify as `p.X``` with the matching packages as
help. A qualified `pkg.Name` must match exactly one package. When the
design declares `base`, that package becomes the preferred resolution
scope: a bare name the base declares resolves there before the
unique-across-packages lookup runs. `base` itself must name an existing
domain package.

Resolution is **validation-only**: the artifact keeps every name exactly as
authored (unqualified where written qualified and vice versa), and
consumers resolve against the domain model.

---

## Validation rules

1. **base** — an unknown base package is an error naming it.
2. **Uniqueness** (application-wide unless noted) — module names; service
   names; design class references (one design per class per application,
   keyed by the resolved class); repository names; operation names within
   one service; operation names within one repository.
3. **Stereotype targets** — every design's `class` must resolve to a class
   in the domain model (entity, value, and dto designs all target classes).
4. **Flag/stereotype compatibility** — `scaffold`, `auditable`, and
   `optimisticLocking` are entity-only; `nonPersistent` is value/dto-only;
   `cache` is legal on any design. `auditable` and `optimisticLocking`
   default **on** for entity designs (Sculptor's polarity): the artifact
   lowers them to `true` unless the design opts out with the negated form
   (`!auditable`, `!optimisticLocking`). Negating a flag that defaults off
   (`!scaffold`, `!nonPersistent`, `!cache`) is rejected wherever written.
5. **Repository placement** — only entity designs may declare a repository.
6. **Aggregate boundary** — see the derivation rule below; an entity design
   derived as a non-root may not declare a repository.
7. **Signature type-check** — declared service and repository operation
   signatures type-check against the domain model: the `rex-syntax`
   primitives resolve everywhere, every other reference resolves through
   the bare/qualified rules above (any declared kind — class, enum,
   datatype, interface, vocabulary). A multiplicity annotation on a return
   type or parameter lowers into the artifact's cardinality slots.
8. **Delegations** — the target must resolve to a service, a repository,
   or a search declared anywhere in the application, and the operation must
   name an operation on it (repository built-ins included; a search exposes
   only the virtual operation `search`). Service → repository and
   service → search delegation across different modules is the
   module-coupling error ("interaction between a Service in one Module and
   a Repository/Search in another Module is not allowed; go via a
   Service"); service → service across modules is allowed.
9. **inject** — every dependency name must resolve to a service, a
   repository, or a search of the application.
10. **Capabilities** — only `compile_ddd_str_with_actors` validates: each
    declared capability name must exist in the union of the actor model's
    blocks' capabilities. Plain compiles record the names unvalidated —
    the design dimension is loosely coupled to the policy dimension by
    design.
11. **Searches** — a search's name is unique application-wide. Its `entity`
    line is required, resolves like a design target, and must bind to an
    entity design **of the same module** (the repository coupling rule;
    non-root entities may be searched — a projection over a contained
    entity is legitimate). Text fields must name a direct feature of the
    entity (inherited features included) that is text-like: the string
    primitives, enums, datatypes — the string-family default the constraint
    rules use — and vocabularies whose key facet is a string primitive;
    boosts are at least 1 and analyzers non-empty. Filters and sorts name
    direct features that are not class references. Document field names are
    unique per search, and every document expression is parsed and
    type-checked with the entity as `self` (any value type is legal; the
    consumer decides the rendered form). Pagination bounds satisfy
    `1 ≤ limit ≤ max`.
12. **Value contract** — a `value` design is an immutable value object
    (Sculptor's ValueObject default, with no opt-out): every stored feature
    of the resolved class (own and inherited, derived features excluded)
    must carry the domain model's `readonly` modifier. A changeable class
    designs as `entity` or `dto` instead.
13. **Aggregate reference constraint** — an outside class may hold a cross
    reference only to an aggregate's root: for every cross-reference
    (`refers`) feature whose target sits under a containment chain (a
    non-root aggregate member), the referring class must share the target's
    containment root. Computed over the whole union like the
    aggregate-derivation rule, and skipped when the verdict would be a
    guess (the target or source sits under two distinct roots, or under a
    containment cycle).
14. **scaffold** — a scaffolded entity design must declare a repository:
    `scaffold` stands for Sculptor's generated CRUD operations (which also
    imply the service), and the artifact stays validation-only — nothing is
    synthesized — so the author declares the repository the scaffolding
    would hang off. The service remains the author's to declare.
15. **dto-with-ops warning** — a `dto` design whose class carries `ops`
    warns once per op, naming the design and the op: a dto is a transfer
    record, and behavior on it is almost always a modeling mistake (see
    the layering heuristic under "Domain operations vs service
    operations"). Warning-severity: the artifact is unaffected, and the
    check runs only against a fully lowered union.

## State machines

An entity design may carry one lifecycle state machine — a
`machine <field>: <Enum>` block, written after the optional repository
block (the formatter's canonical order; one per design). `machine`,
`initial`, `terminal`, and `via` are contextual words like every `.ddd`
word. The normative grammar (`machine_decl`, `machine_member`,
`transition`, `guard_clause`) lives in the parser module docs
(`crates/rex-syntax/src/parser.rs`); the canonical fixture's machine gives
the shape:

```
machine status: PayRunStatus {
    initial Draft;
    Draft -> Calculating via Calculate;
    Calculating -> AwaitingApproval via Submit;
    AwaitingApproval -> Approved via Approve when (total > 0);
    Approved -> Processing via Schedule;
    Processing -> Posted via Post;
    Draft -> Reversed;
    terminal Reversed;
    terminal Posted;
}
```

The driver validates the machine before it lowers (`lower_design_machine`
in `crates/rex-driver/src/ddd.rs`; behavior pinned by
`crates/rex-driver/tests/ddd_machines.rs`):

- Only an **entity** design carries a machine — a value object is
  immutable and a dto is a transfer record; neither moves through states.
- The machine's type must resolve to an **enum**, and the bound feature
  (`<field>`) must exist on the designed class and carry exactly that enum
  type.
- Exactly one `initial` state. Duplicate terminal declarations and
  duplicate identical edges are rejected.
- **Determinism** — one trigger name may leave a state for at most one
  target (the statechart event rule).
- **Terminal states take no outgoing transitions**; every non-terminal
  state takes at least one (no dead ends — declare it `terminal` or add an
  outgoing edge).
- **Reachability** — every state is reachable from the initial state
  (breadth-first over the edges).
- Edge guards and operation `when` conditions type-check through the shared
  expression language (rex-expr) with the designed class as the implicit
  `self`.

**Condition binding.** Machine states bind as enum-typed *names* — guards
read `status == AwaitingApproval`, never `status == "AwaitingApproval"`.
The string literal is a type error under the same-type equality rule (R2 of
`docs/EXPRESSIONS.md`); the anti-magic-strings stance is deliberate. Edge
guards bind only the machine's own states (a foreign machine's states are
unbound — cross-entity state references are a type error). A service
operation is not aggregate-scoped, so its condition requires the module to
have **exactly one** machine-bearing design, and that design's class is the
condition's `self` scope. Two machines in one module declaring the same
state name is an ambiguous-binding error: the name stops binding, with one
error saying so.

**Artifact shape.** The machine lowers to `machine` on the design (a
`rex_ir::ddd::StateMachine`): the bound `field`, the enum `TypeRef`, the
`initial` state, the `terminals`, and the `transitions` — the `via` trigger
(`None` for a bare arrow) and the guard as the verbatim source text between
the parentheses (the `when`-condition rule). The state set is **derived**,
never serialized as a separate list: `StateMachine::states()` is the
initial state, then the terminals, then the remaining transition endpoints
in first-appearance order; terminalness is membership in `terminals`.

**xstate-ir lowering.** `rex_driver::machines::lower_design_machines` maps
every designed machine onto the
[restingstate](https://github.com/yestechgroup/restingstate) toolchain's
language-neutral `xstate_ir::Machine` (rev-pinned as `xstate-ir` in the
workspace `Cargo.toml`; the sigil precedent). The mapping is total and
lossless for the v1 surface: `id` is the design class name; `context` is
one field (the bound feature, typed by the enum's qualified name, default
the initial state); states follow `StateMachine::states()` with terminals
as `final` nodes and the rest `atomic`; transitions attach to their source
in artifact order, and the guard maps to a named guard whose name is the
condition's source text. A bare arrow's event is **derived**: the lowerCamel
of the target state (`Posted` → `posted`, `AwaitingApproval` →
`awaitingApproval`). The committed goldens pin both forms —
`tests/conformance/ddd/payrun.ddd.json` (the design artifact) and
`tests/conformance/ddd/payrun.xstate.json` (the lowered machine);
regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test
ddd_machine_golden`.

**Derived events.** Every designed machine derives the `state_changed` event
contract (entity + field — `rex_driver::machines::derived_events`), which
is what makes the push plane's `state_changed` vocabulary reachable for a
plain CRUD model once its lifecycle is modeled.

## Aggregate derivation

The aggregate boundary derives from the domain model's containment graph,
standing in for Sculptor's `belongsTo`/`!aggregateRoot` markers: a class B
is *contained* when some class A owns a containment feature (`contains`)
whose type resolves to B, transitively. An entity design whose class is
contained by another stereotyped entity's class is a non-root — it may not
declare a repository. The closure is computed only when every imported
domain lowered (a failed domain may hide containments, and a wrong
non-root verdict is worse than none).

## Domain operations vs service operations

Mox class `op`s and `.ddd` service operations are deliberately different
layers, and the split is the layering heuristic:

- **Behavior confined to one aggregate stays a class `op`.** It lives in
  the domain layer, is owned by the aggregate's class, and generates as an
  instance method regardless of the class's stereotype.
- **Behavior spanning aggregates, needing a transaction, or sitting at an
  entry point is the cue for a `.ddd` service operation.** The service
  operation delegates down to the aggregate-owned ops; it never
  re-implements them.
- **Authorization attaches at the service-operation/capability boundary**
  (the `.actor` dimension). A domain `op` is not an entry point, so ops
  never take `capability` clauses — guarding a domain method would bypass
  the service boundary the policy dimension is defined over.

A `dto`-designed class carrying `ops` is therefore flagged by validation
rule 15 (a warning per op): transfer records carry data between layers, and
behavior declared on them is almost always behavior that belongs on the
aggregate's class or on a service. The escape hatch is unchanged: a
body-less `op` stays the abstract hook implemented by hand in target code
(the "Gap classes" divergence below).

## Capabilities and the actor dimension

A service operation's `capability` clause names actor capabilities. Only
the library entry point `compile_ddd_str_with_actors` validates the names
against a caller-provided `ActorModel`; the CLI compiles with the plain
`compile_ddd_str`, which records them unvalidated — the documented
loose-mapping contract. Nothing in the design artifact constructs policy:
authorization remains the `.actor` dimension's job (see the actor-policy
section of the language reference).

---

## CLI usage

```
rexlang check library.ddd                  # validate design + imported domains
rexlang ir library.ddd -o library.ddd.json # emit the DddModel artifact
rexlang fmt --check library.ddd            # canonical formatting gate
rexlang artifact check library.ddd.json    # validate the serialized artifact
```

- Import paths resolve **relative to the `.ddd` file's directory**; each
  imported `.mox` is read from disk and compiled through the ordinary
  domain pipeline. A missing import file is a clean error naming the
  resolved path. An imported domain may itself declare `import schema` and
  `import sigil` — the CLI provides that content too, so designs target
  lowered JSON-Schema and Rune types like any other class (see
  [Importing Rune models](LANGUAGE.md#importing-rune-models)).
- Diagnostics come from every file involved and render grouped per file,
  the way `.actor` compiles render them.
- `rexlang ir` on a `.ddd` file emits only the design artifact; the domain
  IR is emitted by running `ir` on the `.mox` files themselves.
- `rexlang fmt` formats `.ddd` files with the canonical layout rules:
  imports hoisted into a tight first section (each ending in `;`), then
  the `application` block; `base` first, design flags canonicalized to
  `scaffold auditable optimisticLocking nonPersistent cache`, search
  members canonicalized to `entity text filters sort document ranking
  analyzer pagination capability` (clause contents keep their source
  order; document expressions are re-emitted verbatim), and every
  operation/`inject` line ending in `;`.
- Directory batch mode (`rexlang check <dir>`) picks `.ddd` files up like
  every other surface: each design in the directory compiles independently,
  imports resolved relative to it.

---

## Sculptor concept mapping

| Sculptor | rexlang `.ddd` |
|---|---|
| Entity | `entity` design |
| ValueObject | `value` design |
| BasicType | `.mox` datatype (referenced in signatures, never designed) |
| Repository | `repository` block on an entity design |
| Service (+ injected `=>` delegation) | `service` with `inject` lines and `name => target.operation` operations |
| `scaffold` / `auditable` / `optimisticLocking` / `!aggregateRoot`-style negations | design flags (entity-only; `auditable`/`optimisticLocking` default on for entities, `!flag` opts out) |
| `belongsTo` / `!aggregateRoot` | derived from the domain model's containment graph (see aggregate derivation) |
| Module | `module` label grouping services and designs |
| DTO | `dto` design |
| `findByQuery` / `findByCondition` finders | `search` projection (intent only: text fields with boosts, filters, sorts, ranking, pagination, document projections) — the backend chooses the engine (Postgres FTS, Tantivy, OpenSearch, ...) |
| generated `GET /things/search?q=` endpoints | consumer concern — the artifact is backend-independent |

## Deliberate divergences

Sculptor concepts rexlang intentionally does not carry, and why:

- **Composite natural keys** — expressible already: several `id` features on
  one class lower with `isId`, and the composite key is that set; the
  Tier-1 backends give the group its meaning (docs/LANGUAGE.md keeps `id`
  "carried; semantics land with Tier 1").
- **Collection size constraints (`size`, `notEmpty`)** — expressible as the
  multiplicity annotation (`String[1..10] tags`), which the JSON Schema
  backend already lowers to `minItems`/`maxItems`; a constraint-block
  spelling would duplicate it.
- **Ordered collections (`orderby` / `orderColumn`, Set/List/Bag)** —
  persistence-engine metadata (JPA `@OrderBy`/`@OrderColumn`); rexlang
  features keep declaration order, and a backend that needs a different
  ordering derives it from its own configuration, not from the artifact.
- **Fetch / cascade** — JPA persistence knobs with aggregate-derived
  defaults in Sculptor's generator; the artifact is backend-independent,
  so persistence behavior belongs to the backend that owns the storage.
- **Date validation (`past` / `future`) and custom validators** — runtime
  validation semantics; rexlang's constraint system is deliberately
  schema-only (the `unique` precedent), and there is no runtime validator
  hook to hang them on.
- **Gap classes** — Sculptor's generated-base/handwritten-subclass split is
  a generation strategy; rexlang generates whole files, and a body-less
  `op` stays an abstract hook implemented by hand in target code — the
  same separation without a second class.
- **Traits** — a mixin composition feature of Sculptor's Java generation;
  no rexlang surface carries it yet. If a need lands, it belongs in the
  `.mox` domain model (interface `extends` covers the signature-sharing
  half today).
- **Composite value types (BasicType)** — Sculptor's `BasicType` is a
  multi-attribute embeddable (`Money{currency, amount}`); rexlang
  datatypes wrap a single platform type opaquely. Model the composite as
  a class designed `value` (rule 12 then requires its features
  `readonly`); a true embeddable datatype awaits a deliberate `.mox`
  language extension.
- **Multi-target arrows (`A -> B, C`)** — deliberately excluded from the
  machine surface: an unguarded external choice is not a deterministic
  machine; write one edge per transition.
- **Transition entry/exit actions and timers** — v1 transitions carry only
  trigger, guard, and target; domains.toml approval chains and timers are
  downstream concerns.
- **`.actor` guards binding machine states** — the actor dimension cannot
  yet reference a design's machine states cross-file (a fast-follow);
  today only `.ddd`-internal conditions bind them.

---

## Worked example

`tests/conformance/ddd/library.ddd`, the canonical fixture, designs a
library application over `tests/conformance/ddd/library.mox`. The fence
below is the fixture verbatim, its import repointed so it compiles from
this document (the docs fences are compiled by CI):

```ddd
import "../tests/conformance/ddd/library.mox";

application Library {
    base nz.example.library
    module media {
        /// Coordinates lending of physical media across branches.
        service MediaService {
            inject PhysicalMediaRepository;
            inject MovieRepository;
            inject PersonService;
            inject MediaSearch;
            boolean borrow(String mediaId, Person borrower) capability BorrowMedia;
            PhysicalMedia registerMedia(String status);
            protected save => PhysicalMediaRepository.save;
            registerBorrower => PersonService.register;
            findMedia => MediaSearch.search;
        }
        entity Library scaffold cache repository LibraryRepository {
            findById;
            findAll;
            save;
            delete;
            boolean renameLibrary(String name);
        }
        entity Book
        abstract entity Media
        entity Chapter auditable
        entity Movie repository MovieRepository {
            findById;
            findAll;
            save;
            delete;
            Movie findByTitle(String title);
        }
        entity PhysicalMedia scaffold repository PhysicalMediaRepository {
            findById;
            findAll;
            save;
            delete;
            PhysicalMedia findByStatus(String status);
        }
        /// Full-text search over the movie catalogue.
        search MediaSearch {
            entity Movie
            text {
                title boost 3
                synopsis analyzer "english"
            }
            filters {
                genre
            }
            sort {
                title
            }
            document {
                headline = title;
                blurb = title + " - " + synopsis;
            }
            ranking bm25
            analyzer "english"
            pagination {
                limit 20
                max 100
                cursor
            }
            capability SearchMedia
        }
    }
    module person {
        /// Keeps the member register current.
        service PersonService {
            inject PersonRepository;
            register => PersonRepository.save;
            findByName => PersonRepository.findByName;
        }
        entity Person repository PersonRepository {
            findById;
            findAll;
            findByExample;
            findByKeys;
            save;
            delete;
            protected Person findByName(String fullName);
        }
        search MemberDirectory {
            entity Person
            text {
                fullName boost 2
            }
            filters {
                joinedOn
            }
            sort {
                fullName
                joinedOn
            }
            document {
                displayName = fullName;
            }
            ranking tfIdf
            pagination {
                limit 10
            }
        }
    }
}
```

Notes on the example: `Chapter` declares no repository because it is
contained by `Book` (a non-root aggregate, rule 6); `Book` and `Media`
design the inheritance pair the domain declares; `MediaService.save`
delegates to a repository of its own module while `registerBorrower`
delegates across modules to `PersonService` (service → service, allowed).

---

## Conformance

`tests/conformance/ddd/` holds the canonical fixture set: `library.mox`
(the domain), `library.ddd` (the design, formatter-canonical), and
`library.ddd.json` (the committed artifact). The driver golden test
byte-compares the compiled artifact, the CLI fmt gate enforces the
canonical formatting, and `rexlang artifact check` classifies the artifact
as a DDD design. Regenerate goldens with `REX_UPDATE_FIXTURES=1 cargo test
--workspace`.

The machine-bearing pair `payrun.mox`/`payrun.ddd` (formatter-canonical)
commits two goldens: `payrun.ddd.json` (the design artifact) and
`payrun.xstate.json` (the lowered xstate-ir machine), byte-compared by
`crates/rex-driver/tests/ddd_machine_golden.rs`; the machine validation
rules are pinned by `crates/rex-driver/tests/ddd_machines.rs`.

The `examples/` side ships `support.ddd` (entity/value/dto designs,
searches, capabilities validated against `support.actor` through
`compile_ddd_str_with_actors`) and `orders.ddd` (the aggregate-derivation
rule over `ecommerce.mox`), byte-pinned against `examples/artifacts/` by
`crates/rex-driver/tests/examples_artifacts.rs`. Note that
`examples/library.mox` itself is *not* designable: its `Writer.books`
cross reference escapes the Library aggregate (rule 13).
