# The .mox language reference

`rexlang` compiles `.mox` modeling sources (inspired by [Eclipse Xcore]) into a
resolved **Core IR** artifact, from which backends generate Rust models, JSON
Schemas, and canonical instance JSON. This document is the normative language
reference; crate docs cover implementation.

[Eclipse Xcore]: https://eclipse.dev/Xtext/documentation/305_xbase.html

## Lexical rules

- **Comments**: `//` to end of line, `/* ... */` block comments. Preserved
  verbatim by `rexlang fmt`.
- **Doc comments**: `///` line comments and `/** ... */` block comments on
  their own lines directly above the `package` declaration, a declaration,
  feature, or enum literal become that element's **description**. Contiguous
  `///` lines join into one description; a blank line or an intervening
  non-doc comment detaches the run. Descriptions are carried in the IR and
  surface as `description` keywords in generated JSON Schema and as doc
  comments in generated code (the package description is carried in the IR
  only).
- **Strings**: double-quoted with `\"` and `\\` escapes.
- **Integers**: decimal, optional leading `-`.
- **Identifiers**: `[A-Za-z_][A-Za-z0-9_]*`. Any keyword can be escaped with a
  leading `^` (`^class` is an identifier).
- **Keywords**: `package annotation as class extends interface enum type wraps
  opaque contains refers container opposite op derived vocabulary from version
  key facet`. Contextual words usable as plain identifiers: `id readonly get
  set String int ... date`.

## Model structure

A file holds one package and any number of declarations:

```
model        := package_decl (annotation_decl | class_decl | interface_decl
              | enum_decl | type_decl | vocabulary_decl)*
package_decl := doc? "package" qualified_name
```

`doc` marks the optional doc-comment run described under Lexical rules; it
becomes the package's `description` in the Core IR.

A model may span multiple `.mox` files — one package per file, named by its
`package` declaration. Compiling several files produces **one** Core IR
artifact whose `packages` follow the input order (for `rexlang`, the sorted
path order of the expanded inputs). Declarations may reference types from
any package of the compilation:

- A bare single-segment name must be unique across **all** packages; if
  several declare it, the reference is an error (`ambiguous type \`X\`;
  qualify as \`p.X\``) whose help lists the matching packages.
- A qualified `<package>.<Name>` must match exactly one package.

Cross-package `refers`, `extends`, and attribute types (enums, datatypes,
vocabularies) are allowed; constraint family and enum-literal checks see
through the package boundary. `contains`/`container` targets and `opposite`
pairings must live in the declaring class's package: cross-package ownership
and cross-package opposites are errors.

### Classes and features

