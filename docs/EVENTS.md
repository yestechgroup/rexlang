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
artifact and are not implemented — the artifact carries the *declarations*
those generators need (CloudEvents attributes, delivery policy, governance
metadata, filters), never their execution.

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
  In a checked application the name must resolve: `rexlang check <dir>`
  runs the union integrity pass (`rex_driver::check_integrity`, register
  row **V12**) over the import-connected application and errors when a
  consumer names no declared `.deploy` component — a dead inbox, not a
  valid contract. Checking the contract alone stays per-surface and
  silent.
- Payload fields reuse the domain IR's `TypeRef` (adjacent tagging:
  `{"type": "class", "value": {...}}`), serialized under the `"type"` key —
  there is no parallel wire-level type shape.
- Names are plain strings at this layer: `publishes` entries and
  `events` lists reference events *by name*; resolving them is a
  driver/consumer concern, never a wire concern. The production side is
  the `.ddd` service operation's `emits` clause (`docs/DDD.md`): the union
  integrity pass checks that emitted events are declared here and that
  declared events are emitted somewhere.
- The slice-2 delivery model rides under the same discipline: an event's
  `owner`/`classification`/`retention` strings, `pii` array, and
  `cloudEvents` object (`{"type": ..., "source": ..., "subject": ...}` —
  the `"type"` key renamed for serde, `subject` omitted when absent), and
  a subscription's `when` string plus `delivery` object
  (`{"mode": "at_least_once", "maxAttempts": 5, ...}`) are all omitted
  when absent (see [Delivery model & governance](#delivery-model--governance-slice-2)).
- Unknown fields are ignored on deserialize; every field added after v1
  must keep the `#[serde(default)]` + skip discipline (the optional
  `version` is omitted when absent; so are the `scope` keys — a
  channel/subscription without a scope clause serializes byte-identical to
  the pre-scope format, and the version stays 1).

Minimal artifact: an empty model serializes as exactly `{"formatVersion":1}`.

```rust
let compilation = rex_driver::compile_evt_str("orders.evt", source, &domains, &rex_driver::DomainImports::default());
let json = compilation.model.unwrap().to_json_pretty()?; // versioned artifact
let model = rex_ir::events::EventModel::from_json(&json)?; // version-gated reload
```

Excerpt of the committed golden artifact
(`tests/conformance/events/orders.evt.json`) — the fully-loaded event and
the `billing` subscription show every slice-2 shape:

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
      ],
      "owner": "order_team",
      "classification": "confidential",
      "retention": "7y",
      "pii": ["orderId"],
      "cloudEvents": {
        "type": "nz.example.orders.orderplaced.v1",
        "source": "/orders",
        "subject": "{orderId}"
      }
    }
  ],
  "channels": [
    { "name": "orders",
      "scope": "account",
      "publishes": [ { "event": "OrderPlaced" }, { "event": "OrderCancelled" } ] }
  ],
  "subscriptions": [
    { "name": "billing",
      "events": ["OrderPlaced", "OrderCancelled"],
      "consumer": "billing_service",
      "scope": "account",
      "when": "orderId != \"\"",
      "delivery": {
        "mode": "at_least_once",
        "maxAttempts": 5,
        "backoffSeconds": 10,
        "timeoutSeconds": 30,
        "deadLetter": "orders"
      } }
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
4. **Declarations, not behaviors** — the delivery model (slice 2) is
   exactly as far as the contract goes: delivery policy, governance
   metadata, CloudEvents attributes, and filters are *declarations the
   artifact carries for consumers*. Webhook targets are runtime/tenant
   data and are deliberately **not modeled** — the DSL stays
   transport-independent (see the slice-2 scoping notes below).

---

## Grammar

The normative EBNF for the `.evt` surface lives exactly once, in the parser
module docs under [*`.evt` event-contract
sources*](https://docs.rs/rex-syntax/latest/rex_syntax/parser/index.html)
(`crates/rex-syntax/src/parser/mod.rs`) — the single authority for `evt_file`,
`evt_import`, `event_decl`, `field`, `channel_decl`, `publishes_entry`, and
`subscription_decl`; this page keeps only the language-level rules.

Language-level rules:

- **`event <name> (version "<string>")? { <member>* }`** — a named contract
  with its typed payload fields (`name: Type;`, one per line). The
  `version` clause is a **free-form string** pinned into the artifact
  verbatim; rexlang attaches no semantics to it (no comparison, no
  ordering — consumers decide what a version means). Members may appear in
  any source order; the formatter canonicalizes them to fields →
  governance → `cloudEvents`.
- **Governance lines** (inside an event body, each ending in `;`):
  `owner <bare-name>;`, `classification <word>;`, `retention
  "<period-string>";` — three verbatim labels the driver never interprets
  (the retention period is whatever string the source authored, e.g.
  `"7y"` kept as `7y`), and `pii [ <payload-field>* ];` — the payload
  fields carrying personal data (every entry must name a field of the same
  event).
- **`cloudEvents { type "<string>" source "<string>" (subject "<string>")? }`**
  — the CloudEvents attribute declarations. Once the block is present,
  `type` and `source` are required and must be non-empty; `subject` is
  optional. When present, the declarations are **authoritative** for a
  downstream generator (see the slice-2 section). The entries carry no `;`
  — the block's grammar is bracket-only.
- **`channel <name> ("of" <scope>)? { publishes <event>; ... }`** — a
  named channel with the events published on it. A `publishes` entry is
  channel membership, nothing more. The optional `of` clause names the
  channel's **scope**: the topic-template dimension its topics are
  parameterized by — traderX's `/accounts/{id}/trades` family declares as
  `channel account-trades of accounts { ... }`, the transport binding the
  `{id}` segment to the consumer's slice.
- **`subscription <name> { events [ <event>* ] consumer <name>
  ("scope" <name>)? ("when" (...))? (delivery {...})? }`** — a named
  delivery rule: the listed events go to the one consumer, optionally bound
  to one slice of a scoped topic family (`consumer web_front_end_angular
  scope accounts`), optionally filtered by a `when` condition, and
  optionally carrying a delivery policy. The clauses are a fixed chain in
  the canonical order `events` → `consumer` → `scope` → `when` →
  `delivery`. Inside the list, the contextual `consumer` word ends the
  list, so a missing `]` never swallows the consumer line.
- **`when ( <expression> )`** — a subscription filter whose interior is
  raw rex-expr text (stored verbatim). The driver types it **boolean**
  against *every* event in the subscription's list — a filter is a routing
  decision per event, so a field absent from any subscribed event is an
  error by design (rule 13 below).
- **`delivery { mode <word>; (maxAttempts <int>;)? (backoffSeconds
  <int>;)? (timeoutSeconds <int>;)? (deadLetter <channel>;)? }`** — the
  per-subscription delivery policy. `mode` is required once the block is
  present and must be one of `at_most_once`, `at_least_once`,
  `effectively_once`; the three tuning knobs must be positive; the
  dead-letter reference must name a channel declared in the same file.
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
- **Keywords and contextual words**: `event`, `channel`, `publishes`,
  `subscription` are real keywords of the shared chumsky lexer — reserved
  in `.mox`, `.actor`, and `.ddd` sources too, escapable with `^` like
  every keyword. `when` is the shared real condition keyword. Everything
  else the new clauses use is **contextual** — ordinary identifiers special
  only in their clause/block positions: `owner`, `classification`,
  `retention`, `pii`, `cloudEvents` as event-body member introducers;
  `type`/`source`/`subject` inside the `cloudEvents` block (`type` is the
  real `.mox` datatype keyword and stays usable as a payload field name —
  the qname-segment rule); `delivery` as a subscription clause (reserved
  right after the consumer name — `scope ^delivery` escapes);
  `mode`/`maxAttempts`/`backoffSeconds`/`timeoutSeconds`/`deadLetter`
  inside the `delivery` block; and the slice-1 words `events`, `consumer`,
  `of`, `scope`. Escaped forms (`^owner`, `^delivery`, ...) never match.
- Webhook targets remain a **deliberate exclusion** — runtime/tenant data,
  not contract syntax (see the slice-2 section). Event fields stay plain
  typed fields — no constraints, facets, `readonly`/`id`, or
  multiplicities.

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
8. **CloudEvents attributes** — once an event carries a `cloudEvents`
   block, its `type` and `source` must be non-empty
   (`cloudEvents type on event 'X' must not be empty` / the `source`
   twin). When present, the declarations are authoritative for a
   downstream generator — they override any derived `type`/`source`
   mapping (the generator's integration seam, below).
9. **Governance `pii`** — every `pii` entry must name a payload field of
   the same event (`unknown payload field 'x' in pii of event 'Y'`). The
   `owner`/`classification`/`retention` lines are never validated beyond
   syntax — verbatim labels.
10. **Delivery mode** — the `mode` entry must be one of the three
    vocabulary words `at_most_once`, `at_least_once`, `effectively_once`
    (`unknown delivery mode 'x' on subscription 'S'`, help listing them).
11. **Delivery tuning knobs** — `maxAttempts`, `backoffSeconds`, and
    `timeoutSeconds` must be positive (`'maxAttempts' on subscription 'S'
    must be positive`; likewise the other two).
12. **Dead-letter channel** — the `deadLetter` entry must name a channel
    declared in the same file (`unknown dead-letter channel 'x' on
    subscription 'S'`).
13. **Subscription `when`** — the filter's interior (sliced strictly inside
    the parens, trimmed) must parse as a rex-expr condition and type
    `boolean` against **every** event in the subscription's `events [...]`
    list. The expression is typed with a synthetic per-event class — the
    subscribed event's payload fields as the implicit `self`, registered
    over the imported domains' class union (`DomainTypes::insert_class`,
    never a fake `Model`). A field absent from any subscribed event is an
    error **by design**: a filter is a routing decision per event, and an
    event without the field cannot be routed (the help names the
    subscription and, for per-event failures, the event). A parse failure
    leaves the filter out of the artifact; a type failure still lowers the
    verbatim interior (the `.cmmn` precedent) — the artifact drops on any
    error either way.

Scope labels are **never** validated: a channel's `of <scope>` clause and
a subscription's `scope <name>` binding are opaque topic-template labels
carried verbatim into the artifact (`ChannelDef::scope` /
`SubscriptionDef::scope`) — no resolution, no uniqueness, no cross-checks.
Interpretation (which payload field carries the key, which transport
segment it binds) is a consumer/backend concern, never a driver concern.
The same is true of the governance labels (`owner`, `classification`,
`retention`) and the CloudEvents attribute *values*: stored verbatim,
never interpreted.

Diagnostics are ordered deterministically: the contract file's parse
diagnostics first, then the import errors, then each domain's own
diagnostics (import order), then the contract's semantic diagnostics
grouped by declaration kind — events, then channels, then subscriptions —
each group in source order, matching the artifact's declaration order.
Within an event: its fields (source order), then `pii`, then
`cloudEvents`; within a subscription: its `events [...]` list, then
`when`, then `delivery`. Every diagnostic is tagged with its file's path,
so hosts (and the CLI) render them grouped per file. On any error the
result's `model` is `None`; `domains_model` still carries the imported
domains when they lowered.

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
  `name: Type;` one per line, event bodies reordered to fields →
  governance → `cloudEvents` with canonical `;` on every statement line,
  `publishes` entries with a canonical trailing `;`, a channel's
  `of <scope>` clause on the header line, and a subscription body of the
  `events [ A B ]` line, the `consumer C` line carrying an optional
  `scope S` tail, the `when (...)` filter (interior verbatim), and the
  `delivery { ... }` block with one `word value;` entry per line in
  canonical order. Stdin is treated as `.mox`.
- Directory batch mode (`rexlang check <dir>`) picks `.evt` files up like
  every other surface: each contract in the directory compiles
  independently, imports resolved relative to it.
- `rexlang artifact check` classifies a non-empty event artifact by its
  `events`/`channels`/`subscriptions` keys (rule 5 of the classifier's
  precedence list). The **empty** event artifact serializes as exactly
  `{"formatVersion":1}` and therefore classifies as an ActorModel — a
  documented shape-sniffing limitation the empty IFML model shares (see
  the classification rules in the CLI's `artifact_check` module).

---

## Worked example

`tests/conformance/events/orders.evt`, the canonical fixture, contracts the
sample order flow over `tests/conformance/events/orders.mox`. The fence
below is the fixture verbatim, its import repointed so it compiles from
this document (the docs fences are compiled by CI):

```evt
import "../tests/conformance/events/orders.mox"

event OrderPlaced version "1.0.0" {
    orderId: String;
    total: int;
    status: OrderStatus;
    owner order_team;
    classification confidential;
    retention "7y";
    pii [ orderId ];
    cloudEvents {
        type "nz.example.orders.orderplaced.v1"
        source "/orders"
        subject "{orderId}"
    }
}

event OrderCancelled version "1.1.0" {
    orderId: String;
    reason: String;
}

channel orders of account {
    publishes OrderPlaced;
    publishes OrderCancelled;
}

subscription billing {
    events [ OrderPlaced OrderCancelled ]
    consumer billing_service scope account
    when (orderId != "")
    delivery {
        mode at_least_once;
        maxAttempts 5;
        backoffSeconds 10;
        timeoutSeconds 30;
        deadLetter orders;
    }
}

subscription analytics {
    events [ OrderPlaced ]
    consumer analytics_service
    when (total > 0)
}
```

Notes on the example: `status: OrderStatus` resolves to the enum the
imported domain declares (see the golden excerpt above — it serializes as a
qualified enum reference into `nz.example.orders`); `billing` and
`analytics` subscribe to overlapping event sets, each with exactly one
consumer; the `orders` channel and the `billing` subscription carry an
`account` scope (a topic-family dimension, carried verbatim — the
`analytics` subscription shows the scope-less shape); nothing in the file
names a transport. The `billing` filter can reference `orderId` because
*both* subscribed events carry it; the `analytics` filter may use `total`
because it lists only `OrderPlaced` — the same filter on `billing` would
be rule 13's routing error. `deadLetter orders` points at the declared
channel.

---

## Delivery model & governance (slice 2)

Slice 2 adds the delivery model as **declarations the artifact carries for
consumers** — never behaviors the driver executes. The issue-#31 scoping
decisions, recorded here as the contract:

- **CloudEvents attribute declarations override derived mappings.** An
  event's `cloudEvents { type ... source ... subject ... }` block is the
  integration seam with the downstream generator: when present, its
  values are authoritative for the generated CloudEvents envelope — the
  generator does not derive `type` from the event name or `source` from
  the package. Absent, the generator's own derivation applies. The driver
  validates only non-emptiness (rule 8); the CE spec's URI-reference /
  reverse-DNS shape checks are the generator's concern.
- **Webhook targets are runtime/tenant data — deliberately NOT modeled.**
  A subscription says *which consumer* receives *which events* under
  *what policy*; *where* that consumer is reachable (URLs, queue ARNs,
  connection strings) is deployment/tenant state that changes
  independently of the contract. Modeling it would couple the
  transport-independent DSL to a specific delivery topology. The
  `.deploy` surface is where environment binding belongs.
- **Transports, schema-registry publication, and event
  versioning/compatibility matrices are downstream concerns / future
  slices.** The artifact names no broker, no wire encoding, no registry,
  and carries no compatibility contract between `version` strings — a
  backend chooses the transport (CloudEvents, AsyncAPI, MQTT topics, ...)
  exactly the way the `.ddd` search projection leaves the engine to the
  backend.
- **Delivery policy + governance + filters are carried, not executed.**
  `at_most_once`/`at_least_once`/`effectively_once`, the tuning knobs,
  the dead-letter channel, the PII list, and the `when` filters are
  contract facts a consumer implements; rexlang's driver validates their
  *shape* (mode vocabulary, positive knobs, resolvable channel and field
  references, boolean filters) and stops there.

How the pieces compose on a subscription: `events [...]` selects the
routed event set, `when` narrows it per event (typed against every listed
event — rule 13), `delivery` states the reliability contract, and the
consumer binds it all to a transport endpoint that the artifact never
names.

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

The `examples/` side ships `support.evt` and `ecommerce.evt` (the latter
with a vocabulary-typed payload field), compiled and byte-pinned against
`examples/artifacts/` by `crates/rex-driver/tests/examples_artifacts.rs`.
