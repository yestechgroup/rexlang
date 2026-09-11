# The rexlang expression language (Tier 2)

rexlang's expression sublanguage is the deliberate replacement for Eclipse
Xbase inside `.mox` bodies and derived-feature expressions. It is deliberately
small: literals, field/feature access, method calls on model-declared
operations, comparison and arithmetic, `if`/`let`, and a **fixed collection
algebra** (`first`, `filter`, `map`, `any`, `size`, `sum`). There are **no
arbitrary host-library calls** — that is the discipline that keeps expressions
portable across backends.

The pipeline is: parse (`rex-expr`) → type-check once in Rust against the Core
IR (`rex-expr`, over a `rex_ir::Model`) → each backend lowers the **typed**
expression tree to idiomatic target code. Because backends see typed trees,
semantics must be pinned down explicitly where target languages disagree —
integer overflow, string equality, null/`Option`, and division by zero are the
classic four. Those are rules **R1–R4** below; they are numbered so tests and
backends can cite them.

Status: this document is the normative spec for the expression *surface and
typing* (the front half). Backend lowering rules land with each backend.

## Lexical rules

- **Comments**: `//` to end of line, `/* ... */` block comments.
- **Strings**: double-quoted with `\"` and `\\` escapes (same as `.mox`).
- **Integers**: decimal digits, non-negative in the token stream; a leading
  `-` is the unary minus *operator*, never part of the literal. A literal that
  does not fit in `i64` is a lex error.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`. `if else let true false null` are
  reserved keywords. `first filter map any size sum` are **contextual**: they
  name the collection algebra only in `name("...")` position after a `.`
  (or `?.`), so a model feature may freely be called `size` or `first`.
- **Operators**: `== = != < <= > >= + - * / && || !` and the navigation
  operators `.` `?.` `?:`, plus `=>` (lambda arrow) and `;` (let separator).

## Grammar (informal EBNF)

```
expr        := let_expr | lambda_expr | coalesce
let_expr    := "let" NAME "=" expr ";" expr
lambda_expr := NAME "=>" expr
coalesce    := logic_or ("?:" logic_or)*                -- left-associative
logic_or    := logic_and ("||" logic_and)*
logic_and   := comparison ("&&" comparison)*
comparison  := additive cmp_op additive)*               -- left-associative chain
cmp_op      := "==" | "=" | "!=" | "<" | "<=" | ">" | ">="
additive    := multiplicative (("+" | "-") multiplicative)*
multiplicative := unary (("*" | "/") unary)*
unary       := ("!" | "-") unary | postfix
postfix     := primary ((("." | "?.") member))*
member      := NAME ("(" args? ")")?
args        := expr ("," expr)*
primary     := INT | STRING | "true" | "false" | "null" | NAME
             | "(" expr ")"
             | "[" (expr ("," expr)*)? "]"
             | "if" expr "{" expr "}" "else" "{" expr "}"
