# The unified semantic graph

`rexlang graph` compiles a tree of model files into **one queryable,
versioned artifact**: `rex_ir::graph::GraphModel`. It is the substrate for
impact analysis, `graph query`, the MCP server (v0.6), Studio, and AI
agents — the answer to "what is affected if I change capability X" is a
query, not a human reading eight artifacts.

The artifact is **derived, never parsed**: it is built from the compiled
artifacts of every surface after checking, plus the import edges only the
host can resolve (wire artifacts are span-free and, except `.ifml`
imports, path-free). The builder lives in `rex_driver::graph`; the wire
shape lives in `crates/rex-ir/src/graph.rs`.

## Wire contract

The [wire format contract](../crates/rex-ir/src/lib.rs) applies unchanged,
with `GRAPH_MODEL_FORMAT_VERSION` as the independent marker (wire contract
rule 10, the `ActorModel`/`IfmlManifest` precedent):

| Rule | Expectation |
|---|---|
| Casing | struct fields `camelCase`; `kind` values bare `camelCase` (`serviceOperation`, `caseFileItem`, `opaqueGrant`, …) |
| Tagging | `effect` / `sessionKind` reuse `rex_ir::GrantEffect` / `rex_ir::SessionKind` as bare strings; no adjacent tagging in this root |
| Empty | `GraphModel::default()` serializes as **exactly** `{"formatVersion":1}` |
| Gate | `GraphModel::from_json` rejects absent and non-1 versions with `IrError::UnsupportedFormatVersion` |
| Forward compat | unknown JSON fields are ignored; unknown `kind` values survive a round-trip |
| Equality | `Eq` — the graph carries no floating-point values |
| Provenance | **no** `rexVersion` stamp — goldens must not churn on release bumps |

### The open kind vocabulary

Node and edge kinds are **string-typed**, not a closed Rust enum — the one
deliberate departure from the house enum style. A closed enum cannot grow
additively (serde rejects unknown tags), and this artifact must not need a
breaking change before the v0.7 freeze. The v1 constants live in
`rex_ir::graph::kinds`; new kinds are new string values.

**Additive-change policy.** Growth is additive only:

- a new node/edge kind is a new `kinds` constant (readers that do not know
  it keep it intact through a round-trip);
- a new field is `#[serde(default)]`, plus `skip_serializing_if` when
  commonly empty, so older artifacts stay byte-identical;
- an incompatible reshape bumps `GRAPH_MODEL_FORMAT_VERSION` — a breaking
  change, which v0.3–v0.6 must not need.

### Node ids

A node id has the shape `<kind>:<file>#<name>`:

