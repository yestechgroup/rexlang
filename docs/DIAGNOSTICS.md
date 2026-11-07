# Diagnostic codes

Every diagnostic a rexlang surface emits carries a stable string code. The codes live in one place — the `rex-diagnostics` crate — and this page is generated from that registry: a new code is a table row in `rex-diagnostics`, never a local constant. Two schemes are registered side by side: the numeric bands (`E####` errors, `W####` warnings, `X####` cross-surface integrity) are surface-local, human-facing codes the CLI prints, while the driver's camelCase names are payload-carrying LSP diagnostic codes (`rex_driver::DiagnosticCode`); both are first-class registry entries, cross-referenced by band. To allocate a code, claim (or find) your band in the ledger and add the constant plus its entry to the band module. Run `rexlang explain <code>` for one entry, or `rexlang explain --list` for everything.

## Allocation ledger

| Band | Range | Owner | Severity | Next free |
|------|-------|-------|----------|-----------|
| `ifml.resolver` | E0001–E0099 | rex-ifml resolver | error | E0010 |
| `ifml.check` | E0100–E0149 | rex-ifml typed checker | error | E0110 |
| `ifml.lints` | W0150–W0199 | rex-ifml lints | warning | W0155 |
| `driver.payload` | camelCase names | rex-driver DiagnosticCode | error | n/a |
| `driver.integrity` | X0001–X0099 | rex-driver integrity pass | error | X0009 |

## Band `ifml.resolver` (E0001–E0099)

| Code | Class | Title | Meaning | Fix |
|------|-------|-------|---------|-----|
| `E0001` | error | Import not provided | An `.ifml` import has no content in the `IfmlImports` bundle (`imported ifml '<path>' was not provided`). | Provide the import's source in the `IfmlImports` bundle, keyed `(importing path, import path as written)`. |
| `E0002` | error | Parse error | A file (main or imported) failed to parse; the message carries the parse error. | Fix the reported syntax error; `rexlang fmt` restores canonical layout. |
| `E0003` | error | Circular import | A circular `.ifml` import (`circular ifml import involving '<path>'`). | Break the cycle — move the shared module into its own file and import it from both sides. |
| `E0004` | error | Ambiguous module | Two distinct imported files declare the same module name (`ambiguous module '<name>' ...`). | Rename one of the conflicting modules; main-file declarations shadow imports silently. |
| `E0005` | error | Duplicate module | One file declares the same module name twice (`duplicate module '<name>'`). | Remove or rename the second declaration — the first one wins. |
| `E0006` | error | Unknown module | A `use` targets no declared module (`unknown module '<name>'`). | Declare the module locally, or import the file that declares it. |
| `E0007` | error | Unknown input | An override matches neither an input parameter nor a module property (`unknown input '<p>' for module '<m>'`). | Spell the input exactly as the module's `input { }` block declares it. |
| `E0008` | error | Missing input | An input without a default receives no override (`module '<m>' requires input '<p>'`). | Pass the required input at the `use` site, or give it a default in the module. |
| `E0009` | error | Input type mismatch | A literal override's type contradicts the input's declared type (`input '<p>' of module '<m>' expects <ty>`); structural check only — string literals suit `String`/`Uuid`/custom-identifier types, numbers suit `Int`/`Float`, booleans suit `Boolean`. | Use a literal of the declared type, or a non-literal expression (typed by the checker). |

## Band `ifml.check` (E0100–E0149)

