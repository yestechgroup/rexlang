# rexlang

A multi-target modeling language inspired by [Eclipse Xcore](https://eclipse.dev/Xtext/documentation/305_xbase.html#xcore).

`.mox` sources compile to a **Core IR** (an Ecore-like structural metamodel) that is
serialized as a stable artifact. Code generators for multiple target languages
(Rust, JSON Schema, Cedar, ...) consume only that IR. A complementary
`.actor` policy dimension compiles to its own artifact and feeds the Cedar
backend. The language reference lives in
[docs/LANGUAGE.md](docs/LANGUAGE.md).

```
.mox source -> lexer/parser -> AST -> resolve & validate -> Core IR (.rex.json)
                                                                 |
                                     +----------------+---------+-----------+
                                     v                v                     v
                                rust backend   json-schema backend      cedar backend
                                                                      ^
.actor policy file -> imports + actors blocks -> resolve & check ----+-+
                                                    -> ActorModel (.actors.rex.json)
```

## Status

Front-end, Core IR, wire format, Rust backend, canonical JSON instances,
JSON Schema (wire/api), the Tier-2 expression language, hermetic
vocabularies, and the `.actor` authorization dimension with Cedar policy
generation — implemented and CI-enforced. C#/Java backends and filter/query
predicates are future work.

## Usage

```
rexlang check model.mox           # validate, render diagnostics
rexlang ir model.mox -o model.rex.json
rexlang gen rust model.mox -o src-gen/
rexlang gen json-schema model.mox --profile wire -o schemas/
rexlang gen cedar model.mox -o policies/      # from inline actors blocks
rexlang gen cedar policy.actor -o policies/   # from a standalone actor file
rexlang vocab fetch model.mox --provider file:vocab-sources/
rexlang lsp                       # start the language server (stdio)
```

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