- `<file>` is the declaring file as a **root-relative POSIX display path**
  (no absolute paths, no `\` separators) — the same path the node's
  `file` field carries;
- `<name>` is surface-qualified: `nz.example.support.Ticket` (`.mox`),
  `Support.RaiseRefund` (`.actor` capability), `SupportDesk.desk.TicketService.raiseRefund`
  (`.ddd` service operation), `Support.RaiseRefund` (`.evt`),
  `Case.Item` (`.cmmn`).

File attribution is a node's `file` field: **there are no `declares`
edges**.

The `<Model>` prefix is the surface's own model name where the artifact
carries one (`.dmn`'s `decision_model <name>`, a `.ddd`'s
`application <name>`), else the file stem (`support.evt` →
`support.TicketRaised`, a module-less design, an `.ifml` flow).

## Applications

Import-connected applications are the union-find components over
canonicalized import paths — the same relation `check`'s union integrity
pass uses (`rex_driver::applications`). A `.mox` file that other surfaces
import is a **bridge node**: it joins its importers into one application
without contributing nodes of its own beyond its domain declarations.
Unrelated applications in one tree never cross-check, and an import that
leaves the checked set contributes no edge.

`applications[].id` is the component's smallest member path;
`applications[].files` and `.packages` are sorted.

## The `rexlang graph` command

```sh
rexlang graph <paths>... [-o <file>]
```

Compiles every input and emits **one** `GraphModel` — pretty-printed
(2-space) JSON — to stdout, or to `-o <file>`. Directories enter **project
mode** exactly like `check`/`build`: every supported surface under the tree
is gathered into one snapshot, each file compiles independently with its
imports resolving map-first, sibling artifacts are reused compile-once, the
union-wide capability joins run, and formatter-golden `X.fmt.<ext>` files are
skipped. Explicit files compile in their own context, like `build`.

Emission is **best-effort over the compiling subset** (the `build`
precedent): a failing file renders its diagnostics per file and contributes
nothing, and the command still writes the graph for everything that
compiled, exiting `1`. Unresolved wiring never becomes an edge — it stays
the compiler's diagnostic, not a guessed binding.

### Display paths and the base rule

Every emitted path — node ids, `file` fields, `applications[].files` — is
**root-relative POSIX**. The base is the common ancestor of the input roots:

- directories were given → the common ancestor of those directory arguments;
- only explicit files → the common ancestor of their directories.

Display paths are relative to that base, `/`-separated, with `.` components
dropped. The host-normalized absolute keys (`path`/`imports`, canonicalized
where the file exists) are used for clustering and import matching only and
are never emitted — so two machines and two runs serialize byte-identically.

### The examples golden

`examples/artifacts/examples.graph.json` pins the whole `examples/` tree with
base `examples/` — every surface, the support stack, and the unrelated
applications clustered together:

```sh
rexlang graph examples/ -o /tmp/examples.graph.json
diff /tmp/examples.graph.json examples/artifacts/examples.graph.json
REX_UPDATE_FIXTURES=1 cargo test -p rex-cli --test cli graph
```

The tree keeps one pre-existing project-mode failure (`orders.ddd`'s
`PlaceOrder` capability against the union-wide pooled `support.actor`
capabilities — the same finding `check examples/` renders), which exercises
the best-effort path: the artifact is written over the compiling subset and
the command exits `1`.

## Nodes (v1)

| kind | declares | name shape |
|---|---|---|
| `file` | one compiled file | display path |
| `class` | `.mox` class / interface / register | `<package>.<Class>` |
| `enum` | `.mox` enum | `<package>.<Enum>` |
| `datatype` | `.mox` datatype | `<package>.<Type>` |
| `feature` | class attribute, containment, reference, container, derived feature | `<package>.<Class>.<feature>` |
| `operation` | `.mox` operation | `<package>.<Class>.<op>` |
| `actor` | `.actor` actor (inline `.mox` actors blocks too) | `<Block>.<Actor>` |
| `capability` | `.actor` capability | `<Block>.<Capability>` |
| `designApplication` | `.ddd` application | `<App>` |
| `module` | `.ddd` module, and an `.ifml` module declaration (one declaration per pattern) | `<App>.<module>` (`.ddd`), `<Module>` (`.ifml`) |
| `service` | `.ddd` service | `<App>.<module>.<Service>` |
| `serviceOperation` | `.ddd` service operation | `<App>.<module>.<Service>.<op>` |
| `repository` | `.ddd` repository | `<App>.<module>.<Repository>` |
| `repositoryOperation` | `.ddd` repository operation | `<App>.<module>.<Repository>.<op>` |
| `search` | `.ddd` search | `<App>.<module>.<Search>` |
| `stateMachine` | `.ddd` entity machine | `<App>.<module>.<Class>` (`attributes["field"]` = the state-carrying feature) |
| `transition` | machine transition | `<machine>.<from>-><to>`, suffixed `~<n>` (n ≥ 2) when a pair repeats |
| `decisionModel` | `.dmn` model | `<Model>` |
| `decision` | `.dmn` decision | `<Model>.<Decision>` |
| `decisionService` | `.dmn` decision service | `<Model>.<Service>` |
| `bkm` | `.dmn` business knowledge model | `<Model>.<BKM>` |
| `inputData` | `.dmn` input data | `<Model>.<Input>` |
| `knowledgeSource` | `.dmn` knowledge source | `<Model>.<Source>` |
| `event` | `.evt` event | `<Model>.<Event>` |
| `channel` | `.evt` channel | `<Model>.<Channel>` |
| `subscription` | `.evt` subscription | `<Model>.<Subscription>` |
| `flow` | `.ifml` entry flow | `<flow file stem>` |
| `view` | `.ifml` view | `<View>` |
| `container` | `.ifml` container | its declaration path: `<View>.<Container>` (nested containers extend the path) |
| `component` | `.ifml` component | its declaration path: `<View>.<component>` (a component inside a container is `<View>.<Container>.<component>`) |
| `action` | `.ifml` action (top-level declaration) | `<action>` |
| `command` | `.ifml` CLI command | `<command>`; a subcommand joins the path (`<command>.<sub>`) |
| `useCase` | `.uc` use case | `<UseCase>` |
| `case` | `.cmmn` case | `<Case>` |
| `planItem` | `.cmmn` plan item | `<Case>.<Item>` |
| `caseFileItem` | `.cmmn` case-file item | `<Case>.<Item>` |
| `caseRole` | `.cmmn` case role | `<Case>.<Role>` |
| `deployApplication` | `.deploy` application | `<App>` |
| `deployComponent` | `.deploy` component | `<App>.<Component>` |
| `deployProfile` | `.deploy` profile | `<Profile>` |
| `deployment` | `.deploy` deployment | `<App>.<Deployment>` |

## Edges (v1)

**Direction convention: `from` depends on / references / is owned by
`to`.** `affected-by X` is therefore the reverse closure and always reads
"X changed ⇒ these may be affected". The one object-graph edge is
`contains` (container → contained).

| kind | from → to | attributes |
|---|---|---|
| `imports` | importer file → imported file | — |
| `memberOf` | member → owner symbol | — |
| `extends` | class/actor → parent | — |
| `contains` / `refers` | class → class | `feature` |
| `opposite` | feature → opposite feature | `feature` |
| `typeOf` | feature/op/event/case item/participant/op output → type node | `field`, `role=return\|param` |
| `on` | capability → class | — |
| `carries` | serviceOperation/search → capability | — |
| `permits` / `forbids` | actor → capability | `effect`, `when`, `sessionKind`, `obligations` |
| `opaqueGrant` | actor → file (bare `cedar { … }`) | — |
| `delegates` | receiver actor → capability | `effect`, `fromActor`, `delegation`, `purpose` |
| `excludes` | capability → capability (`never_both`, both directions emitted) | — |
| `requires` | view/command/deployComponent/caseRole/useCase → capability\|actor | `via=requires\|roles\|deploy\|ucStep` |
| `invokesDecision` | service op / service / plan item → decision\|decisionService | `via`, `pin` |
| `binds` | `.ifml` action → serviceOperation | `ambiguous` |
| `realizes` | useCase → serviceOperation | — |
| `executes` | planItem → serviceOperation | — |
| `emits` | serviceOperation/decision → event | `when` |
| `publishes` | channel → event | — |
| `subscribes` | subscription → event | — |
| `delivers` | subscription → deployComponent | — |
| `navigates` / `refreshes` / `invokesAction` | view/component/command source → view/container/component/action | `trigger`, `source` |
| `data` | `.ifml` component → class | `asWritten` |
| `readsFeature` | component/search → feature | `via=column\|filter\|sort\|text` |
| `bindsDesign` | deployComponent → module\|designApplication | `module` |
| `bindsFlow` | deployComponent → flow | `module` |
| `connects` | component → component | — |
| `usesModule` | view/command → module | `alias` |
| `designs` | repository/search → class | `via` |
| `forwardsTo` | serviceOperation → repositoryOperation (or serviceOperation, when the delegation targets another service) | `operation` |
| `injects` | service → repository | — |
| `triggers` | serviceOperation → transition | `ambiguous` |
| `assignedTo` | planItem → caseRole | — |
| `provides` | decisionService → decision | — |
| `usesProfile` | deployment → profile | — |
| `deploys` | deployment → deployApplication | — |

## Node attributes

A node's `attributes` map carries the kind-specific facts that are not part
of its identity. v1 uses one: a `stateMachine`'s `field` (the designed
class feature holding the state). The field is additive on the wire
(omitted when empty), so attribute-less nodes stay byte-identical.

## Edges and unresolved references

Edge `from`/`to` are node ids; every edge is emitted only after both ends
exist in the node table, so the artifact is closed under its own edges. A
reference the node table cannot resolve produces **no edge** (see the
resolution rules) — it stays a compiler diagnostic, never a fabricated
binding.

## Resolution rules

The graph mirrors the compiler; it never invents a binding:

- Capability/actor names are block-local but pool union-wide (`.ddd`
  clause resolution, `.ifml` `requires:`/`roles:`, `.uc` steps). One match
  binds; several emit all with `attributes["ambiguous"]="true"`.
- `action → operation` follows the #114 rule: ASCII-case-insensitive name
  equality; unique binds, several emit all with `ambiguous`.
- `.uc` step → operation: by name among the application's design
  operations. The step is not a node kind of its own, so it realizes through
  the use case's `realizes` edge and its capability requirements ride the
  same `requires` edge with `via=ucStep`.
- `.cmmn` `execute`: `package.Service.op` as written; decision-task
  `invokes`: `Model.Service`.
- `.evt` consumer → deploy component: exact name across the application's
  deploy applications.
- **Unresolved references produce no edge.** They remain compiler
  diagnostics; the graph is best-effort over the compiling subset.

## Determinism

- Nodes sort by `(application, file, id)`; edges by
  `(from, kind, to, serialized attribute tuple)`; exact duplicates
  collapse.
- Components sort by their smallest member path; `applications[].files`
  and `.packages` sort.
- Reversed or shuffled input order produces byte-identical JSON.
- No absolute paths, no `\` separators, no `rexVersion`.

## Queries

The query surface runs on the artifact **alone** — no sources, no
recompile, no import resolver. It is the contract the v0.6 MCP `graph query`
tool and Studio wrap, so it is versioned and deterministic.

```sh
rexlang graph query <graph.json|-> node <ref> [--edges] [--pretty]
rexlang graph query <graph.json|-> list [--kind K] [--file F] [--application A] [--pretty]
rexlang graph query <graph.json|-> affected-by <ref> [--kinds K,…] [--direction dependents|dependencies] [--pretty]
rexlang graph query <graph.json|-> dependencies <ref> [--pretty]
```

`-` reads the artifact from stdin. `--pretty` changes whitespace only.
`rexlang graph <paths>...` (compile) and `rexlang graph query …` (query)
coexist; clap matches the subcommand first, so a directory literally named
`query` must be given as `./query`.

### Reference resolution

A `<ref>` resolves **exact id → exact name → unique name suffix**:

```sh
rexlang graph query g.json node 'capability:support.actor#Support.RaiseRefund'  # id
rexlang graph query g.json node 'Support.RaiseRefund'                           # name
rexlang graph query g.json node 'TicketService.raiseRefund'                     # suffix
```

One match binds. Several are **ambiguous** — never guessed — and the CLI
exits `1` with the sorted candidates on stderr. No match is **unknown**, and
the did-you-mean candidates (also sorted) come from the adaptive name/id
suggestion policy in `rex-diagnostics`.

### Result envelopes

Every answer is one JSON envelope: `formatVersion`
(`GRAPH_QUERY_FORMAT_VERSION = 1`), the camelCase `query` name, the
parameters the query took (echoed), the `resolved` id, and `results` — always
an array, sorted by id (`list`) or by `(depth, id)` (the closures):

```json
{
  "formatVersion": 1,
  "query": "affectedBy",
  "reference": "capability:support.actor#Support.RaiseRefund",
  "resolved": "capability:support.actor#Support.RaiseRefund",
  "direction": "dependents",
  "results": [
    {
      "id": "capability:support.actor#Support.RaiseRefund",
      "kind": "capability",
      "name": "Support.RaiseRefund",
      "file": "support.actor",
      "application": "cli.ifml",
      "depth": 0
    },
    {
      "id": "serviceOperation:support.ddd#SupportDesk.desk.TicketService.raiseRefund",
      "kind": "serviceOperation",
      "name": "SupportDesk.desk.TicketService.raiseRefund",
      "file": "support.ddd",
      "application": "cli.ifml",
      "depth": 1,
      "parent": "capability:support.actor#Support.RaiseRefund",
      "via": "carries"
    }
  ]
}
```

- `node` results carry no `depth`/`parent`/`via` (they are not a closure);
  `--edges` adds an `edges` object grouping the node's incident edges by
  direction — `out` (edges with `from` = the node) and `in` (edges with `to` =
  the node), each sorted by the other endpoint, then the edge kind.
- `list` echoes only the filters it was given; no filter lists every node.
- Absent fields are omitted, never `null` — the same discipline as the
  artifact's own wire contract. An empty `--kinds` echoes nothing (it filters
  nothing).
- `affected-by` echoes `direction` (default `dependents`); `dependencies`
  pins the opposite direction and takes no flags.

### Direction: the two closures

`from` depends on / references / is owned by `to`, so `affected-by X` is the
**reverse** walk of that relation. The two ownership edges are the exception:
[`memberOf`](#edges-v1) and `contains` walk **forward**, because a member
changing changes its owner — the service, the module, the design
application, and the deployments that bind it are part of the impact answer.
Walking the reverse of `memberOf` would instead report every sibling
operation, which is the opposite of an impact answer.

`dependencies X` is the exact mirror: every other edge walks forward and the
two ownership edges walk back to the members.

**File nodes answer with the `imports` graph.** `affected-by
file:support.mox#support.mox` is the set of files that import it,
transitively; `dependencies` is the set it imports. The symbol walk never
reports nor expands a `file` node, so a cross-file import hop can never
smuggle a whole file's symbols into a symbol answer.

`--kinds` filters which nodes are **reported**; the walk still traverses
every intermediate node (`--kinds deployment` answers "which deployments are
affected" rather than stopping at the first non-deployment hop).

### Determinism

- BFS neighbours visit in lexicographic node-id order (then edge kind); the
  first discovery names the result's `parent` and `via`.
- Results sort by `(depth, id)` and are deduplicated — one record per node,
  at its smallest depth.
- `list` sorts by id; adjacency groups sort by the other endpoint, then the
  edge kind.
- The same artifact answers byte-identically across runs, and the
  deserialized artifact answers exactly like the in-memory model
  (`crates/rex-driver/tests/graph.rs` pins both).

The committed query goldens live under `tests/conformance/graph/queries/`
(`examples.node.*`, `examples.affected-by.*`), regenerated with
`REX_UPDATE_FIXTURES=1 cargo test -p rex-cli --test cli graph`.

## Cross-policy analytics

Four analytics answer the cross-policy questions the compiler's diagnostics
can only ask one file at a time (issue rexlang#246). They run on the artifact
**alone** — no sources, no recompile — and answer in the same versioned
envelope as the structural queries:

```sh
rexlang graph query <graph.json|-> separation-of-duty [--pair A B] [--actor a] [--session human|agent] [--fail-on-findings] [--pretty]
rexlang graph query <graph.json|-> monotonicity [--actor a] [--fail-on-findings] [--pretty]
rexlang graph query <graph.json|-> dead-capabilities [--session human|agent] [--fail-on-findings] [--pretty]
rexlang graph query <graph.json|-> reachability --from-actor A --to Class.feature [--fail-on-findings] [--pretty]
rexlang graph query <graph.json|-> reachability --from-capability C --to Class.feature [--fail-on-findings] [--pretty]
```

Every parameter is echoed as the id it resolved to (`reference`-style suffix
resolution applies: `App.RaiseRefund`, `Note.internalNotes`). An
unresolvable reference, or one of the wrong kind, exits `1` with the sorted
candidates on stderr. `--fail-on-findings` prints the answer and exits `1`
only when a **definite** finding exists (see the table below).

### Shared terms

The analytics rederive the compiler's policy semantics from the artifact
(`crates/rex-driver/src/lower/actors.rs` is the reference; the artifact's
edges are the source):

| term | definition |
|---|---|
| `lineage(A)` | `A` plus every transitive `extends` ancestor (cycle-guarded) |
| `permits(A)` | every `permit` grant entry of any lineage member, plus every permit an inbound `delegates` edge hands a lineage member — delegation permits count for descendants (the `effective_permits` rule) |
| `forbids(A)` | the same walk for forbid entries (grants and inbound delegations) |
| `open(A)` | `permits(A) \ forbids(A)` |
| `unknown(A)` | any lineage member carries an `opaqueGrant` edge (a bare `cedar { … }` entry) |

A `delegates` edge never contributes a permit unless its effect is `permit`;
a `cedar` entry never contributes one at all. `when` guards are treated as
**satisfiable** (may-analysis): a guarded permit counts, and the tool never
claims a guard can never fire. The optional session filter narrows both
permits and forbids — an unmodified (`any`) entry applies to every session, a
restricted entry only to its own kind.

### The four analyses

**`separation-of-duty`** — default: every declared `never_both` (`excludes`)
pair per application; `--pair A B` asks the question for *any* pair,
declared or not, which is the cross-policy value (an SoD pair nobody
declared). A violation is an actor with **both** capabilities in `open(A)`.
Each result names the pair, `declared`, the `holders` of each side,
`violations[{actor, evidence[]}]` (the granting edges, as `<kind>
<capability>`), and `unknownActors[]` (actors whose opacity leaves the pair
undecidable and who are not already definite violations). A pair whose two
capabilities live in different applications has no shared policy scope and
produces no result.

**`monotonicity`** — for every actor `extends` edge `child → parent`:
`missing = open(parent) \ open(child)`, `dominates = missing.is_empty()`,
`strict = open(child) ⊋ open(parent)`, and `unknown` when either side carries
an opaque grant. A `forbid` on the child is the realistic gap ("Manager
extends Agent but cannot do what Agent can"). Results follow the artifact's
`extends`-edge order; class inheritance never appears. `--actor` restricts
the answer to that child.

**`dead-capabilities`** — each declared capability gets one of four outcomes,
in this precedence (a definite answer only when the artifact supports it):

| status | rule | finding |
|---|---|---|
| `alive` | an actor holds it in `open(A)` | no |
| `unknown` | no open holder, and some actor of the application has an `opaqueGrant` | no (uncertain) |
| `dead` | no open holder, at least one permit edge, and every permitting actor also forbids it | **yes** |
| `latent` | no permit edge at all — declared but ungranted (the compiler's word; never reported as dead) | no |

Results sort by capability id and carry `holders` (`alive`), `blockedBy`
(`dead`: the actors that permit and also forbid it), and `unknownActors`
(`unknown`). `--session` refilters the grants.

**`reachability`** — can the authority of an actor (or one capability) reach a
target through the **domain object graph**? Starts are the classes on which a
capability the actor holds (`open`, every session kind) is `on`; with
`--from-capability` the starts are that capability's classes. The walk follows
`contains`, `refers`, and `opposite` **undirected** — they describe one object
graph, and connectivity is the question. A feature target reaches when its
owning class is reachable (a feature's owning class is only knowable from its
qualified name; the artifact has no `declares` edges). The result carries
`reachable`, the `starts`/`startClasses`, the destination class, and the
deterministic shortest witness `path[{node, edge?, feature?}]` (BFS, neighbours
in lexicographic id order; the `feature` is the traversed edge's object-graph
feature). Unreachable is the finding.

### Assumptions

- **Conservative may-analysis.** `when` guards are satisfiable; a definite
  finding is only reported when the artifact supports it, and opaque `cedar`
  entries make an actor `unknown` — never a definite yes or no.
- **Per application.** Capability names pool only inside one
  import-connected application; actors and the object graph never cross an
  application boundary. Unrelated applications in one artifact never
  cross-check (the #114/#128 rule).
- **Session kinds** filter grants, never manufacture authority: the default
  (no `--session`) asks over every session kind.
- **No cross-surface invention.** The analytics consume the artifact's
  edges and nothing else: an unresolved reference in the sources produced no
  edge, so it can never appear as evidence.

### Findings and the fixture

| query | finding |
|---|---|
| `separation-of-duty` | an actor holds both capabilities (declared or not) |
| `monotonicity` | a definite gap — `missing` non-empty and neither side opaque |
| `dead-capabilities` | the status is `dead` |
| `reachability` | the target class is unreachable |

The analytics' byte goldens (`tests/conformance/graph/queries/app.*`) come
from the self-contained fixture `tests/conformance/graph/` — `app.*` across
all nine surfaces plus the unrelated `other.*` application (an opaque
`cedar` policy). The fixture exercises every outcome: a declared pair with no
violation, an undeclared pair with one (inheritance), a monotonicity gap,
`alive`/`dead`/`latent`/`unknown` capabilities, and a reachable and an
unreachable field. The compiler's two self-narrowing warnings for the
fixture's permit-then-forbid pairs are expected — the gap is the analytic's
subject, so the fixture does not deny warnings. Regenerate with:

```sh
REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test graph_analytics
REX_UPDATE_FIXTURES=1 cargo test -p rex-cli --test cli graph
```

### Validation

`rexlang artifact check` classifies a graph artifact by its `nodes` array —
a rule that must run **before** the deployment rule, because a graph also
carries an `applications` array. The checker validates the version gate, the
`applications`/`nodes`/`edges` shapes (non-empty ids and edge endpoints),
and the snake_case-key scan. The empty graph (`{"formatVersion":1}`) is
byte-identical to the block-less actor artifact and classifies as one — the
documented empty-form limitation shared by every standalone artifact.

## The LSP contract

The artifact and the editor share **one node table**: every spanned
`SurfaceIndex` definition's `(kind, name)` is the graph builder's node
key, pinned by the anti-drift test in
`crates/rex-driver/tests/navigation.rs`. Spans are per surface; resolution
is the graph's.

The span tables live in `rex_driver::navigation`
(`crates/rex-driver/src/navigation/`): one builder per surface, a pure
function of source text that yields an empty index (never a panic) on a
source that does not parse. The graph's node names are derived by the
builder's own helpers (`graph::ids`), and the indexes call the same helpers —
a naming rule is written once.

| surface | builder | definitions (the graph's nodes) |
|---|---|---|
| `.actor` | `navigation::actors_index` | `actor`, `capability` — `<Block>.<Name>` |
| `.ddd` | `navigation::ddd_index` | `designApplication`, `module`, `service`, `serviceOperation`, `repository`, `repositoryOperation`, `search`, `stateMachine`, `transition` (child-scoped aggregate operations are `serviceOperation`s) |
| `.evt` | `navigation::evt_index` (takes the document path: the model prefix is the file stem) | `event`, `channel`, `subscription` — `<Model>.<Name>` |
| `.deploy` | `navigation::deploy_index` | `deployApplication` (`<App>`), `deployComponent` (`<App>.<Component>`), `deployProfile` (`<Profile>`), `deployment` (`<App>.<Deployment>`) |
| `.uc` | `navigation::uc_index` | `useCase` (`<UseCase>` — the artifact carries no model prefix) |
| `.cmmn` | `navigation::cmmn_index` | `case` (`<Case>`), `caseRole`/`caseFileItem`/`planItem` (`<Case>.<Name>`; a stage's nested items are plan items too — the artifact's plan is a flat graph) |
| `.dmn` | `navigation::dmn_index` (takes the document path: the model prefix is the declared `decision_model <name>`, else the file stem) | `decisionModel` (`<Model>`), `inputData`/`knowledgeSource`/`bkm`/`decision`/`decisionService` (`<Model>.<Name>`) |
| `.mox` | `navigation::mox_index` — the graph-node projection of the same parsed model `NavigationIndex` spans | `class` (interfaces and registers included), `enum`, `datatype`, `feature`, `operation`, plus the inline `actors` block's `actor`/`capability` (`<Block>.<Name>`) |
| `.ifml` | `navigation::ifml_surface` — the projection of a compilation's `IfmlIndex` | `module`, `view`, `container`, `component`, `action`, `command` (declaration paths; entry file only) |

`.mox` and `.ifml` keep their richer own span tables: `NavigationIndex`
(typed reference targets, `DefId`s, `opposite` ends) and `IfmlIndex`
(module uses, nested interaction sites) remain each surface's reference
source and document-symbol walker — the shared projection is the same source
table, keyed by the graph's node keys.

Two node kinds are the documented exclusion from the equality: one `file`
node per compiled file (its name is the display path, a host fact a
source-only index cannot know) and one `flow` node per `.ifml` file (its
name is the file stem — the same host fact). Every other node has exactly one
spanned definition site.

Each `IndexedReference` carries the name **as written** and a
`target_kinds` hint — the node kinds the mention may denote — plus a
`ReferenceContext` naming the grammatical position. A name-based
`references_to(kind, name)` prefilter matches exact and last-segment
spellings; binding (application scope, `base` qualification, ambiguity,
cross-surface pooling) stays the graph index's job, never the syntax
layer's.

| surface | references (`target_kinds`) |
|---|---|
| `.actor` | `capability … on <Class>` → `class`; grant/delegation capability names → `capability`; grant actors, `extends`, delegation `from`/`to` → `actor`; `never_both` names → `capability`; delegation `purpose` → none yet (indexed, never bound) |
| `.ddd` | operation/search `capability X` → `capability`; `invokes` / `inject decision` → `decision`, `decisionService`; `emits` → `event`; `inject <Repo>` / body receiver → `repository`; design and search class names → `class`; `=> Repo.op` → `repositoryOperation`, `serviceOperation`; machine `via` triggers and operation `transition` bindings → `transition`; machine state field → `feature`; machine state enum → `enum` |
| `.evt` | subscription `consumer` → `deployComponent`; subscription `events` and channel `publishes` → `event`; event field types → `class`, `enum`, `datatype` |
| `.deploy` | `design "<path>#<Module>"` → `module`, `designApplication` (the mention is the module suffix when written, else the path as written); `flow "<path>[#<Module>]"` → `flow`; component `requires` names → `capability`; `connects a -> b` → `deployComponent`; `deployment … for <App>` → `deployApplication`; `use <Profile>` → `deployProfile` |
| `.uc` | `subject`/actor `: <type>` → `class`; a bare (untyped) actor's name → `actor`; `realizes Service.operation` and a step's `: <operation>` → `serviceOperation`, `useCase`; step `requires [ … ]` → `capability`; a message's `of <type>` payload → `class`, `enum`, `datatype`, `event` (the surface types payloads with a bare reference — class-vs-event is a resolution fact) |
| `.cmmn` | case-file item type → `class`, `enum`, `datatype`; `role … as <Actor>` → `actor`; `execute Service.operation` → `serviceOperation`; `invokes Model.Service` → `decisionService`, `decision`; `performer: <role>` → `caseRole`; a criterion's dotted `on <part>` → `planItem`, `caseFileItem` and a bare one → `event` (the compile's rule 9); a decision task's `inputs`/`outputs` → `caseFileItem`; a stage's `children` → `planItem` |
| `.dmn` | input types → `class`, `enum`, `datatype`; BKM parameter/output types and decision output types (the `ReturnType` contexts) → `class`, `enum`, `datatype`; `logic invoke <model>` → `bkm`; `emits` → `event`; information/knowledge/authority requirement names (both ends) → `decision`, `bkm`, `knowledgeSource`; decision-service `inputs` → `inputData`, `outputs` → `decision`, `encapsulates` → `decision`, `bkm`; a view's `show` names → `decision`, `bkm`, `knowledgeSource`, `inputData`, `decisionService` |
| `.mox` | none in the projection — `.mox` mentions carry typed `Reference` targets inside `NavigationIndex` |
| `.ifml` | `use "…"` → `module`; `navigate("…")` → `view`, `container`, `component`; `refresh("…")` → `container`, `component`; `action("…")` → `action`; a view's `requires:` → `capability` and `roles:` → `actor` (the checker's own split); a component's `data:` → `class` |

Every reference context above has its own `ReferenceContext` spelling
(hover text and LSP dispatch), and a reference site is only a *hint*: the
graph index decides the binding.

### One snapshot, one index

The server answers every navigation capability from one **graph snapshot**
(`crates/rex-lsp/src/server/graph.rs`): the compiled artifacts of the open
document, its open-document siblings, and the disk siblings of its
**directory**, fed to `rex_driver::graph::build`; plus each participating
file's spanned index. Resolution is the graph index's (`GraphIndex`),
locations are the span tables'. A reference whose target the graph cannot
resolve yields **no location** — never an error, never a guess: the
artifact's "unresolved ⇒ no edge" discipline applied to the editor.

**Snapshot scope.** One directory: the open document's own. Candidates are
every supported surface file there (a formatter-golden `X.fmt.<ext>` is
skipped) plus every **open** document that lives there. An open buffer wins
over its disk copy — editing an imported sibling refreshes cross-surface
navigation without a save. A sibling that does not compile contributes
nothing (its own editor shows its diagnostics); the rest of the snapshot is
unaffected. The snapshot is cached per directory and the set of open
`(uri, version)` inputs, and invalidated on every open, change, and close.
The scope is the directory, not a project root: a tree that spreads one
application across directories resolves within the opened file's directory
only.

**The resolution rule.** A reference binds by `(kind, name)`, restricted to
the hint's node kinds and to the referring file's application (a name
declared in another application is never a binding). The spellings are tried
in the compiler's order — exact name, written `.`-suffix
(`TicketService.raiseRefund`), the written name minus its leading alias
(`<alias>.Service.operation`), last segment (`RaiseRefund`). One match binds;
several are ambiguous and the editor answers "no location".

### Definition targets

One row per surface and grammatical position; each row has a green
definition test in `crates/rex-lsp/src/server/tests/graph.rs`.

| From (surface) | At | To |
|---|---|---|
| `.actor` | `capability <X> on <Class>` | `.mox` class |
| `.actor` | grant/delegation capability names | capability declaration (same file / sibling `.actor` / inline `.mox` block) |
| `.actor` | `extends`, delegation `from`/`to` | actor |
| `.actor` | `never_both` names | capability |
| `.ddd` | operation/search `capability X` | `.actor` capability |
| `.ddd` | `invokes D`, `inject decision D` | `.dmn` decision / decision service |
| `.ddd` | operation `emits E` | `.evt` event |
| `.ddd` | `inject Repo`, body receiver | same-file repository |
| `.ddd` | `entity C`, class references | `.mox` class |
| `.ddd` | machine `on` field / state enum | `.mox` feature / enum (the `via <Trigger>` row has no source-side node: the graph encodes the trigger as an operation → `transition` `triggers` edge) |
| `.evt` | subscription `consumer C` | `.deploy` component |
| `.evt` | `publishes` / `events` names, field types | event / domain type |
| `.ifml` | `requires:` / `roles:` | capability / actor |
| `.ifml` | `action("A")` | the `.ddd` operation its `binds` edge names (else the `.ifml` action) |
| `.ifml` | `data:` | `.mox` class |
| `.ifml` | `navigate` / `refresh` targets | view / container / component |
| `.deploy` | `design "<path>#<Module>"` | `.ddd` module |
| `.deploy` | `connects a -> b` | component |
| `.deploy` | `flow "<path>"` | the `.ifml` flow node has no written declaration site (the `file`-node exclusion, above) — no location |
| `.deploy` | `requires` / `use <Profile>` / `for <App>` | capability / profile / deployment application |
| `.uc` | `realizes`, step `: <operation>` | `.ddd` service operation |
| `.uc` | subject/participant types, policy participants | `.mox` class / `.actor` actor |
| `.uc` | step `requires`, event payloads | capability / event |
| `.cmmn` | service task `execute` | `.ddd` service operation |
| `.cmmn` | decision task `invokes` | `.dmn` decision service |
| `.cmmn` | role `as <Actor>`, case-file item types | `.actor` actor / domain type |
| `.dmn` | requirement edges, `invoke` | decision / BKM / knowledge source |
| `.dmn` | `emits`, io types | `.evt` event / domain type |
| `.mox` | inline `actors` grant/`extends`/`never_both` names | same-file capability / actor (the `.mox` navigation index) |

### Rename ownership

Rename is enabled for **declaration-owned** names, and only in the document
that declares them: a `.ddd` `capability` clause naming a `.actor`
capability is *not* renamed from the `.ddd` document. The refusal is an
explicit `invalid_params` message naming the owning file
(`cannot rename capability 'App.RaiseRefund': it is declared in app.actor;
open that file to rename it`), so the client's status bar says
where to go. Declaration-owned renames edit the declaration plus every
reference **in that document** that the graph binds to it; `.ifml`'s
free-string names (modules, views) keep their cross-file rename, and `.mox`
keeps its navigation-index rename (same file).

### Hover and document symbols

Hover is the graph node's facts: a capability's class and its grant holders,
an actor's parents and grants, an event's payload fields and its
publishers/subscribers/emitters, an action's bound operation, a service
operation's carried capabilities/decisions/events, a deploy component's
design/flow bindings. Document symbols are the file's spanned definitions
(one symbol per declaration) for every artifact surface; `.mox` keeps its
richer outline (enum literals included) and `.ifml` its outline with module
`use` sites nested under their declarations.