| Code | Class | Title | Meaning | Fix |
|------|-------|-------|---------|-----|
| `E0100` | error | Unknown data entity | A component's `data:` entity does not resolve to exactly one class in the domain model (`unknown data entity 'X'`, or `ambiguous data entity 'X'` when several packages declare the name). | Make the name unique in the domain union, or qualify it so exactly one `(package, class)` matches. |
| `E0101` | error | Type mismatch | An expression contradicts the type its site expects (`expected <ty>, found <ty> in '<expr>'`), or another rex-expr semantic rule fires (R1/R2/R4/R9/...), carried through verbatim. | Align the expression with the site's expected type — docs/EXPRESSIONS.md rules R1–R4 pin the arithmetic, equality, and division behavior. |
| `E0102` | error | Unknown feature | Unknown feature or operation on a navigation path (rex-expr's `unknown feature ...` / `unknown operation ...`), or an unknown or ambiguous bare package-function call, prefixed with site context. | Check the spelling against the entity's features or the package function table — the did-you-mean hint names the closest declared one. |
| `E0103` | error | Unknown variable | A name that is not a view/container/module parameter, not a `data:` feature, and not an event parameter (`unknown name 'x'`), prefixed with site context. | Declare it as a parameter, read a feature of the component's entity, or fix the spelling. |
| `E0104` | error | Unknown action reference | An `action("X")` invocation whose name resolves to no `action "X" { target: ...; }` declaration (`action 'X' is not declared`). | add action "X" { target: <Entity>; } |
| `E0105` | error | Unknown navigate target | A `navigate("X")` target that is neither a declared view nor a declared container (`navigate target 'X' is not a declared view or container`). | Declare the view or container, or spell the target as declared (wizard steps count as containers). |
| `E0106` | error | Unknown refresh target | A `refresh("X")` target that is not a declared component (`refresh target 'X' is not a declared component`). | Spell the target exactly as the component is declared. |
| `E0107` | error | Unknown capability | A `requires:` capability name no provided actor model declares (`capability 'X' is not declared`). | add "capability X on <Class>;" to an actors block |
| `E0108` | error | Unknown role | A `roles:` name no provided actor model declares as an actor (`role 'X' is not declared`). | add "actor X;" to an actors block |
| `E0109` | error | Unknown action target | An action declaration's `target:` entity does not resolve to exactly one class in the domain model (`unknown target entity 'X'`, or `ambiguous target entity 'X'` when several packages declare the name) — the `data:` discipline applied to the lowering's payload. | Point `target:` at a domain class that resolves to exactly one `(package, class)`. |

## Band `ifml.lints` (W0150–W0199)

| Code | Class | Title | Meaning | Fix |
|------|-------|-------|---------|-----|
| `W0150` | warning | Dead action | A main-file `action "X" { ... }` declaration no `action("X")` invocation names (`action 'X' is declared but never invoked`). References count anywhere in the compilation — an uninstantiated module's invocation is still live intent — and only the main file's declarations are judged, so pattern libraries never warn. | Invoke the action from an event handler, or delete the declaration. |
| `W0151` | warning | Unreachable view | A main-file view that is neither the first view (the entry view by position), nor `landmark: true`, nor reached by any `navigate` target anywhere in the compilation, and whose body carries no content — no components, containers, event handlers, or module uses (`view 'X' is never navigated to ...`). A content view is intentionally routed (the manifest exports every view as a route); the empty shell no navigation reaches is the dead-UI class. | Navigate to it from another view, mark it `landmark: true`, give it content, or delete it. |
| `W0152` | warning | Shadowed module | A main-file module declaration whose name equals an imported file's module declaration (`module 'X' shadows an imported module of the same name (from '<path>')`). Resolution deliberately lets the local declaration win silently; the warning documents the shadowing at the winning declaration's name. | Rename the local module (and its `use` sites), or drop it if the imported one is wanted. |
| `W0153` | warning | Component without data | A main-file component with no `data:` binding that also declares no event handlers, carries no chart (inline spec, chart reference, or `type: chart`), and spells no explicit `Entity.field` table column (`<context> component 'X' binds no entity data`) — a fully static placeholder. Interactive components wire their data flow through handlers; charts aggregate by construction; explicit-column tables use the generation-time `Class.feature` spelling — all exempt. | Bind `data: <Entity>;`, or leave as-is when the component is deliberately static. |
| `W0154` | warning | View without domain binding | A main-file view with no domain tie at all: no `data:` binding on any component (recursively through containers), no explicit `Entity.field` column references, no declared `params`, no module composition, no event handlers anywhere in its body, and no `roles:`/`requires:` policy gate (`view 'X' has no domain binding ...`). Views that bind nothing are placeholders — often leftovers from a deleted flow. | Bind `data:` on a component, declare `params`, compose a module, add the interactions — or delete the view. |

## Band `driver.payload` (camelCase names)

| Code | Class | Title | Meaning | Fix |
|------|-------|-------|---------|-----|
| `attributeWithClassType` | error | Attribute with class type | `feature '<feature>' has class type '<class>'` — the author almost certainly meant `contains` or `refers`; the payload drives a two-way quick fix. | Replace the class type with `contains <Class>` (many) or `refers <Class>` (one) — the LSP offers both. |
| `unknownCapability` | error | Unknown capability | `unknown capability '<capability>' on …` — the name carries no resolution to a declared actor capability (rexlang#128's J3, resolved at compile time); a did-you-mean suggestion is attached when one is close. | Declare the capability (`capability X on <Class>;` in an actors block), or rename the reference to the suggested declared name. |
| `grantedCapabilityUncarried` | error | Granted capability uncarried | `capability '<capability>' is granted but no operation or search carries it` — the granted capability guards a transition that cannot happen (rexlang#128's J4, resolved at compile time); the fix lives in another file, so there is no local quick fix. | Carry the capability — add it to an operation or search across the designs — or drop the grant entry. |

## Band `driver.integrity` (X0001–X0099)

| Code | Class | Title | Meaning | Fix |
|------|-------|-------|---------|-----|
| `X0001` | warning | Unknown operation capability | `unknown capability '<capability>' on operation '<op>' of service '<service>'` (or `on search '<search>'`) — the capability clause names no capability declared in any provided actor block. Since rexlang#128 the compile-time join is the language error (code `unknownCapability`); this union-pass finding is the lint channel for hosts that gather artifacts without compiling. | Declare the capability (`capability X on <Class>;` in an actors block), or rename the reference to a declared capability. |
| `X0002` | warning | Granted capability uncarried | `capability '<capability>' is granted but no operation or search carries it` — the granted (or delegated) capability guards a transition that cannot happen. Since rexlang#128 the compile-time join is the language error (code `grantedCapabilityUncarried`); this union-pass finding is the lint channel. | Carry the capability — add it to an operation or search across the designs — or drop the grant entry. |
| `X0003` | error | Unknown emitted event | `operation '<op>' of service '<service>' emits unknown event '<event>'` — the `emits` clause is loosely coupled by design (the design compile has no event bucket), so the name resolves only across the artifact union. | Declare the event in an `.evt` contract, or fix the spelling. |
| `X0004` | error | Orphan declared event | `event '<event>' is declared but no operation emits it` — nothing produces the declared event, so it is dead weight. Channel `publishes` entries and subscription `events [...]` lists are consumption-side wiring and never satisfy production. | Emit the event from a service operation's `emits` clause, or drop the declaration. |
| `X0005` | error | Unbound action | `action '<action>' does not bind to any service operation` — binding is ASCII-case-insensitive name equality (the PascalCase action ↔ camelCase operation mirror), and no operation carries the name. An unbound action is a button whose backend does not exist. | Name the action after the operation it drives (or the reverse), or add the operation to a design. |
| `X0006` | error | Ambiguous action binding | `action '<action>' binds ambiguously to '<service.op>' and '<service.op>'` — several operations across the designs share the action's case-insensitive mirror name, so the binding names no unique backend (listing every carrier). | Rename the colliding operations so each action's case-insensitive mirror names exactly one service operation. |
| `X0007` | error | Unknown subscription consumer | `subscription '<subscription>' consumer '<consumer>' does not resolve to a declared component` — a dead inbox: the events are published, but no deploy component receives them. | Declare the component in a deploy model (any application), or fix the consumer's spelling. |
| `X0008` | error | Unknown flow actor reference | `unknown capability or actor '<name>' in requires of view/command '<name>'`, or `unknown actor '<name>' in roles of view '<name>'` — the flow clause names neither a capability nor an actor the actor layer declares (`roles:` is strictly actors). | Declare the capability or actor in an actors block, or fix the spelling. |
