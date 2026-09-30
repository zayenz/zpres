---
title: "Wedding signature compositions"
author: "zpres"
theme: "wedding"
theme_dirs:
  - "../../themes"
theme_params:
  variant: "garden"
  ornament_style: "botanical"
  density: "normal"
  footer: "section-progress"
footer: "Wedding · workshop planning"
slide_numbers: true
aspect: "16:9"
---

# Give every session a clear place

::: slide
variant: section-title
classes: [ceremonial]
:::

[fit] Workshop Planning

A lexicographic room-allocation example

::: notes
This compact synthetic Deck tests Wedding's signature compositions.
:::

---

# Search grows with the guest list

::: variant figure
:::

::: vega-lite
{
  "$schema": "https://vega.github.io/schema/vega-lite/v5.json",
  "data": { "url": "data/wedding-search-runtime.csv" },
  "mark": "line",
  "encoding": {
    "x": { "field": "guests", "type": "quantitative" },
    "y": { "field": "seconds", "type": "quantitative" }
  }
}
:::

---

# Structural promises come first

::::: comparison
:::: primary label="Feasible seating"
Keep the room workable

- respect every table capacity
- honour together/apart requests
- control spare seats and balance
::::
:::: supporting label="Welcoming seating"
Then protect shared ground

- score a topic by its strict-majority floor
- protect the weakest-served guest first
- improve total conversational fit second
::::
:::::

---

# The social terms still change the seating

::: slide
variant: claim
classes: [result, claim-short]
:::

[fit] 28 / 35

paired evaluable cases improve; seven tie and none worsen.

> Same assignments, constraints, instances, and five-level objective order.

---

# Build the shared score in three moves

::::: derivation
:::: context label="Invariant"
The seated guests at table t and their interest scores stay fixed.
::::
:::: stage label="Shared support"
For each topic, take the strict-majority order statistic b(t,k).
::::
:::: stage label="Guest fit"
Clip shared support by the focal guest's own score: min(b(t,k), s(g,k)).
::::
:::: stage label="Result"
Keep the guest's best supported topic: q(g,t) = max over k of min(b(t,k), s(g,k)).
::::
:::::

--

## Detail: the scalar keeps all five priorities

::: variant dense
:::

```minizinc reveal="3-5"
goal = total_slack * total_slack_weight
     + max_slack * max_slack_weight
     + total_imbalance * imbalance_weight
     + min_interest_penalty * min_interest_weight
     + total_interest_penalty;
```

Each weight exceeds the complete range of every lower-priority digit.
