---
title: "Theme API v1 teaching Deck"
theme: "reference"
theme_dirs:
  - ".."
aspect: "16:9"
footer: "reference · teaching Deck"
slide_numbers: true
build_lists: true
---

# Semantic regions stay put

::: variant section-title
:::

The renderer fixes the title, body, sources, and footer. Themes shape their appearance.[^guide]

Check meaning and clearance on full-size screen and print pages.[^review]

[^guide]: Theme authoring guide: `docs/theme-authoring.md`.
[^review]: Inspect original-size states and pages before contact sheets.

---

# Detail slides stay attached to one Section

The Main slide keeps the teaching path short.

Detail slides carry optional expansion without pretending to be separate Sections.

--

## Detail: the attachment is semantic, not ornamental

Review the Detail indicator, reading order, and body measure on both surfaces.

- The title still navigates the Section.
- The body still owns the explanation.
- The footer stays reserved instead of becoming spillover space.

---

# One Main slide should make one claim

::: variant claim
:::

The HTML presentation and PDF export are peer Output targets of one Deck.

Everything else on the page should help the audience interpret that single contract.

---

# Figure evidence should dominate the body

::: variant figure
:::

::: figure src="contract-field.svg" alt="A quiet contract diagram with title, body, sources, and footer lanes beside a highlighted evidence panel" caption="One authored figure can carry the argument while the caption and source stay secondary." fit="contain" radius="12"
:::

[^figure-source]: Source cue: local specimen SVG inside `themes/reference/`.

---

# Comparison keeps both sides parallel

::::: comparison
:::: primary label="Renderer-owned"
The renderer fixes geometry, order, and type floors.

- Semantic regions
- Safe content slot
- Static readiness
::::

:::: supporting label="Theme-owned"
The Theme supplies voice without moving authored meaning.

- Palette and typography
- Rules and accents
- Surface treatment
::::
:::::

---

# Derivation can preserve context while Steps advance

::::: derivation pdf="pages"
:::: context label="Stable context"
One Source file produces one Deck model before any Output target is rendered.
::::
:::: stage label="Live Step"
The HTML presentation may advance one meaningful stage at a time.
::::
:::: stage label="Static policy"
The PDF export may expand pedagogically important stages into separate pages.
::::
:::::

---

# Ordinary Steps collapse to one coherent print page

The screen route exposes live Step states.

::: steps
1. Name the contract in the title.
2. Keep the changed relation visible.
3. Preserve one coherent static page by default.
:::

[^step]: Inspect the active Step cue on screen and the final coherent state in print.

---

# Page-per-Step export is opt-in

Use a separate print page for each Step only when the intermediate states are themselves the lesson.

::: steps pdf="pages"
1. Keep the title stable.
2. Advance one relation at a time.
:::

[^step-pages]: Inspect the page sequence in `pages/` and confirm that the screen state order matches the exported print pages.

---

# Dense evidence belongs in Detail

The Main path states the contract before the exact ledger.

Detail slides may become technical, but they still need hierarchy and a readable floor.

--

## Detail: keep Technical evidence readable

::: variant dense
:::

::::: columns widths="5/4" gap="6" align="start"
:::: column name="Measured floors"
| Contract | Screen | Print |
| --- | ---: | ---: |
| Body minimum | 32 px | 32 px |
| Technical minimum | 24 px | 24 px |
| Micro floor | 18 px | 18 px |
::::

:::: column name="Portable assertion"
```rust
let targets = ["html", "pdf"];
assert_eq!(targets.len(), 2);
```

$$
\text{Theme voice}\quad \ne \quad\text{Deck meaning}
$$
::::
:::::
