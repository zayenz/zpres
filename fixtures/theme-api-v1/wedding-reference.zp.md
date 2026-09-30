---
title: "Wedding Theme API v1 reference"
author: "zpres"
theme: "wedding"
theme_dirs:
  - "../../themes"
theme_params:
  variant: "garden"
  ornament_style: "botanical"
  density: "normal"
  footer: "slide-number"
footer: "Wedding v1 · event planning"
slide_numbers: true
aspect: "16:9"
---

# Better tables, better conversations

::: variant section-title
:::

A seating plan should create room for conversation, not merely fit everyone in the room.

---

# Structure first, conversation next

::: theme ornament_style=wildflower density=spacious
:::

The model protects the practical requirements before it rewards shared interests.

- respect table capacities
- balance the room
- improve the least-served guest

::: notes
The ornament stays outside the content lanes, including at spacious density.
:::

---

# A short claim

::: slide
variant: claim
classes: [claim-short]
:::

[fit] Feasible is not the same as welcoming.

---

# The social objective needs context

::: slide
variant: claim
classes: [claim-medium]
:::

::: theme ornament_style=vine
:::

[fit] Shared interests matter only after capacity, slack, and balance have been settled.

---

# A longer claim steps down deliberately

::: slide
variant: claim
classes: [claim-long]
:::

::: theme ornament_style=bow
:::

[fit] A useful seating model preserves the structural priorities while making the conversational outcome inspectable for every guest and every table.

---

# Two views of a good table

::::: comparison
:::: primary label="Host's view"
The room works

- capacities respected
- slack controlled
- balance protected
::::
:::: supporting label="Guest's view"
The table works

- a supported topic
- no isolated guest
- transparent score
::::
:::::

---

# Build the objective in order

::: variant derivation
:::

::: theme ornament_style=minimal
:::

```text reveal="1-2|3-4|5"
minimise total slack
minimise maximum table slack
minimise total imbalance
maximise minimum guest interest
maximise total guest interest
```

---

# The feasible region contracts

::: variant figure
:::

::: figure src="assets/science-orbit.svg" alt="Nested feasible regions used to represent successive seating priorities" caption="Each objective keeps the best structural solution before conversation quality breaks the remaining ties." fit="contain" radius="2"
:::

---

# Search time grows with the guest list

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/columns-runtime.csv" },
  "mark": "line",
  "encoding": {
    "x": { "field": "n", "type": "quantitative" },
    "y": { "field": "ms", "type": "quantitative" }
  }
}
:::

--

## Detail: the evidence table

::: variant dense
:::

| Guests | Tables | Minimum interest | Runtime |
| ---: | ---: | ---: | ---: |
| 48 | 6 | 5 | 1.8 s |
| 72 | 9 | 6 | 4.3 s |
| **96** | **12** | **7** | **9.7 s** |

$$
q(S) = \min_{g \in G} \max_{t \in T(S)} I(g,t)
$$

---

# Decorative atmosphere stays silent

::: background src="assets/science-orbit.svg" intent="decorative" dim="92" grayscale="100" saturate="35"
:::

::: theme density=compact footer=section-progress ornament_style=botanical
:::

The image adds atmosphere; the foreground carries the complete argument.

---

# Context can occupy its own lane

![bg left:36% contain intent="contextual" alt="Nested regions suggesting progressively stronger seating requirements"](assets/science-orbit.svg)

::: theme ornament_style=wildflower accent="#6f826e"
:::

The setting is meaningful, but it is not evidence for the claim.

---

::: slide
variant: claim
theme:
  ornament_style: bow
  background: "#171a1c"
  surface: "#24282a"
  text: "#fffaf3"
  muted: "#c8c0b7"
  accent: "#dda8b4"
  accent_alt: "#a9c5b0"
  rule: "#535a5d"
background:
  src: assets/science-orbit.svg
  intent: evidence
  alt: "A small dark feasible region nested inside a larger pale region"
  description: "The inner region occupies about one quarter of the baseline region, representing how successive priorities remove most candidate seatings."
  split: right:40%
  dim: 82
:::

# Evidence remains visible and readable

[fit] Successive priorities remove most candidate seatings before social quality breaks the final ties.
