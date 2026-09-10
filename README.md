# rexlang

A multi-target modeling language inspired by [Eclipse Xcore](https://eclipse.dev/Xtext/documentation/305_xbase.html#xcore).

`.mox` sources compile to a **Core IR** (an Ecore-like structural metamodel) that is
serialized as a stable artifact. Code generators for multiple target languages
(Rust, C#, JSON Schema, ...) consume only that IR.

```
.mox source -> lexer/parser -> AST -> resolve & validate -> Core IR (.rex.json)
                                                                 |
                                    +----------------+-----------+----------+
                                    v                v                      v
                               rust backend     csharp backend          json-schema
```

## Status

Milestones 1-3 (front-end, Core IR, Rust backend, canonical JSON instances) — in progress.

## Usage

```
rexlang check model.mox           # validate, render diagnostics
rexlang ir model.mox -o model.rex.json
rexlang gen rust model.mox -o src-gen/
```

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

