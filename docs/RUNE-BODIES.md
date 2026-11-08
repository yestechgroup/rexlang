# Rune body translation — design (RFC)

Status: **landed** — all three implementation stages shipped. This doc
records the design decisions for making sigil-lowered Rune function bodies
executable; the sections below stay as the design record. The `"rosetta"`
body target still carries the declaration's ends-trimmed verbatim source
(wire contract rule 15); statement-bearing functions additionally carry the
driver-checked `"rune"` statement body (sigil mapping rule 8),
`rex-backend-rust` emits it as straight-line Rust — the `todo!()` stub
remains only for functions without a `"rune"` target (#61 is closed) — and
the bare-call surface is open (#63).

The sigil side of this contract lives in the `rex-driver` sigil module docs
(`crates/rex-driver/src/sigil.rs`, mapping rule 8); the Rust-backend stub
policy lives in `docs/BACKENDS.md`.

## Problem

`Package::functions` carries signature-only operations. Their behavior
exists only as Rune source text: `func` bodies are built from statement
forms rex-expr does not have, and nothing in rexlang parses or executes
them. Concretely, a body is:

- **Accumulation statements** — `set root (-> feature)*: expr` and
  `add root (-> feature)*: expr` (`OperationDef`: an `add` flag, an assign
  root, a `->`-separated feature path, one expression). `set` assigns the
  output (or a reachable output slot); `add` appends to a list-valued slot.
  Multiple statements accumulate into the same output root.
- **`alias` bindings** — `alias name: expr` (`ShortcutDef`), a local
  let-style binding, referenced by later expressions.
- **Named guards** — `condition Name:` / `post-condition Name:` blocks
  (`ConditionDef`): boolean expressions evaluated before/after the
  accumulation statements.
- **Dispatch heads** — `func Foo(attr: Enum->VALUE)` (`FunctionDispatchDef`):
  the head constrains the function to one enum case. The Rosetta
  multi-branch spelling is several same-name declarations; sigil resolution
  rejects duplicate function names within a kind, so what survives lowering
  is one declaration per surviving name, head included.

The expression *fragments* inside these statements overlap heavily with
rex-expr (navigation, arithmetic, `exists`/`choice`, functional operators);
only the statement layer is missing.

## The CEL verdict (recorded from #58)

Adopting CEL (via `cel-rust`) was evaluated and **rejected**:

- CEL has no statement form. `set`/`add` output accumulation cannot be
  expressed, so CEL would still need a statement wrapper — the hard part
  stays ours.
- `cel-rust` is an interpreter, not a codegen. Adopting it means either
  writing a CEL→target codegen anyway (so CEL buys nothing) or embedding a
  runtime dependency in every generated crate (contrary to the backend's
  self-contained-output contract).
- CEL's semantics fork from every other rexlang surface: null/optional
  handling and arithmetic would diverge from R1–R4
  (`docs/EXPRESSIONS.md`), giving the same expression different meaning in
  a Rune body than in an `expr` body.

## Statement IR

rex-expr keeps owning expressions (parse, typecheck, lower). A new
statement layer sits above it, in `rex-expr` (so every surface can share
it) with a neutral serialization:

```
RuneBody {
  inputs:  checked against DomainTypes
  guards:  Vec<{ name, expr, phase: Pre | Post }>   // condition / post-condition
  bindings: Vec<{ name, expr }>                      // alias, in declaration order
  steps:  Vec<Set | Add { root, path: Vec<name>, expr }>
}
```

- One neutral body target per function (the `expr`-style pseudo-target,
  spelling decided at implementation), additive and
  `skip_serializing_if`-omitted like every other body map entry; the
  verbatim `"rosetta"` target stays for provenance either way.
- Statement lowering is deterministic source-order: guards, aliases, and
  steps keep declaration order (Rune semantics are sequential for alias
  reuse; guards are order-pinned for diagnostics).
- Expressions inside every slot are rex-expr programs, so they reuse the
  existing parser/typechecker/lowerings unchanged.

## Dispatch semantics

Pinned decision: **dispatch lowers to branching inside one operation**, not
one operation per branch.

- Lowered Rune names must stay unique (`Package::functions` is keyed by
  name; sigil already rejects same-kind duplicates), so per-branch
  operations would need synthesized names — a wire-visible invention for
  host languages to trip over.
- A dispatched function lowers to: a plain signature (the dispatch head's
  attribute is just another input), then a `match` on the head's
  `(attr: Enum->VALUE)` predicate guarding the body. Where several branch
  declarations survive for one conceptual function (a future lowering
  change would have to group them by name), the branches lower to
  sequential guarded statement groups; the first matching head wins and the
  last group is the fallback, mirroring Rune's declaration-order dispatch.

## Typechecking

Everything checks against `rex_expr::DomainTypes` — extended, never a
synthesized `Model`:

- `DomainTypes::insert_functions(package, name, signature)` joins the
  union-package function namespace so a body's calls and navigations
  resolve: receivers are the function's inputs, `alias` names join the
  local scope, and `set`/`add` targets type-check against the output
  slot's declared type (navigating through the `->` path).
- Guard expressions must be `boolean`; alias expressions may be any value
  type; step expressions must unify with the target slot (list append for
  `add`, element type for `set`).
- The same `DomainTypes` extension later unlocks the call surface (#63):
  `name(...)` resolving against the union package namespace instead of only
  the receiver's class.

## Semantics pinned against EXPRESSIONS.md

- **R1–R4 carry over unchanged** into Rune bodies: overflow is an error
  (R1), equality is total and null-safe (R2), `Option` propagates only
  through `?.` (R3), division by zero is an error (R4). Rune bodies get
  exactly one expression semantics; nothing is re-derived per surface.
- **`set` to an unset slot** yields the declared default shape: scalar
  assignment overwrites, `add` starts from the empty list. Reading a slot
  no statement has set is the R3 story (`Option`), not an implicit zero.
- **Guards** are assertions, not control flow: a false `condition` is a
  runtime error naming the guard (Rune has no alternative-succession
  semantics to port); `post-condition` runs after the steps.

## Per-backend lowering order

1. **rex-backend-rust** (landed): a present `"rune"` target emits
   straight-line Rust — guards → `assert!(expr, "guard '<name>' failed")`
   (unnamed guards carry the checker's phase-derived slot labels, `unnamed
   pre guard` / `unnamed post guard`), `alias` bindings → sequential
   `let`s, `set`/`add` steps → assignment/`push` into the output local. The
   output local is introduced at its declared default shape — `Vec::new()`
   for a `(0..*)` output, `None` for `(0..1)`, the Tier-1 scalar default for
   a required scalar, the arena's default-initialized object
   (`res.new_<class>()`) for a class-typed output. A body that reaches the
   arena (a class-typed output, a `->` path step, navigation on a
   class-typed value in any slot) makes the function take `res: &Resource`
   as its first parameter, and bare `expr`-body calls pass it. The
   `todo!()` stub remains **only** for functions without a `"rune"` target
   (statement-less, driver-deferred, or library functions — library
   functions never carry one); a present target that fails to decode,
   re-check, or lower is a hard generate-time error naming the function and
   the slot — artifact corruption, not deferral, because the driver never
   attaches an unchecked body.
2. **rex-backend-jsonschema**: unaffected — functions never enter schemas;
   instances stay class-shaped.
3. Later backends consume the same statement IR; nobody re-parses the
   `"rosetta"` text.

## Implementation stages

1. Statement IR + parser over the retained Rune statement grammar (the
   sigil AST shapes above are the reference), typechecked against the
   extended `DomainTypes`; the driver lowers at sigil time and attaches the
   neutral body beside the verbatim `"rosetta"` one.
   — landed as `rex_expr::body::RuneBody` plus the `"rune"` wire target
   (rule 15) and the driver's checked lowering (sigil mapping rule 8:
   dispatch heads lower as the leading synthetic pre guard; a deferral
   attaches no target and emits one warning naming the function).
2. rex-backend-rust emits bodies instead of stubs; the stub remains only
   for functions whose statements could not be checked (the documented
   generation-time deferral contract, mirroring the `.ifml` checker's).
   — landed as `codegen::rune_body`: the stub only where no `"rune"`
   target exists; a present-but-invalid target is a generate-time error
   (the per-backend lowering order above pins the emission shapes).
3. The call surface (#63): rex-expr resolves `name(...)` against union
   package functions; `.uc` steps and IFML handlers decide their
   subject-free-step question with the typing index in place.
   — landed as the bare-call surface: rex-expr resolves and types
   `name(...)` (return multiplicity included — `List`/`Option`/plain,
   mirroring the parameter mapping) and the Rust backend emits the matching
   shapes and passes `res` when the callee needs the arena; the `.uc`/IFML
   subject-free-step adoption is still each surface's call to make.

Each stage keeps the wire format additive: function-less models stay
byte-identical, and the golden diff of stage 1 is reviewed as a spec
change.
