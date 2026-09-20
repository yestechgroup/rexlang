# AGENTS.md

Rust cargo workspace. `.mox` modeling sources compile to a Core IR artifact; backends (Rust codegen, JSON Schema) consume only that IR. Language reference: `docs/LANGUAGE.md`; expression spec: `docs/EXPRESSIONS.md`.

## Pipeline (crate ownership)

`rex-syntax` (logos lexer, chumsky parser, **fmt**) → `rex-driver` (salsa resolve/validate/lower, navigation index, diagnostics) → `rex-ir` (Core IR = the product) → `rex-backend-rust` / `rex-backend-jsonschema` / `rex-backend-cedar`. Plus `rex-expr` (expression parser/typechecker), `rex-ifml` (IFML interaction-flow DSL: Pest parser lowering `.ifml` sources directly to the versioned `rex_ir::ifml::IfmlModel` artifact; docs/IFML.md), `rex-vocab` (providers/lockfile), `rex-runtime` (generated code's support lib), `rex-lsp`, `rex-cli` (the `rexlang` binary). `.actor` policy files (standalone `import` + `actors` surface) compile via `compile_actors_str` to a separate `rex_ir::ActorModel` artifact that feeds `rex-backend-cedar` as the policy dimension alongside the domain `Model`.

- Backends must consume **only** `rex_ir::Model` (and, for Cedar, the `ActorModel` pair) — never the AST. Lowering happens in `rex-driver`.
- The serialized IR is a versioned wire format: camelCase, adjacent tagging, `formatVersion` gate. Contract and rules live in the `rex-ir` crate docs. New fields must be `#[serde(default)]` (+ `skip_serializing_if`) so body-less/vocabulary-less models stay **byte-identical** — golden tests enforce this.

## Commands

```sh
cargo test --workspace                          # full suite (~600 tests)
cargo test -p rex-driver --test navigation      # one test suite
cargo clippy --workspace --all-targets          # must be ZERO warnings
cargo fmt --all                                 # rustfmt
cargo run -p rex-cli -- fmt --check tests/conformance/models/*.mox tests/conformance/models/*.actor   # fixture format gate
REX_UPDATE_FIXTURES=1 cargo test --workspace    # regenerate golden files
```

CI (`.github/workflows/ci.yml`) runs build, tests, `clippy -- -D warnings`, then **both** format gates: `cargo fmt --all -- --check` **and** `rexlang fmt --check` on the canonical fixtures (`.mox` and `.actor`). A warning anywhere is a failure.

## Testing quirks

- **Golden fixtures** under `tests/conformance/` (IR artifacts, wire/api schemas, instances, fmt output) are byte-compared. Regenerate with `REX_UPDATE_FIXTURES=1`, then read the diff — goldens are the spec. Changing any emitter usually invalidates several goldens; body-less/vocabulary-less models must stay byte-identical.
- **Scratch-crate e2e** (`rex-backend-rust/tests/codegen_end_to_end.rs`): tests generate Rust code into a *separate cargo project* under `target/scratch/` and run `cargo test` in it (tries `--offline` first). Deleting `target/scratch/` is always safe; first run recompiles it (~10s). The scratch crate must keep `[workspace]` (empty) to stay out of ours.
- **Examples are CI-tested** (`examples/`): `examples_conformance` compiles each example, generates it into `target/scratch/rex-examples-test`, and runs behavioral assertions; `examples_schemas` validates `examples/instances/*.json` against generated wire schemas; `examples_cedar` (rex-backend-cedar) validates the `support` pair's Cedar output with the real `cedar-policy` crate under strict validation. Adding an example means adding its scratch assertions + instance — the harness fails otherwise. The example list is hardcoded in three places: `examples_flow.rs`, `examples_schemas.rs`, and `rex-cli/tests/cli.rs`.
- **No network in tests**: `HttpProvider` is only URL-template unit-tested. `rexlang vocab fetch` is the only command that fetches.
- **LSP tests are in-process** via `LspService` + client socket (`tower-lsp` 0.20); the only subprocess test is the framed stdio handshake in `rex-cli`.
- Wire-schema conformance validation uses the `jsonschema` crate against the committed golden schema; proptest suites (rex-backend-rust scratch, rex-backend-jsonschema) generate instances and validate round-trips — treat shrinking counterexamples as real bugs, not flaky tests.

## Gotchas

- `rex-syntax` permits `unsafe` (documented in its Cargo.toml): chumsky 0.10's `Input` trait requires `unsafe fn` implementations. Don't "fix" this; don't copy that custom `Input` impl elsewhere — `rex-expr` deliberately uses chumsky `Stream` + `.boxed()` levels instead (unboxed towers overflow the 2 MB test-thread stack).
- Comments: `lex()` drops comments for the parser; `lex_with_comments()` is what `fmt` uses. Parser spans are **byte offsets**; LSP maps them to UTF-16 (`rex-lsp/src/position.rs`).
- `id`/`readonly` are contextual identifiers, not keywords — they remain usable as feature names. Actor words (`actors actor capability grant permit forbid when obligation on never_both cedar import`) ARE real keywords, escapable with `^`.
- `when` conditions are **fully type-checked** against the capability's class (single-file and `.actor` compiles) via rex-expr; the Cedar backend separately rejects constructs it cannot map at generate time (errors name the capability). Multiple obligations on one grant entry serialize as ONE Cedar annotation with comma-joined values (Cedar rejects duplicate annotation keys).
- `cedar-policy` is a **dev-dependency of rex-backend-cedar only** — the backend never links Cedar. It enables serde_json's `preserve_order`, which unifies workspace-wide and flips `serde_json::Map` to insertion order; emitters must canonicalize key order themselves (see the jsonschema backend's `canonical_key_order`) or goldens drift.
- `rexlang fmt` dispatches by file extension (`.mox` vs `.actor` formatters); stdin is treated as `.mox`.
- `.actor` import resolution lives in the CLI (paths relative to the `.actor` file, read from disk); the driver receives texts only and stays filesystem-free (`compile_actors_str`). Import-path strings must match provided domain paths exactly.
- `.mox` `import schema "<path>" as <Alias>` registers a **nominal, feature-less class** (alias, else the file stem) in the package namespace — JSON content is validated but never lowered into features (v1 is opaque). `schema` is a contextual keyword (only after `import`); the driver stays filesystem-free via `SchemaImports` keyed `(mox path, import path)` — the CLI resolves paths relative to the `.mox` file, `compile_str`/`compile_files` provide nothing (→ `imported schema '<path>' was not provided`), use `compile_str_with_imports`/`compile_files_with_imports`. `rexlang fmt` hoists the declarations into a section directly after `package`.
- `rust` op bodies are emitted **verbatim** into the generated method; an `expr` body is parsed/typed/lowered at generate time (generate-time errors name the operation). No `rust` body ⇒ no generated method.
- Salsa session pattern (driver): `Database::new()` (Clone), `SourceFile::new(&db, path, text)`, `file.set_text(&mut db).to(...)`; tracked fns return rich `Clone + PartialEq` types (no Hash needed).
- `docs/EXPRESSIONS.md` rules R1–R4 are test-citable contracts (overflow, null-safe equality, Option propagation, division); lowering changes must keep them aligned.
