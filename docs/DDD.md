# DDD design DSL (.ddd)

## Overview

A Sculptor-style domain-driven-design layer over `.mox` domain models. The
domain model defines the data (entities, features, constraints); the `.ddd`
design defines the application architecture on top of it: the application,
its modules, the application services with their use-case operations, and
per-class design decisions (stereotype, flags, repository, delegation
targets). Parsed by **rex-syntax** (`parse_ddd`, the chumsky parser) at
compile time, lowered and validated by **rex-driver**
(`compile_ddd_str`) against the imported domains, and emitted as the
versioned `rex_ir::ddd::DddModel` artifact (the rex-ir crate).

The `.ddd` DSL lives alongside the `.mox` domain model as a *complementary*
input: a design never declares classes or features — it references the
classes the imported domains declare and records what the application makes
of them. Downstream consumers ingest the artifact exactly like the
`ActorModel` policy artifact; there is no in-tree backend yet.

The normative contract — entry points, resolution semantics, the ten
validation rules, the aggregate-derivation rule, and the capabilities
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
  repository operations camelCase (`"findById"`, `"findAll"`, `"save"`,
  `"delete"`).
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
let compilation = rex_driver::compile_ddd_str("library.ddd", source, &domains);
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

```
ddd_file      := import_decl* application_decl
import_decl   := "import" string (";")?
application   := "application" name "{" base_decl? module* "}"
base_decl     := "base" qualified_name
module        := "module" name "{" (service_decl | design_decl | search_decl)* "}"
service_decl  := "service" name "{" (service_op | inject_decl)* "}"
service_op    := signature capability_clause? ";"
               | name "=>" target "." operation capability_clause? ";"
signature     := type_ref multiplicity? name "(" (param ("," param)*)? ")"
param         := type_ref multiplicity? name
capability_clause := "capability" name ("," name)*
inject_decl   := "inject" name ";"
design_decl   := "abstract"? stereotype name flag* repository_decl?
stereotype    := "entity" | "value" | "dto"
flag          := "scaffold" | "auditable" | "optimisticLocking"
               | "nonPersistent" | "cache"
repository_decl  := "repository" name "{" repository_op* "}"
repository_op := ("findById" | "findAll" | "save" | "delete") ";"
               | signature ";"
search_decl   := "search" name "{" search_member* "}"
search_member := "entity" qualified_name
               | "text" "{" search_field+ "}"
               | "filters" "{" qualified_name+ "}"
               | "sort" "{" qualified_name+ "}"
               | "document" "{" (name "=" expr ";")* "}"
               | "ranking" ("bm25" | "tfIdf" | "exact" | "custom" string)
               | "analyzer" string
               | "pagination" "{" pagination_member* "}"
               | capability_clause
search_field  := qualified_name ("boost" int)? ("analyzer" string)?
pagination_member := "limit" int | "max" int | "cursor"
```

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
- A `///` doc comment run directly above a `service` declaration becomes
  the service's description in the artifact.
- Delegated operations (`name => target.operation`) copy the target's
  signature conceptually and declare none of their own; a delegated op may
  be named `save` (contextual keywords). Declared service operations may
  carry a `capability a, b` clause; declared repository operations take
  none (the wire artifact's repository operations carry no capabilities, so
  accepting one would silently drop data).
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
   `cache` is legal on any design.
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

## Aggregate derivation

The aggregate boundary derives from the domain model's containment graph,
standing in for Sculptor's `belongsTo`/`!aggregateRoot` markers: a class B
is *contained* when some class A owns a containment feature (`contains`)
whose type resolves to B, transitively. An entity design whose class is
contained by another stereotyped entity's class is a non-root — it may not
declare a repository. The closure is computed only when every imported
domain lowered (a failed domain may hide containments, and a wrong
non-root verdict is worse than none).

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
- Directory inputs stay `.mox`-only: designs are compiled by naming the
  `.ddd` file explicitly.

---

## Sculptor concept mapping

| Sculptor | rexlang `.ddd` |
|---|---|
| Entity | `entity` design |
| ValueObject | `value` design |
| BasicType | `.mox` datatype (referenced in signatures, never designed) |
| Repository | `repository` block on an entity design |
| Service (+ injected `=>` delegation) | `service` with `inject` lines and `name => target.operation` operations |
| `scaffold` / `auditable` / `optimisticLocking` | design flags (entity-only) |
| `belongsTo` / `!aggregateRoot` | derived from the domain model's containment graph (see aggregate derivation) |
| Module | `module` label grouping services and designs |
| DTO | `dto` design |
| `findByQuery` / `findByCondition` finders | `search` projection (intent only: text fields with boosts, filters, sorts, ranking, pagination, document projections) — the backend chooses the engine (Postgres FTS, Tantivy, OpenSearch, ...) |
| generated `GET /things/search?q=` endpoints | consumer concern — the artifact is backend-independent |

---

## Worked example

`tests/conformance/ddd/library.ddd`, the canonical fixture, designs a
library application over `tests/conformance/ddd/library.mox`:

```
import "library.mox";

application Library {
    base nz.example.library
    module media {
        /// Coordinates lending of physical media across branches.
        service MediaService {
            inject PhysicalMediaRepository;
            inject MovieRepository;
            inject PersonService;
            boolean borrow(String mediaId, Person borrower) capability BorrowMedia;
            PhysicalMedia registerMedia(String status);
            save => PhysicalMediaRepository.save;
            registerBorrower => PersonService.register;
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
            save;
            delete;
            Person findByName(String fullName);
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
