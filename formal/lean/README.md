# Lean spike: a paper semantics for the rex-expr fragment R1–R4

Issue [#96](https://github.com/yestechgroup/rexlang/issues/96) (phase 7 of
[#83](https://github.com/yestechgroup/rexlang/issues/83)): evaluate whether
a second theorem prover — the issue names Lean — earns its place next to
Creusot, by writing the formal semantics of one small, closed fragment and
proving a few of its rules.

This directory is the spike. **`RexExpr.lean`** defines the evaluation
semantics of the `docs/EXPRESSIONS.md` fragment R1–R4 (checked arithmetic,
null-safe equality, option propagation, division) as a total Lean
function into `Option Value` — `none` being the R1/R4 trap — and states
twelve theorems mirroring the rules. `lean RexExpr.lean` checks the file;
every theorem depends only on Lean's standard axioms (`propext`,
`Classical.choice`, `Quot.sound`) — **no `sorry`**. Pin:
`lean-toolchain` (Lean 4.35.0, no Mathlib — core only).

## What the theorems say

| Theorem | Rule |
|---|---|
| `eval_add_traps_iff_overflow` | R1: `+` traps exactly on overflow — never wraps, never saturates |
| `eval_add_no_trap_without_overflow` | R1: the lowering discipline ("plain `+` with a panic contract") |
| `eval_eq_is_total` | R2: equality never traps; always yields a boolean |
| `veq_null_null`, `veq_null_some` | R2: `null == null` is `true`; `null == some v` is `false` |
| `veq_symmetric` | R2: equality is symmetric, recursively through options |
| `eval_div_traps_iff_zero_divisor` | R4: division traps exactly on a zero divisor |
| `eval_div_truncating` | R4: otherwise truncating (rounds toward zero) |
| `eval_optMap_propagates_none` | R3: `?.` propagates `None` without evaluating the lookup |
| `eval_optMap_traps_iff` | R3: `?.` is compositional — traps only via the receiver trap or the continuation |
| `eval_coalesce_no_trap`, `eval_coalesce_present` | R3: `?:` coalesces; never traps with an evaluating default |

Modeling notes: values are integers/strings/booleans plus freely nested
options (`null` = `opt none`); traps are the meta-level `Option`, so no
lowering can silently swallow them; only the `int` width is modeled
(`long` is the same contract at a wider bound); the static halves of
R1/R4 (constant folding, zero-divisor rejection) are type-checker
contracts outside a runtime semantics, and the theorems carry the
fragment's well-typedness as hypotheses — mismatched shapes are exactly
the type errors rex-expr's checker rejects before evaluation.

## The spike's three questions

**1. Effort per theorem vs. the same statement as Creusot contracts over
the interpreter.**

The semantics took roughly a day of elapsed effort including the
toolchain install and Lean-syntax ramp-up (pattern-match normalization,
`induction ... using f.induct` for nested inductives, `LawfulBEq`
arithmetic). The twelve theorems were a few hours. The equivalent as
Creusot contracts over the rex-expr interpreter would need: creusot-std
to translate `rex-expr` (currently blocked by the same wire-type
derives that keep V10/V15/V16 as spec mirrors — `rex_expr` depends on
`rex-ir`), plus loop invariants for every recursive evaluator walk, plus
trigger management. For *trap-freedom-shaped* statements (this
fragment), Creusot contracts would be comparable or slightly cheaper
once unblocked, because the interpreter is already typed and Creusot
stays in Rust. For *rule-precision-shaped* statements (the exact
propagation structure of R3, the recursion of R2), the Lean version was
easier: Lean's definitional unfolding does the work SMT triggers
struggle with.

**2. Does a paper semantics catch anything the executable specs miss?**

One thing, and it is real but modest: writing `eval` as a total function
forces every combination of operand shapes to be *decided* — the catch
here was that `?.`/`?:` on non-optional receivers and arithmetic on
non-integers must be explicitly ruled out, which is exactly rex-expr's
static-typing precondition written down where the semantics can see it.
The executable specs in `docs/EXPRESSIONS.md` encode the same rules as
test-citable behavior, and the Creusot kernel already proved a false
spec impossible once (the `well_formed` `u32::MAX` catch recorded in
`docs/VERIFICATION.md`) — the same class of catch, available to both
tools. Nothing in the R1–R4 fragment surfaced a discrepancy between the
doc rules and the implementation-observable behavior; the paper
semantics *corroborated* rather than corrected.

**3. Maintenance cost forecast.**

Non-trivial and continuous: the Lean file pins an exact toolchain
(`lean-toolchain`, here 4.35.0); every toolchain bump re-checks the file
(cheap — no Mathlib, but `simp`/equation-lemma churn across a major
version is plausible); the semantics drifts the moment rex-expr grows a
feature outside the fragment (each new rule is a new table row *and* a
new Lean definition + theorems), and nothing enforces that drift — the
Lean file and the Rust interpreter share no types, so the correspondence
is maintained by review (the same spec-mirror honesty V10/V15/V16
already practice). CI would need a Lean job or a scheduled re-check
(`lean RexExpr.lean` is a one-command gate, ~seconds here).

## Recommendation: **down** — do not adopt a second prover now

- The fragment that made the spike tractable (R1–R4) is precisely the
  part Creusot already covers best once translation unblocks; the rules
  are pinned executably and citably today.
- The next genuinely Lean-shaped step — semantics for state machines,
  DMN/CMMN decisioning — is explicitly out of the spike's scope and
  carries the whole maintenance burden of question 3 without a proving
  consumer.
- Revisit when (a) creusot-std can translate `rex-ir`/`rex-expr` in
  crate (which would collapse the spec-mirror pattern and free the
  proof effort), or (b) a real defect class appears that executable
  specs plus Creusot demonstrably cannot state. Until then this file
  stays as the evaluated artifact: small, self-contained, and re-runnable
  with one command.
