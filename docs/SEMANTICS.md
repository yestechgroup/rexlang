# Semantics: the claim ladder

This document states everything the proof-carrying-models program (epic
#202, phase P0) claims, everything it refuses to claim, and why. It
supersedes #189 (the 47 KB research proposal, closed pointing here): the
claim ladder below is #189's distillation,
[VERIFICATION.md](VERIFICATION.md) owns the per-row evidence and all file
pointers, and this file carries what the register cannot — the composition
story: which judgments combine, into what theorem, under which assumptions.

The standing non-goal, restated so it cannot be lost: **this program is not
"rexlang is formally verified"** and no claim below may be read as one. What
exists is a proved kernel, a tested integrity pass, and a set of stated
obligations with named assumptions. "Stated, not proved" is a legal and
preferred status; an unstated claim is the only forbidden one.

## The claim ladder

One table, six claims. Statuses are the register's statuses, cross-checked;
each claim cites register rows ([VERIFICATION.md](VERIFICATION.md)), which own
the pointers. Phase is the epic's phase numbering.

| Claim | Statement | Surface | Evidence today | Phase |
|---|---|---|---|---|
| **C1** Structural soundness | the kernel's invariants survive every mutation | `rex-verify`, `rex-ir`, driver | proved + tested (V1–V10, V17), with the declared residues: V3 argued (its hard residue is open), V8/V9 carry the register's `#[trusted]` assumptions | P6 |
| **C2** Declaration integrity | no reference resolves to a non-declaration; the integrity rules are deterministic, disjoint, cluster-independent | union pass; #194 graph substrate (pending) | tested (V11–V14) — the rules run; the *relation* and its metaproperties are stated, machine-checked in P2, not proved | P2 |
| **C3** Compositional consistency | WF(domain) ∧ WT(design) ∧ Secure(policy) ∧ Consistent ⇒ Correct(system) | `.mox`+`.ddd`+`.actor` slice | stated only (this file: [Judgment forms](#judgment-forms), [Theorem S](#theorem-s)); zero antecedents proved | P4 |
| **C4a** Preservation | every wire rule survives IR → artifact → backend without silent loss | JSON Schema, Rust, DMN backends | one proved mirror (V16) | P3 |
| **C4b** Behavioural refinement | the engine refines the model's operational semantics | `.cmmn` / cmmn-rs | unclaimed (external engine; no operational semantics stated to refine) | P5 |
| **C5** Expression soundness | progress + preservation for the `rex-expr` fragment | `rex-expr` | argued — the compile-time halves of R1–R4 are tested and one component is a proved mirror (V15, the domain-type index); the Lean spike proved a paper semantics for R1–R4 and its verdict is **down** ([../formal/lean/README.md](../formal/lean/README.md)) | P5 |

C1's evidence classes differ per row, deliberately: most of V1–V10 is
SMT-discharged, V3's functional residue is argued and pinned by tests, and the
`#[trusted]` set (V8/V9) is an assumption the register declares. The ladder
reports the weakest honest class, never the average one.

## Judgment forms

Enough formality to state theorem S — explicitly **not** enough to maintain a
second compiler. These judgments name what the compile-time checks judge; they
are not an operational semantics, and none of them carries a soundness proof.
Where the anchor says *tested*, the claim is "the check exists and is pinned
by tests", not "the check is complete". Exactly one judgment is stated
for-later: the composition operator's metaproperties are P2's deliverable.

| Judgment | Statement | Register anchor | Status today |
|---|---|---|---|
| `WF(D)` | the domain model `D` is well-formed: declared names are unique per namespace; every type reference resolves to a declared class, enum, or datatype; feature ids equal the frozen declaration indices; containment and inheritance are acyclic | V4, V17, V10, V3 (+ the register's semantic-analysis pipeline row) | enforced at compile time today — error severity blocks publication; acyclicity argued + tested (V3's residue); feature ids proved over the spec mirror (V10) |
| `WT(d)` | the design `d` type-checks against its imported domain: operation signatures reference declared classes; machine states are literals of the machine's enum; every `when` guard types `boolean`; obligations resolve against the declared vocabulary | V4, V13, V14 | enforced at compile time today (tested) |
| `Secure(p)` | the policy `p` is well-formed against `D`: grants and delegations name declared actors and declared capabilities; granted capabilities are carried by an operation or search; capability nouns grant on aggregate roots (a grant on a non-root warns); delegation `to` names a declared actor | V12 (the compile-time joins), V14 | enforced at compile time today (tested); the non-root-noun warning (#144) is a classification lint with no register row yet — stated |
| `Consistent(d, p)` | the design/policy binding resolves: every action binds to exactly one `(service, operation)` carrier, and every cross-layer reference (capability, obligation, event, flow requirement) resolves into a declared counterpart | V12 (X0006 uniqueness; the X0001–X0008 band) | enforced today — X0006 as error, capability joins promoted to compile time (#128); the union pass carries the lint channel; all tested |

**The composition operator `⨟`.** The artifact union: import-connected
artifacts cluster into applications, and unrelated applications never
cross-check. Cluster independence is a metaproperty of the union pass — the
union-find invariant over canonicalized import paths — stated here as part of
C2's claim and **machine-checked in P2** against #194's `GraphModel` (never a
second graph). It is not claimed proved today. What exists today is the pass
itself: a pure function over span-free artifacts, tested at the artifact level
(V12), with diagnostics that are deterministic by construction.

## Theorem S

For a domain model `D`, a design `d` importing `D`, and a policy `p` over `D`
— the `.mox`+`.ddd`+`.actor` slice, which joins at compile time since #128:

> **Theorem S.** `WF(D) ∧ WT(d) ∧ Secure(p) ∧ Consistent(d, p) ⇒ Correct(System)`,
> where `System` is the code generated from the union `D ⨟ d ⨟ p`.

**`Correct(System)`** is deliberately narrow — an access/trace property on
generated code: *every state-changing request that the generated code accepts
corresponds to a transition the design's machine permits from the current
state, taken by an actor whose grant the policy carries.* It says nothing
about the values requests compute, nothing about requests the code rejects
(a rejection is always safe), and nothing about behavior outside the
generated code. It is a statement about the sequence of accepted requests,
not about program equality with the model.

**Assumptions** (numbered; per the epic's P4 exit gate, the receipt and
`rexlang verify --compose` must list exactly this set):

| # | Assumption | Anchor |
|---|---|---|
| i | Generated-code shape follows the backend contract: naming, feature access through the arena, body emission (verbatim `rust` bodies are trusted code by construction) | [BACKENDS.md](BACKENDS.md) |
| ii | Expression evaluation satisfies R1–R4 — checked arithmetic, null-safe equality, `Option` propagation, division — and nothing beyond the type system is relied on | [EXPRESSIONS.md](EXPRESSIONS.md) |
| iii | The runtime I/O layer — transport, persistence, concurrency, clocks — is out of scope; `Correct` quantifies only over requests the generated code accepts | boundary, not a register row |
| iv | The artifact union passes `check_integrity` (no dangling cross-layer wiring; X0006 uniqueness among them) | V12 |

**Status: a stated obligation, not a result.** Today **zero antecedents are
proved**. The four judgments are enforced heuristically by compile-time checks
(each pinned by tests, anchored above); the soundness of those checks — that
passing them entails the judgment — is unproved, and assumption iv is a test
pin, not a theorem. Machine-checking even one antecedent is P4's exit gate:
it would mean that gate's judgment stops being a hope the checks gesture at
and becomes an obligation a prover discharges over all inputs, and the
receipt could then report a *proved* antecedent instead of an assumed one.
Until then S is a statement of direction, not evidence.

Two narrowings from #189's illustrative sketch are deliberate and recorded:
`Valid(Decisions)` is dropped (the `.dmn` dimension is outside the slice; its
declared-contract ↔ table-header correspondence is a P3 candidate), and
`Consistent(Workflows, Behaviour)` narrows to the design/policy binding (the
workflow surfaces are C2 and C4b territory, not S antecedents).

## Non-goals

Each: what it is, why not now, and what would change the answer.

**Full abstraction.** A translation is fully abstract when it preserves and
reflects observational equivalence: source terms indistinguishable in the
source exactly when their translations are indistinguishable in the target.
Rexlang's backends are deliberately partial views — a JSON Schema artifact
cannot express a design's behaviour, and generated Rust exposes target-level
behaviour the model never stated — so the reflection half fails by
construction. The working substitute is C4a's per-rule preservation. A
concrete defect class where source-distinguishable models generate
indistinguishable artifacts (or the reverse) in a way consumers can observe
would re-open this.

**Lens laws / bidirectional transformations.** `get`/`put` lenses with the
GetPut and PutGet laws, keeping model representations consistent under edits
from either side. The pipeline is one-directional by design — source →
artifact → backend, artifacts versioned and golden-pinned — so there is no
view-update problem in-tree; the formatter's byte-identical round-trip is the
only bidirectionality and it is tested, not theorized. A bidirectional surface
(a visual editor, artifact round-trip editing) entering the roadmap is the
trigger.

**Separation logic and ownership.** Frame-rule reasoning about the generated
Rust heap — repositories, aggregate boundaries, mutable domain objects. The
generated code's ownership discipline is pinned by the backend contract and
the scratch-crate behavioural tests; the resource-safety defects this theory
addresses have not appeared, and Creusot already carries the borrow reasoning
the kernel needs. A defect class in generated repositories or aggregates that
tests plus contracts cannot localize is the trigger.

**Noninterference / information-flow security.** The property that
high-security inputs cannot influence what a low-privileged actor observes —
confidentiality, strictly stronger than authorization. `.actor` judges
permission (who may act), not observation (what may be inferred); there is no
observation model or lattice to state interference against, and `Correct` is
an access/trace property. A surface declaring observation levels or data
classification, or a concrete confidentiality requirement from the re-scoped
#173 security phase, is the trigger.

**Three-dimensional refinement algebra (POPL 2025).** Refinement composed
across modules, abstraction levels, and state components — a general algebra
for exactly the shape of rexlang's stack. The in-tree need is enumerated, not
general: C4a's contract table (P3) covers the wire rules one row at a time,
and a finite table beats an algebra with no consumer. A P3 table that stops
staying finite — rule families resisting row-wise treatment, or a second
backend family needing the same algebra twice — is the trigger.

**PolyVer-style SMT-heavy MDE verification.** Composing per-language
verification tools through abstraction and synthesis for polyglot systems.
Rexlang's surfaces share one IR and one expression language, so the polyglot
problem does not arise; the composition point is the union pass, not a
synthesis layer. Surfaces whose engines are genuinely external (cmmn-rs, the
dsntk fork) needing cross-engine verification is the trigger — that is C4b
and gated there (R2).

**Lean expansion without a defect-class trigger.** Adopting a second prover
and porting the semantic foundation into it. The spike's verdict is **down**
([../formal/lean/README.md](../formal/lean/README.md)): executable specs plus
Creusot cover the fragment that made the spike tractable; the next
Lean-shaped steps (state machines, decisioning) carry the full
toolchain-maintenance burden without a proving consumer; and the Lean file
shares no types with the interpreter, so the correspondence would be
review-maintained — the same honesty the V10/V15/V16 spec mirrors already
practice. Re-open triggers (epic P5/R1): creusot-std translates the wire std
traits (collapsing the spec-mirror pattern), or a defect class appears that
executable specs demonstrably cannot state.

## The #189 mapping

Every heading of #189 resolves below to exactly one landing: a ladder row, a
non-goal, or a register row ([VERIFICATION.md](VERIFICATION.md)). Nothing is
silently lost; where a section genuinely makes no claim, the row says why.

| #189 section | Resolves to |
|---|---|
| Preamble — the umbrella research question and the seven touched areas | **C3** — the composition question is the ladder's apex; the "reusable theories, not a verified implementation" stance is this file's purpose plus the standing non-goal |
| §1 What I see in the Rexlang project (the inventory) | no claim — descriptive; the per-surface guarantees live in the per-surface docs and the register owns the evidence |
| §1.1 A — multiple languages, semantic consistency across them | **C2** (+ `Consistent` above) |
| §1.1 B — the IR architecture; structural validity vs faithful meaning | **C1** (structural) and **C4a** (faithful preservation) — the section's distinction is exactly the C1/C4a split |
| §1.1 C — generated software behaves consistently with its models | **C3** (`Correct`) + **C4b** (the engine leg) |
| §2A Research positioning (Q1–Q5) | no claim — research-metre questions; the ladder itself is the scoping answer (C3 central, the rest ranked or refused) |
| §2B Formal semantics (Q6–Q12) | the judgment forms above + **C3**; Q9/Q10 (compilation preserves semantics) → **C4a**; Q14 (progress/preservation) → **C5** |
| §2C Type systems (Q13–Q19) | Q13/Q17 → `WF`/`WT` + **C1**; Q14 → **C5**; Q15/Q16 (refinement types, invalid states unrepresentable) → **C1**'s kernel invariants are the in-tree form; refinement/dependent types themselves → no claim (no defect class demands them — the Lean non-goal's trigger family); Q18 (external imports) → `WF` judges declared names; schema/sigil imports stay opaque by design (no claim) |
| §2D Model-driven engineering (Q20–Q25) | Q20–Q22 (transformation correctness, refinement, codegen preservation) → **C4a**; Q23 → lens non-goal; Q24 (cross-surface consistency) → **C2** + **C3**; Q25 (evolution, back-compat) → no claim — [VERSIONING.md](VERSIONING.md) owns compatibility; not a verification claim |
| §2E Verification strategy (Q26–Q31) | Q26/Q27 (prover choice) → register rows (Creusot in-tree; the second-prover evaluation) + the Lean non-goal; Q28/Q29 (what to verify first; trusted base) → **C1** + the register's `#[trusted]` policy (V8/V9); Q30 (compositional proofs for new DSLs) → **C3**'s judgment discipline (epic P5/R3 mechanizes it); Q31 (measure effectiveness) → no claim here — epic P1 (receipts, anti-vacuity mutants) owns the measurement plan |
| §3.1 Type soundness: progress and preservation | **C5** (the Lean spike artifact restates the R1–R4 fragment as proved paper theorems; the recommendation stands down) |
| §3.2 Compiler correctness and semantic preservation | **C4a** (+ **C4b** for the engine leg; CompCert is a methodological reference, not an adopted design) |
| §3.3 Compositionality and compositional semantics | **C3** |
| §3.4 Bisimulation and trace equivalence | **C4b** — bisimulation/trace equivalence is the toolbox; the in-tree claim is narrowed to `Correct`'s trace property |
| §3.5 Full abstraction | full-abstraction non-goal |
| §3.6 Refinement and data refinement | **C4a** + **C4b** (the concrete refinement claims); the Event-B/refinement-calculus development method → no claim (no artifact-level development-method adoption) |
| §3.7 Bidirectional transformations and lens laws | lens non-goal |
| §3.8 Separation logic and ownership | separation-logic non-goal (Creusot itself is register apparatus, in use) |
| §3.9 Noninterference and information-flow security | noninterference non-goal |
| §4 reading list (CompCert, Event-B, bidirectionalization, …) | distributes to the rows above (CompCert → **C4a**'s reference; Event-B → §3.6's row; bidirectional transformations → lens non-goal) |
| §4.1 Three-Dimensional Refinement Algebra (POPL 2025) | three-dimensional-refinement-algebra non-goal |
| §4.2 PolyVer (FMCAD 2025) | PolyVer-style non-goal |
| §4.3 Formal verification integrated into MDE (SLCO) | PolyVer-style non-goal (same family: MDE-integrated verification without a defect-class consumer) |
| §4.4 Creusot and Why3 | register rows — the in-tree commitment: the proved kernel, the `#[trusted]` policy, the proof workflow |
| §5 The prioritised research areas (ranked 1–10) | **C3** (compositional semantics), **C4a** (preservation, transformations), **C4b** (bisimulation, refinement), **C5** (type soundness); noninterference, bidirectional, full abstraction, separation logic → the non-goals above — the ladder is the re-ranking |
| §6 Thesis structure (titles, framing) | no claim — thesis framing retired with the research programme; the engineering artifact is this file |
| §6 RQ1 — language semantics | the judgment forms + **C3** |
| §6 RQ2 — semantic preservation | **C4a** |
| §6 RQ3 — compositional verification | **C3** |
| §6 RQ4 — extensibility | no claim here — epic P5/R3 owns it (a new surface ships a judgment or a non-goal, a register row, and a diagnostic family; a docs/test gate enforces) |
| §7 Two representations (implementation IR + formally specified semantic representation) | register rows V10/V15/V16 — the spec-mirror pattern is that separation practiced today; re-seated in-crate on the toolchain trigger (epic P6/T2) |
| §7 Lean 4 as the formal foundation | Lean non-goal (verdict down) |
| §7 technology table (TLA+/Alloy model checking; Proptest) | Proptest → register rows (the proptest suites are register evidence); TLA+/Alloy → no claim — model checking needs an operational semantics first (**C4b** territory, gated) |
| §7 caveat — a Lean spec needs a separate correspondence proof to bind the Rust implementation | register rows V10/V15/V16 — the spec-mirror honesty is this caveat practiced, with the migration trigger recorded |
| §8 Stage 1 — formalise rex-expr | **C5** (the Lean spike executed Stage 1's first half as a paper artifact) |
| §8 Stage 2 — formalise the Core IR | **C1** — done as the `rex-verify` kernel (V1–V10); the open residue rows are the register's declared items |
| §8 Stage 3 — formalise composition (.mox/.ddd/.actor) | **C3** — exactly the P4 slice; the support stack is the slice |
| §8 Stage 4 — verify model transformation | **C4a** — V16 is the first correspondence (JSON Schema boundary); P3 widens rule-by-rule |
| §8 Stage 5 — behavioural verification (.cmmn / cmmn-rs) | **C4b** — gated as R2 |
| §9 Potential PhD publications (four papers) | paper 1 → **C3**; paper 2 → **C4a**; paper 3 → **C3**+**C4b**; paper 4 → the program overall; the publication goal itself → no claim — research outputs retired with the thesis framing |
| §10 Supervisor questions (eight) | Q1 → **C3** (chosen); Q3 → **C3**+**C4a** (chosen: per-surface judgments plus verified boundary contracts); Q4 → **C4a** (elevated to a ladder row); Q5 → the Lean non-goal (answered: verdict down); Q6 → **C3**'s slice (the support stack is the structural+behavioural+security case study); Q2, Q7, Q8 → no claim — literature-survey and academic-metre questions, retired with the framing |
| §11 The compositional correctness theorem (the recommendation) | **C3** / Theorem S above — stated with assumptions and the two deliberate narrowings (`Valid(Decisions)` dropped; workflow consistency narrowed); the final framing ("formal foundations, demonstrated through rexlang") is this file's purpose |
