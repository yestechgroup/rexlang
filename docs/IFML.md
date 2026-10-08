# IFML DSL

## Overview

A Domain-Specific Language that is semantically aligned with [OMG IFML 1.0](https://www.omg.org/spec/IFML/) but uses a modern, C-like expression-heavy syntax instead of XML/XMI. Parsed by **Pest** (Rust PEG parser) at build time into a versioned `IfmlModel` artifact, and by **Tree-sitter** at edit time for IDE features.

The IFML DSL lives alongside the `.mox` domain model as a *complementary* input: the domain model defines the data (entities, fields, constraints), the IFML DSL defines the interaction model (views, navigation, events, data binding). The parser crate is **`rex-ifml`**; the artifact types live in **`rex_ir::ifml`** (the rex-ir crate). Downstream consumers — today the [codegraph](https://github.com/magick93/codegraph) UI generators — ingest the artifact and emit running applications.

---

## Artifact wire format

`IfmlModel` is a standalone, versioned wire artifact following the rex-ir [wire format contract](BACKENDS.md) (see also the `rex_ir::ifml` module docs):

- Versioned independently of the domain-model marker via **`IFML_MODEL_FORMAT_VERSION`** (= 1), the `ActorModel` precedent. `IfmlModel::from_json` rejects absent/wrong versions with `IrError::UnsupportedFormatVersion`.
- All struct field names serialize as `camelCase` (`formatVersion`, `eventType`, `moduleUses`, ...).
- Enums with payloads are *adjacently* tagged: `{"type": "<variant>", "value": <payload>}` (tags camelCase: `"navigate"`, `"stringLit"`, `"fieldExpr"`, `"bar"`). Unit-only enums serialize as bare camelCase strings (`"eq"`, `"regexMatch"`).
- Unknown fields are ignored on deserialize; `PropertyAssignment::span` is parse-only and never serialized.

Minimal artifact: an empty model serializes as exactly `{"formatVersion":1}` — every struct `Vec` field is `#[serde(default, skip_serializing_if = "Vec::is_empty")]` (the `ActorModel`/`DddModel` discipline), so empty vecs are omitted on write and absent vecs read back as empty.

```rust
let model = rex_ifml::parse_ifml(source)?;
let json = model.to_json_pretty()?;      // versioned artifact
let model = IfmlModel::from_json(&json)?; // version-gated reload
```

---

## Philosophy

1. **Semantic identity with IFML** — every IFML concept has a direct DSL equivalent. No lossy mapping.
2. **C-like expression syntax** — conditions, filters, parameter expressions use familiar infix/prefix syntax.
3. **Composable blocks** — view containers nest, components nest, events nest inside components.
4. **Schema-aware** — entity/field references are validated against the loaded domain model.
5. **Text-first, diagram-aligned** — the text representation is the source of truth; diagrams are derived views.

---

## Syntax Examples

### Minimal CRUD

```ifml
domain "sales" {
    schema "sales";
}

view "CustomerList" {
    component "grid" {
        type: list;
        data: Customer;
        fields: [name, email, phone, status];

        on select(row) -> navigate("CustomerDetail", {
            customerId: row.id
        });
    }
}

view "CustomerDetail" {
    params { customerId: Uuid };

    component "info" {
        type: details;
        data: Customer;

        on edit -> navigate("CustomerEdit", {
            customerId: params.customerId
        });
    }
}

view "CustomerEdit" {
    params { customerId: Uuid };

    component "form" {
        type: form;
        data: Customer;
        mode: edit;

        on save(values) -> action("UpdateCustomer", {
            body: values;
            on success -> navigate("CustomerDetail", {
                customerId: params.customerId
            });
            on error -> stay;
        });

        on cancel -> navigate("CustomerDetail");
    }
}
```

### Multi-Step Wizard (conditional navigation)

```ifml
view "WizardPage" {
    xor: true;

    container "Step1" {
        default: true;

        component "personalInfo" {
            type: form;
            data: Customer;
            fields: [name, email, phone];

            on submit(values) -> navigate("Step2", {
                name: values.name,
                email: values.email
            });
        }
    }

    container "Step2" {
        component "addressInfo" {
            type: form;
            data: Address;

            on submit(values) -> navigate("Step3", {
                street: values.street,
                city: values.city
            });
        }
    }

    container "Step3" {
        component "review" {
            type: details;
            data: CustomerReview;

            on confirm -> action("CreateCustomer");

            on back -> navigate("Step2");
        }
    }
}
```

### Dashboard with Data Flows

```ifml
view "Dashboard" {
    on load -> refresh("recentOrders");

    component "recentOrders" {
        type: list;
        data: Order;
        fields: [id, customerName, total, date];
        filter: date == today();
    }

    component "topProducts" {
        type: list;
        data: Product;
        fields: [name, salesCount];
        filter: salesCount > 1000;
        sort: salesCount desc;
    }
}
```

### Search → Results → Detail

```ifml
view "SearchPage" {
    component "searchForm" {
        type: form;
        data: ProductSearchQuery;

        on submit(values) -> navigate("SearchPage", {
            query: values.term
        });
    }

    component "results" {
        type: list;
        data: Product;
        fields: [name, sku, price];
        filter: name ~= params.query;

        on select(row) -> navigate("ProductDetail", {
            productId: row.id
        });
    }
}
```

### Modal Dialog

```ifml
view "DeleteConfirm" {
    modal: true;

    component "confirmForm" {
        type: form;
        data: ConfirmDelete;

        on submit(values) -> action("DeleteProduct", {
            on success -> navigate("ProductList");
            on error -> stay;
        });

        on cancel -> navigate("ProductList");
    }
}
```

---

## Grammar

**The normative grammar is the Pest grammar file itself:**
`crates/rex-ifml/src/grammar/ifml.pest`. It is machine-checked —
`pest_derive` compiles it at build time and every parse runs through it — so
it is never restated in prose. What follows is a non-normative orientation,
naming the top-level rules per category; read the `.pest` file for the exact
productions.

- **Entry point** — `ifml_model`: imports and `domain_declaration`s first,
  then `view_declaration` / `action_declaration` / `module_declaration` /
  `actor_declaration` items.
- **Views and containers** — `view_declaration` / `container_declaration`
  share a body shape: optional `params_block` and `label_declaration`,
  property assignments, then nested containers, components, event handlers,
  condition statements, and module uses.
- **Components** — `component_declaration` with `column_decl`, `field_decl`
  (typed inputs), and `chart_decl` members alongside properties and events.
- **Events** — `event_handler`: `on <type> (params)? (requires ...)? (if
  expr)? -> <action> ;` with `navigate_action`, `refresh_action`,
  `action_invocation`, or `stay_statement`.
- **Actions and modules** — `action_declaration` (properties + events) and
  `module_declaration` (required `input`/`output` parameter blocks; the body
  may also contain `module_use_statement`s, composing other declared
  modules).
- **Expressions** — the C-like precedence chain `expression` → `logical_or`
  → `logical_and` → `comparison` → `addition` → `multiplication` → `unary`
  → `primary` (calls, field access, literals, groups).
- **Lexical rules** — `identifier`, `string` (with escapes), `number`,
  `boolean`, `type_ref`, `comment`, whitespace.

---

## Artifact Types (Rust)

Parsed Pest pairs are lowered directly into the `rex_ir::ifml` types (transitional architecture: there is no separate syntax tree yet; a future chumsky/spanned-AST re-platform will introduce a proper syntax → IR lowering). The root artifact:

```rust
pub struct IfmlModel {
    pub format_version: u32,   // always IFML_MODEL_FORMAT_VERSION (= 1)
    pub domains: Vec<DomainDeclaration>,
    pub views: Vec<ViewDeclaration>,
    pub actions: Vec<ActionDeclaration>,
    pub modules: Vec<ModuleDeclaration>,
    pub actors: Vec<ActorDeclaration>,
    pub imports: Vec<String>,
}
```

Key element types (all camelCase on the wire, enums `{"type": ..., "value": ...}`-tagged):

```rust
pub struct ViewDeclaration {
    pub name: String,
    pub label: Option<String>,
    pub is_landmark: bool,
    pub is_xor: bool,
    pub is_modal: bool,
    pub params: Vec<ParameterDecl>,
    pub properties: Vec<PropertyAssignment>,
    pub containers: Vec<ContainerDeclaration>,
    pub components: Vec<ComponentDeclaration>,
    pub events: Vec<EventHandler>,
    pub module_uses: Vec<ModuleUse>,
    pub roles: Vec<String>,      // extracted from the property bag
    pub requires: Vec<String>,   // capability requirements
    pub condition: Option<Expression>,
    pub position: Option<Position>,
}

pub enum EventAction {
    Navigate { target: String, binding: Option<ParameterBinding> },
    Refresh { target: String, binding: Option<ParameterBinding> },
    ActionInvocation { name: String, body: Option<ActionBody> },
    Stay,
}

pub enum Expression {
    Ident(String),
    StringLit(String),
    NumLit(f64),
    BoolLit(bool),
    FieldExpr { object: Box<Expression>, field: String },
    BinOp { left: Box<Expression>, op: BinOp, right: Box<Expression> },
    UnaryOp { op: UnaryOp, operand: Box<Expression> },
    Group(Box<Expression>),
    Call { name: String, args: Vec<Expression> },
}

pub enum BinOp { Eq, Ne, Lt, Le, Gt, Ge, RegexMatch, NegRegex, Add, Sub, Mul, Div, Mod, And, Or }
pub enum UnaryOp { Not, Neg }
```

`rex_ifml::render_expression(&expr)` renders an expression back to its deterministic DSL source form.

---

## Downstream Consumption

The `IfmlModel` artifact is the integration boundary. Consumers `parse_ifml_file` (or deserialize a serialized artifact with `IfmlModel::from_json`) and walk the model to build applications. codegraph, the primary consumer today, projects the artifact into its graph (ViewContainer/ViewComponent/Event/Action/ParameterDefinition/DataBinding/ModuleDefinition nodes, NavigationFlow/DataFlow/HasParameter/... edges) and generates behavior-wired UI plus Playwright tests from it.

### Expression Evaluation

C-like expressions appear in:

| Context | Example | Evaluation |
|---|---|---|
| Filter conditions | `filter: status == "active"` | Target-language predicate |
| Parameter bindings | `navigate("Detail", { id: row.id })` | Route parameter mapping |
| Conditional navigation | `if amount > 10000 -> navigate("Review")` | Branching logic in page controller |
| Default values | `sort: createdAt desc` | Query ordering |

Expression evaluation is **deferred to generation time** — the artifact stores the expression tree, and each consumer translates it to the target language (SQL, Svelte, Rust, etc.). `render_expression` provides a canonical source-form rendering.

---

## Crate Structure

```
crates/rex-ifml/
├── Cargo.toml
├── src/
│   ├── lib.rs            # Public API: parse_ifml, parse_ifml_file, parse_ifml_indexed, compile_ifml_str + re-exports of rex_ir::ifml
│   ├── parser.rs         # Pest parser wrapper + IfmlParseError
│   ├── index.rs          # Span side-table (IfmlIndex): module decls/uses, views, actions, actors — never serialized
│   ├── resolve.rs        # Cross-file module resolution: IfmlImports bundle, compile_ifml_str, IfmlDiagnostic (E0001–E0009)
│   └── grammar/
│       └── ifml.pest     # Pest grammar file
└── tests/
    ├── golden.rs         # Conformance golden (REX_UPDATE_FIXTURES=1 regenerates)
    └── resolve.rs        # Resolver conformance tests
```

### Module Resolution

`compile_ifml_str(path, text, &imports)` compiles a file plus its `.ifml`
imports in one call. Imports resolve through the `IfmlImports` bundle
(keyed `(importing path, import path)`, provided by the host — the resolver
never touches the filesystem); imports not ending in `.ifml` pass through
untouched. Local modules shadow imported ones; one name exported by two
distinct imported files is ambiguous. Every `use` in the main file — in
views, containers, and module bodies — is resolved and its overrides
validated against the target module's inputs and properties. Results live
in the `IfmlIndex` side-table (`ModuleUseSite::resolved_module`); the
returned `IfmlModel` is the main file's own parse, unchanged, so wire
artifacts stay byte-identical. See `crates/rex-ifml/src/resolve.rs` for the
diagnostic-code table.

Goldens live next to the other conformance fixtures: `tests/conformance/ifml/app.ifml` (canonical source) and `tests/conformance/ifml/app.ifml.json` (committed artifact).

---

## DSL + Domain Model Symbiosis

The IFML DSL is designed to feel like a *companion* to the `.mox` domain model:

```ifml
// The domain model defines "Customer" with features: name, email, phone, status
// The IFML DSL references these by name:

component "grid" {
    type: list;
    data: Customer;           // references the domain class
    fields: [name, email, phone];  // references features on Customer
    filter: status == "active";     // expression validated against field types
}
```

The `domain "sales" { schema "sales"; }` declaration at the top of an `.ifml` file establishes which domain provides the entity definitions.

---

## Grammar Sync Strategy (Pest ↔ Tree-sitter)

Both parse the same language but use different formalisms:

| Aspect | Pest | Tree-sitter |
|---|---|---|
| Algorithm | PEG (recursive descent) | GLR (Generalized LR) |
| Purpose | One-shot parsing for codegen | Incremental parsing for IDE |
| Location | Rust crate (`rex-ifml`) | Editor grammar (consumer-side) |
| Error handling | Error on first failure | Produces ERROR nodes, recovers |
| Performance | Very fast | Extremely fast (C library) |

**Sync approach**: Both grammars target the *same language spec*. In practice, the Pest grammar is the authoritative reference since it drives codegen; Tree-sitter is derived.

---

## Crate Dependency Graph

```
rex-ifml
  ├── pest (parser)
  ├── pest_derive (grammar compile)
  └── rex-ir (IfmlModel artifact types, versioned wire format)
```

`rex-ifml` is dependency-clean: no consumer-specific dependencies, so any backend can parse `.ifml` sources or consume `IfmlModel` artifacts directly.

---

## Testing Strategy

| Scope | Test | Method |
|---|---|---|
| Grammar | Valid `.ifml` files parse without errors | Pest `parse()` |
| Grammar | Invalid `.ifml` files produce expected errors | Pest error reporting |
| Artifact | Parse → `to_json_pretty` → golden byte-compare | `tests/conformance/ifml/` golden |
| Artifact | `to_json_pretty` → `from_json` round trip | Golden + unit tests |
| Wire format | `formatVersion` gate, unknown-field tolerance | Unit tests (`rex_ir::ifml`) |
| Expression parsing | C-like expressions produce correct trees | Unit test |
