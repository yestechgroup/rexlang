# Verification (proof-obligation register)

rexlang's formal-verification strategy follows
[issue #83](https://github.com/yestechgroup/rexlang/issues/83): identify the
properties that must **always** hold, specify them once, and let a prover —
[Creusot](https://creusot.rs) (Rust → Coma → Why3 → SMT) — discharge them for
**all** inputs, complementing unit tests (particular examples) and property
tests (many generated inputs). The goal is deliberately not "formally verify
rexlang"; it is *a formally specified, incrementally verified semantic
foundation that the languages, transformations and generators depend on*.

## What is verified today

The crate `rex-verify` hosts the first verified component: a semantic-graph
kernel (uniquely identified nodes, multi-parent DAG containment shaped like
rexlang's `extends` lists, deduplicated reference edges) specified with
Creusot contracts. **`cargo creusot -p rex-verify` is fully green**
("Proved (42 files)"): every proof obligation below is SMT-discharged.

- `Graph::well_formed` (`crates/rex-verify/src/lib.rs`) is the kernel
  invariant: node count below the `u32` id bound, every parent/edge endpoint
  valid, and the child half of the parent/child agreement. The mutators
  carry `requires(well_formed(*self))` / `ensures(well_formed(^self))`, so
  *any* observable graph is well-formed and *every* mutation provably
  preserves well-formedness (issue property 5). Per-mutation facts are
  bridged against the trusted mutation primitives' contracts
  (`push_default_node`, `set_parent`, `push_containment`,
  `push_reference` — one-statement bodies, exact model-effect contracts).
- `Graph::add_node` carries the freshness contract (issue property 1): the
  returned id equals the previous node count, the count grows by one, and
  the id differs from every pre-existing id — combined with ids being
  indices, identities are unique forever.
- `Graph::link` / `Graph::reference` carry endpoint-existence preconditions
  (issue property 2); `reference` additionally ensures a recorded edge
  stays recorded. `link` models rexlang's `extends` lists: several parents
  are legal, a duplicated edge is an idempotent success (never a cycle),
  and a refused edge is exactly a closing cycle — which is how
  `rex-driver`'s inheritance/actor cycle passes now detect cycles (see V3).
- `Graph::is_ancestor` is proved safe (indexing) and terminating (bounded
  parent walk); `link`'s cycle rejection relies on it operationally.
- Containment acyclicity (issue property 3) is *enforced operationally* by
  `link`'s ancestor guard and pinned by the property tests. The reachability
  machinery the formal statement needs is now defined and partly proved:
  bounded-`k` reachability (`anc_k` reflexive, `anc_strict_k` strict), the
  budgeted closures (`anc` / `anc_strict`, witness ≤ node count — the
  executable walk's budget), `acyclic` (no strict cycle within the budget),
  and the proved **shift lemma** (`lemma_anc_shift`, induction on the
  bound): bounded reachability in both directions transfers across any
  mutation that keeps every old parent list and appends parentless slots —
  the shape of every mutator except `link`. What remains open is exactly
  the hard residue: linking the executable walk to the predicate
  (`is_ancestor`'s functional spec, register V7) and preserving `acyclic`
  across `link` — both need a path-shortening (loop-cutting) lemma and,
  for completeness, a DFS-completeness invariant; this is the registered
  open obligation below. The
  parent/child agreement for the *whole* containment edge set (not just the
  newly added edge) is likewise carried per-edge by `link`'s ensures plus
  the property tests rather than by the consolidated invariant.
- Issue property 4 ("type references always resolve") is carried by the Core
  IR design itself: `rex_ir::TypeRef` is *never an unresolved name*, and the
  driver's resolution emits diagnostics instead of unresolved references,
  with any error severity blocking publication of the model
  (`compile_file`). See the register's pipeline rows.

The same properties are pinned empirically by `crates/rex-verify/tests/`
(unit tests named `p1_*`…`p5_*` plus proptest mutation sequences; register
row V11 adds the `p11_*` interaction-reference discipline — see its row
below). The oracle's two bounded walk helpers are themselves proved against
the kernel's loop discipline: `contains_edge` carries the full functional
spec (`result == has_contains(parent, child)`), and `has_containment_cycle`
is proved memory-safe and terminating (out-of-range parent edges — possible
only on malformed graphs — are skipped rather than indexed), shrinking the
`#[trusted]` set to the oracle dispatcher and the four one-statement
mutation primitives (V8/V9). The
property tests are not decoration: during development they shrank a real
counterexample (stale subtree state under re-rooting) that the unit suite
missed, and the prover caught a second: `well_formed` originally allowed
`len == u32::MAX`, under which `add_node`'s fresh id would overflow — the
preservation ensures was *unprovable because false*, exactly the class of
mistake formal specification exists to catch.

Avoidances, recorded for the next toolchain bump: the `Hash`,
`PartialOrd` and `Ord` derives are omitted from `NodeId` because creusot-std
0.14.0 leaves their refinement obligations unresolved upstream; std
`Vec::contains` is avoided in `link` (its spec carries an unprovable
resolution obligation) in favour of a proved bounded loop; in-crate
`rex-ir` proofs are blocked by the same family of gaps (the wire types'
standard `PartialEq`/`Hash`/`Ord` derives demand `DeepModel` instances and
refinement obligations creusot-std 0.14.0 does not discharge — hence the
`feature_ids` spec mirror, to be moved in-crate on a toolchain bump); and the
`#[trusted]` mutation primitives exist because the prover cannot yet
re-establish the consolidated invariant directly after a raw
`Vec`-indexing borrow (quantifier instantiation across the borrow boundary)
— with the primitives in place everything else is proved.

## Running the proofs

Contracts compile away under plain `cargo build` (stable); CI runs only the
ordinary test/clippy/fmt gates. To discharge the proof obligations, install
Creusot (see <https://guide.creusot.rs/installation.html>) and run:

```sh
cargo creusot -p rex-verify
```

`creusot-std` is pinned to the matching `0.14.x` line in the workspace
manifest; the proof setup lives in `why3find.json` (repo root, provers
alt-ergo + z3, tactic `split_vc`) and the generated Coma lands in `verif/`
(git-ignored, like `.creusot-cache*`). Notes from the first bring-up:

- the pinned Why3 revision ships `compute_specified` as a
  transform-with-arguments, which this `why3find` cannot use as a tactic —
  the `Tactic compute_specified not found (skipped)` warning is cosmetic;
- `creusot-std`'s contracts must still compile (warning-free) under plain
  stable rustc — CI depends on it.

## Proof-obligation register

Statuses: **proved** (SMT-discharged under `cargo creusot`), **tested**
(pinned by the test suites), **argued** (manual argument + tests, formal
proof open), **assumed** (`#[trusted]` or absent checks — an assumption, not
a proof). Every `#[trusted]` in the tree must have a row here.

| # | Property (issue #83 wording) | Formal specification | Enforcing implementation | Status |
|---|---|---|---|---|
| V1 | Nodes always have unique identities | `add_node` `ensures`: fresh id = old count, count + 1, id ∉ old ids | `Graph::add_node` (`rex-verify`); ids are arena indices, never reused | proved |
| V2 | Relationships always reference existing nodes | `well_formed` endpoint clauses; `link`/`reference` `requires` | `Graph` API (only constructor of ids); driver `resolve*` for the real IR | proved |
| V3 | Containment graphs cannot contain cycles | reachability machinery defined and partly proved: `anc_k`/`anc_strict_k` (bounded-`k` reflexive/strict closure), `anc`/`anc_strict` (budgeted witness ≤ node count), `acyclic` (no strict cycle within the budget); **`lemma_anc_shift` proved** — bounded reachability (reflexive + strict, both directions) transfers across any mutation keeping old parent lists and appending parentless slots (induction on the bound) | `link` ancestor guard (`is_ancestor`: proved safe + terminating); `tests/invariants.rs` `p3_*` + proptest `p3_containment_stays_acyclic`; **`rex-driver` inheritance/actor cycle passes now detect cycles via rejected `link` calls** (`detect_inheritance_cycles` / `detect_actor_cycles` on `rex_verify::Graph`); use-case `check_graph` still DFS-based | argued + tested; spec machinery proved (shift lemma); `is_ancestor` functional spec and acyclicity preservation under `link` open — they need a path-shortening (loop-cutting) lemma plus a DFS-completeness invariant, deliberately budgeted as #93's hard residue; driver detection kernel-backed |
| V4 | Type references always resolve | `rex_ir::TypeRef` "never an unresolved name" (crate docs, wire rule set) | `rex-driver` resolution + error-severity block in `compile_file`; `rex-expr::DomainTypes` for expressions | tested |
| V5 | Mutations preserve all established invariants | `well_formed(*self)` → `well_formed(^self)` on every mutator | every `rex-verify` mutator; bridged against the trusted mutation primitives; proptest sequences with `check_invariants` oracle | proved (the children-agreement clause for the *whole* edge set is carried per-edge by `link`'s ensures + tests; consolidated `children_ok` forall documented open) |
| V6 | Reference edges are recorded and deduplicated | `reference` `ensures` (`has_ref` persists), `requires` endpoints | `Graph::reference`; `p2_*` tests | proved |
| V7 | `is_ancestor` is a safe, terminating guard | `requires(well_formed)`, loop variant `budget` | bounded parent walk in `is_ancestor` | proved (safety/termination; functional spec = V3, open) |
| V8 | `check_invariants` (executable oracle) mirrors the invariant | `contains_edge`: full functional spec (`ensures result == has_contains(parent, child)`); `has_containment_cycle`: safety + termination (bounded mark-on-push walk; `requires` the kernel's `u32` length bound and a valid `start`) | `Graph::check_invariants` (`#[trusted]` — it must inspect *arbitrary*, possibly malformed graphs, states the invariant excludes) + `contains_edge` / `has_containment_cycle` (proved against the logical `has_contains` / `is_ancestor`-shaped loop discipline) | oracle dispatcher assumed (test-side; validated by the property suites; never relied on by proofs); both walk helpers proved |
| V10 | Wire rule 3: a feature's stable id equals its 0-based declaration index; `assign_feature_ids` establishes it and changes nothing else | `rex_verify::feature_ids::assign_ids` `ensures` (length unchanged, `id(i) == i` for all slots, within the `u32` bound) | proved over the spec mirror `rex_verify::feature_ids` (identical loop shape + trusted one-statement primitive `set_id`); `rex-ir`'s `ClassDef::assign_feature_ids` pinned executably by `crates/rex-ir/tests/feature_ids.rs` | proved (spec mirror); in-crate `rex-ir` translation blocked upstream (see avoidances) |
 | V9 | Mutation primitives do exactly what their contracts state | full model-effect contracts (snoc + changed fields) | `push_default_node`, `add_parent`, `push_containment`, `push_reference` (all `#[trusted]`, one-statement bodies) | assumed bodies, proved contracts (see `#[trusted]` policy) |
| V11 | Interaction references always resolve to declarations ([issue #112](https://github.com/yestechgroup/rexlang/issues/112)): every interaction reference edge — `action("X")`, `navigate("X")`, `refresh("X")` — terminates at a **declaration-backed** node, never a synthesized one | every invocation edge's endpoint ∈ the declared set (`InvocationGraph::check_invariants` in `crates/rex-verify/tests/invariants.rs`, `p11_*`) | `rex-ifml`'s `check_references` cross-reference pass (`E0104`–`E0106`, entry file plus **instantiated-module** sites, union-wide declarations; container targets included) flowing into `rexlang check`'s error block and the LSP's published diagnostics — and, as the artifact boundary, `rexlang manifest`: the control/route manifest ([issue #118](https://github.com/yestechgroup/rexlang/issues/118), `rex_ir::ifml_manifest`) is derived post-check, so every manifest edge's target resolved by construction and consumers cannot re-derive resolution or synthesize endpoints; the proptest validates the oracle against an independent recomputation over random histories | tested (honest staging per V4's precedent — the kernel tracks no node metadata yet; the graph-level `proved` form belongs in `rex-verify` once the ingest stops synthesizing endpoints from invocations, the codegraph companion — which the manifest boundary now makes structurally impossible on the artifact path) |
| V12 | Lifecycle wiring always resolves across the artifact union ([issue #114](https://github.com/yestechgroup/rexlang/issues/114)): a capability clause names a declared capability; a granted or delegated capability is carried by an operation or search; an `.ifml` action declaration binds to a service operation (case-insensitive verb equality); a subscription consumer names a declared deploy component; flow `requires:`/`roles:` resolve into the actor layer; an operation's `emits` event name is declared in some `.evt` contract; a declared event is emitted by at least one service operation (channel `publishes`/subscription lists are consumption-side and never satisfy production). Since rexlang#128 the two capability joins resolve at **compile time** (rule 10 pooling every provided actor model, with the `unknownCapability` did-you-mean code; grant coverage in the actor engine, code `grantedCapabilityUncarried`) — the union pass keeps them as its lint channel (warning severity). Every union-pass diagnostic carries its stable registry code prefixed on the message (`X0001`–`X0008`, the `driver.integrity` band of [the diagnostic-code registry](DIAGNOSTICS.md)), and an action whose case-insensitive mirror matches **several** service operations binds to no unique backend — an error (`X0006`), the mirror naming exactly one `(service, operation)` carrier | `rex_driver::check_integrity` over the gathered artifact buckets — a pure function, pinned at the artifact level with exact diagnostic text plus its silent gates in `crates/rex-driver/tests/integrity.rs`; the promoted capability joins are pinned in `crates/rex-driver/tests/ddd.rs` (rule 10, did-you-mean), `crates/rex-driver/tests/actors_file.rs` (coverage), `crates/rex-cli/tests/cli.rs` (directory mode, phase 1 owns the errors), and `crates/rex-lsp/src/server/tests.rs` (diagnostics + the rename quick-fix) | the CLI's directory batch mode: per-file artifacts cluster into import-connected applications (the house import graph) and the pass runs per application, error diagnostics failing the batch; the compile-time capability joins run in the CLI project pipeline and the LSP per document against the sibling artifacts, with the union-pass lint suppressed for files phase 1 already failed; per-surface single-file compiles are unchanged | tested (enforcement landed; the kernel/proptest staging follows V11's precedent once the graph ingest stops synthesizing endpoints — the same codegraph companion) |
| V13 | State machines are closed over their enum, and lifecycle guards are predicates: every machine state (initial, terminal, transition endpoint) is a literal of the machine's enum type, and every `when` condition — transition guard, service operation, repository operation — types as `boolean` | state-set membership ⊆ the enum's literal list; `check_condition`'s result type = `boolean` | `lower_design_machine` + `check_condition` in `crates/rex-driver/src/ddd.rs`, pinned by `crates/rex-driver/tests/ddd_machines.rs` (six tests: three boolean sites, typo'd endpoint, non-literal initial/terminal, all-literals clean) | tested |
| V14 | Obligations are a declared vocabulary: every grant or delegation entry's obligation names resolve against the union of the actors blocks' `obligation` declarations | references ⊆ the union declared set | `validate_actor_obligations` in `crates/rex-driver/src/lower/actors.rs` over the `ActorsRecord` obligation refs, pinned by `crates/rex-driver/tests/actors.rs` (IR shape, grant/delegation errors with spans, duplicates, union pooling) | tested |
| V15 | Domain-type lookup agreement (issue #94, the first verified statement for expression typing): a domain-type index is **total** over what was registered — every key resolves to its registration, last registration winning — and **merge preserves per-source lookups** (the merged-in index wins on collision, the surviving index resolves everywhere else) | `TypeIndex::lookup` full functional spec (`result == lookup_spec(entries, key)`), `insert` replace semantics, `merge` right-wins concat semantics (all proved; helper `lemma_bound_at_prefix`, induction on the prefix length) | spec mirror `rex_verify::domain_types` — insert-replaces + linear-scan lookup standing in for `rex_expr::DomainTypes`' `BTreeMap` (whose std specs creusot-std 0.14.0 cannot translate — the V10 upstream-gap family), executably pinned by `crates/rex-verify/tests/domain_types.rs` and shape-pinned on the real API by `crates/rex-expr/tests/typecheck.rs` (`domain_types_is_assembled_incrementally_and_merges`); moves in-crate on a toolchain bump | proved (mirror) |
| V16 | Class→artifact correspondence at the IR → JSON Schema boundary (issue #95, the first proved transformation statement): the collector admits exactly the declarations with distinct class names (duplicates rejected — `Context::collect`'s `bail!`), and the emitted `$defs` are in correspondence with the declared classes — **one def per class, in declaration order, none dropped, none synthesized** | `correspondence::collect` two-directional contract (`Some` ⇒ keys unchanged + `distinct`; `None` ⇒ a witnessed `has_duplicate`), `def_keys` identity emit (`result@ == classes@`) | spec mirror `rex_verify::correspondence` — interned keys standing in for class names, identity emit standing in for `wire_defs`' per-class loop (the `rex-ir` wire types cannot translate under creusot-std 0.14.0 — the V10 upstream-gap family); executably pinned by `crates/rex-verify/tests/correspondence.rs` and on the real emitter by `crates/rex-backend-jsonschema`'s `wire_defs_map_every_declared_class` / `collect_rejects_duplicate_class_names`; moves in-crate on a toolchain bump | proved (mirror) |

### Pipeline-level obligations (issue's five boundaries)

| Boundary | What we want (issue) | Current enforcement | Status |
|---|---|---|---|
| Parsing | parsed structures satisfy AST invariants | logos+chumsky parser tests; `rexlang fmt` byte-identical round-trip goldens | tested |
| Semantic analysis | invalid models cannot pass validation | driver validators: duplicate names, reference resolution, opposite pairing, inheritance/actor cycles, `never_both`; error ⇒ no model | tested |
| Intermediate representation | IR satisfies structural/semantic invariants | `rex-ir` wire contract (version gate, frozen feature ids, deterministic equality) + golden/byte-identity suites; `rex-verify` kernel as the future verified core | tested (kernel subset: proved) |
| Transformations | transformations preserve meaning | golden artifacts per surface + examples harness; **V16: the IR → JSON Schema class correspondence is proved over its spec mirror** (`rex_verify::correspondence`) | tested (mirror proved; full preservation = issue phase 8, open) |
| Code generation | generated artefacts satisfy contracts | backends consume only `rex_ir::Model`; scratch-crate `cargo test` e2e; schema conformance proptests | tested |

## Second-prover evaluation (issue #96)

The phase-7 question — whether a second theorem prover (Lean) earns a
place next to Creusot — was answered by a spike: `formal/lean/` carries a
paper semantics of the `docs/EXPRESSIONS.md` fragment R1–R4 with twelve
theorem restatements (`lean RexExpr.lean`, no `sorry`, core-only
toolchain pinned in `lean-toolchain`). The verdict, with the effort and
maintenance analysis, is recorded in `formal/lean/README.md`:
**recommendation down** — executable specs plus Creusot cover the
covered-best fragment, the next Lean-shaped steps carry the full
toolchain-maintenance burden without a proving consumer, and the file
stays as the re-runnable artifact pending a creusot-std translation
unblock or a defect class executable specs cannot state.

## `#[trusted]` policy

`#[trusted]` means the prover assumes the body: it is an **assumption**, not
a proof (issue #83's explicit warning). Rules for this repository:

1. every `#[trusted]` must have a register row above, with the reason it is
   not proved;
2. `#[trusted]` may not be used on anything that feeds a proved contract's
   *meaning* — the mutation primitives' contracts state their full model
   effect and everything else is proved against those contracts, so the
   trusted surface is minimal and auditable by eye (one-statement bodies);
3. shrinking the trusted set is always a welcome PR.

## Non-goals (for now)

- Creusot in CI (prover availability; proofs are run locally/per-release).
- Formal specs inside `rex-driver`/`rex-syntax` (salsa and parser internals
  stay outside the verified kernel; the kernel is the seed of the verified
  semantic foundation, per the issue's roadmap phases 4–8).
- Semantic-preservation proofs for lowerings (phase 8).
