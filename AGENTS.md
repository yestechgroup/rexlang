# AGENTS.md

Rust cargo workspace. `.mox` modeling sources compile to a Core IR artifact; backends (Rust codegen, JSON Schema) consume only that IR. Language reference: `docs/LANGUAGE.md`; expression spec: `docs/EXPRESSIONS.md`.

## Pipeline (crate ownership)

`rex-syntax` (logos lexer, chumsky parser, **fmt**) → `rex-driver` (salsa resolve/validate/lower, navigation index, diagnostics) → `rex-ir` (Core IR = the product) → `rex-backend-rust` / `rex-backend-jsonschema`. Plus `rex-expr` (expression parser/typechecker), `rex-vocab` (providers/lockfile), `rex-runtime` (generated code's support lib), `rex-lsp`, `rex-cli` (the `rexlang` binary).

- Backends must consume **only** `rex_ir::Model` — never the AST. Lowering happens in `rex-driver`.
- The serialized IR is a versioned wire format: camelCase, adjacent tagging, `formatVersion` gate. Contract and rules live in the `rex-ir` crate docs. New fields must be `#[serde(default)]` (+ `skip_serializing_if`) so body-less/vocabulary-less models stay **byte-identical** — golden tests enforce this.

## Commands

```sh
cargo test --workspace                          # full suite (~430 tests)
cargo test -p rex-driver --test navigation      # one test suite
cargo clippy --workspace --all-targets          # must be ZERO warnings
cargo fmt --all                                 # rustfmt
cargo run -p rex-cli -- fmt --check tests/conformance/models/*.mox   # .mox format gate
REX_UPDATE_FIXTURES=1 cargo test --workspace    # regenerate golden files
```

CI (`.github/workflows/ci.yml`) runs build, tests, `clippy -- -D warnings`, then **both** format gates: `cargo fmt --all -- --check` **and** `rexlang fmt --check` on the canonical fixtures. A warning anywhere is a failure.

## Testing quirks

- **Golden fixtures** under `tests/conformance/` (IR artifacts, wire/api schemas, instances, fmt output) are byte-compared. Regenerate with `REX_UPDATE_FIXTURES=1`, then read the diff — goldens are the spec. Changing any emitter usually invalidates several goldens; body-less/vocabulary-less models must stay byte-identical.
- **Scratch-crate e2e** (`rex-backend-rust/tests/codegen_end_to_end.rs`): tests generate Rust code into a *separate cargo project* under `target/scratch/` and run `cargo test` in it (tries `--offline` first). Deleting `target/scratch/` is always safe; first run recompiles it (~10s). The scratch crate must keep `[workspace]` (empty) to stay out of ours.
- **No network in tests**: `HttpProvider` is only URL-template unit-tested. `rexlang vocab fetch` is the only command that fetches.
- **LSP tests are in-process** via `LspService` + client socket (`tower-lsp` 0.20); the only subprocess test is the framed stdio handshake in `rex-cli`.
- Wire-schema conformance validation uses the `jsonschema` crate against the committed golden schema; proptest suites (rex-backend-rust scratch, rex-backend-jsonschema) generate instances and validate round-trips — treat shrinking counterexamples as real bugs, not flaky tests.

## Gotchas

- `rex-syntax` permits `unsafe` (documented in its Cargo.toml): chumsky 0.10's `Input` trait requires `unsafe fn` implementations. Don't "fix" this; don't copy that custom `Input` impl elsewhere — `rex-expr` deliberately uses chumsky `Stream` + `.boxed()` levels instead (unboxed towers overflow the 2 MB test-thread stack).
- Comments: `lex()` drops comments for the parser; `lex_with_comments()` is what `fmt` uses. Parser spans are **byte offsets**; LSP maps them to UTF-16 (`rex-lsp/src/position.rs`).
- `id`/`readonly` are contextual identifiers, not keywords — they remain usable as feature names.
- `rust` op bodies are emitted **verbatim** into the generated method; an `expr` body is parsed/typed/lowered at generate time (generate-time errors name the operation). No `rust` body ⇒ no generated method.
- Salsa session pattern (driver): `Database::new()` (Clone), `SourceFile::new(&db, path, text)`, `file.set_text(&mut db).to(...)`; tracked fns return rich `Clone + PartialEq` types (no Hash needed).
- `docs/EXPRESSIONS.md` rules R1–R4 are test-citable contracts (overflow, null-safe equality, Option propagation, division); lowering changes must keep them aligned.