```

Notes:

- `let`, lambdas, and `?:` chains appear at the top level of an expression or
  inside parentheses/brackets/argument lists. A lambda body extends as far
  right as possible; parenthesize to use a lambda as a binary operand.
- `==` is the canonical equality operator. A single `=` is accepted where an
  operator is expected and means exactly the same thing (see R2); it cannot be
  confused with the `=` of `let`, which follows the `let NAME` prefix.
- `NAME ("(" args? ")")` after `.` is a call. If `NAME` is one of the six
  algebra names, the argument shape is validated syntactically (see the
  algebra table): `first` takes an optional lambda, `filter`/`map`/`any`
  require exactly one lambda, `size`/`sum` take none. Any other `NAME` with
  parentheses is an operation call on a model-declared `op`.
- A bare `NAME` is a variable reference: a `let` binding or a lambda
  parameter. Feature access always has an explicit receiver.

### Precedence (loosest to tightest)

| Level | Operators | Associativity |
|---|---|---|
| `let` / lambda | prefix forms | body maximal |
| coalesce | `?:` | left |
| logic or | `||` | left |
| logic and | `&&` | left |
| comparison | `==` `=` `!=` `<` `<=` `>` `>=` | left |
| additive | `+` `-` | left |
| multiplicative | `*` `/` | left |
| unary | `!` `-` | prefix |
| postfix | `.` `?.` calls, algebra | left |

`if` is a primary: its condition and both branche bodies are full
expressions.

## Type system

Types (`Ty`):

- **Primitives**: `string int long short float double boolean byte char`
  (mirroring `rex_ir::PrimitiveType`).
- **Named types**: class, enum, datatype, interface, vocabulary — always
  package-qualified in the checker (`Named { kind, package, name }`), because
  they come from a resolved `rex_ir::Model`.
- `List<T>` — a to-many feature value or algebra result.
- `Option<T>` — an optional (0..1) feature value; also the result of `?.`
  chains and `first`.
- **Null** — the type of the `null` literal. It is not `Option<T>`; it exists
  so `null` can participate in equality (R2) and `?:` (R3) and nowhere else.

Booleans are `boolean` (`true`/`false` literals). Comparisons yield
`boolean`.

**L1 (integer-literal polymorphism).** An integer literal is typed `int` by
default. In a context whose expected type is `long` — the other operand of an
arithmetic/comparison operator, an operation argument, or a list element base
— an integer literal is typed `long` instead (its value always fits `i64`).
Everywhere else, and for everything that is not an integer *literal*, there
are **no implicit conversions**: in particular there is no implicit
`int → long` (see U1). A literal typed `int` whose value does not fit in 32
bits is an R1 constant-overflow error.

**L2 (numeric operand rule).** `+ - * /` require both operands to have the
*same* numeric type and produce it. `< <= > >=` require the same numeric type
and produce `boolean`. `&& || !` operate on `boolean` only; unary `-` takes a
numeric. There is no ordering on `string`; strings support only equality.

**U1 (if unification).** `if` requires a `boolean` condition. Both branches
must have the *same* type — compared exactly, with no implicit `int → long`
and no literal adaptation. The `if` expression's type is that type.

**Feature typing** (mirrors the resolver's IR):

- `attribute`/`derived`: to-many (`is_many`) → `List(T)`; lower bound 0 →
  `Option(T)`; otherwise `T` (the declared type).
- `contains`/`refers`: to-many → `List(Named)`; single-valued → `Named`
  (never `Option` — a single containment/reference is always present).
- `container`: always `Option(Named)`, whatever the declared multiplicity.
- `op` call: the declared return type, after argument checking (arity and
  per-parameter types; a class-typed parameter expects that `Named` type).
  L1 applies to arguments.

Unknown features, unknown operations, and unknown names are type errors whose
diagnostic span points at the offending name token.

**Navigation.** Accessing a feature or operation on a receiver whose type is
`Option(T)` without `?.` is a type error (R3). `?.` unwraps an optional
receiver for the access and makes the *whole access* yield `Option` of what
the access would have yielded, wrapping only if the result is not already
`Option`. Collection algebra requires a `List` receiver (`List` is never
optional in this typing: a to-many feature is `List(T)` even when its lower
bound is 0).

## The four semantic rules

These four are where target languages disagree. They are **normative**: every
backend lowering a typed expression tree must preserve them, and the
type-checker enforces their compile-time halves. Each rule is cited by tests
as `R1`–`R4`.

### R1 — INTEGER OVERFLOW

Arithmetic on `int`/`long` is **checked**: overflow is a **runtime panic
contract**. The lowered Rust backend uses plain `+`/`-`/`*` (Rust panics on
overflow in debug builds); **every other target must trap equivalently** —
never wrap, never saturate.

Compile-time half: constants that overflow are a **type-check error**. Two
integer constants combined by `+`/`-`/`*` are folded with checked arithmetic
and must fit the result type (`int` = 32-bit signed, `long` = 64-bit signed);
an `int`-typed literal whose value does not fit 32 bits is likewise rejected.
Unary minus folds too (`-i64::MIN` overflows). Non-constant overflow is a
runtime panic per the contract above; `long` constant pairs never fold in
practice because a literal only becomes `long` in a non-constant context (L1),
so `long` overflow remains purely a runtime contract.

### R2 — STRING EQUALITY

`==` (and its alias `=`) on `string` operands is **value equality**.
Equality is **null-safe**: `null == x` is `false` unless `x` is `null` (and
symmetrically); `null` compares with a value of any type.

`==` on `Option<T>` is **element-wise**: `Some(a) == Some(b)` iff `a == b`
(recursively, so nested options and strings compare by value);
`None == None` is `true`; `None == Some(_)` is `false`. `==`/`!=` yield
`boolean`.

Typing: both operands must have the same type (L1 literal adaptation
applies), or one of them must be the `null` literal. Comparing values of two
different named/primitive types is a type error.

### R3 — OPTION/NULL PROPAGATION

Navigating an optional feature (multiplicity `0..1`, or a `container`)
yields `Option<T>`.

- Calling a feature or operation on an **optional receiver without `?.`** is a
  **type error**.
- `?.` **propagates `None`**: `a?.b` evaluates to `None` when `a` is `None`,
  otherwise to the access result, wrapped in `Option` (unless it already is
  one, e.g. another `?.` hop or a `container` feature).
- `?: default` **coalesces**: `a ?: b` evaluates to `a` unless `a` is `None`,
  in which case it evaluates to `b`. Its type is the inner type `T` of
  `Option<T>` (or the right-hand type when the left is the `null` literal).
- A **to-many** `contains`/`refers` feature yields `List<T>` — the empty list
  is a valid value; there is no `Option` around a collection.

### R4 — DIVISION BY ZERO

Integer division by zero is a **runtime panic contract** — the same behavior
as Rust: `/` on `int`/`long` with a zero divisor panics; other targets must
trap equivalently. (Floating-point `/` follows IEEE semantics and never
panics.)

`/` on integers is **truncating** (rounds toward zero, the Rust/C family
behavior), for both `int` and `long`.

Compile-time half: a **constant zero divisor** (any expression that folds to
the integer constant `0`, including `-0`) is a **type-check error**.

## Collection algebra

The algebra is fixed — these six, and nothing else. Lowerings must emit the
target language's idiomatic equivalent; no other collection operations exist
in the language (no host-library calls, per the mandate).

| # | Form | Receiver | Lambda | Result | Notes |
|---|---|---|---|---|---|
| A1 | `x.first()` / `x.first(p => e)` | `List(T)` | optional; `T → boolean` | `Option(T)` | first matching / first element |
| A2 | `x.filter(p => e)` | `List(T)` | required; `T → boolean` | `List(T)` | order preserved |
| A3 | `x.map(p => e)` | `List(T)` | required; `T → U` | `List(U)` | any `U`, including `boolean` |
| A4 | `x.any(p => e)` | `List(T)` | required; `T → boolean` | `boolean` | `false` when empty |
| A5 | `x.size()` | `List(T)` | none | `int` | element count |
| A6 | `x.sum()` | `List(T)`, `T` numeric | none | `T` | `0` when empty |

Lambda parameters are single, untyped, and **inferred**: the parameter of a
lambda passed to an algebra call has the receiver's element type `T`. A lambda
outside an algebra call (where its parameter type cannot be inferred) is a
type error. Inside the lambda body the parameter shadows outer bindings of the
same name (shadowing is always allowed).

## Scoping and evaluation shape

- `let x = e1; e2`: `x` is bound to the value of `e1` while checking/evaluating
  `e2`. Shadowing an existing binding (including lambda parameters) is
  allowed, including shadowing with a different type.
- Expressions are total in the checker's model: every well-typed expression
  yields a value of its type or panics per R1/R4 — there is no third failure
  mode other than `Option` (R3).

## Errors

`rex_expr::parse` never panics and performs best-effort recovery like
`rex-syntax`: it returns the recovered expression (if any) plus every error
with byte spans. `TypeChecker::type_of` collects **all** type errors in one
pass (subexpression errors do not cascade into spurious operator errors).