```
class_decl  := "class" name ("extends" type_ref ("," type_ref)*)? "{" feature* "}"
feature     := modifier* ( attribute | containment | reference | container
                         | op_decl | derived_decl )
modifier    := "id" | "readonly"
attribute   := type_ref multiplicity? name ("=" default)? constraint_block?
containment := "contains" type_ref multiplicity? name ("opposite" name)?
reference   := "refers" type_ref multiplicity? name ("opposite" name)?
container   := "container" type_ref name ("opposite" name)?
op_decl     := "op" type_ref name "(" params? ")" op_body?
derived_decl:= "derived" type_ref multiplicity? name op_body?
op_body     := "{" target_body+ "}" | "{" raw "}"
target_body := name "{" raw "}"
multiplicity:= "[" (int (".." (int | "*"))?)? "]"
constraint_block := "{" (constraint_keyword (string | int)?)* "}"
constraint_keyword := "pattern" | "minLength" | "maxLength" | "minimum"
                    | "maximum" | "unique"
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
- **`op`** bodies are per-target: `{ rust { ... } csharp { ... } }` blocks
  whose text is embedded **verbatim** by the backend that owns the target
  (`rust` in Tier 1). `expr` is the Tier-2 pseudo-target: it holds the
  neutral expression language, which every backend lowers (see
  `docs/EXPRESSIONS.md`). A `rust` and an `expr` body on the same operation
  conflict. Body-less operations stay abstract hooks, implemented by hand in
  target code.
- **`derived`** features are computed, not stored; a body must be the neutral
  expression language in a single `expr { ... }` block (Tier 2), e.g.
  `derived String citation { expr { title } }`. Backends lower it into a
  getter; a bare `{ ... }` body is rejected.
- **Defaults**: string/int/boolean literals or an enum literal name (for
  enum-typed attributes only).
- **Constraints** (attributes only) declare value bounds, e.g.
  `String sku { pattern "[A-Z]{3}-[0-9]{4}" minLength 3 maxLength 12 }` or
  `int stock { minimum 0 maximum 1000 }`. Each keyword may appear at most
  once. A constraint family is admitted by the most explicit type knowledge
  available: the string family (`pattern`/`minLength`/`maxLength`) requires
  a `string` primitive; the numeric family (`minimum`/`maximum`) requires a
  numeric primitive — the integer primitives and the IEEE `float`/`double`
  alike (bounds are declared as integers and hold for the floating values
  they bound; IEEE semantics apply, so the expression language's
  checked-overflow rule R1 is not a concern here). A datatype-typed
  attribute defaults to the string
  family — its platform type is opaque — and rejects numeric bounds. A
  vocabulary-typed attribute follows its `key` facet's declared primitive
  type (`String` admits the string family, a numeric key the numeric
  family); constraints are rejected outright when the key facet cannot be
  resolved. An enum-typed attribute carries a dual value space: all five
  keywords apply — the string family bounds literal names, the numeric
  family bounds literal values — and both may coexist; a numeric or length
  bound that admits zero literals is a compile error, while `pattern` is
  allowed but never statically validated (descriptive only). Class and
  interface types take no constraints. Length bounds must be non-negative
  and `min` ≤ `max` in both families. On a many-valued attribute the
  value bounds apply to the elements. The sixth keyword, **`unique`**, takes
  no value (`String[] tags { unique }`) and is the one collection-level
  constraint: it is admitted only on many-valued attributes — of any element
  type — and rejected on single-valued ones. It is schema-only: it surfaces
  as `uniqueItems: true` on the attribute's array schema, with element
  equality following JSON value equality, and implies no runtime
  validation. They surface as the JSON Schema
  keywords of the same names (`pattern`, `minLength`, `maxLength`,
  `minimum`, `maximum`, `uniqueItems`).

Multiplicity shorthand: `[]` = `[0..*]`; absent: attributes are `1..1`,
`contains`/`refers` are `0..*`, `container` and `derived` are `0..1`.

### Primitives and the `date` type

Attribute, parameter, and facet positions admit the built-in primitives
`String int long short float double boolean byte char` and, since issue #9,
the calendar **`date`**: a timezone-less, time-less civil day that
serializes as an ISO-8601 `YYYY-MM-DD` string. In expressions a date value
is written `date("2026-09-17")` and carries typed ordering plus the
calendar algebra (`plus_days`, `plus_months`, `diff_days`) — the normative
rules are R5–R8 in `docs/EXPRESSIONS.md`. A declared type shadows the
like-named built-in, so a pre-existing `type Date wraps opaque { ... }`
datatype keeps working; write lowercase `date` for the primitive. Dates
take **no declared defaults** (a `= "…"` default on a `date` attribute is a
compile error; generated required date fields anchor at the epoch
1970-01-01) and admit no constraint keywords. `datetime` does not exist in
this milestone, and the expression language has no clock: "as at date D"
queries take dates as parameters or attributes. The date facet type
(`facet date …`) is likewise not supported yet.

### Enums, datatypes, interfaces, vocabularies

```
enum_decl    := "enum" name "{" literal+ "}"
literal      := name ("as" string)? ("=" int)?
type_decl    := "type" name "wraps" ("opaque" | qualified_name)? datatype_block?
datatype_block := "{" ( name string | "format" string )* "}"
binding_block:= "{" (name string)* "}"
interface_decl := "interface" name "{" (name string)* "}"
vocabulary_decl := "vocabulary" name "from" string "{" ( version "string"
                 | key name | facet type_ref name )* "}"
```

- **Enums** carry integer values (required) and optional labels: `Mystery as
  "M" = 0`. Backends emit real enums; canonical JSON uses the literal name.
- **Datatypes** wrap platform types opaquely: `type Date wraps opaque { rust
  "chrono::NaiveDate" ... }`. Bindings are per-target hints, never generated
  dependencies. A datatype may also declare one **`format`** hint:
  `type Email wraps String { format "email" }`. `format` is a reserved key
  inside the datatype block: the unescaped key declares the format (never a
  target binding), at most once, and a binding target literally named
  `format` (only writable escaped, `^format "…"`) is an error. The hint
  surfaces as the JSON Schema `format` keyword on that datatype's schema in
  both profiles; it is a wire-level contract only — no runtime validation is
  implied.
- **Vocabularies** reference well-known external code sets. The declaration
  names the source, version, key facet, and typed facets. Snapshots are
  vendored to `vocab/<source>@<version>.json` and pinned by sha256 in
  `model.lock` — only `rexlang vocab fetch` touches the network; compilation
  is hermetic. Entries are embedded in the IR, and every backend materializes
  the vocabulary as a closed set of keys with facet accessors.

## Actor policy files (.actor)

Actor policy surfaces can also live in standalone `.actor` files, compiled
separately from the domain and emitted as their own ActorModel artifact for
the Cedar backend:

```
actor_file  := import_decl* actors_block+
import_decl := "import" string
actors_block:= "actors" name "{" (actor_decl | agent_decl)* capability*
                purpose_decl* grant* delegation* never_both* "}"
