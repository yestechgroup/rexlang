# Rexlang brand assets

Starter identity for the web presence (epic #227, task EP1-02). Treat this as a
**direction to validate and refine**, not a finished identity. The logo mark is
the most provisional asset here; the token rules below are the part worth
keeping.

Design direction, from the strategy (#226): *architectural precision* — Swiss
editorial design meets modern engineering software. Precise, minimal, confident,
technical.

## The mark

Two crossing strokes with four terminal nodes and an electric-lime core.

- The **X** is taken from the name — *rex*, and the `x` of cross-reference. It
  reads as two systems meeting at one shared semantic core, which is the product
  thesis: many surfaces, one model.
- The **square terminal nodes** are diagram nodes, not decoration; they are what
  makes the mark read as engineering rather than as a generic cross.
- The **lime core** is the model itself. It is the only colour in the mark, so
  the accent belongs to the idea rather than to the chrome.

The mark is built on a 64-unit grid with a 2-unit base: arms at 45°, stroke 6,
terminal nodes 8×8, core lozenge 24 wide.

## Files

| File | Use |
|---|---|
| `rexlang-mark.svg` | Primary mark, dark ink. Use at 32px and above on light surfaces |
| `rexlang-mark-inverse.svg` | Primary mark, light ink. Use at 32px and above on graphite (and on photography) |
| `rexlang-mark-compact.svg` | Compact mark, dark ink. **16–31px only** — the terminal nodes are removed at this size because they turn to noise |
| `rexlang-mark-compact-inverse.svg` | Compact mark, light ink, 16–31px on dark |
| `rexlang-tile.svg` | 512×512 graphite tile with rounded corners — GitHub avatar, social profile, OG base |
| `rexlang-lockup.svg` | Horizontal lockup, mark + wordmark, light surfaces |
| `rexlang-lockup-inverse.svg` | Horizontal lockup, dark surfaces |
| `rexlang-palette.svg` | Palette reference sheet |

Always pair the name with its descriptor on first use in a page or profile:
**Rexlang — Verifiable Model-Driven Engineering**. This is the search and brand
differentiator against the unrelated REXYGEN `rexlang` VS Code extension
(EP1-01); the strategy is to own the results with consistent usage, not to
rename.

## Provisional items

- **The wordmark is not outlined.** `rexlang-lockup*.svg` sets the wordmark in
  a `Geist`/`Inter` font stack, so it renders differently wherever those fonts
  are absent. Produce outlined paths before the lockup is used in a production
  header, a social card or anything that cannot ship a webfont.
- The mark has not been tested below 16px, in embroidery, or as a watermark. It
  does not need to be.
- No favicon set exists yet. Generate it from `rexlang-tile.svg` (16, 32, 180,
  192, 512 plus `favicon.ico`) and from the **compact** mark for the 16/32
  sizes, not the primary one.

## Palette

| Token | Hex | Role |
|---|---|---|
| Graphite | `#111318` | Primary background (dark surfaces) and body text (light surfaces) |
| Charcoal | `#1C2028` | Raised surface on graphite |
| Warm white | `#F5F4F0` | Reading surface, and text on dark |
| Silver | `#A7ADB7` | Borders, rules, diagram chrome — **decoration only on light surfaces** |
| Electric lime | `#C4F56A` | Accent: CTA fills, focus states, the model core, diagram highlights |
| Deep blue | `#5369D8` | Technical diagrams and model visualisation |

## Contrast rules (measured, not estimated)

Every value below is a real WCAG 2.2 relative-luminance ratio. These are
constraints on the design system, not suggestions — EP1-04 turns them into an
automated test.

| Pair | Ratio | Verdict |
|---|---|---|
| Graphite on warm white | 16.88 | AAA — body text |
| Warm white on charcoal | 14.83 | AAA — body text on dark |
| Lime on graphite | 14.70 | AAA — lime text **is** legal on dark |
| Graphite on lime | 14.70 | AAA — the CTA: lime fill, graphite label |
| Silver on graphite | 8.23 | AAA — muted text on dark |
| **Silver on warm white** | **2.05** | **FAIL** — never text on light. Borders and decoration only |
| **Lime on warm white** | **1.15** | **FAIL** — never text, never a thin icon on light |
| **Deep blue on warm white** | **4.36** | **FAIL** for body text (needs 4.5) — diagrams only |
| **Deep blue on graphite** | **3.88** | **FAIL** — diagrams only |

Three colours in the strategy's palette cannot carry text. The system needs
substitutes, and these are measured:

| Token | Hex | Ratio | Use |
|---|---|---|---|
| Muted text (light) | `#5A616B` | 5.68 on warm white | Secondary text on light surfaces — the legal replacement for silver |
| Link (light) | `#3F51B5` | 6.24 on warm white | Links and interactive text on light |
| Link (dark) | `#8B9AF5` | 7.11 on graphite | Links on dark surfaces |

Derive the rest of the ramp from these rather than inventing new hues; if a new
colour is needed, measure it before it enters `packages/tokens`.

## Type

- **UI**: Geist, falling back to Inter. Self-host, subset, `font-display: swap`.
- **Code and technical labels**: IBM Plex Mono.
- **Editorial display**: one distinctive face for large headlines, licence
  verified before adoption. The wordmark currently assumes the UI face.

Large headlines are the site's defining typographic characteristic. The
strategy's example is the standard to write to: *"Software is becoming easier to
generate. Making it understandable is the real engineering challenge."* — not
*"Welcome to Rexlang, the next-generation multi-target DSL platform."*
