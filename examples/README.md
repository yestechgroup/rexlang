# rexlang examples

The canonical examples double as the repo's capability harness: the test
suites under `crates/rex-backend-rust/tests/examples_conformance.rs`,
`crates/rex-backend-jsonschema/tests/examples_schemas.rs`,
`crates/rex-backend-cedar/tests/examples_cedar.rs`,
`crates/rex-driver/tests/examples_artifacts.rs`, and
`crates/rex-cli/tests/cli.rs` (`examples_suite_checks_clean_and_is_fmt_canonical`)
compile every file below, generate code and schemas from the domains, execute
the generated Rust in a scratch cargo crate, byte-check the canonical instance
documents in `instances/` and the compiled artifacts in `artifacts/`, and
validate the Cedar output with the real `cedar-policy` crate. A missing or
broken example fails the suite.

## Capability map

### Domain surfaces (`.mox` / `.actor` / `.ifml`)

| Example | Package | Capabilities exercised | Tier-2 (`expr`) bodies |
|---|---|---|---|
| `library.mox` | `nz.example.library` | enum, opaque datatype with target bindings, containment/opposite, cross references, `op` with an `expr` body, `derived` feature | `op Book getBook(String title) { expr { books.first(b => b.title == title) } }` (`List(Book)` → `Option(Book)`); `derived String citation { expr { if pages > 400 { title } else { "short read" } } }` (U1 unification, both branches `string`) |
| `ecommerce.mox` | `nz.example.shop` | vocabulary (`iso:4217`, hermetic snapshot + `model.lock`), vocabulary-typed attribute, `id readonly` (no generated setter, schema `readOnly`), one-way-paired `refers`, containment lines, derived numeric pipeline | `derived int total { expr { lines.map(l => l.quantity * l.unitPrice).sum() } }` (A3 + A6, R1-checked arithmetic) |
| `org.mox` | `nz.example.org` | two containment pairs, `refers`↔`refers` pair, three-level hierarchy (Department → Team → Employee), navigation accessor both ways, predicate and aggregate operations | `op boolean any_member_earns_over(int threshold) { expr { members.any(m => m.salary > threshold) } }` (A4); `op int payroll() { expr { members.map(m => m.salary).sum() } }` (A3 + A6) |
| `iot.mox` | `nz.example.iot` | Tier-1 datatype bodies (`create`/`convert` embedded verbatim), labeled enum (`as "on" = 0`), readonly identity feature, derived display name | `derived String displayName { expr { if sensors.size() > 0 { name } else { "unprovisioned device" } } }` (A5 + U1) |
| `shapes.mox` | `nz.example.shapes` | `extends` inheritance: schema `allOf` with base `$type` enum and leaf closure; Rust codegen materializes **own features only** (declared, not inherited, structure) | — |
| `support.mox` | `nz.example.support` | domain of the support stack (below): refers↔refers pair, readonly identity features, labeled enum state, derived boolean, refund-ceiling operations | `derived boolean needsRefund { expr { refundCents > 0 } }`; `op boolean raiseRefund(int cents) { expr { refundCents + cents <= 5000 } }` and the finance ceiling twin `approveRefund` |
| `support.actor` | imports `support.mox` | standalone actor-policy file (`import "support.mox"` + `actors Support { ... }`): actor hierarchy via `extends`, capabilities on a class, `when` conditions over primitive attributes, obligations (incl. two on one entry), native `forbid`, the verbatim `cedar { ... }` escape hatch, `never_both` separation of duty. Cedar output is validated against the real `cedar-policy` crate (strict mode) in `examples_cedar.rs` | `permit RaiseRefund when (refundCents > 0 && refundCents <= 5000) obligation audit obligation notifyCustomer` |
| `support.ifml`, `ecommerce.ifml` | imports `support.mox` / `ecommerce.mox` | interaction flows over the pattern library (`patterns/ifml`): views, landmark, list/table/form components, guards, module `use` composition, typed expressions checked against the domain union (pinned by `crates/rex-ifml/tests/examples.rs`) | `column "Refund" -> expr refundCents / 100` |

### Artifact surfaces (`.evt` / `.ddd` / `.uc` / `.deploy` / `.cmmn`)

The support stack is the full vertical: every layer is a complementary
artifact over the same domain, and `support.uc` imports all four dimensions
at once. Compiled artifacts are byte-pinned under `artifacts/` by
`crates/rex-driver/tests/examples_artifacts.rs`.

