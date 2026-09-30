---
title: "Canonical zpres fixture"
author: "Zayenz"
theme: "science"
theme_dirs:
  - "../../themes"
footer: "Canonical fixture"
slide_numbers: true
theme_params:
  accent: "#0f766e"
  footer: "slide-number"
  mode: "light"
aspect: "16:9"
autoscale: true
transition: "fade"
---

image-corner-radius: 18

# Canonical zpres fixture

::: variant section-title
:::

This deck is deliberately small and broad. It exists to pressure the deck model, HTML presentation, PDF export, theme contract, and live authoring loop.

::: notes
The title slide should stay boring: if this fails, the pipeline is broken before the interesting content starts.
:::

---

# Claim: structure leaks through search

::: preset spotlight
:::

[fit] Search order is part of the model.

Constraint models do not only state what is feasible. They also shape what the solver sees first[^model-shape].

* typed content
* explicit intent

> The model's shape becomes part of the search story.

> [!NOTE] Theme contract
> Claims, lists, quotes, and callouts should all expose stable hooks.

::: steps
1. Start from the model.
2. Reveal the symmetry.
3. Show the branching consequence.
:::

[^model-shape]: Footnotes stay attached to the slide that cites them, which keeps source context visible in static exports.

---

# Theorem: propagation is local

::: variant derivation
:::

For a propagator \(p\), the strongest useful local statement is often:

$$
p(D_1 \cap D_2) \subseteq p(D_1) \cap p(D_2)
$$

The equation above is a typed display math block, and the paragraph contains typed inline math.

```rust
fn propagate(domain: Domain) -> Domain {
    domain.tighten()
}
```

---

# Table: search policy comparison

::: variant dense
:::

| Policy | Strength | Risk |
| --- | --- | --- |
| first-fail | fast pruning | unstable tie breaks |
| activity | adapts quickly | can chase noise |
| impact | explains choices | needs calibration |

---

# Chart: runtime by model size

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/runtime.csv" },
  "mark": "line",
  "encoding": {
    "x": { "field": "size", "type": "quantitative" },
    "y": { "field": "runtime_ms", "type": "quantitative" },
    "color": { "field": "model", "type": "nominal" }
  }
}
:::

---

# Figure: phase space sketch

::: variant figure
:::

::: background intent="contextual" src="assets/phase-space.svg" alt="Phase-space sketch background" position="center 42%" dim="86" grayscale="35" saturate="72"
:::

![Phase-space sketch](assets/phase-space.svg)
*A small synthetic phase-space sketch used by the canonical fixture.*

---

# Layout: two-column explanation

::: variant comparison
:::

::: columns widths="40/60" gap="4" align="start"
Left column:

- compact model
- visible invariant

Right column:

The solver-facing story should remain next to the model-facing story.
:::

---

# Main path with detail slides

This section checks that the main slide and detail slides stay attached in the model and linearize correctly for PDF export.

--

## Detail: explicit HTML-only content

Unsupported until issue 0004/0005 rendering: HTML-only content is explicit so output targets can decide how to handle it.

::: html
<aside class="fixture-callout">HTML-only fixture block</aside>
:::

--

## Detail: backup note

This detail slide exists to verify one main slide with two detail slides.
