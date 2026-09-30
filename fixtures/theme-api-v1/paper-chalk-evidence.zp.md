---
title: "Paper Chalk evidence-shaped variants"
author: "zpres"
theme: "paper-chalk"
theme_dirs:
  - "../../themes"
theme_params:
  mode: "light"
  density: "normal"
  footer: "slide-number"
footer: "Paper Chalk · evidence notebook"
slide_numbers: true
aspect: "16:9"
---

# Search is a sequence of consequences

::: slide
variant: section-title
classes: [paper-section-index]
:::

[fit] Follow the evidence

One fixed constraint model. Three ways to see where propagation stops and search begins.

::: notes
Use the contact sheet to check the alternation from orientation to claim, visual evidence, worked consequence, and closing synthesis.
:::

---

# Propagation earns attention by deleting futures

::: slide
variant: claim
classes: [paper-claim-short, annotation-result-bracket]
:::

[fit] Pruning is the result.

Node counts matter only after the instance, propagators, and branching policy share a controlled baseline.

---

# One retained branch explains the reduction

::: slide
variant: figure
classes: [paper-evidence-canvas, annotation-margin-observation]
:::

::: figure src="assets/paper-chalk-search-tree.svg" alt="A constraint-programming search tree in which propagation prunes most alternatives and leaves one retained branch from 63 candidates to a single solution" caption="The retained path stays visible while crossed branches record alternatives removed by propagation." fit="contain" radius="0"
:::

> Margin observation · The crossed branches are eliminated states, not unexplored decoration.

---

# Search turns superlinear before the largest instance

::: slide
variant: figure
classes: [paper-evidence-canvas, annotation-focal-circle]
:::

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/paper-chalk-search-nodes.csv" },
  "mark": { "type": "line", "point": true },
  "encoding": {
    "x": { "field": "variables", "type": "quantitative", "title": "Decision variables" },
    "y": { "field": "nodes", "type": "quantitative", "title": "Search nodes" },
    "color": { "field": "protocol", "type": "nominal" }
  }
}
:::

> Evidence note · 82 → 31,200 search nodes while variables move 4 → 16.

---

# Branching changes the same instance by 75%

::: slide
variant: comparison
classes: [paper-comparison-baseline, annotation-delta-bracket]
:::

::::: comparison
:::: primary label="Chronological"
::: fit
12,480
:::

search nodes

Fixed instance · domain propagation
::::
:::: supporting label="Dom/wdeg"
::: fit
3,110
:::

search nodes

Same instance · same propagation
::::
:::::

> Consequence · The shared baseline isolates a 75% reduction from branching alone.

---

# Each stage preserves the controlled baseline

::: slide
variant: derivation
classes: [paper-derivation-path, annotation-checkpoint]
:::

::::: derivation
:::: context label="Invariant"
The instance, propagators, and stopping condition stay fixed.
::::
:::: stage label="Encode"
Post the same finite-domain model and initial constraints.
::::
:::: stage label="Propagate"
Reach a fixed point before selecting the next variable.
::::
:::: stage label="Consequence"
Only the branching policy explains the remaining node-count delta.
::::
:::::

--

## Detail: the certificate ledger

::: slide
variant: dense
classes: [paper-detail-ledger]
:::

| Stage | Remaining states | Certificate |
| --- | ---: | --- |
| Encoded | 63 | model hash |
| Propagated | 11 | fixed-point trace |
| **Solved** | **1** | **checked assignment** |

> Qualification · Counts use the same instance and propagation level; only branching changes.

--

## Detail: the propagation trace

::: slide
variant: dense
classes: [paper-detail-trace]
:::

```text reveal="2|3"
post(all_different(rows))
propagate(to_fixed_point)
branch(dom_over_wdeg)
```

---

# Keep the consequence visible during questions

::: slide
variant: claim
classes: [paper-claim-short, annotation-result-bracket]
:::

[fit] Compare on shared ground.

Control the baseline, annotate the delta, and keep the certificate close to the evidence.