| Example | Imports | Capabilities exercised |
|---|---|---|
| `support.evt` | `support.mox` | event contracts with `version` clauses, enum-typed payload fields (`state: TicketState`), two channels publishing overlapping event sets, subscriptions with one consumer each |
| `ecommerce.evt` | `ecommerce.mox` | **vocabulary-typed payload field** (`currency: Currency` resolves into the imported `iso:4217` vocabulary), class-ref payload fields, per-consumer subscriptions |
| `support.ddd` | `support.mox` | two modules; `entity Ticket scaffold cache repository` (scaffold ⇒ repository rule); all built-in repository operations plus a declared finder and a `protected` operation; `value RefundDecision nonPersistent` (the all-readonly value contract); `dto User nonPersistent`; service `inject` + delegations (same-module → repository, → search's virtual `search`); `capability` clauses naming real `support.actor` capabilities (validated by `compile_ddd_str_with_actors` in the harness); `search` projection: text boost, filters, sort, document expressions (U1 `if` unification), `ranking`, `analyzer`, `pagination` with cursor |
| `orders.ddd` | `ecommerce.mox` | the **aggregate-derivation rule**: `OrderLine` is contained by `Order`, so its entity design may declare no repository while the root's does; cross-module delegation service → service only (never a foreign repository); vocabulary-typed signature (`Currency` parameter); two-module split with per-module repositories |
| `support.uc` | `support.mox`, `support.ddd` (aliased), `support.actor`, `support.evt` | the four pattern iterations: subject + bare policy actor (1), named occurrences + fully anchored message (2a), message-owned ends (2b), and steps + `requires [RaiseRefund]` + event payload (`of RefundRequested`) + domain payload (`of RefundDecision`) + `realizes support.TicketService.raiseRefund` (3); typed domain actor (`Reporter: User`); the `never_both` pair kept in separate use cases (`RaiseRefund` / `ApproveRefund`); actors-only `MonitorQueue` |
| `support.deploy` | `support.ddd`, `support.ifml` | one logical application (`api` bound to `design "support.ddd#desk"`, `frontend` bound to `flow "support.ifml"`, `database`), `connects`; two profiles on different targets — kubernetes (postgres, resource `mappings`, `require`/`prohibit` policies) and cloudflareWorkers (D1 engine default, edge `runtime: worker`); `configure` override beating profile defaults; the same application projected onto both targets because nothing in it requires a capability the edge lacks |
| `support.cmmn` | `support.mox`, `support.evt`, `support.actor`, `support.ddd` (aliased) | the case layer: roles bound to `.actor` actors, a typed case file (`Ticket[1..1]`, `RefundDecision[0..1]`), entry on a case-file transition, `when (decision == null)` typed against the synthetic case universe, a service task whose `execute` binds the designed `TicketService.raiseRefund` (the engine's executor-seam `processRef`), `required;`, two discretionary tasks → a planning table, a milestone, and an exit criterion — lowered by `rex-backend-cmmn` to the cmmn-rs `CaseDefinition` JSON pinned at `artifacts/support.case.json` |

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
rexlang gen cedar examples/support.actor -o /tmp/support-cedar

# validate an artifact surface and emit its versioned artifact
rexlang check examples/support.uc
rexlang ir examples/support.uc -o /tmp/support.uc.json
rexlang artifact check /tmp/support.uc.json
rexlang ir examples/support.deploy -o /tmp/support.deploy.json

# the artifact surfaces: compile, emit, and lint-check the artifact
rexlang ir examples/support.uc -o /tmp/support.uc.json
rexlang artifact check /tmp/support.uc.json
rexlang ir examples/support.deploy -o /tmp/support.deploy.json
rexlang ir examples/support.cmmn -o /tmp/support.cmmn.json

# the cmmn-rs seam: the case artifact lowers to the engine's
# CaseDefinition JSON (see artifacts/support.case.json for the pinned bytes)
rexlang ir examples/support.cmmn -o /tmp/support.cmmn.json
rexlang artifact check /tmp/support.cmmn.json

# the sigil import: the Rune namespace lowers to a synthetic package
rexlang check examples/pricing.mox
rexlang ir examples/pricing.mox -o /tmp/pricing.rex.json
#   -> packages: nz.example.pricing, pricing.model (lowered functions with
#      verbatim "rosetta" bodies), com.rosetta.model (the builtin). The Rust
#      backend emits each lowered function as a free todo!() stub — the
#      Rune text is carried, never translated (docs/RUNE-BODIES.md is the
#      design for executing it later).

# canonical formatting (CI gates all of examples/ with --check)
rexlang fmt examples/org.mox
rexlang fmt --check examples/*.mox examples/*.actor examples/*.ifml
rexlang fmt --check examples/*.evt examples/*.ddd examples/*.uc examples/*.deploy
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
