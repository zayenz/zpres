---
title: "Typed Columns reference"
theme: "reference"
theme_dirs:
  - "../../themes"
aspect: "16:9"
footer: "Theme API v1 · typed Columns"
slide_numbers: true
---

# Evidence with explanation

:::: columns widths="3/2" gap="6" align="start"
Evidence column:
::: figure src="assets/science-orbit.svg" alt="Nested feasible regions and a search trajectory" caption="The evidence uses the normal Figure block." fit="contain" radius="10"
:::

Explanation column:
> [!NOTE] Propagation
> Nested blocks retain their root semantics.

1. Reading order
2. Source identity
3. Static output
::::

[^contract]: Columns contain typed child Content blocks, not a second Markdown dialect.

---

# Nested progression remains one Slide

:::: columns widths="1/1" gap="4" align="center"
Model column:
::: steps
1. Establish the domain.
2. Apply the propagator.
3. State the fixed point.
:::

Measurement column:
::: vega-lite data="data/columns-runtime.csv"
{
  "mark": "line",
  "encoding": {
    "x": { "field": "n" },
    "y": { "field": "ms" }
  }
}
:::
::::

The Step state changes without rebuilding the chart or the two-column relation.[^contract]
