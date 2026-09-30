---
title: "Wedding ornament and evidence grammar"
author: "zpres"
theme: "wedding"
theme_dirs:
  - "../../themes"
theme_params:
  variant: "garden"
  ornament_style: "botanical"
  density: "normal"
  footer: "section-progress"
footer: "Wedding · semantic ornament grammar"
slide_numbers: true
aspect: "16:9"
---

# Shared ground deserves an occasion

::: slide
variant: section-title
classes: [ceremonial, ornament-frame]
theme:
  ornament_style: botanical
:::

[fit] Workshop Planning

A formal opening uses one botanical frame, then yields to the evidence.

::: notes
The specimen reserves expressive ornament for opening, result, and conclusion.
:::

---

# Search cost rises with the guest list

::: slide
variant: figure
classes: [ornament-point]
theme:
  ornament_style: vine
:::

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/wedding-search-runtime.csv" },
  "mark": { "type": "line", "point": true },
  "encoding": {
    "x": { "field": "guests", "type": "quantitative" },
    "y": { "field": "seconds", "type": "quantitative" }
  }
}
:::

---

# Two promises belong to one seating plan

::: slide
variant: comparison
classes: [ornament-join]
theme:
  ornament_style: bow
:::

::::: comparison
:::: primary label="Structural promise"
The room works

- capacity holds
- slack stays controlled
::::
:::: supporting label="Social promise"
The table works

- shared support is visible
- the weakest guest is protected
::::
:::::

---

# The definition becomes executable evidence

::: slide
variant: derivation
classes: [ornament-boundary]
theme:
  ornament_style: minimal
:::

::::: derivation
:::: context label="Invariant"
The table and guest scores stay fixed.
::::
:::: stage label="Shared support"
Take the strict-majority floor b(t,k).
::::
:::: stage label="Guest fit"
Clip it by the guest score s(g,k).
::::
:::: stage label="Result"
Keep the best topic: q(g,t) = max over k of min(b(t,k), s(g,k)).
::::
:::::

--

## Detail: exact evidence keeps a minimal boundary

::: slide
variant: dense
classes: [ornament-boundary]
theme:
  ornament_style: minimal
:::

| Evidence | Reading |
| --- | ---: |
| strict-majority support | 7 |
| guest-specific fit | 5 |

```minizinc reveal="1"
q[g,t] = max(k)(min(shared[t,k], score[g,k]));
```

> [!NOTE] Boundary
> The minimal mark separates the title register without competing with exact evidence.

---

# The social terms change the seating

::: slide
variant: claim
classes: [result, claim-short, ornament-celebrate]
theme:
  ornament_style: wildflower
:::

[fit] 28 / 35

paired evaluable cases improve; seven tie and none worsen.

> Same assignments, constraints, instances, and five-level objective order.

---

# Protect shared ground without blocking search

::: slide
variant: claim
classes: [claim-medium, ornament-frame]
theme:
  ornament_style: botanical
:::

[fit] Structure first. Conversation next.

The conclusion reprises the opening frame once; ordinary evidence never uses it as wallpaper.
