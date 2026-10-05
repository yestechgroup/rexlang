# Event contracts (.evt)

## Overview

A transport-independent event-contract layer over `.mox` domain models. The
domain model defines the data; the `.evt` contract declares the **events**
flowing through the system (named contracts with typed payload fields), the
**channels** that publish them, and the **subscriptions** that deliver event
sets to named consumers — with nothing about brokers, wire encodings, or
delivery guaranteed. Parsed by **rex-syntax** (`parse_evt`, the chumsky
parser) at compile time, lowered and validated by **rex-driver**
(`compile_evt_str`) against the imported domains, and emitted as the
versioned `rex_ir::events::EventModel` artifact (the rex-ir crate).

The `.evt` DSL lives alongside the `.mox` domain model as a *complementary*
input: an event never declares types of its own — its payload fields resolve
against the classes, enums, datatypes, interfaces, and vocabularies the
imported domains declare. Downstream consumers ingest the artifact exactly
like the `ActorModel` and `DddModel` artifacts. **No in-tree backend exists
yet**: CloudEvents and AsyncAPI generation are the planned consumers of this
artifact and are not implemented — the artifact deliberately carries no
CloudEvents attributes, delivery policy, or webhook targets in slice 1.

The normative contract — entry points, resolution semantics, and the
validation rules — is documented on `rex_driver::compile_evt_str`; this page
is the language reference.

---

## Artifact wire format

`EventModel` is a standalone, versioned wire artifact following the rex-ir
[wire format contract](BACKENDS.md) (see also the `rex_ir::events` module
docs):

- Versioned independently of the domain-model marker via
  **`EVENT_MODEL_FORMAT_VERSION`** (= 1), the `ActorModel` precedent.
  `EventModel::from_json` rejects absent/wrong versions with
  `IrError::UnsupportedFormatVersion` (an absent version reports as 0).
- All struct field names serialize as `camelCase` (`formatVersion`,
  `events`, `channels`, `subscriptions`). There
  are no tagged enums in this artifact: a publication is exactly
  `{"event": "<name>"}`, and a subscription's consumer is a bare string.
- Payload fields reuse the domain IR's `TypeRef` (adjacent tagging:
  `{"type": "class", "value": {...}}`), serialized under the `"type"` key —
  there is no parallel wire-level type shape.
- Names are plain strings at this layer: `publishes` entries and
  `events` lists reference events *by name*; resolving them is a
  driver/consumer concern, never a wire concern.
- Unknown fields are ignored on deserialize; every field added after v1
  must keep the `#[serde(default)]` + skip discipline (the optional
  `version` is omitted when absent).

Minimal artifact: an empty model serializes as exactly `{"formatVersion":1}`.

```rust
let compilation = rex_driver::compile_evt_str("orders.evt", source, &domains, &rex_driver::DomainImports::default());
let json = compilation.model.unwrap().to_json_pretty()?; // versioned artifact
let model = rex_ir::events::EventModel::from_json(&json)?; // version-gated reload
```

Excerpt of the committed golden artifact
(`tests/conformance/events/orders.evt.json`):

```json
{
  "formatVersion": 1,
  "events": [
    {
      "name": "OrderPlaced",
      "version": "1.0.0",
      "fields": [
        { "name": "orderId", "type": { "type": "primitive", "value": "string" } },
        { "name": "total", "type": { "type": "primitive", "value": "int" } },
        { "name": "status",
          "type": { "type": "enum",
                    "value": { "package": "nz.example.orders", "name": "OrderStatus" } } }
      ]
    }
  ],
  "channels": [
    { "name": "orders",
      "publishes": [ { "event": "OrderPlaced" }, { "event": "OrderCancelled" } ] }
  ],
  "subscriptions": [
    { "name": "billing",
      "events": ["OrderPlaced", "OrderCancelled"],
      "consumer": "billing_service" }
  ]
}
```

---

## Philosophy

1. **Transport-independent contracts** — events, channels, and
   subscriptions say nothing about brokers or encodings; a backend chooses
   the transport (CloudEvents, AsyncAPI, MQTT topics, ...) exactly the way
   the `.ddd` search projection leaves the engine to the backend.
2. **Validated against the real domain** — payload field types type-check
   against the imported `.mox` models; a typo'd type is a compile error,
   not a silent generation mismatch.
