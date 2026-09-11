# rexlang examples

The six canonical examples double as the repo's capability harness: the test
suites under `crates/rex-backend-rust/tests/examples_conformance.rs`,
`crates/rex-backend-jsonschema/tests/examples_schemas.rs`,
`crates/rex-backend-cedar/tests/examples_cedar.rs`, and
`crates/rex-cli/tests/cli.rs` (`examples_suite_checks_clean_and_is_fmt_canonical`)
compile every file below, generate code and schemas from it, execute the
generated Rust in a scratch cargo crate, and byte-check the canonical instance
documents in `instances/`. A missing or broken example fails the suite.

## Capability map

| Example | Package | Capabilities exercised | Tier-2 (`expr`) bodies |
|---|---|---|---|
| `library.mox` | `nz.example.library` | enum, opaque datatype with target bindings, containment/opposite, cross references, `op` with an `expr` body, `derived` feature | `op Book getBook(String title) { expr { books.first(b => b.title == title) } }` (`List(Book)` → `Option(Book)`); `derived String citation { expr { if pages > 400 { title } else { "short read" } } }` (U1 unification, both branches `string`) |
| `ecommerce.mox` | `nz.example.shop` | vocabulary (`iso:4217`, hermetic snapshot + `model.lock`), vocabulary-typed attribute, `id readonly` (no generated setter, schema `readOnly`), one-way-paired `refers`, containment lines, derived numeric pipeline | `derived int total { expr { lines.map(l => l.quantity * l.unitPrice).sum() } }` (A3 + A6, R1-checked arithmetic) |
| `org.mox` | `nz.example.org` | two containment pairs, `refers`↔`refers` pair, three-level hierarchy (Department → Team → Employee), navigation accessor both ways, predicate and aggregate operations | `op boolean any_member_earns_over(int threshold) { expr { members.any(m => m.salary > threshold) } }` (A4); `op int payroll() { expr { members.map(m => m.salary).sum() } }` (A3 + A6) |
| `iot.mox` | `nz.example.iot` | Tier-1 datatype bodies (`create`/`convert` embedded verbatim), labeled enum (`as "on" = 0`), readonly identity feature, derived display name | `derived String displayName { expr { if sensors.size() > 0 { name } else { "unprovisioned device" } } }` (A5 + U1) |
| `shapes.mox` | `nz.example.shapes` | `extends` inheritance: schema `allOf` with base `$type` enum and leaf closure; Rust codegen materializes **own features only** (declared, not inherited, structure) | — |
| `support.mox` | `nz.example.support` | actors model (`actors Support { ... }`): actor hierarchy via `extends`, capabilities on a class, `when` conditions over primitive attributes, obligations (incl. two on one entry), native `forbid`, the verbatim `cedar { ... }` escape hatch, `never_both` separation of duty; refers↔refers pair, readonly identity features, labeled enum state, derived boolean. Cedar output is validated against the real `cedar-policy` crate (strict mode) in `examples_cedar.rs` | `derived boolean needsRefund { expr { refundCents > 0 } }` |

Expression typing follows `docs/EXPRESSIONS.md`; in particular `+` is numeric
only (L2), so string assembly is expressed with `if` unification instead.

## Walking an example by hand

```sh
# validate (ecommerce resolves its vocabulary snapshot hermetically from
# examples/vocab/ and pins it against examples/model.lock)
rexlang check examples/ecommerce.mox

# emit the Core IR artifact
rexlang ir examples/library.mox -o /tmp/library.rex.json

# generate Rust models (arena + typed ids + mutators + canonical JSON)
rexlang gen rust examples/library.mox -o /tmp/library-rs

# generate both JSON Schema profiles
rexlang gen json-schema examples/library.mox --profile wire -o /tmp/library-wire
rexlang gen json-schema examples/library.mox --profile api   -o /tmp/library-api

# generate Cedar policies + schema from the actors example
rexlang gen cedar examples/support.mox -o /tmp/support-cedar

# canonical formatting (CI gates all of examples/ with --check)
rexlang fmt examples/org.mox
rexlang fmt --check examples/*.mox
```

Vocabulary example only — refresh the vendored snapshot and lockfile pin
(the only command that touches the network; compilation never does):

```sh
rexlang vocab fetch examples/ecommerce.mox
```

## Canonical instances

`instances/<name>.instance.json` pins the canonical exchange format for each
model (`$type`/`$id`/`$ref`, containment inlined, container features omitted,
enums by literal name, vocabulary attributes by entry key, derived features
never serialized). For `library`, `ecommerce`, `org`, and `support` the
documents are
byte-checked against the serialization of a mutator-built resource inside the
scratch crate (`REX_UPDATE_FIXTURES=1` regenerates them, same convention as
the `tests/conformance` goldens). `iot` and `shapes` are validated against
their wire schemas instead: `Sensor.sensorId` is `readonly`, so no generated
mutator can set it, and `shapes` inheritance is not materialized in Rust
codegen, so the Rust serialization intentionally covers own features only.
