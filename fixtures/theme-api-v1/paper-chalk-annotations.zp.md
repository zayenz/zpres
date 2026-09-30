---
title: "Paper Chalk semantic annotation specimen"
author: "zpres"
theme: "paper-chalk"
theme_dirs:
  - "../../themes"
theme_params:
  mode: "light"
  density: "normal"
  footer: "slide-number"
footer: "Paper Chalk · authored scientific judgment"
slide_numbers: true
aspect: "16:9"
---

# Read the notebook as evidence

::: slide
variant: section-title
classes: [paper-section-index]
:::

[fit] Semantic annotations

Circles, brackets, connectors, and observations appear only when the Source identifies their job.

---

# The result survives without rose ink

::: slide
variant: claim
classes: [paper-claim-short, annotation-result-bracket]
:::

[fit] Structure beats scale.

The underline repeats the claim; the bracket marks it as the result rather than adding a new fact.

---

# Runtime bends after the controlled threshold

::: slide
variant: figure
classes: [annotation-focal-circle]
:::

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/columns-runtime.csv" },
  "mark": { "type": "line", "point": true },
  "encoding": {
    "x": { "field": "n", "type": "quantitative" },
    "y": { "field": "ms", "type": "quantitative" }
  }
}
:::

---

# The feasible boundary is the evidence

::: slide
variant: figure
classes: [annotation-margin-observation]
:::

::: figure src="assets/science-orbit.svg" alt="Nested feasible regions with the outer boundary identified as the comparison threshold" caption="The outer boundary separates the retained feasible region from the rejected alternatives." fit="contain" radius="2"
:::

> Margin observation · The outer boundary, not the fill colour, carries the threshold.

---

# The final transformation is the checkpoint

::: slide
variant: derivation
classes: [annotation-checkpoint]
:::

::::: derivation
:::: context label="Invariant"
The family and solver settings stay fixed.
::::
:::: stage label="Normalize"
Divide every runtime by the baseline median.
::::
:::: stage label="Compare"
Align the paired families on the same scale.
::::
:::: stage label="Result"
The 16×16 family is the first discriminating checkpoint.
::::
:::::
