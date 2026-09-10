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
rexlang check model.mox        # validate, render diagnostics
rexlang ir model.mox -o model.rex.json
rexlang gen rust model.mox -o src-gen/
```
