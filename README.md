# rexlang

A multi-target modeling language inspired by [Eclipse Xcore](https://eclipse.dev/Xtext/documentation/305_xbase.html#xcore).

`.mox` sources compile to a **Core IR** (an Ecore-like structural metamodel) that is
serialized as a stable artifact. Code generators for multiple target languages
(Rust, JSON Schema, Cedar, ...) consume only that IR. A complementary
`.actor` policy dimension compiles to its own artifact and feeds the Cedar
backend. The language reference lives in
[docs/LANGUAGE.md](docs/LANGUAGE.md); if you are writing a backend that
consumes the IR — in or out of tree — start with
[docs/BACKENDS.md](docs/BACKENDS.md).

```
.mox source  -> lexer/parser -> AST -> resolve & validate -> Core IR (.rex.json)
   ^                                                              |
   |                                   +----------------+--------+-----------+
   |                                   v                v                     v
   |                              rust backend   json-schema backend      cedar backend
   |                                                                      ^
   +-- import schema "x.json"    .actor policy file -> resolve & check ---+-+
   +-- import sigil "x.rosetta"                       -> ActorModel (.actors.rex.json)
                                    .ddd design file  -> resolve & validate
                                                    -> DddModel (.ddd.json)
                                    .ifml flow file   -> parse
                                                    -> IfmlModel (.ifml.json)
```

## Status

Front-end, Core IR, wire format, Rust backend, canonical JSON instances,
JSON Schema (wire/api), the Tier-2 expression language, hermetic
vocabularies, the `.actor` authorization dimension with Cedar policy
generation, the `.ddd` Sculptor-style design layer over imported domains,
the `.ifml` interaction-flow surface, and structural `import schema` /
`import sigil` (Rune DSL) imports — implemented and CI-enforced. C#/Java
backends and filter/query predicates are future work.

## Usage

```
rexlang check model.mox           # validate, render diagnostics
rexlang ir model.mox -o model.rex.json
rexlang artifact check model.rex.json   # validate any serialized artifact
rexlang gen rust model.mox -o src-gen/
rexlang gen json-schema model.mox --profile wire -o schemas/
rexlang gen cedar model.mox -o policies/      # from inline actors blocks
rexlang gen cedar policy.actor -o policies/   # from a standalone actor file
rexlang vocab fetch model.mox --provider file:vocab-sources/
rexlang lsp                       # start the language server (stdio)
```

## Consuming rexlang from other projects

There are two supported integration styles; both are consumed in anger by
sibling projects today.

**Style A — artifacts through the CLI (language-agnostic).** Each surface
compiles to a versioned JSON artifact with a stable wire contract (camelCase
keys, a `formatVersion` gate, additive-only evolution):
`rexlang ir model.mox -o model.rex.json` (Core IR), `policy.actor` →
`ActorModel`, `design.ddd` → `DddModel`, `flow.ifml` → `IfmlModel`.
`rexlang artifact check <files>` validates any serialized artifact without
the toolchain. Treat the artifact as the interface and any Rust version
becomes an implementation detail.

**Style B — library crates pinned by git rev (in-process).** Depend on the
crates the way [sigil](https://github.com/yestechgroup/sigil) embedders do —
a git dependency pinned to a revision:

```toml
[dependencies]
rex-ir     = { git = "https://github.com/yestechgroup/rexlang.git", rev = "<rev>" }
rex-driver = { git = "https://github.com/yestechgroup/rexlang.git", rev = "<rev>" }
rex-expr   = { git = "https://github.com/yestechgroup/rexlang.git", rev = "<rev>" }
rex-ifml   = { git = "https://github.com/yestechgroup/rexlang.git", rev = "<rev>" }
```

The crates are the compatibility boundary:

| Crate | Role for a consumer |
|---|---|
| `rex-ir` | The artifacts (`Model`, `ActorModel`, `ddd::DddModel`, `ifml::IfmlModel`) with `from_json`/`to_json` and their version gates — the type-level boundary |
| `rex-driver` | Compilation: one call per surface, rich byte-span diagnostics, ariadne `render` |
| `rex-expr` | The neutral expression language (parser, type checker) behind `when` conditions and `expr` bodies |
| `rex-ifml` | The `.ifml` parser (`parse_ifml`/`parse_ifml_file` → `IfmlModel`) |

Everything else (`rex-syntax`, the backends, `rex-vocab`, `rex-lsp`,
`rex-runtime`) is implementation detail for the compiler itself.

Entry points per surface — the driver is filesystem-free: sources and any
import content arrive as text:

```rust
// .mox (multi-package): import content provided by the host
let compilation = rex_driver::compile_files_with_imports(
    &sources,                                   // (path, source) pairs
    &schema_imports,                            // SchemaImports: import schema JSON
    &sigil_imports,                             // SigilImports: import sigil rosetta
);
// .actor — policy artifact + union domain model; imports bundle for the domains
let actor = rex_driver::compile_actors_str("policy.actor", &source, &domains, &imports);
// .ddd — design artifact; the _with_actors variant also validates capabilities
let design = rex_driver::compile_ddd_str_with_actors("design.ddd", &source, &domains, &imports, &actors);
// .ifml — parses directly, no domain needed
let ifml = rex_ifml::parse_ifml(&source)?;
```

`Compilation`/`ActorCompilation`/`DddCompilation` carry every file's
diagnostics tagged with its path (`sigil` content errors are tagged with the
rosetta path); `rex_driver::render(path, source, &diagnostics)` produces
ariadne output, and every diagnostic span is a byte offset into its file.
Hosts read `import schema` JSON, `import sigil` rosetta, and imported
domains from disk themselves — relative to the declaring file — and thread
them through `SchemaImports`/`SigilImports`/`DomainImports`.

Normative contracts: the language in [docs/LANGUAGE.md](docs/LANGUAGE.md),
the design layer in [docs/DDD.md](docs/DDD.md), interaction flows in
[docs/IFML.md](docs/IFML.md), expression rules in
[docs/EXPRESSIONS.md](docs/EXPRESSIONS.md), and the out-of-tree backend
contract in [docs/BACKENDS.md](docs/BACKENDS.md).

## Actor policies (.actor)

Authorization is a **separate dimension** from the domain model: actors,
capabilities, grants, obligations, and prohibitions never appear in the
domain IR or its JSON Schemas. They live in `.actor` files that import the
domain so everything stays type safe:

```
import "support.mox"

actors Support {
    actor Agent extends Customer
    capability RaiseRefund on Ticket

    grant Agent {
        permit RaiseRefund when (refundCents <= 5000) obligation audit
    }

    never_both { RaiseRefund, ApproveRefund }
}
```

- `when` conditions are **fully type-checked** against the imported classes
  (a typo'd feature is a compile error, not a silent Cedar mismatch).
- Policy checks run over the **union** of an `.actor` file's blocks and any
  inline `actors` blocks in the imported domain models: separation of duty
  (`never_both`), inheritance cycles, and self-narrowing are caught across
  files.
- `rexlang gen cedar` emits a Cedar policy set plus a Cedar entity schema;
  generated policies are validated against the real `cedar-policy` crate in
  CI (strict mode). Obligations become Cedar annotations; `never_both`
  groups become review evidence comments.
- Inline `actors` blocks in `.mox` files remain legal for small policies;
  both surfaces feed the same `ActorModel` artifact.


## Language server

`rexlang lsp` speaks LSP 3.x over stdio (tower-lsp). Any LSP client works;
capabilities:

- **Diagnostics** — full-document compile on open/change, incremental
  (salsa); the flagship diagnostic carries a machine-readable code with
  structured data
- **Quick fix** — on `feature 'x' has class type 'C'`: two actions inserting
  `contains` or `refers` before the type
- **Go to definition** — type references, `extends`, and **opposite
  mentions** (jumping from `opposite library` to `Book.library`, as in
  Xcore's F3)
- **Hover** — feature/class/enum/datatype/vocabulary summaries rendered from
  the AST
- **Document symbols**, **completion** (keywords, types in scope), and
  **rename** (bidirectionally consistent: renaming a feature rewrites the
  opposite mentions on the other side)

Example client config (VS Code `settings.json`):

```json
{
  "mox.server.path": "/path/to/rexlang",
  "mox.server.args": ["lsp"],
  "mox.server.languages": ["mox"]
}
```

## Vocabularies

Well-known external vocabularies are declared in the model, vendored once,
pinned by digest, and materialized identically by every backend:

```
vocabulary Currency from "iso:4217" {
    version "2024-01-01"
    key alpha3
    facet String symbol
    facet int minorUnits
}
```

- `rexlang vocab fetch` is the ONLY command that fetches: it writes
  `vocab/<source>@<version>.json` and pins `sha256` digests in `model.lock`.
- Compilation is **hermetic**: it reads only the vendored snapshot and verifies
  the lockfile digest; a missing or tampered snapshot is a diagnostic, never a
  network call.
- Backends materialize the vocabulary as a closed enum over its keys with
  facet accessors (Rust), an `enum` of keys (JSON Schema), and key strings in
  canonical instance JSON (`"currency": "USD"`).

## The canonical instance format

Model instances exchange through a cross-language JSON contract:

```json
{
  "$type": "rex.instance",
  "formatVersion": 1,
  "model": "nz.example.library",
  "objects": [
    {
      "$type": "Library",
      "$id": "library/0",
      "name": "Default Name",
      "books": [
        {
          "$type": "Book",
          "$id": "book/0",
          "title": "Dune",
          "authors": [ { "$ref": "writer/0" } ]
        }
      ]
    },
    { "$type": "Writer", "$id": "writer/0", "name": "Frank", "books": [ { "$ref": "book/0" } ] }
  ]
}
```

- Contained objects serialize **inline** under their containment feature;
  container back-references are **omitted** (derived from nesting, reconstructed
  on load — serializing them would recurse forever).
- Cross references are **links**: `{"$ref": "<id>"}`.
- Object ids are `<singular>/<n>` in per-class insertion order.
- Enums serialize the declared literal name; datatypes their inner string;
  optional attributes are omitted when unset; derived features are never
  serialized.
- Loading is strict: unknown features, serialized containers, duplicate ids,
  and unresolved refs are errors.

## Conformance

`tests/conformance/` holds the flagship model plus its golden artifacts (Core
IR JSON and canonical instance JSON). Every backend must round-trip them.
Regenerate goldens with `REX_UPDATE_FIXTURES=1 cargo test --workspace`.