3. **Names as authored** — the artifact keeps every reference exactly as
   written; consumers resolve `publishes` and `events` names against the
   declaring file.
4. **Slice-1 minimalism** — a publication is deliberately nothing more than
   a channel-membership entry: no `when` filters, no delivery policy, no
   webhooks, no CloudEvents attributes. They are the planned extensions,
   not silent gaps.

---

## Grammar

The normative EBNF for the `.evt` surface lives exactly once, in the parser
module docs under [*`.evt` event-contract
sources*](https://docs.rs/rex-syntax/latest/rex_syntax/parser/index.html)
(`crates/rex-syntax/src/parser.rs`) — the single authority for `evt_file`,
`evt_import`, `event_decl`, `field`, `channel_decl`, `publishes_entry`, and
`subscription_decl`; this page keeps only the language-level rules.

Language-level rules:

- **`event <name> (version "<string>")? { <field>* }`** — a named contract
  with its typed payload fields (`name: Type;`, one per line). The
  `version` clause is a **free-form string** pinned into the artifact
  verbatim; rexlang attaches no semantics to it (no comparison, no
  ordering — consumers decide what a version means).
- **`channel <name> { publishes <event>; ... }`** — a named channel with
  the events published on it. A `publishes` entry is channel membership,
  nothing more.
- **`subscription <name> { events [ <event>* ] consumer <name> }`** — a
  named delivery rule: the listed events go to the one consumer. The body
  is exactly those two lines. Inside the list, the contextual `consumer`
  word ends the list, so a missing `]` never swallows the consumer line.
- **Field types** are a rex-syntax primitive (`String`, `int`, ...) or any
  declared domain kind — class, enum, datatype, interface, vocabulary —
  resolved against the imported domains (below).
- **Imports come first**: `import "<path>"` (no trailing `;`, the `.actor`
  production) must precede the first declaration; a later `import` is a
  reported, recovered syntax error. Duplicate imports are a driver concern,
  not a syntax error (the CLI reads the file once).
- The three declaration kinds may be **interleaved** in any order; the
  artifact groups them per kind and the formatter canonicalizes to
  import → event → channel → subscription order (source order kept within
  a kind).
- **Keywords**: `event`, `channel`, `publishes`, `subscription` are real
  keywords of the shared chumsky lexer — reserved in `.mox`, `.actor`, and
  `.ddd` sources too, escapable with `^` like every keyword. `events` and
  `consumer` are **contextual** words, special only inside a subscription
  body (the delegation-`to` precedent; `^events`/`^consumer` never match).
- Slice-1 exclusions are deliberate: event fields are plain typed fields —
  no constraints, facets, `readonly`/`id`, or multiplicities — and there
  are no `when` filters, delivery policies, or webhooks.

---

## Resolution & validation

Payload field types resolve against the **union namespace of the imported
domains** (the same rule multi-file `.mox` compiles follow): the
`rex-syntax` primitives resolve everywhere; every other reference follows
the `.mox` cross-package rules (a bare single-segment name must be unique
across all domain packages; a qualified `pkg.Name` must match exactly one
package — ambiguity and unknown names report through the same diagnostics
the `.mox` and `.ddd` surfaces use).

Validation rules (any error-severity diagnostic anywhere — the contract
file or an imported domain — drops the artifact; warnings do not):

1. **Syntax** — parse errors are reported against the `.evt` file.
2. **Imports** — every `import "<path>"` must name a provided domain, else
   `imported file "<path>" was not provided`.
3. **Field types** — a payload field type that is neither a primitive nor
   resolvable in the domain namespaces errors as `unknown type 'X'`.
4. **Publications** — every `publishes X` entry must name an event declared
   in the same file, else `unknown event 'X' on channel 'C'`.
5. **Subscription events** — every entry of a subscription's `events [...]`
   list must name an event declared in the same file, else
   `unknown event 'X' in subscription 'S'`.
6. **Uniqueness** — event, channel, and subscription names are unique
   within the file (`duplicate event 'X'` / `duplicate channel 'X'` /
   `duplicate subscription 'X'`, each with help `'X' is already declared in
   this file`), and payload field names are unique within their event
   (`duplicate payload field 'x' in event 'Y'`).
7. **Subscription shape** — an empty `events [...]` list errors as
   `subscription 'S' subscribes to no events`; an empty consumer errors as
   `subscription 'S' names an empty consumer` (only reachable through
   parser recovery, whose syntax error fires alongside — both name the
   subscription, so the pairing is deterministic).

Diagnostics are ordered deterministically: the contract file's parse
diagnostics first, then the import errors, then each domain's own
diagnostics (import order), then the contract's semantic diagnostics
grouped by declaration kind — events, then channels, then subscriptions —
each group in source order, matching the artifact's declaration order.
Every diagnostic is tagged with its file's path, so hosts (and the CLI)
render them grouped per file. On any error the result's `model` is `None`;
`domains_model` still carries the imported domains when they lowered.

---

## CLI usage

```
rexlang check orders.evt                  # validate contract + imported domains
rexlang ir orders.evt -o orders.evt.json  # emit the EventModel artifact
rexlang fmt --check orders.evt            # canonical formatting gate
rexlang artifact check orders.evt.json    # validate the serialized artifact
```

Try it on the canonical fixture from the repository root:

```
cargo run -p rex-cli -- check tests/conformance/events/orders.evt
cargo run -p rex-cli -- ir tests/conformance/events/orders.evt -o orders.evt.json
```

- Import paths resolve **relative to the `.evt` file's directory**; each
  imported `.mox` is read from disk and compiled through the ordinary
  domain pipeline. A missing import file is a clean error naming the
  resolved path. An imported domain may itself declare `import schema` and
  `import sigil` — the CLI provides that content too, so event fields
  target lowered JSON-Schema and Rune types like any other class.
- Diagnostics come from every file involved and render grouped per file,
  the way `.actor` compiles render them.
- `rexlang ir` on an `.evt` file emits only the event artifact; the domain
  IR is emitted by running `ir` on the `.mox` files themselves.
- `rexlang fmt` formats `.evt` files with the canonical layout: imports
  hoisted into a tight first section, then declarations grouped by kind
  (event → channel → subscription, source order within a kind), fields as
  `name: Type;` one per line, `publishes` entries with a canonical
  trailing `;`, and a subscription body of exactly the two lines
  `events [ A B ]` and `consumer C`. Stdin is treated as `.mox`.
- Directory inputs stay `.mox`-only: contracts are compiled by naming the
  `.evt` file explicitly.
- `rexlang artifact check` classifies a non-empty event artifact by its
  `events`/`channels`/`subscriptions` keys (rule 4 of 6). The **empty**
  event artifact serializes as exactly `{"formatVersion":1}` and therefore
  classifies as an ActorModel — a documented shape-sniffing limitation the
  empty IFML model shares (see the classification rules in the CLI's
  `artifact_check` module).

---

## Worked example

`tests/conformance/events/orders.evt`, the canonical fixture, contracts the
sample order flow over `tests/conformance/events/orders.mox`:

```
import "orders.mox"

event OrderPlaced version "1.0.0" {
    orderId: String;
    total: int;
    status: OrderStatus;
}

event OrderCancelled version "1.1.0" {
    orderId: String;
    reason: String;
}

channel orders {
    publishes OrderPlaced;
    publishes OrderCancelled;
}

subscription billing {
    events [ OrderPlaced OrderCancelled ]
    consumer billing_service
}

subscription analytics {
    events [ OrderPlaced ]
    consumer analytics_service
}
```

Notes on the example: `status: OrderStatus` resolves to the enum the
imported domain declares (see the golden excerpt above — it serializes as a
qualified enum reference into `nz.example.orders`); `billing` and
`analytics` subscribe to overlapping event sets, each with exactly one
consumer; nothing in the file names a transport.

---

## Conformance

`tests/conformance/events/` holds the canonical fixture set: `orders.mox`
(the domain), `orders.evt` (the contract, formatter-canonical), and
`orders.evt.json` (the committed artifact). The driver golden test
(`crates/rex-driver/tests/evt_golden.rs`) byte-compares the compiled
artifact, round-trips it through `from_json`, and pins the negative
diagnostics; the wire shape is pinned by
`crates/rex-ir/tests/event_model.rs`; the formatter golden
`tests/conformance/fmt/events.fmt.evt` is byte-compared by
`crates/rex-syntax/tests/fmt.rs`, and the CI fmt gate covers
`tests/conformance/events/`. Regenerate goldens with
`REX_UPDATE_FIXTURES=1 cargo test --workspace`.
