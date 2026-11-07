/-!
# Formal semantics of the rex-expr fragment R1–R4 (issue #96 spike)

A paper semantics for the closed rex-expr fragment of
`docs/EXPRESSIONS.md` rules R1–R4, with the rules restated as theorems
(no `sorry`): checked arithmetic (R1), null-safe value equality (R2),
option propagation (R3), and division (R4).

Modeling choices, kept deliberately small:

* Values are integers, strings, booleans, and options (nests freely —
  R2's equality is recursive). `null` is modeled as `Value.opt none`.
* Traps (the R1/R4 "runtime panic contract") are the meta-level
  `Option`: `eval = none` means the expression traps, `some v` means it
  produces `v`. Traps are *not* values, so no lowering can silently
  swallow them.
* Only the `int` width is modeled; `long` is the same contract at a
  wider bound (R1).
* The static halves of R1/R4 (constant folding, zero-divisor rejection)
  are type-checker contracts, outside a runtime semantics; the compile
  half of R1 appears here only as the literal-range guard.
-/

/-- The 32-bit signed range: rexlang's `int` (R1's checked width). -/
def inI32 (x : Int) : Bool :=
  -2147483648 ≤ x && x ≤ 2147483647

/-- The fragment's values. `Value.opt none` is rexlang's `null`. -/
inductive Value where
  | int (v : Int)
  | str (v : String)
  | bool (v : Bool)
  | opt (o : Option Value)

/-- R2's value equality: total, null-safe, recursive on options. -/
def veq : Value → Value → Bool
  | .int x, .int y => x == y
  | .str x, .str y => x == y
  | .bool x, .bool y => x == y
  | .opt none, .opt none => true
  | .opt (some x), .opt (some y) => veq x y
  | _, _ => false

/-- R1's checked arithmetic: the result must fit the width; overflow
traps (never wraps, never saturates). -/
def chk (r : Int) : Option Int :=
  if inI32 r then some r else none

/-- An environment binds every variable to a value (rex-expr is
statically checked: free variables are impossible). -/
def Env := String → Value

inductive Expr where
  /-- An `int` literal; out-of-range literals are an R1 error. -/
  | lit (n : Int)
  | var (name : String)
  | str (s : String)
  | bool (b : Bool)
  | add (a b : Expr)
  | sub (a b : Expr)
  | mul (a b : Expr)
  /-- R4: truncating integer division; a zero divisor traps. -/
  | div (a b : Expr)
  /-- R2: null-safe value equality, yields `boolean`. -/
  | eq (a b : Expr)
  /-- R3: `a?.k` — propagates `None`, otherwise evaluates the continuation
  on the unwrapped value (the field lookup as a continuation). -/
  | optMap (a : Expr) (k : Value → Expr)
  /-- R3: `a ?: d` — coalesces. -/
  | coalesce (a d : Expr)

/-- The evaluation relation as a total function into `Option Value`:
`none` is the R1/R4 trap. -/
def eval : Expr → Env → Option Value
  | .lit n, _ => if inI32 n then some (.int n) else none
  | .var x, env => some (env x)
  | .str s, _ => some (.str s)
  | .bool b, _ => some (.bool b)
  | .add a b, env =>
      match eval a env, eval b env with
      | some (.int x), some (.int y) => (.int ·) <$> chk (x + y)
      | _, _ => none
  | .sub a b, env =>
      match eval a env, eval b env with
      | some (.int x), some (.int y) => (.int ·) <$> chk (x - y)
      | _, _ => none
  | .mul a b, env =>
      match eval a env, eval b env with
      | some (.int x), some (.int y) => (.int ·) <$> chk (x * y)
      | _, _ => none
  | .div a b, env =>
      match eval a env, eval b env with
      | some (.int x), some (.int y) =>
          if y = 0 then none else some (.int (x.tdiv y))
      | _, _ => none
  | .eq a b, env =>
      match eval a env, eval b env with
      | some x, some y => some (.bool (veq x y))
      | _, _ => none
  | .optMap a k, env =>
      match eval a env with
      | some (.opt o) =>
          match o with
          | none => some (.opt none)
          | some v =>
              match eval (k v) env with
              | none => none
              | some w => some (.opt (some w))
      | _ => none
  | .coalesce a d, env =>
      match eval a env with
      | some (.opt o) =>
          match o with
          | some v => some v
          | none => eval d env
      | _ => none

/-!
## Theorems

Each theorem restates one numbered rule over the fragment's well-typed
expressions (rex-expr is statically checked: `+`/`/` see `int` operands,
`?.`/`?:` see optional receivers — mismatched shapes are type errors,
outside a runtime semantics). Together they are the paper semantics'
version of `docs/EXPRESSIONS.md` R1–R4.
-/

/-- **R1**: checked addition traps exactly on overflow — never wraps,
never saturates. -/
theorem eval_add_traps_iff_overflow {a b : Expr} {env : Env} {x y : Int}
    (ha : eval a env = some (.int x)) (hb : eval b env = some (.int y)) :
    eval (.add a b) env = none ↔ ¬inI32 (x + y) := by
  simp only [eval, ha, hb, chk]
  cases h' : inI32 (x + y) <;> simp

/-- **R1**: when both operands evaluate, the sum never traps unless it
overflows — the "plain `+` with a panic contract" lowering discipline. -/
theorem eval_add_no_trap_without_overflow {a b : Expr} {env : Env} {x y : Int}
    (ha : eval a env = some (.int x)) (hb : eval b env = some (.int y))
    (h : inI32 (x + y)) :
    eval (.add a b) env = some (.int (x + y)) := by
  simp [eval, ha, hb, chk, h]

