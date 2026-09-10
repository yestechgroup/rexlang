# The .mox language reference

`rexlang` compiles `.mox` modeling sources (inspired by [Eclipse Xcore]) into a
resolved **Core IR** artifact, from which backends generate Rust models, JSON
Schemas, and canonical instance JSON. This document is the normative language
reference; crate docs cover implementation.

[Eclipse Xcore]: https://eclipse.dev/Xtext/documentation/305_xbase.html

## Lexical rules

- **Comments**: `//` to end of line, `/* ... */` block comments. Preserved
  verbatim by `rexlang fmt`.
- **Strings**: double-quoted with `\"` and `\\` escapes.
- **Integers**: decimal, optional leading `-`.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`. Any keyword can be escaped with a
  leading `^` (`^class` is an identifier).
- **Keywords**: `package annotation as class extends interface enum type wraps
  opaque contains refers container opposite op derived vocabulary from version
  key facet`. Contextual words usable as plain identifiers: `id readonly get
  set String int ...`.

## Model structure

A file holds one package and any number of declarations:

```
model        := package_decl (annotation_decl | class_decl | interface_decl
              | enum_decl | type_decl | vocabulary_decl)*
package_decl := "package" qualified_name
```

### Classes and features

```
class_decl  := "class" name ("extends" type_ref ("," type_ref)*)? "{" feature* "}"
feature     := modifier* ( attribute | containment | reference | container
                         | op_decl | derived_decl )
modifier    := "id" | "readonly"
attribute   := type_ref multiplicity? name ("=" default)?
containment := "contains" type_ref multiplicity? name ("opposite" name)?
reference   := "refers" type_ref multiplicity? name ("opposite" name)?
container   := "container" type_ref name ("opposite" name)?
op_decl     := "op" type_ref name "(" params? ")"
derived_decl:= "derived" type_ref multiplicity? name
multiplicity:= "[" (int (".." (int | "*"))?)? "]"
```

- **`contains`** — by-value ownership (Ecore containment). The child's
  container back-pointer is its `opposite`.
- **`refers`** — a cross reference: a link, serialized as `{"$ref": "id"}`.
- **`container`** — the owner slot; always `0..1`, never written with a
  multiplicity. Opposites are **mutual and validated**: `contains` pairs with
  `container`, `refers` with `refers`.
- **Modifiers**: `readonly` suppresses setters (and marks schema properties
  `readOnly`); `id` declares identity (carried; semantics land with Tier 1).
  Both warn when applied to `op`.
- **`op`** bodies are Tier 0-unsupported: operations are abstract hooks,
  implemented by hand in target code.
- **`derived`** features are computed, not stored; `get` bodies are Tier 2.
- **Defaults**: string/int/boolean literals or an enum literal name (for
  enum-typed attributes only).

Multiplicity shorthand: `[]` = `[0..*]`; absent: attributes are `1..1`,
`contains`/`refers` are `0..*`, `container` and `derived` are `0..1`.

### Enums, datatypes, interfaces, vocabularies

```
enum_decl    := "enum" name "{" literal+ "}"
literal      := name ("as" string)? ("=" int)?
type_decl    := "type" name "wraps" ("opaque" | qualified_name)? binding_block?
binding_block:= "{" (name string)* "}"
interface_decl := "interface" name "{" (name string)* "}"
vocabulary_decl := "vocabulary" name "from" string "{" ( version "string"
                  | key name | facet type_ref name )* "}"
```

- **Enums** carry integer values (required) and optional labels: `Mystery as
  "M" = 0`. Backends emit real enums; canonical JSON uses the literal name.
- **Datatypes** wrap platform types opaquely: `type Date wraps opaque { rust
  "chrono::NaiveDate" ... }`. Bindings are per-target hints, never generated
  dependencies.
- **Vocabularies** reference well-known external code sets. The declaration
  names the source, version, key facet, and typed facets. Snapshots are
  vendored to `vocab/<source>@<version>.json` and pinned by sha256 in
  `model.lock` — only `rexlang vocab fetch` touches the network; compilation
  is hermetic. Entries are embedded in the IR, and every backend materializes
  the vocabulary as a closed set of keys with facet accessors.

## Canonical instance JSON

The cross-language exchange format (`$type`/`$id`/`$ref`):

```json
{
  "$type": "rex.instance",
  "formatVersion": 1,
  "model": "nz.example.library",
  "objects": [
    { "$type": "Library", "$id": "library/0", "name": "Default Name",
      "books": [
        { "$type": "Book", "$id": "book/0", "title": "Dune",
          "authors": [ { "$ref": "writer/0" } ] }
      ]
    },
    { "$type": "Writer", "$id": "writer/0", "name": "Frank",
      "books": [ { "$ref": "book/0" } ] }
  ]
}
```

Rules: ids are `<singular>/<n>` in per-class insertion order; containment is
inlined; **container features are never serialized** (reconstructed from
nesting on load); cross references are `$ref` links; enums serialize the
literal name, datatypes their inner string; optional attributes are omitted
when unset; loading is strict (unknown features, serialized containers,
duplicate ids, unresolved refs are errors). `load(save(x)) == x` is a tested
property, not an aspiration.

## Tools

| Command | Purpose |
|---|---|
| `rexlang check <file>` | validate; ariadne-rendered diagnostics |
| `rexlang ir <file> -o <out>` | emit the Core IR artifact |
| `rexlang gen rust <file> -o <dir>` | arena-based Rust models |
| `rexlang gen json-schema <file> --profile wire\|api -o <dir>` | JSON Schema |
| `rexlang vocab fetch <file>` | vendor + pin vocabulary snapshots |
| `rexlang fmt [--check] <files>\|-` | canonical formatting (comments kept) |
| `rexlang lsp` | language server (stdio) |