actor_decl  := "actor" name ("extends" name)?
agent_decl  := "agent" name ("extends" name)?
purpose_decl := "purpose" name
delegation  := "delegation" name "{" "from" name "to" name
                purpose_decl? delegation_entry* "}"
delegation_entry := ("permit" | "forbid") name ("when" expr)? obligation*
```

The `actors` block grammar is identical to the inline `actors` block of a
`.mox` model. Import paths are resolved **relative to the `.actor` file's
directory** and may name any `.mox` domain; multiple imports are allowed and
duplicates are collapsed. The compiled policy set is the **union** of the
actor file's blocks followed by every imported domain's inline blocks
(in that order), so capabilities typecheck against the imported domain's
classes (`permit EscalateTicket when (amount > 0)` resolves `amount` on
`Ticket`), and the separation-of-duty checks span files. `when` conditions
are restricted to a decidable subset at Cedar generation time — literals,
primitive attributes, and the operators — and anything without a Cedar
mapping (`let`, `if`, lambdas, collection algebra, `?.`, `?:`, `null`, list
literals, operation calls, and date attributes or `date("…")` literals,
since Cedar has no date type) is a generate-time error naming the
capability.

`agent` declares an autonomous LLM agent; the plain `actor` remains the
human principal. Both take the same optional `extends` clause, and a
declared kind is inherited: an actor that declares no kind takes the
nearest ancestor's declared kind, defaulting to human.

A `delegation` is a named transfer of authority from one actor to another:
`from` and `to` are required, in that order, and the body holds only effect
entries (`cedar { ... }` is rejected inside a delegation; an empty body is
legal). Blocks declare purposes with `purpose <name>` lines: several per
block are allowed, duplicates within a block are errors, and the same name
in different blocks is legal. A delegation may carry one optional `purpose`
line — after `to`, before the entries — naming a purpose declared in the
union of the file's blocks and every imported domain's inline blocks; an
undeclared name is a compile error naming the delegation and the purpose.
The driver enforces a containment invariant: every capability a
delegation permits must already be an effective permit of the target agent
— through its own grants, inherited ones included — so a delegation cannot
confer authority the target does not already hold, and `to` must name an
agent. `never_both` exclusivity is checked over grant and delegation
permits combined. In Cedar output delegations surface only as evidence
comments (`// delegation Name: from -> to (N capabilities)`, suffixed
` purpose: <P>` when a purpose is declared); like `never_both`, the driver
enforces them at compile time, and runtime enforcement — of permits and
purpose alike — belongs to the authorization gateway. `rexlang gen tools`
projects this authorization model into each agent's tool manifest — the
agent's effective grant permits plus the delegation bindings scoped to it;
human actors are omitted.

`rexlang fmt` formats `.actor` files with the same canonical layout rules
(imports first, one per line; then blocks).

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
literal name, datatypes their inner string, `date` attributes their
ISO-8601 `YYYY-MM-DD` string (loading strict-parses it); optional attributes
are omitted when unset; loading is strict (unknown features, serialized
containers, duplicate ids, unresolved refs are errors). `load(save(x)) == x`
is a tested property, not an aspiration.

## Tools

| Command | Purpose |
|---|---|
| `rexlang check <file>...` | validate; ariadne-rendered diagnostics grouped per file (`.mox` and `.actor`; each input may be a directory, scanned recursively for `*.mox`) |
| `rexlang ir <file>... -o <out>` | emit the Core IR artifact (`.actor`: the ActorModel artifact; several `.mox`: one multi-package model) |
| `rexlang gen rust <file>... -o <dir>` | arena-based Rust models |
| `rexlang gen json-schema <file>... --profile wire\|api -o <dir>` | JSON Schema |
| `rexlang gen cedar <file>... -o <dir>` | Cedar policies + schema (`.mox`: inline blocks; `.actor`: file + imported domains) |
| `rexlang gen tools <file>...` | per-agent tool manifest JSON on stdout (`.mox`: inline blocks; `.actor`: file + imported domains) |
| `rexlang vocab fetch <file>` | vendor + pin vocabulary snapshots |
| `rexlang fmt [--check] <files>\|-` | canonical formatting (comments kept) |
| `rexlang lsp` | language server (stdio) |