/-- **R2**: equality is total — it never traps and always yields a
boolean. -/
theorem eval_eq_is_total {a b : Expr} {env : Env} (ha : eval a env ≠ none)
    (hb : eval b env ≠ none) :
    ∃ v, eval (.eq a b) env = some v := by
  cases ha' : eval a env with
  | none => exact absurd ha' ha
  | some va =>
    cases hb' : eval b env with
    | none => exact absurd hb' hb
    | some vb => exact ⟨.bool (veq va vb), by simp [eval, ha', hb']⟩

/-- **R2**: null-safety — `null == null` is `true`. -/
theorem veq_null_null : veq (.opt none) (.opt none) = true := by rfl

/-- **R2**: null-safety — `null == some v` is `false`, symmetrically. -/
theorem veq_null_some (v : Value) :
    veq (.opt none) (.opt (some v)) = false ∧
    veq (.opt (some v)) (.opt none) = false := by
  constructor <;> rfl

/-- The equality operator itself is symmetric (`beq` for any lawful
`BEq` type — `Int`, `String`, `Bool` all qualify). -/
theorem beq_symm {α : Type} [BEq α] [LawfulBEq α] (x y : α) :
    (x == y) = (y == x) := by
  cases hb : (x == y) with
  | true =>
      have hxy : x = y := eq_of_beq hb
      subst hxy
      simp
  | false =>
      cases hc : (y == x) with
      | false => rfl
      | true =>
          have hyx : y = x := eq_of_beq hc
          subst hyx
          simp at hb

/-- **R2**: equality is symmetric on every value, options included
(element-wise, recursively). -/
theorem veq_symmetric (x y : Value) : veq x y = veq y x := by
  induction x, y using veq.induct with
  | case1 v w => exact beq_symm v w
  | case2 v w => exact beq_symm v w
  | case3 v w => exact beq_symm v w
  | case4 => rfl
  | case5 v w ih => exact ih
  | case6 =>
      next a b hint hstr hbool hnn hsos =>
      cases a with
      | int v =>
          cases b with
          | int w => exact (hint v w rfl rfl).elim
          | _ => simp [veq]
      | str v =>
          cases b with
          | str w => exact (hstr v w rfl rfl).elim
          | _ => simp [veq]
      | bool v =>
          cases b with
          | bool w => exact (hbool v w rfl rfl).elim
          | _ => simp [veq]
      | opt o1 =>
          cases o1 with
          | none =>
              cases b with
              | opt o2 =>
                  cases o2 with
                  | none => exact (hnn rfl rfl).elim
                  | some _ => rfl
              | _ => simp [veq]
          | some v =>
              cases b with
              | opt o2 =>
                  cases o2 with
                  | none => rfl
                  | some _ => exact (hsos v _ rfl rfl).elim
              | _ => simp [veq]

/-- **R4**: division traps exactly when the divisor evaluates to zero —
never otherwise (trapping operands are outside the well-typed
fragment's live paths, and are covered by `eval_eq_is_total`'s shape). -/
theorem eval_div_traps_iff_zero_divisor {a b : Expr} {env : Env} {x y : Int}
    (ha : eval a env = some (.int x)) (hb : eval b env = some (.int y)) :
    eval (.div a b) env = none ↔ y = 0 := by
  simp only [eval, ha, hb]
  by_cases h' : y = 0 <;> simp [h']

/-- **R4**: with a nonzero divisor division never traps and the result is
truncating division (Rust/C-family semantics: rounds toward zero). -/
theorem eval_div_truncating {a b : Expr} {env : Env} {x y : Int}
    (ha : eval a env = some (.int x)) (hb : eval b env = some (.int y))
    (hne : y ≠ 0) :
    eval (.div a b) env = some (.int (x.tdiv y)) := by
  simp [eval, ha, hb, hne]

/-- **R3**: `?.` propagates absence — a `None` receiver yields `None`
without evaluating the continuation (the lookup cannot trap on
nothing). -/
theorem eval_optMap_propagates_none {a : Expr} {k : Value → Expr} {env : Env}
    (ha : eval a env = some (.opt none)) :
    eval (.optMap a k) env = some (.opt none) := by
  simp [eval, ha]

/-- **R3**: propagation is compositional — `?.` traps exactly when the
continuation traps on the unwrapped value; a `None` receiver never
traps. -/
theorem eval_optMap_traps_iff {a : Expr} {k : Value → Expr} {env : Env}
    {o : Option Value} (ha : eval a env = some (.opt o)) :
    eval (.optMap a k) env = none ↔ ∃ v, o = some v ∧ eval (k v) env = none := by
  simp only [eval, ha]
  cases o with
  | none => simp
  | some v => cases hk : eval (k v) env <;> simp [hk]

/-- **R3**: coalescing with an evaluating default never traps. -/
theorem eval_coalesce_no_trap {a d : Expr} {env : Env}
    (ha : ∃ o, eval a env = some (.opt o)) (hd : eval d env ≠ none) :
    ∃ v, eval (.coalesce a d) env = some v := by
  obtain ⟨o, ha⟩ := ha
  cases o with
  | some v => exact ⟨v, by simp [eval, ha]⟩
  | none =>
      cases hd' : eval d env with
      | none => exact absurd hd' hd
      | some w => exact ⟨w, by simp [eval, ha, hd']⟩

/-- **R3**: the coalesced value is the receiver's value when present. -/
theorem eval_coalesce_present {a d : Expr} {env : Env} {v : Value}
    (ha : eval a env = some (.opt (some v))) :
    eval (.coalesce a d) env = some v := by
  simp [eval, ha]

/-- The equality operator itself is symmetric — restated from
`veq_symmetric` so `#print axioms` covers it in one place. -/
example : ∀ (x y : Value), veq x y = veq y x := veq_symmetric
